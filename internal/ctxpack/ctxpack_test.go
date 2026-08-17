package ctxpack

import (
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/graph"
)

// fakeQuerier is a tiny in-memory graph.
type fakeQuerier struct {
	nodes map[graph.NodeID]graph.Node
	edges []graph.Edge
}

func (f *fakeQuerier) Root() graph.RootInfo { return graph.RootInfo{} }
func (f *fakeQuerier) Node(id graph.NodeID) (graph.Node, bool, error) {
	n, ok := f.nodes[id]
	return n, ok, nil
}
func (f *fakeQuerier) Nodes(filter graph.NodeFilter) ([]graph.Node, error) {
	var out []graph.Node
	for _, n := range f.nodes {
		if len(filter.Kinds) > 0 && n.Kind != filter.Kinds[0] {
			continue
		}
		match := true
		for k, v := range filter.Attrs {
			if n.Attrs[k] != v {
				match = false
			}
		}
		if match {
			out = append(out, n)
		}
	}
	return out, nil
}
func (f *fakeQuerier) Neighbors(center graph.NodeID, opt graph.TraverseOpt) (graph.Subgraph, error) {
	sub := graph.Subgraph{Depth: map[graph.NodeID]int{center: 0}, Via: map[graph.NodeID]graph.NodeID{}}
	for _, e := range f.edges {
		var other graph.NodeID
		if e.From == center {
			other = e.To
		} else if e.To == center {
			other = e.From
		} else {
			continue
		}
		sub.Depth[other] = 1
		sub.Via[other] = center
		sub.Nodes = append(sub.Nodes, f.nodes[other])
	}
	sub.Nodes = append(sub.Nodes, f.nodes[center])
	return sub, nil
}
func (f *fakeQuerier) Search(q string, opt graph.SearchOpt) ([]graph.Hit, error) {
	var hits []graph.Hit
	for _, n := range f.nodes {
		if strings.Contains(strings.ToLower(n.Body+n.Title), strings.ToLower(q)) {
			hits = append(hits, graph.Hit{ID: n.ID, Kind: n.Kind, Title: n.Title})
		}
	}
	return hits, nil
}
func (f *fakeQuerier) EvidenceFor(graph.NodeID) ([]graph.EvidenceRow, error) { return nil, nil }
func (f *fakeQuerier) Findings(graph.FindingFilter) ([]graph.FindingRow, error) {
	return nil, nil
}
func (f *fakeQuerier) ResolveSymbol(string) ([]graph.Node, error) { return nil, nil }
func (f *fakeQuerier) Stats() (graph.IndexStats, error)           { return graph.IndexStats{}, nil }
func (f *fakeQuerier) Close() error                               { return nil }

func fixture() *fakeQuerier {
	body := strings.Repeat("word ", 100) // ~125 tokens + overhead
	return &fakeQuerier{
		nodes: map[graph.NodeID]graph.Node{
			"INT-001": {ID: "INT-001", Kind: graph.KindIntent, Title: "Greet", Body: body},
			"REQ-001": {ID: "REQ-001", Kind: graph.KindRequirement, Title: "Greeting", Body: body, Attrs: map[string]string{"class": "behavior"}},
			"REQ-002": {ID: "REQ-002", Kind: graph.KindRequirement, Title: "Invariant", Body: body, Attrs: map[string]string{"class": "invariant"}},
			"DEC-001": {ID: "DEC-001", Kind: graph.KindDecision, Title: "Simple", Body: body},
		},
		edges: []graph.Edge{
			{From: "INT-001", To: "REQ-001", Kind: graph.EdgeMotivates},
		},
	}
}

func TestCompileSelectsAndAttributes(t *testing.T) {
	pack, err := Compile(fixture(), []graph.NodeID{"REQ-001"}, "greeting", 16000)
	if err != nil {
		t.Fatal(err)
	}
	if pack.EstimatedTokens == 0 || pack.EstimatedTokens > 16000 {
		t.Fatalf("estimated = %d", pack.EstimatedTokens)
	}
	byCat := map[string][]Item{}
	for _, s := range pack.Sections {
		byCat[s.Category] = s.Items
	}
	// The invariant REQ is always included, before budgeting.
	if len(byCat["global_invariants"]) != 1 || byCat["global_invariants"][0].ID != "REQ-002" {
		t.Fatalf("invariants = %+v", byCat["global_invariants"])
	}
	// The seed is selected with its retrieval path.
	found := false
	for _, item := range byCat["requirements"] {
		if item.ID == "REQ-001" {
			found = true
			if len(item.Why) == 0 {
				t.Fatalf("why missing: %+v", item)
			}
		}
	}
	if !found {
		t.Fatalf("seed not selected: %+v", pack.Sections)
	}
	// The graph neighborhood pulled the intent.
	if len(byCat["intent"]) == 0 {
		t.Fatalf("intent missing: %+v", pack.Sections)
	}
	// Determinism.
	again, _ := Compile(fixture(), []graph.NodeID{"REQ-001"}, "greeting", 16000)
	if again.EstimatedTokens != pack.EstimatedTokens || len(again.Sections) != len(pack.Sections) {
		t.Fatal("compile is not deterministic")
	}
}

func TestCompileBudgetTooSmall(t *testing.T) {
	_, err := Compile(fixture(), nil, "", 50)
	e, ok := coded.As(err)
	if !ok || e.Code != "TELOS_BUDGET_TOO_SMALL" {
		t.Fatalf("err = %v", err)
	}
}

func TestCompileOmissionsReported(t *testing.T) {
	q := fixture()
	// A budget that fits the invariant plus barely anything else.
	pack, err := Compile(q, []graph.NodeID{"REQ-001"}, "greeting", 400)
	if err != nil {
		t.Fatal(err)
	}
	if len(pack.Omitted) == 0 {
		t.Fatalf("omissions not reported: %+v", pack)
	}
}
