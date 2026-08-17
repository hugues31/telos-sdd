package view

import (
	"html/template"
	"net/http"
	"sort"
	"strconv"
	"strings"

	"github.com/hugues31/telos-sdd/internal/graph"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

type site struct {
	q       graph.Querier
	status  func() (kernel.ProjectStatus, error)
	version string
}

var pageTemplate = template.Must(template.New("page").Parse(`<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{.Title}} — Telos</title>
<style>
:root { --fg:#1a1a1a; --bg:#fdfdfc; --muted:#6b6b6b; --line:#e4e2dd; --accent:#0a5c36; --bad:#a3261f; --chip:#f0efeb; }
@media (prefers-color-scheme: dark) { :root { --fg:#e8e6e1; --bg:#161514; --muted:#9a978f; --line:#2e2c29; --accent:#79c99e; --bad:#e58e88; --chip:#232220; } }
* { box-sizing:border-box } body { margin:0; font:15px/1.55 system-ui,sans-serif; color:var(--fg); background:var(--bg) }
header { border-bottom:1px solid var(--line); padding:0.7rem 1.2rem; display:flex; gap:1.1rem; align-items:baseline; flex-wrap:wrap }
header b { font-size:1.05rem } header a { color:var(--muted); text-decoration:none } header a:hover { color:var(--accent) }
main { max-width:60rem; margin:0 auto; padding:1.4rem 1.2rem 4rem }
h1 { font-size:1.4rem } h2 { font-size:1.1rem; margin-top:2rem } a { color:var(--accent) }
table { border-collapse:collapse; width:100%; margin:0.8rem 0 } td,th { text-align:left; padding:0.35rem 0.6rem; border-bottom:1px solid var(--line); vertical-align:top }
.chip { background:var(--chip); border-radius:4px; padding:0.05rem 0.45rem; font-size:0.82rem; white-space:nowrap }
.ok { color:var(--accent) } .bad { color:var(--bad) } .muted { color:var(--muted) }
pre, .md pre { background:var(--chip); padding:0.7rem; border-radius:6px; overflow-x:auto }
.md code { background:var(--chip); padding:0 0.25rem; border-radius:3px }
.g-kw { color:var(--accent); font-weight:600 } .g-sec { font-weight:700 } .g-str { color:#8a6d1f } .g-com,.g-pipe { color:var(--muted) } .g-param { font-style:italic }
svg text { fill:var(--fg); font:12px system-ui } svg a text { fill:var(--accent) }
</style></head>
<body><header><b>Telos</b>
<a href="/">Overview</a><a href="/contract">Contract</a><a href="/changes">Changes</a>
<a href="/evidence">Evidence</a><a href="/findings">Findings</a><a href="/graph">Graph</a>
<a href="/health">Health</a><span class="muted">{{.Version}}</span></header>
<main><h1>{{.Title}}</h1>{{.Body}}</main></body></html>`))

func (s *site) render(w http.ResponseWriter, title string, body template.HTML) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_ = pageTemplate.Execute(w, struct {
		Title   string
		Version string
		Body    template.HTML
	}{title, s.version, body})
}

func esc(v string) string { return template.HTMLEscapeString(v) }

func nodeLink(id graph.NodeID) string {
	return `<a href="/node/` + esc(string(id)) + `">` + esc(string(id)) + `</a>`
}

func (s *site) overview(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	var b strings.Builder
	st, err := s.status()
	if err != nil {
		b.WriteString(`<p class="bad">` + esc(err.Error()) + `</p>`)
	} else {
		cls, label := "ok", strings.ToUpper(st.State)
		if st.State != kernel.StateCertified {
			cls = "bad"
		}
		b.WriteString(`<p>State: <b class="` + cls + `">` + esc(label) + `</b>`)
		if st.Certificate != nil {
			b.WriteString(` — change ` + esc(st.Certificate.Change) + `, sealed ` + esc(st.Certificate.SealedAt))
		}
		if st.Reason != "" {
			b.WriteString(` <span class="muted">(` + esc(st.Reason) + `)</span>`)
		}
		b.WriteString(`</p>`)
		if st.Salvage != nil {
			b.WriteString(`<p class="bad">` + esc(st.Salvage.Prompt) + `</p>`)
		}
		if st.Contract != nil {
			b.WriteString(`<p>` + strconv.Itoa(st.Contract.Intents) + ` intent(s), ` +
				strconv.Itoa(st.Contract.Requirements) + ` requirement(s), ` +
				strconv.Itoa(st.Contract.Decisions) + ` decision(s).</p>`)
		}
		if len(st.Changes) > 0 {
			b.WriteString(`<h2>Open changes</h2><table><tr><th>Change</th><th>Status</th><th>Category</th><th>Base</th></tr>`)
			for _, c := range st.Changes {
				stale := ""
				if c.BaseStale {
					stale = ` <span class="bad">stale</span>`
				}
				b.WriteString(`<tr><td>` + nodeLink(graph.NodeID(c.ID)) + `</td><td>` + esc(c.Status) + `</td><td>` + esc(c.Category) + `</td><td class="muted">` + esc(short(c.Base)) + stale + `</td></tr>`)
			}
			b.WriteString(`</table>`)
		}
	}
	if blocking, err := s.q.Findings(graph.FindingFilter{Blocking: true}); err == nil && len(blocking) > 0 {
		b.WriteString(`<h2 class="bad">Blocking findings</h2><ul>`)
		for _, f := range blocking {
			b.WriteString(`<li>` + esc(f.ChangeID+"/"+f.ID) + ` — ` + esc(f.Rationale) + `</li>`)
		}
		b.WriteString(`</ul>`)
	}
	s.render(w, "Overview", template.HTML(b.String()))
}

func (s *site) contract(w http.ResponseWriter, _ *http.Request) {
	var b strings.Builder
	for _, kind := range []graph.NodeKind{graph.KindIntent, graph.KindRequirement, graph.KindDecision} {
		nodes, err := s.q.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{kind}})
		if err != nil || len(nodes) == 0 {
			continue
		}
		b.WriteString(`<h2>` + esc(strings.Title(string(kind))) + `s</h2><table>`)
		for _, n := range nodes {
			extra := ""
			if class := n.Attrs["class"]; class != "" {
				extra = ` <span class="chip">` + esc(class) + `</span>`
			}
			b.WriteString(`<tr><td>` + nodeLink(n.ID) + `</td><td>` + esc(n.Title) + extra + `</td></tr>`)
		}
		b.WriteString(`</table>`)
	}
	s.render(w, "Contract", template.HTML(b.String()))
}

func (s *site) node(w http.ResponseWriter, r *http.Request) {
	id := graph.NodeID(strings.TrimPrefix(r.URL.Path, "/node/"))
	id = graph.NodeID(strings.TrimSuffix(string(id), ".html"))
	n, ok, err := s.q.Node(id)
	if err != nil || !ok {
		http.NotFound(w, r)
		return
	}
	var b strings.Builder
	b.WriteString(`<p class="muted">` + esc(string(n.Kind)))
	if n.Origin != "" {
		b.WriteString(` · ` + esc(n.Origin))
	}
	b.WriteString(` · <a href="/graph?focus=` + esc(string(n.ID)) + `">ego graph</a></p>`)
	if n.Body != "" {
		b.WriteString(`<div class="md">` + string(RenderMarkdown(n.Body)) + `</div>`)
	}
	if n.Kind == graph.KindRequirement {
		if rows, err := s.q.EvidenceFor(n.ID); err == nil && len(rows) > 0 {
			b.WriteString(`<h2>Evidence</h2><table><tr><th>Kind</th><th>Result</th><th>Fresh</th><th>Change</th></tr>`)
			for _, row := range rows {
				fresh := `<span class="bad">stale</span>`
				if row.Fresh {
					fresh = `<span class="ok">fresh</span>`
				}
				b.WriteString(`<tr><td>` + esc(row.Kind) + `</td><td>` + esc(row.Result) + `</td><td>` + fresh + `</td><td>` + nodeLink(graph.NodeID(row.ChangeID)) + `</td></tr>`)
			}
			b.WriteString(`</table>`)
		}
	}
	if sub, err := s.q.Neighbors(n.ID, graph.TraverseOpt{MaxDepth: 1}); err == nil && len(sub.Edges) > 0 {
		grouped := map[string][]string{}
		for _, e := range sub.Edges {
			if e.From == n.ID {
				grouped[string(e.Kind)] = append(grouped[string(e.Kind)], nodeLink(e.To))
			} else {
				grouped[string(e.Kind)+" ←"] = append(grouped[string(e.Kind)+" ←"], nodeLink(e.From))
			}
		}
		keys := make([]string, 0, len(grouped))
		for k := range grouped {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		b.WriteString(`<h2>Relations</h2><table>`)
		for _, k := range keys {
			sort.Strings(grouped[k])
			b.WriteString(`<tr><td class="muted">` + esc(k) + `</td><td>` + strings.Join(grouped[k], " · ") + `</td></tr>`)
		}
		b.WriteString(`</table>`)
	}
	s.render(w, string(n.ID)+" — "+n.Title, template.HTML(b.String()))
}

func (s *site) changes(w http.ResponseWriter, _ *http.Request) {
	nodes, _ := s.q.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{graph.KindChange}})
	var b strings.Builder
	b.WriteString(`<table><tr><th>Change</th><th>Title</th><th>Status</th><th>Category</th></tr>`)
	for _, n := range nodes {
		b.WriteString(`<tr><td>` + nodeLink(n.ID) + `</td><td>` + esc(n.Title) + `</td><td>` + esc(n.Attrs["status"]) + `</td><td>` + esc(n.Attrs["category"]) + `</td></tr>`)
	}
	b.WriteString(`</table>`)
	s.render(w, "Changes", template.HTML(b.String()))
}

func (s *site) evidence(w http.ResponseWriter, _ *http.Request) {
	reqs, _ := s.q.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{graph.KindRequirement}})
	var b strings.Builder
	covered, missing := 0, 0
	b.WriteString(`<table><tr><th>Requirement</th><th>Class</th><th>Evidence</th></tr>`)
	for _, req := range reqs {
		rows, _ := s.q.EvidenceFor(req.ID)
		cell := `<span class="bad">missing</span>`
		if len(rows) > 0 {
			covered++
			var chips []string
			for _, row := range rows {
				cls := "ok"
				if !row.Fresh || row.Result != "pass" {
					cls = "bad"
				}
				chips = append(chips, `<span class="chip `+cls+`">`+esc(row.Kind)+`</span>`)
			}
			cell = strings.Join(chips, " ")
		} else {
			missing++
		}
		b.WriteString(`<tr><td>` + nodeLink(req.ID) + `</td><td>` + esc(req.Attrs["class"]) + `</td><td>` + cell + `</td></tr>`)
	}
	b.WriteString(`</table><p>` + strconv.Itoa(covered) + ` covered, ` + strconv.Itoa(missing) + ` without direct evidence records.</p>`)
	s.render(w, "Evidence", template.HTML(b.String()))
}

func (s *site) findings(w http.ResponseWriter, _ *http.Request) {
	rows, _ := s.q.Findings(graph.FindingFilter{})
	var b strings.Builder
	b.WriteString(`<table><tr><th>Finding</th><th>Critic</th><th>Proposed</th><th>Effective</th><th>Status</th><th>Rationale</th></tr>`)
	for _, f := range rows {
		eff := f.EffectiveSeverity
		if eff == "" {
			eff = "—"
		}
		status := esc(f.Status)
		if f.Resolution != "" {
			status += ` <span class="chip">` + esc(f.Resolution) + `</span>`
		}
		b.WriteString(`<tr><td>` + esc(f.ChangeID+"/"+f.ID) + `</td><td>` + esc(f.Critic) + `</td><td>` + esc(f.ProposedSeverity) + ` <span class="muted">` + strconv.FormatFloat(f.Confidence, 'f', 2, 64) + `</span></td><td>` + esc(eff) + `</td><td>` + status + `</td><td>` + esc(f.Rationale) + `</td></tr>`)
	}
	b.WriteString(`</table>`)
	s.render(w, "Findings", template.HTML(b.String()))
}

func (s *site) health(w http.ResponseWriter, _ *http.Request) {
	var b strings.Builder
	st, err := s.status()
	row := func(name, value, cls string) {
		b.WriteString(`<tr><td>` + esc(name) + `</td><td class="` + cls + `">` + value + `</td></tr>`)
	}
	b.WriteString(`<table>`)
	if err == nil {
		cls := "ok"
		if st.State != kernel.StateCertified {
			cls = "bad"
		}
		row("Certification", esc(strings.ToUpper(st.State)), cls)
	}
	stats, serr := s.q.Stats()
	if serr == nil {
		row("Requirements", strconv.Itoa(stats.Nodes[graph.KindRequirement]), "")
		row("Symbols indexed", strconv.Itoa(stats.Nodes[graph.KindSymbol]), "")
	}
	blocking, _ := s.q.Findings(graph.FindingFilter{Blocking: true})
	cls := "ok"
	if len(blocking) > 0 {
		cls = "bad"
	}
	row("Blocking findings", strconv.Itoa(len(blocking)), cls)
	if serr == nil {
		for critic, rate := range stats.CriticFPRate {
			row("Critic FP rate — "+critic, strconv.FormatFloat(rate, 'f', 2, 64), "")
		}
	}
	root := s.q.Root()
	row("Index commit", `<span class="chip">`+esc(short(root.IndexedCommit))+`</span>`, "")
	b.WriteString(`</table>`)
	s.render(w, "Health", template.HTML(b.String()))
}

func short(s string) string {
	if len(s) > 12 {
		return s[:12]
	}
	return s
}
