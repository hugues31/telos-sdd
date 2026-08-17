package view

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/graph"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

type fakeQuerier struct {
	nodes map[graph.NodeID]graph.Node
}

func (f *fakeQuerier) Root() graph.RootInfo { return graph.RootInfo{IndexedCommit: "abc123def456789"} }
func (f *fakeQuerier) Node(id graph.NodeID) (graph.Node, bool, error) {
	n, ok := f.nodes[id]
	return n, ok, nil
}
func (f *fakeQuerier) Nodes(filter graph.NodeFilter) ([]graph.Node, error) {
	var out []graph.Node
	for _, n := range f.nodes {
		if len(filter.Kinds) > 0 {
			match := false
			for _, k := range filter.Kinds {
				if n.Kind == k {
					match = true
				}
			}
			if !match {
				continue
			}
		}
		out = append(out, n)
	}
	return out, nil
}
func (f *fakeQuerier) Neighbors(center graph.NodeID, _ graph.TraverseOpt) (graph.Subgraph, error) {
	if _, ok := f.nodes[center]; !ok {
		return graph.Subgraph{}, nil
	}
	sub := graph.Subgraph{Depth: map[graph.NodeID]int{center: 0}, Via: map[graph.NodeID]graph.NodeID{}}
	sub.Nodes = append(sub.Nodes, f.nodes[center])
	for id, n := range f.nodes {
		if id != center {
			sub.Depth[id] = 1
			sub.Nodes = append(sub.Nodes, n)
			sub.Edges = append(sub.Edges, graph.Edge{From: center, To: id, Kind: graph.EdgeMotivates})
		}
	}
	return sub, nil
}
func (f *fakeQuerier) Search(string, graph.SearchOpt) ([]graph.Hit, error) { return nil, nil }
func (f *fakeQuerier) EvidenceFor(graph.NodeID) ([]graph.EvidenceRow, error) {
	return []graph.EvidenceRow{{ID: "EVD-1", Kind: "witnessed_red_green", Result: "pass", Fresh: true, ChangeID: "CHG-001"}}, nil
}
func (f *fakeQuerier) Findings(graph.FindingFilter) ([]graph.FindingRow, error) { return nil, nil }
func (f *fakeQuerier) ResolveSymbol(string) ([]graph.Node, error)               { return nil, nil }
func (f *fakeQuerier) Stats() (graph.IndexStats, error) {
	return graph.IndexStats{Nodes: map[graph.NodeKind]int{}, Edges: map[graph.EdgeKind]int{}, CriticFPRate: map[string]float64{}}, nil
}
func (f *fakeQuerier) Close() error { return nil }

func testOptions() Options {
	return Options{
		Querier: &fakeQuerier{nodes: map[graph.NodeID]graph.Node{
			"INT-001": {ID: "INT-001", Kind: graph.KindIntent, Title: "Greet", Body: "### INT-001 — Greet\n\nBody <script>x</script>\n"},
			"REQ-001": {ID: "REQ-001", Kind: graph.KindRequirement, Title: "Greeting", Body: "prose", Attrs: map[string]string{"class": "behavior"}},
			"CHG-001": {ID: "CHG-001", Kind: graph.KindChange, Title: "Greeting", Attrs: map[string]string{"status": "promoted", "category": "behavior_change"}},
		}},
		Status: func() (kernel.ProjectStatus, error) {
			return kernel.ProjectStatus{Context: "root", State: kernel.StateCertified,
				Certificate: &kernel.CertStatus{Commit: "abc", Change: "CHG-001", SealedAt: "2026-01-01T00:00:00Z"}}, nil
		},
		Version: "test",
	}
}

func TestRoutesServe(t *testing.T) {
	handler := Handler(testOptions())
	for _, path := range []string{"/", "/contract", "/node/REQ-001", "/changes", "/evidence", "/findings", "/graph", "/graph?focus=REQ-001&depth=2", "/health"} {
		req := httptest.NewRequest(http.MethodGet, "http://127.0.0.1"+path, nil)
		rec := httptest.NewRecorder()
		handler.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Errorf("%s: HTTP %d", path, rec.Code)
		}
		if csp := rec.Header().Get("Content-Security-Policy"); !strings.Contains(csp, "default-src 'self'") {
			t.Errorf("%s: CSP missing (%q)", path, csp)
		}
		if strings.Contains(rec.Body.String(), "<script>x</script>") {
			t.Errorf("%s: unescaped content leaked", path)
		}
	}
}

func TestReadOnlyAndHostGuard(t *testing.T) {
	handler := Handler(testOptions())

	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1/", strings.NewReader("x"))
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("POST = %d, want 405", rec.Code)
	}

	req = httptest.NewRequest(http.MethodGet, "http://evil.example/", nil)
	req.Host = "evil.example"
	rec = httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("foreign Host = %d, want 403", rec.Code)
	}

	req = httptest.NewRequest(http.MethodGet, "http://127.0.0.1/node/REQ-404", nil)
	rec = httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("unknown node = %d, want 404", rec.Code)
	}
}

func TestStaticExport(t *testing.T) {
	dir := t.TempDir()
	written, err := StaticExport(testOptions(), dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(written) < 8 {
		t.Fatalf("written = %v", written)
	}
	data, err := os.ReadFile(filepath.Join(dir, "index.html"))
	if err != nil || !strings.Contains(string(data), "CERTIFIED") {
		t.Fatalf("index.html = %q, %v", data, err)
	}
	if _, err := os.Stat(filepath.Join(dir, "node", "REQ-001.html")); err != nil {
		t.Fatal("node page not exported")
	}
}
