// Package ctxpack compiles bounded context packs: the smallest canonical
// neighborhood likely to preserve correctness for the work at hand. Global
// invariants are charged before any budgeting and never truncated; every
// selected item carries its retrieval path (why), and the top omissions are
// reported so an agent can pull more via `telos show`. Pack content is
// canonical bytes only — summaries never replace sources.
package ctxpack

import (
	"sort"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/graph"
)

// charsPerToken is the deterministic, model-agnostic token estimate; the 10%
// safety margin absorbs real-tokenizer variance.
const (
	charsPerToken = 4
	itemOverhead  = 24
	safetyMargin  = 0.9
)

// Item is one selected piece of canonical content.
type Item struct {
	ID      graph.NodeID `json:"id"`
	Kind    string       `json:"kind"`
	Tokens  int          `json:"tokens"`
	Why     []string     `json:"why"`
	Content string       `json:"content"`
}

// Section groups items by category.
type Section struct {
	Category string `json:"category"`
	Items    []Item `json:"items"`
}

// Omission names a candidate that did not fit.
type Omission struct {
	ID     graph.NodeID `json:"id"`
	Kind   string       `json:"kind"`
	Score  float64      `json:"score"`
	Tokens int          `json:"tokens"`
}

// Pack is the compiled context.
type Pack struct {
	Budget          int        `json:"budget"`
	EstimatedTokens int        `json:"estimated_tokens"`
	Sections        []Section  `json:"sections"`
	Omitted         []Omission `json:"omitted,omitempty"`
}

type candidate struct {
	node  graph.Node
	score float64
	why   []string
}

func tokensOf(n graph.Node) int {
	content := n.Body
	if content == "" {
		content = n.Title
	}
	return len(content)/charsPerToken + itemOverhead
}

func contentOf(n graph.Node) string {
	if n.Body != "" {
		return n.Body
	}
	return n.Title
}

func categoryOf(kind graph.NodeKind) string {
	switch kind {
	case graph.KindIntent:
		return "intent"
	case graph.KindRequirement:
		return "requirements"
	case graph.KindDecision:
		return "decisions"
	case graph.KindFinding:
		return "findings"
	default:
		return "code"
	}
}

var fractions = map[string]float64{
	"intent": 0.15, "requirements": 0.35, "decisions": 0.10, "findings": 0.10, "code": 0.20,
}

// Compile assembles a pack from seeds and free-text intent over the graph.
func Compile(q graph.Querier, seeds []graph.NodeID, intentText string, budget int) (Pack, error) {
	if budget <= 0 {
		budget = 16000
	}
	effective := int(float64(budget) * safetyMargin)
	pack := Pack{Budget: budget}

	// Global invariants: unconditional, before any budgeting.
	invariants, err := q.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{graph.KindRequirement}, Attrs: map[string]string{"class": "invariant"}})
	if err != nil {
		return pack, err
	}
	var invariantItems []Item
	spent := 0
	for _, n := range invariants {
		t := tokensOf(n)
		spent += t
		invariantItems = append(invariantItems, Item{ID: n.ID, Kind: string(n.Kind), Tokens: t, Why: []string{"class:invariant"}, Content: contentOf(n)})
	}
	if spent > effective {
		return pack, coded.New("TELOS_BUDGET_TOO_SMALL",
			"the global invariants alone need more than the budget; retry with at least "+itoa(int(float64(spent)/safetyMargin)+1)+" tokens")
	}
	if len(invariantItems) > 0 {
		pack.Sections = append(pack.Sections, Section{Category: "global_invariants", Items: invariantItems})
	}

	// Candidates: exact seeds, full-text hits, graph neighborhood.
	seen := map[graph.NodeID]*candidate{}
	addCandidate := func(n graph.Node, score float64, why string) {
		if categoryOf(n.Kind) == "code" && n.Body == "" && n.Kind != graph.KindSymbol && n.Kind != graph.KindChange {
			return
		}
		if existing, ok := seen[n.ID]; ok {
			if score > existing.score {
				existing.score = score
			}
			existing.why = append(existing.why, why)
			return
		}
		seen[n.ID] = &candidate{node: n, score: score, why: []string{why}}
	}
	for _, id := range seeds {
		if n, ok, err := q.Node(id); err != nil {
			return pack, err
		} else if ok {
			addCandidate(n, 1.0, "seed")
		}
	}
	if intentText != "" {
		hits, err := q.Search(intentText, graph.SearchOpt{Limit: 30})
		if err == nil {
			for i, h := range hits {
				if n, ok, _ := q.Node(h.ID); ok {
					addCandidate(n, 0.8-float64(i)*0.01, "fts")
				}
			}
		}
	}
	for _, id := range seeds {
		sub, err := q.Neighbors(id, graph.TraverseOpt{MaxDepth: 2})
		if err != nil {
			continue
		}
		for _, n := range sub.Nodes {
			if n.ID == id {
				continue
			}
			decay := 0.6
			if sub.Depth[n.ID] == 2 {
				decay = 0.36
			}
			addCandidate(n, decay, "graph:"+string(sub.Via[n.ID]))
		}
	}
	for _, inv := range invariantItems {
		delete(seen, inv.ID) // already charged
	}

	// Deterministic ordering: score desc, then id.
	var ordered []*candidate
	for _, c := range seen {
		ordered = append(ordered, c)
	}
	sort.Slice(ordered, func(i, j int) bool {
		if ordered[i].score != ordered[j].score {
			return ordered[i].score > ordered[j].score
		}
		return ordered[i].node.ID < ordered[j].node.ID
	})

	// Per-category greedy fill, then global overflow redistribution.
	remaining := effective - spent
	budgets := map[string]int{}
	for cat, frac := range fractions {
		budgets[cat] = int(float64(remaining) * frac)
	}
	selected := map[string][]Item{}
	var leftovers []*candidate
	for _, c := range ordered {
		cat := categoryOf(c.node.Kind)
		t := tokensOf(c.node)
		if t <= budgets[cat] {
			budgets[cat] -= t
			spent += t
			sort.Strings(c.why)
			selected[cat] = append(selected[cat], Item{ID: c.node.ID, Kind: string(c.node.Kind), Tokens: t, Why: c.why, Content: contentOf(c.node)})
		} else {
			leftovers = append(leftovers, c)
		}
	}
	pool := effective - spent
	for _, c := range leftovers {
		t := tokensOf(c.node)
		if t <= pool {
			pool -= t
			spent += t
			cat := categoryOf(c.node.Kind)
			sort.Strings(c.why)
			selected[cat] = append(selected[cat], Item{ID: c.node.ID, Kind: string(c.node.Kind), Tokens: t, Why: c.why, Content: contentOf(c.node)})
			c.score = -1 // consumed
		}
	}

	for _, cat := range []string{"intent", "requirements", "decisions", "findings", "code"} {
		if len(selected[cat]) > 0 {
			pack.Sections = append(pack.Sections, Section{Category: cat, Items: selected[cat]})
		}
	}
	for _, c := range leftovers {
		if c.score < 0 {
			continue
		}
		pack.Omitted = append(pack.Omitted, Omission{ID: c.node.ID, Kind: string(c.node.Kind), Score: c.score, Tokens: tokensOf(c.node)})
		if len(pack.Omitted) >= 20 {
			break
		}
	}
	pack.EstimatedTokens = spent
	return pack, nil
}

func itoa(v int) string {
	if v == 0 {
		return "0"
	}
	var digits []byte
	for v > 0 {
		digits = append([]byte{byte('0' + v%10)}, digits...)
		v /= 10
	}
	return string(digits)
}
