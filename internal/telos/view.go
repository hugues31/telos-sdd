package telos

import (
	"bytes"
	"fmt"
	"html/template"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"time"
)

type viewRule struct {
	ID          string
	Title       string
	Traces      []string
	Body        template.HTML
	Implemented bool
	Files       []string
	Tests       []string
}

type viewDomain struct {
	File  string
	Title string
	Rules []viewRule
}

type viewObjective struct {
	ID    string
	Title string
	Rules []string
}

type viewData struct {
	ProductTitle string
	Product      template.HTML
	Phase        string
	Problems     []string
	Pending      []string
	Changed      []string
	Objectives   []viewObjective
	Domains      []viewDomain
	Cfg          Config
	RuleTotal    int
	RuleTested   int
	Generated    string
	GeneratedISO string
	Version      string
	SpecRoot     string
	CodeRoot     string
}

// runView renders the whole executable contract — product intent, rules,
// traceability, phase — as one self-contained static HTML page. Read-only: it
// tolerates any project state and shows it instead of failing.
func runView(root, version, out string, open bool) (map[string]any, error) {
	cfg, err := readConfig(root)
	if err != nil {
		return nil, err
	}
	st, err := loadState(root)
	if err != nil {
		return nil, err
	}
	code, specFiles, err := inventories(root)
	if err != nil {
		return nil, err
	}
	model, problems := loadSpec(root, specFiles)
	tested, err := testedRules(root, cfg, code)
	if err != nil {
		return nil, err
	}
	implementedBy, testedBy := traceMaps(root, cfg, code)
	untested := untestedRules(model, tested)
	phase, _ := derivePhase(st, rootHashMap(specFiles), rootHashMap(code), len(untested))

	now := time.Now().UTC()
	data := viewData{
		ProductTitle: "Product",
		Phase:        phase,
		Problems:     problems,
		Pending:      changedPaths(st.Spec.Files, specFiles),
		Changed:      changedPaths(st.Code.Files, code),
		Cfg:          cfg,
		RuleTotal:    len(model.Rules),
		RuleTested:   len(model.Rules) - len(untested),
		Generated:    now.Format("2006-01-02 15:04 UTC"),
		GeneratedISO: now.Format(time.RFC3339),
		Version:      version,
		SpecRoot:     shortHash(st.Spec.Root),
		CodeRoot:     shortHash(st.Code.Root),
	}

	rulesByObjective := map[string][]string{}
	for _, id := range sortedRuleIDs(model) {
		for _, obj := range model.Rules[id].Traces {
			rulesByObjective[obj] = append(rulesByObjective[obj], id)
		}
	}
	if productRaw, readErr := os.ReadFile(filepath.Join(root, filepath.FromSlash(productFile))); readErr == nil {
		body := string(normalize(productRaw))
		if title := firstHeading(body); title != "" {
			data.ProductTitle = title
		}
		data.Product = renderMarkdown(body)
		for _, m := range objHeading.FindAllStringSubmatch(body, -1) {
			obj := viewObjective{ID: m[1], Rules: rulesByObjective[m[1]]}
			if len(m) > 2 {
				obj.Title = strings.TrimSpace(m[2])
			}
			data.Objectives = append(data.Objectives, obj)
		}
	}

	domainRules := map[string][]viewRule{}
	for _, id := range sortedRuleIDs(model) {
		info := model.Rules[id]
		domainRules[info.File] = append(domainRules[info.File], viewRule{
			ID:          id,
			Title:       info.Title,
			Traces:      info.Traces,
			Body:        renderMarkdown(ruleBody(info.Section)),
			Implemented: tested[id],
			Files:       implementedBy[id],
			Tests:       testedBy[id],
		})
	}
	for _, file := range sortedKeys(specFiles) {
		if file == productFile {
			continue
		}
		domain := viewDomain{File: file, Title: strings.TrimSuffix(filepath.Base(file), ".md"), Rules: domainRules[file]}
		if raw, readErr := os.ReadFile(filepath.Join(root, filepath.FromSlash(file))); readErr == nil {
			if title := firstHeading(string(normalize(raw))); title != "" {
				domain.Title = title
			}
		}
		data.Domains = append(data.Domains, domain)
	}

	if out == "" {
		out = filepath.Join(os.TempDir(), "telos-view-"+slugify(filepath.Base(root))+".html")
	}
	abs, err := filepath.Abs(out)
	if err != nil {
		return nil, err
	}
	if rel, relErr := filepath.Rel(root, abs); relErr == nil && !strings.HasPrefix(rel, "..") {
		if exec.Command("git", "-C", root, "check-ignore", "-q", rel).Run() != nil {
			return nil, coded("TELOS_INPUT_INVALID", "--out inside the repository must be git-ignored; a generated page would otherwise corrupt the declared code tree")
		}
	}
	var buf bytes.Buffer
	if err := viewTemplate.Execute(&buf, data); err != nil {
		return nil, err
	}
	if err := atomicWrite(abs, buf.Bytes(), 0o644); err != nil {
		return nil, err
	}
	result := map[string]any{"path": abs, "rules": data.RuleTotal, "objectives": len(data.Objectives), "phase": phase}
	if open {
		if openErr := openInBrowser(abs); openErr != nil {
			result["open_error"] = openErr.Error()
		}
	}
	return result, nil
}

// ruleBody strips the heading and Traces lines already rendered as badges.
func ruleBody(section string) string {
	lines := strings.Split(section, "\n")
	if len(lines) > 0 {
		lines = lines[1:]
	}
	out := lines[:0]
	dropped := false
	for _, line := range lines {
		if !dropped && traceLine.MatchString(line) {
			dropped = true
			continue
		}
		out = append(out, line)
	}
	return strings.TrimSpace(strings.Join(out, "\n"))
}

func firstHeading(body string) string {
	for _, line := range strings.Split(body, "\n") {
		if title, ok := strings.CutPrefix(line, "# "); ok {
			return strings.TrimSpace(title)
		}
	}
	return ""
}

func slugify(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	s = regexp.MustCompile(`[^a-z0-9]+`).ReplaceAllString(s, "-")
	s = strings.Trim(s, "-")
	if s == "" {
		return "project"
	}
	return s
}

func openInBrowser(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", path).Start()
	case "windows":
		return exec.Command("rundll32", "url.dll,FileProtocolHandler", path).Start()
	default:
		return exec.Command("xdg-open", path).Start()
	}
}

var (
	inlineCode = regexp.MustCompile("`([^`]+)`")
	inlineBold = regexp.MustCompile(`\*\*([^*]+)\*\*`)
)

func renderInline(s string) string {
	s = template.HTMLEscapeString(s)
	s = inlineCode.ReplaceAllString(s, "<code>$1</code>")
	s = inlineBold.ReplaceAllString(s, "<strong>$1</strong>")
	return s
}

func headingOf(line string) (int, string) {
	level := 0
	for level < len(line) && line[level] == '#' {
		level++
	}
	if level == 0 || level > 4 || level >= len(line) || line[level] != ' ' {
		return 0, ""
	}
	return level, strings.TrimSpace(line[level:])
}

// renderMarkdown covers exactly the subset the spec format uses: headings,
// paragraphs, dash lists, fenced code blocks, inline code and bold. Everything
// is HTML-escaped; unknown constructs degrade to plain paragraphs.
func renderMarkdown(md string) template.HTML {
	var b strings.Builder
	var para []string
	inCode, inList := false, false
	flushPara := func() {
		if len(para) > 0 {
			b.WriteString("<p>" + renderInline(strings.Join(para, " ")) + "</p>\n")
			para = nil
		}
	}
	closeList := func() {
		if inList {
			b.WriteString("</ul>\n")
			inList = false
		}
	}
	for _, line := range strings.Split(md, "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "```") {
			flushPara()
			closeList()
			if inCode {
				b.WriteString("</code></pre>\n")
				inCode = false
			} else {
				lang := template.HTMLEscapeString(strings.TrimSpace(strings.TrimPrefix(trimmed, "```")))
				b.WriteString(`<pre class="code ` + lang + `"><code>`)
				inCode = true
			}
			continue
		}
		if inCode {
			b.WriteString(template.HTMLEscapeString(line) + "\n")
			continue
		}
		if level, text := headingOf(trimmed); level > 0 {
			flushPara()
			closeList()
			fmt.Fprintf(&b, "<h%d>%s</h%d>\n", level, renderInline(text), level)
			continue
		}
		if item, ok := strings.CutPrefix(trimmed, "- "); ok {
			flushPara()
			if !inList {
				b.WriteString("<ul>\n")
				inList = true
			}
			b.WriteString("<li>" + renderInline(item) + "</li>\n")
			continue
		}
		if trimmed == "" {
			flushPara()
			closeList()
			continue
		}
		para = append(para, trimmed)
	}
	flushPara()
	closeList()
	if inCode {
		b.WriteString("</code></pre>\n")
	}
	return template.HTML(b.String())
}

var viewTemplate = template.Must(template.New("view").Parse(`<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{.ProductTitle}} — Telos spec</title>
<style>
:root {
  --bg: #f7f6f3; --panel: #ffffff; --ink: #1f2430; --muted: #6b7280;
  --line: #e5e2da; --accent: #3f6b4f; --ok-bg: #e3efe6; --ok-ink: #2c5e3f;
  --todo-bg: #fdf0dc; --todo-ink: #92600f; --bad-bg: #fbe3e0; --bad-ink: #a13c31;
  --code-bg: #f0eee9;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #16181d; --panel: #1e2128; --ink: #e8e6e1; --muted: #9aa0ab;
    --line: #2c303a; --accent: #8fb89b; --ok-bg: #233529; --ok-ink: #9fd0ad;
    --todo-bg: #3a2f1a; --todo-ink: #e3b566; --bad-bg: #3d2320; --bad-ink: #e59a8e;
    --code-bg: #14161b;
  }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink);
  font: 16px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 60rem; margin: 0 auto; padding: 2rem 1.25rem 4rem; }
header.top { display: flex; flex-wrap: wrap; align-items: baseline; gap: .75rem; margin-bottom: .5rem; }
header.top h1 { font-size: 1.7rem; margin: 0; }
.counts { color: var(--muted); font-size: .9rem; }
.badge { display: inline-block; border-radius: 999px; padding: .1rem .65rem;
  font-size: .78rem; font-weight: 600; white-space: nowrap; }
.badge.ok { background: var(--ok-bg); color: var(--ok-ink); }
.badge.todo { background: var(--todo-bg); color: var(--todo-ink); }
.badge.bad { background: var(--bad-bg); color: var(--bad-ink); }
.badge.ref { background: var(--code-bg); color: var(--muted); font-weight: 500; }
.banner { border-radius: .6rem; padding: .7rem 1rem; margin: 1rem 0; font-size: .92rem; }
.banner.todo { background: var(--todo-bg); color: var(--todo-ink); }
.banner.bad { background: var(--bad-bg); color: var(--bad-ink); }
section.panel { background: var(--panel); border: 1px solid var(--line);
  border-radius: .8rem; padding: 1.25rem 1.5rem; margin: 1.25rem 0; }
section.panel > h2 { margin-top: 0; font-size: 1.15rem; }
.md h1 { font-size: 1.3rem; } .md h2 { font-size: 1.05rem; } .md h3 { font-size: 1rem; }
.md pre.code { background: var(--code-bg); border-radius: .5rem; padding: .8rem 1rem;
  overflow-x: auto; font-size: .85rem; line-height: 1.5; }
.md code, .rule-meta code { background: var(--code-bg); border-radius: .3rem;
  padding: .08rem .3rem; font-size: .85em; }
.md pre.code code { background: none; padding: 0; }
article.rule { border-top: 1px solid var(--line); padding: 1rem 0; }
article.rule:first-of-type { border-top: 0; }
.rule-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: .6rem; }
.rule-head h3 { margin: 0; font-size: 1.02rem; }
.rule-id { font-family: ui-monospace, monospace; font-size: .85rem; color: var(--accent); font-weight: 700; }
.rule-meta { color: var(--muted); font-size: .82rem; margin: .3rem 0 .5rem; }
.rule-meta a { color: inherit; }
#q { width: 100%; padding: .6rem .9rem; border-radius: .6rem; border: 1px solid var(--line);
  background: var(--panel); color: var(--ink); font-size: .95rem; margin-top: 1rem; }
.tabs { display: flex; gap: .25rem; border-bottom: 1px solid var(--line); margin-top: 1rem; }
.tab { appearance: none; background: none; border: 0; border-bottom: 2px solid transparent;
  color: var(--muted); font: inherit; font-size: .95rem; font-weight: 600;
  padding: .5rem .9rem; cursor: pointer; }
.tab:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.tab[aria-selected="true"] { color: var(--ink); border-bottom-color: var(--accent); }
@media print { .tabs { display: none; } .tab-panel[hidden] { display: block !important; } }
table { border-collapse: collapse; width: 100%; font-size: .9rem; }
th, td { text-align: left; padding: .45rem .6rem; border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--muted); font-weight: 600; }
a { color: var(--accent); }
footer { color: var(--muted); font-size: .8rem; margin-top: 2rem; }
</style>
</head>
<body>
<main>
<header class="top">
  <h1>{{.ProductTitle}}</h1>
  {{if eq .Phase "clean"}}<span class="badge ok">clean</span>
  {{else if eq .Phase "corrupted"}}<span class="badge bad">corrupted</span>
  {{else}}<span class="badge todo">{{.Phase}}</span>{{end}}
  <span class="counts">{{len .Objectives}} objectives · {{.RuleTotal}} rules · {{.RuleTested}} proven</span>
</header>

{{if .Changed}}<div class="banner bad">Code changed outside the broker: {{range $i, $p := .Changed}}{{if $i}}, {{end}}<code>{{$p}}</code>{{end}}. Recover via Git or re-baseline deliberately.</div>{{end}}
{{if .Pending}}<div class="banner todo">Pending spec changes awaiting review/approval: {{range $i, $p := .Pending}}{{if $i}}, {{end}}<code>{{$p}}</code>{{end}}.</div>{{end}}
{{if .Problems}}<div class="banner bad">Spec problems: {{range $i, $p := .Problems}}{{if $i}} · {{end}}{{$p}}{{end}}</div>{{end}}

<nav class="tabs" role="tablist" hidden>
  <button class="tab" id="tab-intent" role="tab" aria-selected="true" aria-controls="panel-intent">Intent</button>
  <button class="tab" id="tab-contract" role="tab" aria-selected="false" aria-controls="panel-contract">Contract</button>
</nav>

<div id="panel-intent" class="tab-panel" role="tabpanel" aria-labelledby="tab-intent">
<section class="panel md" id="product">
{{.Product}}
</section>
</div>

<div id="panel-contract" class="tab-panel" role="tabpanel" aria-labelledby="tab-contract">
<input id="q" type="search" placeholder="Filter rules… (id, title, text, file)">

{{range .Domains}}
<section class="panel domain" id="{{.File}}">
  <h2>{{.Title}} <span class="badge ref">{{.File}}</span></h2>
  {{if not .Rules}}<p class="counts">No rules in this domain yet.</p>{{end}}
  {{range .Rules}}
  <article class="rule" id="{{.ID}}">
    <div class="rule-head">
      <span class="rule-id">{{.ID}}</span>
      <h3>{{.Title}}</h3>
      {{range .Traces}}<a class="badge ref" href="#{{.}}">{{.}}</a>{{end}}
      {{if .Implemented}}<span class="badge ok">proven by tests</span>{{else}}<span class="badge todo">not implemented</span>{{end}}
    </div>
    <div class="rule-meta">
      {{if .Files}}implemented in {{range $i, $f := .Files}}{{if $i}}, {{end}}<code>{{$f}}</code>{{end}}{{else}}no annotated file{{end}}
      · {{if .Tests}}tested by {{range $i, $f := .Tests}}{{if $i}}, {{end}}<code>{{$f}}</code>{{end}}{{else}}no tagged test{{end}}
    </div>
    <div class="md">{{.Body}}</div>
  </article>
  {{end}}
</section>
{{end}}

<section class="panel" id="setup">
  <h2>Verification setup</h2>
  <table>
    <tr><th>Test commands</th><td>{{if .Cfg.TestCommands}}{{range $i, $c := .Cfg.TestCommands}}{{if $i}} · {{end}}<code>{{$c}}</code>{{end}}{{else}}<span class="badge todo">none configured</span>{{end}}</td></tr>
    <tr><th>Test files</th><td>{{if .Cfg.TestFiles}}{{range $i, $p := .Cfg.TestFiles}}{{if $i}} · {{end}}<code>{{$p}}</code>{{end}}{{else}}<span class="badge todo">none configured</span>{{end}}</td></tr>
    <tr><th>Untraced patterns</th><td>{{if .Cfg.Untraced}}{{range $i, $p := .Cfg.Untraced}}{{if $i}} · {{end}}<code>{{$p}}</code>{{end}}{{else}}none{{end}}</td></tr>
    <tr><th>Agents</th><td>{{if .Cfg.Agents}}{{range $i, $a := .Cfg.Agents}}{{if $i}} · {{end}}<code>{{$a}}</code>{{end}}{{else}}none{{end}}</td></tr>
  </table>
  <p class="counts">The rules of the game, from <code>telos.toml</code>: a rule is “proven” when a test-file match references its id and every test command passes; untraced patterns may exist without tracing to a rule but remain integrity-checked.</p>
</section>

<section class="panel">
  <h2>Traceability</h2>
  <table>
    <tr><th>Objective</th><th>Rules</th></tr>
    {{range .Objectives}}
    <tr>
      <td id="{{.ID}}"><span class="rule-id">{{.ID}}</span> {{.Title}}</td>
      <td>{{if .Rules}}{{range $i, $r := .Rules}}{{if $i}} {{end}}<a class="badge ref" href="#{{$r}}">{{$r}}</a>{{end}}{{else}}<span class="badge todo">no rule yet</span>{{end}}</td>
    </tr>
    {{end}}
  </table>
</section>
</div>

<footer>Generated <time id="gen" datetime="{{.GeneratedISO}}">{{.Generated}}</time> by telos {{.Version}} · spec {{.SpecRoot}} · code {{.CodeRoot}}</footer>
</main>
<script>
(function () {
  var gen = document.getElementById('gen');
  if (gen) {
    var d = new Date(gen.getAttribute('datetime'));
    if (!isNaN(d)) gen.textContent = d.toLocaleString();
  }

  var tabs = document.querySelectorAll('.tab');
  function activate(panelID) {
    tabs.forEach(function (t) {
      t.setAttribute('aria-selected', String(t.getAttribute('aria-controls') === panelID));
    });
    document.querySelectorAll('.tab-panel').forEach(function (p) {
      p.hidden = p.id !== panelID;
    });
  }
  document.querySelector('.tabs').hidden = false;
  tabs.forEach(function (t) {
    t.addEventListener('click', function () { activate(t.getAttribute('aria-controls')); });
  });
  activate('panel-intent');
  function revealHash() {
    var el = location.hash && document.getElementById(decodeURIComponent(location.hash.slice(1)));
    if (!el) return;
    var panel = el.closest('.tab-panel');
    if (panel && panel.hidden) {
      activate(panel.id);
      el.scrollIntoView();
    }
  }
  window.addEventListener('hashchange', revealHash);
  revealHash();

  document.getElementById('q').addEventListener('input', function (e) {
    var q = e.target.value.toLowerCase();
    document.querySelectorAll('section.domain').forEach(function (s) {
      var any = false;
      s.querySelectorAll('article.rule').forEach(function (r) {
        var hit = q === '' || r.textContent.toLowerCase().indexOf(q) !== -1;
        r.hidden = !hit;
        if (hit) any = true;
      });
      s.hidden = q !== '' && !any;
    });
  });
})();
</script>
</body>
</html>
`))
