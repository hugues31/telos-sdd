package index

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/graph"
)

// fixture builds a repository holding every artifact kind the index derives
// from, committed as plain files — the index must reconstruct the graph from
// certified artifacts alone, so the fixture bypasses the kernel on purpose.
func fixture(t *testing.T) *gitx.Repo {
	t.Helper()
	dir := t.TempDir()
	for _, args := range [][]string{
		{"init", "--quiet", "-b", "main"},
		{"config", "user.email", "telos@test"},
		{"config", "user.name", "telos test"},
		{"config", "core.autocrlf", "false"},
		{"config", "gc.auto", "0"},
	} {
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	repo, err := gitx.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	write := func(rel, content string) {
		path := filepath.Join(dir, filepath.FromSlash(rel))
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	write("spec/PRODUCT.md", "# Product\n\n### INT-001 — Application greets reliably\n\nThe application greets.\n")
	write("spec/core.md", "# Core\n\n### REQ-001 — Emit the greeting\nClass: behavior\nMotivated by: INT-001\n\nThe greeting is emitted exactly once.\n\n```gherkin\nScenario: greeting\n  Given the app runs\n  Then the greeting is produced\n```\n")
	write("spec/DECISIONS.md", "### DEC-001 — Keep it simple\nStatus: accepted\n\n### DEC-002 — Old approach\nStatus: superseded by DEC-001\n")
	write("go.mod", "module example.com/toy\n\ngo 1.24\n")
	write("pkga/a.go", "package pkga\n\nfunc A() int { return 1 }\n")
	write("changes/CHG-001/change.json", `{"change":1,"id":"CHG-001","category":"behavior_change","title":"Greeting","base":"","target_branch":"main","branch":"telos/CHG-001","status":"promoted","approvals":[],"created_at":"2026-01-01T00:00:00Z"}`)
	write("changes/CHG-001/provenance.json", `{"provenance":1,"change":"CHG-001","relations":[
		{"req":"REQ-001","rel":"changed_by","authority":"canonical","origin":"CHG-001"},
		{"req":"REQ-001","rel":"verified_by","path":"tests/core_test.txt","evidence":"EVD-abc","authority":"canonical","origin":"CHG-001"},
		{"req":"REQ-001","rel":"implemented_by","symbol":"A","path":"pkga/a.go","authority":"derived","origin":"go_ast"}]}`)
	write("changes/CHG-001/findings.json", `[
		{"finding":1,"id":"FND-001","change":"CHG-001","source":{"kind":"critic","name":"verifier"},"target":{"requirements":["REQ-001"]},"proposed_severity":"blocking","confidence":0.8,"rationale":"empty greeting uncovered","status":"resolved","resolution":{"kind":"not_an_issue","by":"human"},"created_at":"2026-01-01T00:00:00Z"},
		{"finding":1,"id":"FND-002","change":"CHG-001","source":{"kind":"human","name":"human"},"target":{"requirements":["REQ-001"]},"proposed_severity":"minor","severity":"minor","rationale":"wording","status":"open","created_at":"2026-01-01T00:00:00Z"}]`)

	// One evidence record whose closure digest matches the committed tree, so
	// live freshness computes true.
	if _, err := repo.CommitAll("fixture without evidence"); err != nil {
		t.Fatal(err)
	}
	tree, err := repo.TreeOf("HEAD")
	if err != nil {
		t.Fatal(err)
	}
	dep, err := evidence.TreeClosure(repo, tree)
	if err != nil {
		t.Fatal(err)
	}
	record := evidence.Record{Schema: 1, Kind: evidence.KindRedGreen, Requirements: []string{"REQ-001"},
		Command: "probe", Cwd: ".", DependsOn: dep,
		Result: evidence.Result{Status: "pass"}, Reusable: true, Change: "CHG-001", CreatedAt: "2026-01-01T00:00:00Z"}
	record.ID = "EVD-" + record.Key()[:12]
	raw, _ := json.Marshal(record)
	write("changes/CHG-001/evidence/"+evidence.FileName(record.Key()), string(raw))
	if _, err := repo.CommitAll("fixture"); err != nil {
		t.Fatal(err)
	}
	return repo
}

func dump(t *testing.T, db *DB) string {
	t.Helper()
	nodes, err := db.Nodes(graph.NodeFilter{})
	if err != nil {
		t.Fatal(err)
	}
	out, _ := json.Marshal(nodes)
	return string(out)
}

func TestRebuildIsDeterministicAndDisposable(t *testing.T) {
	repo := fixture(t)
	if _, err := Rebuild(repo.WorkDir); err != nil {
		t.Fatal(err)
	}
	db, err := Open(repo.WorkDir, RequireFresh)
	if err != nil {
		t.Fatal(err)
	}
	first := dump(t, db)
	db.Close()

	// Disposability: delete and rebuild from certified artifacts alone.
	if err := os.Remove(filepath.Join(repo.WorkDir, filepath.FromSlash(dbRelPath))); err != nil {
		t.Fatal(err)
	}
	if _, err := Rebuild(repo.WorkDir); err != nil {
		t.Fatal(err)
	}
	db, err = Open(repo.WorkDir, RequireFresh)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if second := dump(t, db); second != first {
		t.Fatal("rebuild is not deterministic")
	}
}

func TestRootBinding(t *testing.T) {
	repo := fixture(t)
	if _, err := Rebuild(repo.WorkDir); err != nil {
		t.Fatal(err)
	}
	// Move HEAD: the index is stale.
	if err := os.WriteFile(filepath.Join(repo.WorkDir, "new.txt"), []byte("x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := repo.CommitAll("move"); err != nil {
		t.Fatal(err)
	}
	_, err := Open(repo.WorkDir, RequireFresh)
	if e, ok := coded.As(err); !ok || e.Code != "TELOS_INDEX_STALE" {
		t.Fatalf("stale open = %v", err)
	}
	db, err := Open(repo.WorkDir, AutoRebuild)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if db.Root().Stale {
		t.Fatal("AutoRebuild served a stale index")
	}
}

func TestQueries(t *testing.T) {
	repo := fixture(t)
	db, err := Open(repo.WorkDir, AutoRebuild)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	// Node + attrs.
	req, ok, err := db.Node("REQ-001")
	if err != nil || !ok || req.Kind != "requirement" || req.Attrs["class"] != "behavior" {
		t.Fatalf("REQ-001 = %+v, %v, %v", req, ok, err)
	}

	// Traversal reaches intent, change, test, and implementation.
	sub, err := db.Neighbors("REQ-001", graph.TraverseOpt{MaxDepth: 2})
	if err != nil {
		t.Fatal(err)
	}
	found := map[string]bool{}
	for _, n := range sub.Nodes {
		found[string(n.ID)] = true
	}
	for _, want := range []string{"INT-001", "CHG-001", "test:tests/core_test.txt", "sym:pkga.A"} {
		if !found[want] {
			t.Errorf("neighbors miss %s: %v", want, found)
		}
	}
	if _, err := db.Neighbors("REQ-404", graph.TraverseOpt{}); err == nil {
		t.Fatal("unknown node must error")
	}

	// Search finds the requirement by content.
	hits, err := db.Search("greeting emitted", graph.SearchOpt{})
	if err != nil || len(hits) == 0 || hits[0].ID != "REQ-001" {
		t.Fatalf("search = %+v, %v", hits, err)
	}

	// Symbol resolution by bare name.
	syms, err := db.ResolveSymbol("A")
	if err != nil || len(syms) != 1 || syms[0].ID != "sym:pkga.A" {
		t.Fatalf("resolve = %+v, %v", syms, err)
	}

	// Evidence with live freshness (the fixture digest matches HEAD... after
	// the evidence commit the tree changed, so freshness is recomputed and
	// reported honestly).
	rows, err := db.EvidenceFor("REQ-001")
	if err != nil || len(rows) != 1 || rows[0].Kind != evidence.KindRedGreen {
		t.Fatalf("evidence = %+v, %v", rows, err)
	}

	// Findings and the critic false-positive metric.
	open, err := db.Findings(graph.FindingFilter{Status: "open"})
	if err != nil || len(open) != 1 || open[0].ID != "FND-002" {
		t.Fatalf("open findings = %+v, %v", open, err)
	}
	stats, err := db.Stats()
	if err != nil {
		t.Fatal(err)
	}
	if stats.CriticFPRate["verifier"] != 1.0 {
		t.Fatalf("FP rate = %v", stats.CriticFPRate)
	}
	if stats.Nodes[graph.KindRequirement] != 1 || stats.Edges[graph.EdgeMotivates] != 1 {
		t.Fatalf("stats = %+v", stats)
	}
}
