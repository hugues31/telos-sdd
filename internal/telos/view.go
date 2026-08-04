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

var (
	gherkinStep    = regexp.MustCompile(`^(\s*)(Given|When|Then|And|But|\*)(\s)`)
	gherkinSection = regexp.MustCompile(`^(\s*)(Feature|Rule|Background|Scenario Outline|Scenario Template|Scenario|Example|Examples|Scenarios):`)
	gherkinString  = regexp.MustCompile(`&#34;.*?&#34;`)
	gherkinParam   = regexp.MustCompile(`&lt;[^&]+&gt;`)
)

// gherkinInline escapes s, then marks quoted strings and <parameters>. The
// span wrapping happens after escaping, so spec content can never inject HTML.
func gherkinInline(s string) string {
	s = template.HTMLEscapeString(s)
	s = gherkinString.ReplaceAllString(s, `<span class="g-str">$0</span>`)
	s = gherkinParam.ReplaceAllString(s, `<span class="g-param">$0</span>`)
	return s
}

// renderGherkinLine highlights one line inside a gherkin fence: comments,
// tags, table rows, section and step keywords, quoted strings, <parameters>.
func renderGherkinLine(line string) string {
	trimmed := strings.TrimSpace(line)
	switch {
	case strings.HasPrefix(trimmed, "#"):
		return `<span class="g-com">` + template.HTMLEscapeString(line) + `</span>`
	case strings.HasPrefix(trimmed, "@"):
		return `<span class="g-tag">` + template.HTMLEscapeString(line) + `</span>`
	case strings.HasPrefix(trimmed, `"""`):
		return `<span class="g-str">` + template.HTMLEscapeString(line) + `</span>`
	case strings.HasPrefix(trimmed, "|"):
		cells := strings.Split(line, "|")
		for i, cell := range cells {
			cells[i] = template.HTMLEscapeString(cell)
		}
		return strings.Join(cells, `<span class="g-pipe">|</span>`)
	}
	if m := gherkinSection.FindStringSubmatch(line); m != nil {
		return m[1] + `<span class="g-sec">` + m[2] + `:</span>` + gherkinInline(line[len(m[0]):])
	}
	if m := gherkinStep.FindStringSubmatch(line); m != nil {
		return m[1] + `<span class="g-kw">` + template.HTMLEscapeString(m[2]) + `</span>` + m[3] + gherkinInline(line[len(m[0]):])
	}
	return gherkinInline(line)
}

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
	codeLang := ""
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
				codeLang = strings.TrimSpace(strings.TrimPrefix(trimmed, "```"))
				if esc := template.HTMLEscapeString(codeLang); esc != "" {
					fmt.Fprintf(&b, `<pre class="code %s" data-lang="%s"><code>`, esc, esc)
				} else {
					b.WriteString(`<pre class="code"><code>`)
				}
				inCode = true
			}
			continue
		}
		if inCode {
			if codeLang == "gherkin" {
				b.WriteString(renderGherkinLine(line) + "\n")
			} else {
				b.WriteString(template.HTMLEscapeString(line) + "\n")
			}
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
<script>try{var t=localStorage.getItem("telos-theme");if(t==="light"||t==="dark")document.documentElement.setAttribute("data-theme",t)}catch(e){}</script>
<style>
:root {
  --bg: #fbfbfa; --ink: #16181c; --muted: #6d7280; --faint: #a6aab3;
  --line: #dedddb; --hair: #ebeae8; --code-bg: #f3f2f0;
  --accent: #0e7a4c; --amber: #96660d; --red: #ac3a2e;
  --g-str: #8a5c15; --g-param: #2f6e7e;
}
:root[data-theme="dark"] {
  --bg: #0d0e10; --ink: #e6e6e3; --muted: #8f939c; --faint: #585d66;
  --line: #26282c; --hair: #1b1d20; --code-bg: #141519;
  --accent: #4dc08b; --amber: #d3a457; --red: #dd8375;
  --g-str: #cfa365; --g-param: #7bb6c4;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0d0e10; --ink: #e6e6e3; --muted: #8f939c; --faint: #585d66;
    --line: #26282c; --hair: #1b1d20; --code-bg: #141519;
    --accent: #4dc08b; --amber: #d3a457; --red: #dd8375;
    --g-str: #cfa365; --g-param: #7bb6c4;
  }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink);
  font: 16px/1.65 system-ui, -apple-system, "Segoe UI", sans-serif;
  -webkit-font-smoothing: antialiased; }
code, pre, .rule-id, .badge, .counts, .tab, #q, th, footer, .rule-meta, .theme {
  font-family: ui-monospace, "SF Mono", "Cascadia Code", "JetBrains Mono", Menlo, Consolas, monospace; }
main { max-width: 60rem; margin: 0 auto; padding: 3.5rem 1.5rem 5rem; }
header.top { display: flex; flex-wrap: wrap; align-items: baseline; gap: .9rem; }
header.top h1 { margin: 0; font-size: 1.85rem; font-weight: 650; letter-spacing: -.02em; }
.counts { color: var(--muted); font-size: .78rem; text-transform: uppercase; letter-spacing: .08em; }
.theme { appearance: none; background: none; border: 1px solid var(--line); border-radius: 3px;
  color: var(--muted); font-size: .72rem; font-weight: 600; text-transform: uppercase;
  letter-spacing: .08em; padding: .3em .6em; cursor: pointer; margin-left: auto; }
.theme:hover { color: var(--ink); border-color: var(--muted); }
.theme:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.badge { display: inline-block; font-size: .74rem; font-weight: 600;
  text-transform: uppercase; letter-spacing: .08em; white-space: nowrap; }
.badge.ok { color: var(--accent); }
.badge.todo { color: var(--amber); }
.badge.bad { color: var(--red); }
.badge.ok::before, .badge.todo::before, .badge.bad::before {
  content: "\25CF\00A0"; font-size: .6rem; vertical-align: .1em; }
.badge.ref { color: var(--muted); font-weight: 500; border: 1px solid var(--line);
  border-radius: 3px; padding: .15em .45em; text-transform: none; letter-spacing: .02em; }
a.badge.ref { text-decoration: none; }
a.badge.ref:hover { color: var(--ink); border-color: var(--muted); }
.banner { border: 1px solid; border-radius: 4px; padding: .75rem 1rem; margin: 1.5rem 0 0; font-size: .95rem; }
.banner.todo { color: var(--amber); border-color: color-mix(in srgb, var(--amber) 40%, transparent); }
.banner.bad { color: var(--red); border-color: color-mix(in srgb, var(--red) 40%, transparent); }
section.panel { border-top: 1px solid var(--line); margin: 2.5rem 0 0; padding-top: 1.5rem; }
section.panel > h2 { margin: 0 0 1rem; font-size: 1.1rem; font-weight: 650; letter-spacing: -.01em; }
.md { max-width: 48rem; }
.md h1 { font-size: 1.4rem; letter-spacing: -.015em; }
.md h2 { font-size: 1.1rem; } .md h3 { font-size: 1rem; }
.md pre.code { position: relative; background: var(--code-bg); border: 1px solid var(--hair);
  border-radius: 4px; padding: .9rem 1.1rem; overflow-x: auto; font-size: .875rem; line-height: 1.65; }
.md pre.code[data-lang]::after { content: attr(data-lang); position: absolute;
  top: .6rem; right: .8rem; font-size: .66rem; font-weight: 600;
  text-transform: uppercase; letter-spacing: .1em; color: var(--faint); }
.g-kw { color: var(--accent); font-weight: 600; }
.g-sec { font-weight: 700; }
.g-str { color: var(--g-str); }
.g-param { color: var(--g-param); font-style: italic; }
.g-com, .g-pipe { color: var(--faint); }
.g-tag { color: var(--muted); }
.md code { background: var(--code-bg); border-radius: 3px; padding: .1em .35em; font-size: .85em; }
.md pre.code code { background: none; padding: 0; font-size: inherit; }
article.rule { border-top: 1px solid var(--hair); padding: 1.15rem 0; }
article.rule:first-of-type { border-top: 0; }
.rule-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: .65rem; }
.rule-head h3 { margin: 0; font-size: 1.05rem; font-weight: 600; }
.rule-id { font-size: .82rem; color: var(--accent); font-weight: 700; letter-spacing: .03em; }
.rule-meta { color: var(--muted); font-size: .78rem; margin: .35rem 0 .6rem; }
.rule-meta a { color: inherit; }
.search { position: sticky; top: 0; z-index: 5; background: var(--bg); padding: .9rem 0 .6rem; }
#q { width: 100%; padding: .55rem .8rem; border-radius: 4px; border: 1px solid var(--line);
  background: transparent; color: var(--ink); font-size: .88rem; }
#q:focus { outline: none; border-color: var(--accent); }
#q::placeholder { color: var(--faint); }
.tabs { display: flex; gap: 1.4rem; border-bottom: 1px solid var(--line); margin-top: 2rem; }
.tab { appearance: none; background: none; border: 0; margin-bottom: -1px;
  border-bottom: 1px solid transparent; color: var(--muted); font-size: .76rem;
  font-weight: 600; text-transform: uppercase; letter-spacing: .1em;
  padding: .6rem .1rem; cursor: pointer; }
.tab:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.tab[aria-selected="true"] { color: var(--ink); border-bottom-color: var(--ink); }
@media print { .tabs, .theme { display: none; } .tab-panel[hidden] { display: block !important; } }
table { border-collapse: collapse; width: 100%; font-size: .92rem; }
th, td { text-align: left; padding: .5rem .6rem .5rem 0; border-bottom: 1px solid var(--hair); vertical-align: top; }
th { color: var(--muted); font-weight: 600; font-size: .74rem; text-transform: uppercase; letter-spacing: .08em; }
a { color: var(--accent); text-decoration-thickness: 1px; text-underline-offset: 2px; }
footer { color: var(--faint); font-size: .75rem; margin-top: 3rem; }
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
  <button class="theme" id="theme" type="button" title="Theme (auto / light / dark)" hidden>auto</button>
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
<div class="search"><input id="q" type="search" placeholder="Filter rules… (id, title, text, file)"></div>

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

  var themeBtn = document.getElementById('theme');
  var themeMode = 'auto';
  try { themeMode = localStorage.getItem('telos-theme') || 'auto'; } catch (e) {}
  function applyTheme(mode) {
    themeMode = mode;
    if (mode === 'auto') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', mode);
    themeBtn.textContent = '◐ ' + mode;
  }
  applyTheme(themeMode);
  themeBtn.hidden = false;
  themeBtn.addEventListener('click', function () {
    var order = ['auto', 'light', 'dark'];
    var next = order[(order.indexOf(themeMode) + 1) % order.length];
    try { localStorage.setItem('telos-theme', next); } catch (e) {}
    applyTheme(next);
  });

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
