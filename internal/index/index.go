// Package index implements graph.Querier over a derived SQLite database at
// .telos/cache/index.db. The database is disposable by definition: deleting
// it and rebuilding must restore the complete graph from certified artifacts
// alone — nothing writes to it except Rebuild, and no command persists
// derived knowledge anywhere else. It is root-bound: queries refuse to
// present a stale cache as current.
package index

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "modernc.org/sqlite"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/gosrc"
	"github.com/hugues31/telos-sdd/internal/graph"
	"github.com/hugues31/telos-sdd/internal/kernel"
	"github.com/hugues31/telos-sdd/internal/provenance"
)

const schemaVersion = 1

const dbRelPath = ".telos/cache/index.db"

const ddl = `
PRAGMA journal_mode=WAL;
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE nodes (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL DEFAULT '', attrs TEXT NOT NULL DEFAULT '{}',
  authority TEXT NOT NULL DEFAULT 'canonical', origin TEXT NOT NULL DEFAULT '',
  change_id TEXT NOT NULL DEFAULT ''
);
CREATE INDEX nodes_kind ON nodes(kind);
CREATE TABLE edges (
  src TEXT NOT NULL, dst TEXT NOT NULL, kind TEXT NOT NULL,
  authority TEXT NOT NULL DEFAULT 'canonical', origin TEXT NOT NULL DEFAULT '',
  change_id TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (src, kind, dst)
) WITHOUT ROWID;
CREATE INDEX edges_dst ON edges(dst, kind);
CREATE TABLE symbols (
  node_id TEXT PRIMARY KEY, package TEXT NOT NULL, file TEXT NOT NULL,
  name TEXT NOT NULL, sym_kind TEXT NOT NULL, exported INTEGER NOT NULL,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL
);
CREATE INDEX symbols_name ON symbols(name);
CREATE TABLE evidence (
  id TEXT NOT NULL, key TEXT NOT NULL, kind TEXT NOT NULL,
  result TEXT NOT NULL, reusable INTEGER NOT NULL, change_id TEXT NOT NULL,
  created_at TEXT NOT NULL, record TEXT NOT NULL,
  PRIMARY KEY (change_id, key)
) WITHOUT ROWID;
CREATE TABLE evidence_reqs (
  change_id TEXT NOT NULL, key TEXT NOT NULL, req_id TEXT NOT NULL,
  PRIMARY KEY (change_id, key, req_id)
) WITHOUT ROWID;
CREATE INDEX evidence_reqs_req ON evidence_reqs(req_id);
CREATE TABLE findings (
  id TEXT NOT NULL, change_id TEXT NOT NULL, critic TEXT NOT NULL,
  proposed_severity TEXT NOT NULL, confidence REAL NOT NULL DEFAULT 0,
  severity TEXT NOT NULL DEFAULT '', status TEXT NOT NULL,
  resolution TEXT NOT NULL DEFAULT '', subject TEXT NOT NULL DEFAULT '',
  rationale TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (change_id, id)
) WITHOUT ROWID;
CREATE VIRTUAL TABLE fts USING fts5(id UNINDEXED, kind UNINDEXED, title, body, tokenize='porter unicode61');
`

// OpenMode controls staleness behavior.
type OpenMode int

const (
	// RequireFresh refuses a stale index (TELOS_INDEX_STALE).
	RequireFresh OpenMode = iota
	// AutoRebuild rebuilds a stale or missing index, then serves.
	AutoRebuild
)

// DB implements graph.Querier.
type DB struct {
	sql  *sql.DB
	repo *gitx.Repo
	root graph.RootInfo
}

// BuildReport summarizes a rebuild.
type BuildReport struct {
	IndexedCommit string `json:"indexed_commit"`
	Nodes         int    `json:"nodes"`
	Edges         int    `json:"edges"`
	DurationMS    int64  `json:"duration_ms"`
}

func dbPath(repo *gitx.Repo) string {
	return filepath.Join(repo.WorkDir, filepath.FromSlash(dbRelPath))
}

// Open opens the root-bound index for the repository containing dir.
func Open(dir string, mode OpenMode) (*DB, error) {
	repo, err := gitx.Open(dir)
	if err != nil {
		return nil, coded.New("TELOS_GIT_REPOSITORY_REQUIRED", err.Error())
	}
	head, err := repo.Head()
	if err != nil {
		return nil, coded.New("TELOS_NOT_INITIALIZED", "no commits to index; run `telos init`")
	}
	stale := true
	if _, err := os.Stat(dbPath(repo)); err == nil {
		if conn, err := sql.Open("sqlite", dbPath(repo)); err == nil {
			var indexed, version string
			row := conn.QueryRow(`SELECT value FROM meta WHERE key='indexed_commit'`)
			if row.Scan(&indexed) == nil {
				_ = conn.QueryRow(`SELECT value FROM meta WHERE key='schema_version'`).Scan(&version)
				stale = indexed != string(head) || version != fmt.Sprint(schemaVersion)
			}
			conn.Close()
		}
	}
	if stale {
		switch mode {
		case AutoRebuild:
			if _, err := Rebuild(dir); err != nil {
				return nil, err
			}
		default:
			return nil, coded.New("TELOS_INDEX_STALE", "the derived index does not match the current tree; run `telos index rebuild`")
		}
	}
	conn, err := sql.Open("sqlite", dbPath(repo))
	if err != nil {
		return nil, err
	}
	db := &DB{sql: conn, repo: repo}
	var builtAt string
	_ = conn.QueryRow(`SELECT value FROM meta WHERE key='built_at'`).Scan(&builtAt)
	parsed, _ := time.Parse(time.RFC3339, builtAt)
	tree, _ := repo.TreeOf("HEAD")
	db.root = graph.RootInfo{IndexedCommit: string(head), TreeFingerprint: string(tree), SchemaVersion: schemaVersion, BuiltAt: parsed, Stale: false}
	return db, nil
}

// Close releases the underlying store.
func (d *DB) Close() error { return d.sql.Close() }

// Root reports the binding of the index to the tree it was built from.
func (d *DB) Root() graph.RootInfo { return d.root }

// Rebuild derives the complete graph from the certified artifacts at HEAD
// into a fresh database, atomically replacing the previous one.
func Rebuild(dir string) (BuildReport, error) {
	start := time.Now()
	var report BuildReport
	repo, err := gitx.Open(dir)
	if err != nil {
		return report, coded.New("TELOS_GIT_REPOSITORY_REQUIRED", err.Error())
	}
	head, err := repo.Head()
	if err != nil {
		return report, coded.New("TELOS_NOT_INITIALIZED", "no commits to index; run `telos init`")
	}
	report.IndexedCommit = string(head)

	target := dbPath(repo)
	if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
		return report, err
	}
	tmp := target + ".tmp"
	os.Remove(tmp)
	conn, err := sql.Open("sqlite", tmp)
	if err != nil {
		return report, err
	}
	defer os.Remove(tmp)
	if _, err := conn.Exec(ddl); err != nil {
		conn.Close()
		return report, err
	}
	tx, err := conn.Begin()
	if err != nil {
		conn.Close()
		return report, err
	}
	b := &builder{tx: tx}
	if err := b.build(repo); err != nil {
		tx.Rollback()
		conn.Close()
		return report, err
	}
	for k, v := range map[string]string{
		"schema_version": fmt.Sprint(schemaVersion),
		"indexed_commit": string(head),
		"built_at":       time.Now().UTC().Format(time.RFC3339),
	} {
		if _, err := tx.Exec(`INSERT INTO meta(key, value) VALUES(?, ?)`, k, v); err != nil {
			tx.Rollback()
			conn.Close()
			return report, err
		}
	}
	if err := tx.Commit(); err != nil {
		conn.Close()
		return report, err
	}
	report.Nodes, report.Edges = b.nodes, b.edges
	if err := conn.Close(); err != nil {
		return report, err
	}
	if err := os.Rename(tmp, target); err != nil {
		return report, err
	}
	report.DurationMS = time.Since(start).Milliseconds()
	return report, nil
}

type builder struct {
	tx           *sql.Tx
	nodes, edges int
}

func (b *builder) node(id, kind, title, body string, attrs map[string]string, authority, origin, changeID string) error {
	attrJSON, _ := json.Marshal(attrs)
	if attrs == nil {
		attrJSON = []byte("{}")
	}
	if _, err := b.tx.Exec(`INSERT OR REPLACE INTO nodes(id,kind,title,body,attrs,authority,origin,change_id) VALUES(?,?,?,?,?,?,?,?)`,
		id, kind, title, body, string(attrJSON), authority, origin, changeID); err != nil {
		return err
	}
	b.nodes++
	switch kind {
	case "intent", "requirement", "decision", "change", "finding":
		if _, err := b.tx.Exec(`INSERT INTO fts(id,kind,title,body) VALUES(?,?,?,?)`, id, kind, title, body); err != nil {
			return err
		}
	}
	return nil
}

func (b *builder) edge(src, kind, dst, authority, origin, changeID string) error {
	if _, err := b.tx.Exec(`INSERT OR REPLACE INTO edges(src,dst,kind,authority,origin,change_id) VALUES(?,?,?,?,?,?)`,
		src, dst, kind, authority, origin, changeID); err != nil {
		return err
	}
	b.edges++
	return nil
}

func (b *builder) build(repo *gitx.Repo) error {
	files, err := repo.LsTree("HEAD")
	if err != nil {
		return err
	}
	read := func(path string) []byte {
		oid, ok := files[path]
		if !ok {
			return nil
		}
		content, err := repo.CatBlob(oid)
		if err != nil {
			return nil
		}
		return content
	}

	// Contract: intents, requirements, decisions.
	specFiles := map[string][]byte{}
	goFiles := map[string][]byte{}
	for path := range files {
		if strings.HasPrefix(path, contract.Dir+"/") {
			specFiles[path] = read(path)
		}
		if strings.HasSuffix(path, ".go") || path == "go.mod" {
			goFiles[path] = read(path)
		}
	}
	parsed, _ := contract.Parse(specFiles)
	for id, intent := range parsed.Intents {
		if err := b.node(id, "intent", intent.Title, intent.Section, nil, "canonical", intent.File, ""); err != nil {
			return err
		}
	}
	for id, req := range parsed.Requirements {
		attrs := map[string]string{"class": string(req.Class)}
		if err := b.node(id, "requirement", req.Title, req.Section, attrs, "canonical", req.File, ""); err != nil {
			return err
		}
		for _, intent := range req.MotivatedBy {
			if err := b.edge(intent, "motivates", id, "canonical", req.File, ""); err != nil {
				return err
			}
		}
	}
	for id, dec := range parsed.Decisions {
		attrs := map[string]string{"status": dec.Status}
		if err := b.node(id, "decision", dec.Title, dec.Section, attrs, "canonical", dec.File, ""); err != nil {
			return err
		}
		if dec.SupersededBy != "" {
			if err := b.edge(dec.SupersededBy, "supersedes", id, "canonical", dec.File, ""); err != nil {
				return err
			}
		}
	}

	// Changes, their provenance, evidence, and findings.
	for path := range files {
		rest, ok := strings.CutPrefix(path, "changes/")
		if !ok || !strings.HasSuffix(path, "/change.json") {
			continue
		}
		id := strings.SplitN(rest, "/", 2)[0]
		var doc kernel.ChangeDoc
		if json.Unmarshal(read(path), &doc) != nil {
			continue
		}
		attrs := map[string]string{"status": doc.Status, "category": doc.Category}
		if err := b.node(id, "change", doc.Title, "", attrs, "canonical", path, id); err != nil {
			return err
		}

		var prov provenance.Doc
		if data := read("changes/" + id + "/provenance.json"); data != nil && json.Unmarshal(data, &prov) == nil {
			for _, rel := range prov.Relations {
				switch rel.Rel {
				case "changed_by":
					if err := b.edge(rel.Req, "changed_by", id, rel.Authority, rel.Origin, id); err != nil {
						return err
					}
				case "verified_by":
					testID := "test:" + rel.Path
					if err := b.node(testID, "test", rel.Path, "", nil, "canonical", id, id); err != nil {
						return err
					}
					if err := b.edge(rel.Req, "verified_by", testID, rel.Authority, rel.Origin, id); err != nil {
						return err
					}
				case "implemented_by":
					implID := "file:" + rel.Path
					if rel.Symbol != "" {
						implID = "sym:" + pkgOf(rel.Path) + "." + rel.Symbol
					}
					if err := b.edge(implID, "implements", rel.Req, rel.Authority, rel.Origin, id); err != nil {
						return err
					}
				}
			}
		}

		var findings []kernel.Finding
		if data := read("changes/" + id + "/findings.json"); data != nil && json.Unmarshal(data, &findings) == nil {
			for _, f := range findings {
				resolution := ""
				if f.Resolution != nil {
					resolution = f.Resolution.Kind
				}
				subject := ""
				if len(f.Target.Requirements) > 0 {
					subject = f.Target.Requirements[0]
				}
				if _, err := b.tx.Exec(`INSERT OR REPLACE INTO findings(id,change_id,critic,proposed_severity,confidence,severity,status,resolution,subject,rationale) VALUES(?,?,?,?,?,?,?,?,?,?)`,
					f.ID, id, f.Source.Name, f.ProposedSeverity, f.Confidence, f.Severity, f.Status, resolution, subject, f.Rationale); err != nil {
					return err
				}
				if err := b.node(id+"/"+f.ID, "finding", f.Rationale, "", map[string]string{"severity": f.Severity, "status": f.Status}, "canonical", id, id); err != nil {
					return err
				}
			}
		}
	}

	// Evidence records.
	for path := range files {
		if !strings.HasPrefix(path, "changes/") || !strings.Contains(path, "/evidence/EVD-") || !strings.HasSuffix(path, ".json") {
			continue
		}
		var record evidence.Record
		if json.Unmarshal(read(path), &record) != nil || record.Schema != 1 {
			continue
		}
		raw, _ := json.Marshal(record)
		if _, err := b.tx.Exec(`INSERT OR REPLACE INTO evidence(id,key,kind,result,reusable,change_id,created_at,record) VALUES(?,?,?,?,?,?,?,?)`,
			record.ID, record.Key(), record.Kind, record.Result.Status, boolInt(record.Reusable), record.Change, record.CreatedAt, string(raw)); err != nil {
			return err
		}
		for _, req := range record.Requirements {
			if _, err := b.tx.Exec(`INSERT OR REPLACE INTO evidence_reqs(change_id,key,req_id) VALUES(?,?,?)`, record.Change, record.Key(), req); err != nil {
				return err
			}
		}
	}

	// Code model.
	analysis := gosrc.Analyze(goFiles, gosrc.ModulePath(read("go.mod")))
	for _, pkg := range analysis.Packages {
		if err := b.node("pkg:"+pkg, "package", pkg, "", nil, "derived", "analysis:go", ""); err != nil {
			return err
		}
	}
	seenFiles := map[string]bool{}
	for _, sym := range analysis.Symbols {
		fileID := "file:" + sym.File
		if !seenFiles[sym.File] {
			seenFiles[sym.File] = true
			if err := b.node(fileID, "file", sym.File, "", nil, "derived", "analysis:go", ""); err != nil {
				return err
			}
		}
		attrs := map[string]string{"kind": sym.Kind, "package": sym.Package}
		if err := b.node(sym.ID(), "symbol", sym.Name, "", attrs, "derived", "analysis:go", ""); err != nil {
			return err
		}
		if _, err := b.tx.Exec(`INSERT OR REPLACE INTO symbols(node_id,package,file,name,sym_kind,exported,start_line,end_line) VALUES(?,?,?,?,?,?,?,?)`,
			sym.ID(), sym.Package, sym.File, sym.Name, sym.Kind, boolInt(sym.Exported), sym.StartLine, sym.EndLine); err != nil {
			return err
		}
		if err := b.edge(sym.ID(), "declared_in", fileID, "derived", "analysis:go", ""); err != nil {
			return err
		}
	}
	for _, imp := range analysis.Imports {
		if err := b.edge("pkg:"+imp.From, "imports", "pkg:"+imp.To, "derived", "analysis:go", ""); err != nil {
			return err
		}
	}
	return nil
}

func pkgOf(path string) string {
	if i := strings.LastIndexByte(path, '/'); i >= 0 {
		return path[:i]
	}
	return "."
}

func boolInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
