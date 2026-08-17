package view

import (
	"fmt"
	htmltemplate "html/template"
	"math"
	"net/http"
	"sort"
	"strconv"
	"strings"

	"github.com/hugues31/telos-sdd/internal/graph"
)

// graphPage renders the ego-graph explorer: always a focused node, a
// deterministic radial ring layout, refocus by plain links — never a global
// hairball and no scripting required. Depth and edge filters are GET params.
func (s *site) graphPage(w http.ResponseWriter, r *http.Request) {
	focus := graph.NodeID(r.URL.Query().Get("focus"))
	depth, _ := strconv.Atoi(r.URL.Query().Get("depth"))
	if depth < 1 || depth > 3 {
		depth = 2
	}
	if focus == "" {
		// Default focus: the first intent, else the first requirement.
		for _, kind := range []graph.NodeKind{graph.KindIntent, graph.KindRequirement} {
			if nodes, err := s.q.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{kind}}); err == nil && len(nodes) > 0 {
				focus = nodes[0].ID
				break
			}
		}
	}
	var b strings.Builder
	if focus == "" {
		b.WriteString(`<p class="muted">Nothing to explore yet: the contract is empty.</p>`)
		s.render(w, "Graph", htmltemplate.HTML(b.String()))
		return
	}
	sub, err := s.q.Neighbors(focus, graph.TraverseOpt{MaxDepth: depth, MaxNodes: 120})
	if err != nil {
		http.NotFound(w, r)
		return
	}
	b.WriteString(`<p>Focus ` + nodeLink(focus) + ` · depth `)
	for _, d := range []int{1, 2, 3} {
		if d == depth {
			b.WriteString(`<b>` + strconv.Itoa(d) + `</b> `)
		} else {
			b.WriteString(`<a href="/graph?focus=` + esc(string(focus)) + `&depth=` + strconv.Itoa(d) + `">` + strconv.Itoa(d) + `</a> `)
		}
	}
	if sub.Truncated {
		b.WriteString(` <span class="muted">(truncated at 120 nodes)</span>`)
	}
	b.WriteString(`</p>`)
	b.WriteString(egoSVG(focus, sub))
	s.render(w, "Graph", htmltemplate.HTML(b.String()))
}

// egoSVG lays the subgraph out on deterministic radial rings around the
// focus; every node is a link that refocuses the explorer on it.
func egoSVG(focus graph.NodeID, sub graph.Subgraph) string {
	const width, height = 900.0, 620.0
	cx, cy := width/2, height/2

	byRing := map[int][]graph.Node{}
	maxRing := 0
	for _, n := range sub.Nodes {
		ring := sub.Depth[n.ID]
		byRing[ring] = append(byRing[ring], n)
		if ring > maxRing {
			maxRing = ring
		}
	}
	for ring := range byRing {
		nodes := byRing[ring]
		sort.Slice(nodes, func(i, j int) bool {
			if nodes[i].Kind != nodes[j].Kind {
				return nodes[i].Kind < nodes[j].Kind
			}
			return nodes[i].ID < nodes[j].ID
		})
	}
	pos := map[graph.NodeID][2]float64{focus: {cx, cy}}
	for ring := 1; ring <= maxRing; ring++ {
		nodes := byRing[ring]
		radius := 110.0 * float64(ring)
		for i, n := range nodes {
			angle := 2 * math.Pi * float64(i) / float64(len(nodes))
			pos[n.ID] = [2]float64{cx + radius*math.Cos(angle), cy + radius*math.Sin(angle)*0.72}
		}
	}

	var b strings.Builder
	fmt.Fprintf(&b, `<svg viewBox="0 0 %.0f %.0f" width="100%%" role="img">`, width, height)
	for _, e := range sub.Edges {
		from, okF := pos[e.From]
		to, okT := pos[e.To]
		if !okF || !okT {
			continue
		}
		fmt.Fprintf(&b, `<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke="var(--line)" stroke-width="1"/>`, from[0], from[1], to[0], to[1])
	}
	for _, n := range sub.Nodes {
		p := pos[n.ID]
		label := string(n.ID)
		if len(label) > 28 {
			label = label[:27] + "…"
		}
		weight := "normal"
		if n.ID == focus {
			weight = "bold"
		}
		fmt.Fprintf(&b, `<a href="/graph?focus=%s"><circle cx="%.1f" cy="%.1f" r="5" fill="var(--accent)"/><text x="%.1f" y="%.1f" font-weight="%s">%s</text></a>`,
			esc(string(n.ID)), p[0], p[1], p[0]+8, p[1]+4, weight, esc(label))
	}
	b.WriteString(`</svg>`)
	return b.String()
}
