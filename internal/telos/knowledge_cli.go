package telos

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/ctxpack"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/graph"
	"github.com/hugues31/telos-sdd/internal/index"
	"github.com/hugues31/telos-sdd/internal/kernel"
	"github.com/hugues31/telos-sdd/internal/view"
)

func osReadFile(root, rel string) ([]byte, error) {
	return os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
}

func contractReqRefs(data []byte) []string {
	return contract.ReqRefs(data)
}

// indexBlock is attached to every knowledge-layer result so agents can see
// what tree the answer describes.
func indexBlock(db *index.DB) map[string]any {
	root := db.Root()
	return map[string]any{"indexed_commit": root.IndexedCommit, "stale": root.Stale}
}

func openIndex(root string) (*index.DB, error) {
	return index.Open(root, index.AutoRebuild)
}

func runIndex(root string, args []string) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "index requires a verb: rebuild or status")
	}
	switch args[0] {
	case "rebuild":
		report, err := index.Rebuild(root)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("Index rebuilt from %s: %d node(s), %d edge(s).", report.IndexedCommit[:12], report.Nodes, report.Edges)
		return commandExecution{Command: "index.rebuild", Result: report, Human: human}, nil
	case "status":
		db, err := openIndex(root)
		if err != nil {
			return commandExecution{}, err
		}
		defer db.Close()
		stats, err := db.Stats()
		if err != nil {
			return commandExecution{}, err
		}
		result := map[string]any{"index": indexBlock(db), "nodes": stats.Nodes, "edges": stats.Edges, "critic_fp_rate": stats.CriticFPRate}
		return commandExecution{Command: "index.status", Result: result}, nil
	default:
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown index verb %q", args[0]))
	}
}

func runSearch(root string, args []string) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "search takes a query")
	}
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	hits, err := db.Search(strings.Join(args, " "), graph.SearchOpt{})
	if err != nil {
		return commandExecution{}, err
	}
	result := map[string]any{"index": indexBlock(db), "hits": hits}
	human := fmt.Sprintf("%d hit(s).", len(hits))
	if len(hits) > 0 {
		human = fmt.Sprintf("%d hit(s); best: %s — %s.", len(hits), hits[0].ID, hits[0].Title)
	}
	return commandExecution{Command: "search", Result: result, Human: human}, nil
}

func runShow(root string, args []string) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "show takes a node id (REQ-042, INT-001, CHG-104, sym:pkg.Name)")
	}
	id := graph.NodeID(strings.TrimSpace(args[0]))
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	node, ok, err := db.Node(id)
	if err != nil {
		return commandExecution{}, err
	}
	if !ok {
		return commandExecution{}, coded.New("TELOS_NODE_NOT_FOUND", "no node "+string(id)+"; use `telos search` to locate it")
	}
	sub, err := db.Neighbors(id, graph.TraverseOpt{MaxDepth: 1})
	if err != nil {
		return commandExecution{}, err
	}
	edges := map[string][]string{}
	for _, e := range sub.Edges {
		if e.From == id {
			edges[string(e.Kind)] = append(edges[string(e.Kind)], string(e.To))
		} else {
			edges[string(e.Kind)+"_of"] = append(edges[string(e.Kind)+"_of"], string(e.From))
		}
	}
	for k := range edges {
		sort.Strings(edges[k])
	}
	result := map[string]any{"index": indexBlock(db), "node": node, "edges": edges}
	if node.Kind == graph.KindRequirement {
		if rows, err := db.EvidenceFor(id); err == nil {
			result["evidence"] = rows
		}
	}
	return commandExecution{Command: "show", Result: result, Human: fmt.Sprintf("%s — %s (%s).", node.ID, node.Title, node.Kind)}, nil
}

func runRelated(root string, args []string, stderr io.Writer) (commandExecution, error) {
	f := flags("related", stderr)
	depth := f.Int("depth", 2, "traversal depth (1-4)")
	all := f.Bool("all-edges", false, "include code edges (imports, declared_in)")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	if f.NArg() == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "related takes a node id")
	}
	id := graph.NodeID(f.Arg(0))
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	opt := graph.TraverseOpt{MaxDepth: *depth}
	if *all {
		opt.EdgeKinds = append([]graph.EdgeKind{graph.EdgeImports, graph.EdgeDeclaredIn, graph.EdgeCalls},
			graph.EdgeMotivates, graph.EdgeRefines, graph.EdgeSupersedes, graph.EdgeDependsOn,
			graph.EdgeConstrains, graph.EdgeConflicts, graph.EdgeImplements, graph.EdgeVerifiedBy,
			graph.EdgeChangedBy, graph.EdgeIntroducedBy)
	}
	sub, err := db.Neighbors(id, opt)
	if err != nil {
		return commandExecution{}, err
	}
	type entry struct {
		ID    graph.NodeID `json:"id"`
		Kind  string       `json:"kind"`
		Title string       `json:"title"`
		Depth int          `json:"depth"`
		Via   graph.NodeID `json:"via,omitempty"`
	}
	var entries []entry
	for _, n := range sub.Nodes {
		if n.ID == id {
			continue
		}
		entries = append(entries, entry{ID: n.ID, Kind: string(n.Kind), Title: n.Title, Depth: sub.Depth[n.ID], Via: sub.Via[n.ID]})
	}
	sort.Slice(entries, func(i, j int) bool {
		if entries[i].Depth != entries[j].Depth {
			return entries[i].Depth < entries[j].Depth
		}
		return entries[i].ID < entries[j].ID
	})
	result := map[string]any{"index": indexBlock(db), "center": id, "nodes": entries, "truncated": sub.Truncated}
	return commandExecution{Command: "related", Result: result, Human: fmt.Sprintf("%d related node(s) within depth %d of %s.", len(entries), *depth, id)}, nil
}

func runImpact(root string, args []string) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "impact takes a node id (INT-*, REQ-*, CHG-*)")
	}
	id := graph.NodeID(args[0])
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	sub, err := db.Neighbors(id, graph.TraverseOpt{MaxDepth: 3, MaxNodes: 300})
	if err != nil {
		return commandExecution{}, err
	}
	grouped := map[string][]string{}
	for _, n := range sub.Nodes {
		if n.ID == id {
			continue
		}
		grouped[string(n.Kind)] = append(grouped[string(n.Kind)], string(n.ID))
	}
	for k := range grouped {
		sort.Strings(grouped[k])
	}
	result := map[string]any{"index": indexBlock(db), "center": id, "impact": grouped, "truncated": sub.Truncated}
	total := len(sub.Nodes) - 1
	return commandExecution{Command: "impact", Result: result, Human: fmt.Sprintf("%d node(s) in the impact neighborhood of %s.", total, id)}, nil
}

func runContext(repo *gitx.Repo, root string, args []string, stderr io.Writer) (commandExecution, error) {
	f := flags("context", stderr)
	budget := f.Int("budget", 16000, "token budget for the pack")
	focus := f.String("focus", "", "comma-separated node ids to seed retrieval")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	var seeds []graph.NodeID
	for _, id := range strings.Split(*focus, ",") {
		if id = strings.TrimSpace(id); id != "" {
			seeds = append(seeds, graph.NodeID(strings.ToUpper(id)))
		}
	}
	intentText := strings.Join(f.Args(), " ")

	// Inside a candidate, the change itself seeds retrieval: its intent text
	// and the requirement ids its delta declares.
	if doc, err := kernel.LoadChange(repo); err == nil {
		if data, rerr := osReadFile(repo.WorkDir, "changes/"+doc.ID+"/intent.md"); rerr == nil {
			intentText = strings.TrimSpace(intentText + " " + string(data))
		}
		if data, rerr := osReadFile(repo.WorkDir, "changes/"+doc.ID+"/contract.delta.md"); rerr == nil {
			for _, req := range contractReqRefs(data) {
				seeds = append(seeds, graph.NodeID(req))
			}
		}
	}
	if len(seeds) == 0 && intentText == "" {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "context needs --focus ids or free text (or a candidate with an intent)")
	}

	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	pack, err := ctxpack.Compile(db, seeds, intentText, *budget)
	if err != nil {
		return commandExecution{}, err
	}
	result := map[string]any{"index": indexBlock(db), "pack": pack}
	human := fmt.Sprintf("Context pack: %d/%d estimated tokens across %d section(s), %d omission(s).",
		pack.EstimatedTokens, pack.Budget, len(pack.Sections), len(pack.Omitted))
	return commandExecution{Command: "context", Result: result, Human: human}, nil
}

func runView(repo *gitx.Repo, root string, args []string, stdout io.Writer, stderr io.Writer) (commandExecution, error) {
	f := flags("view", stderr)
	port := f.Int("port", 7343, "listen port (0 for ephemeral)")
	open := f.Bool("open", false, "open in the default browser")
	static := f.String("static", "", "render every page into a directory and exit")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	opts := view.Options{
		Port:    *port,
		Querier: db,
		Status:  func() (kernel.ProjectStatus, error) { return kernel.Status(repo) },
	}
	if *static != "" {
		written, err := view.StaticExport(opts, *static)
		if err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "view", Result: map[string]any{"dir": *static, "pages": written},
			Human: fmt.Sprintf("Rendered %d page(s) into %s.", len(written), *static)}, nil
	}
	if *open {
		go func() {
			// Best effort once the server is listening.
			_ = view.OpenInBrowser(fmt.Sprintf("http://127.0.0.1:%d", *port))
		}()
	}
	url, err := view.Serve(opts, stdout)
	if err != nil {
		return commandExecution{}, err
	}
	return commandExecution{Command: "view", Result: map[string]any{"url": url}, Human: "View stopped."}, nil
}

func runExplain(root string, args []string) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "explain takes a symbol name (Login, Service.Login, sym:pkg.Service.Login)")
	}
	db, err := openIndex(root)
	if err != nil {
		return commandExecution{}, err
	}
	defer db.Close()
	matches, err := db.ResolveSymbol(args[0])
	if err != nil {
		return commandExecution{}, err
	}
	switch len(matches) {
	case 0:
		return commandExecution{}, coded.New("TELOS_NODE_NOT_FOUND", "no symbol matches "+strconv.Quote(args[0]))
	case 1:
	default:
		var candidates []string
		for _, m := range matches {
			candidates = append(candidates, string(m.ID))
		}
		return commandExecution{}, coded.WithPaths("TELOS_SYMBOL_AMBIGUOUS", "several symbols match; requalify", candidates)
	}
	symbol := matches[0]
	sub, err := db.Neighbors(symbol.ID, graph.TraverseOpt{MaxDepth: 2, EdgeKinds: []graph.EdgeKind{graph.EdgeImplements, graph.EdgeMotivates, graph.EdgeVerifiedBy}})
	if err != nil {
		return commandExecution{}, err
	}
	var reqs, intents []string
	for _, n := range sub.Nodes {
		switch n.Kind {
		case graph.KindRequirement:
			reqs = append(reqs, string(n.ID))
		case graph.KindIntent:
			intents = append(intents, string(n.ID))
		}
	}
	sort.Strings(reqs)
	sort.Strings(intents)
	result := map[string]any{
		"index": indexBlock(db), "symbol": symbol,
		"implements": reqs, "motivated_by": intents,
		"unmotivated": len(reqs) == 0,
	}
	human := fmt.Sprintf("%s implements %s.", symbol.ID, strings.Join(reqs, ", "))
	if len(reqs) == 0 {
		human = fmt.Sprintf("%s implements no certified requirement — that is a signal, not an error.", symbol.ID)
	}
	return commandExecution{Command: "explain", Result: result, Human: human}, nil
}
