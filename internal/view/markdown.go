// Package view renders human-facing projections of the certified knowledge
// model. At M1 it carries the markdown/Gherkin renderer extracted from the V1
// static view; the loopback web server arrives at M8 and builds on the same
// renderer. Everything is HTML-escaped before any span wrapping, so contract
// content can never inject HTML.
package view

import (
	"fmt"
	"html/template"
	"os/exec"
	"regexp"
	"runtime"
	"strings"
)

var (
	inlineCode = regexp.MustCompile("`([^`]+)`")
	inlineBold = regexp.MustCompile(`\*\*([^*]+)\*\*`)

	gherkinStep    = regexp.MustCompile(`^(\s*)(Given|When|Then|And|But|\*)(\s)`)
	gherkinSection = regexp.MustCompile(`^(\s*)(Feature|Rule|Background|Scenario Outline|Scenario Template|Scenario|Example|Examples|Scenarios):`)
	gherkinString  = regexp.MustCompile(`&#34;.*?&#34;`)
	gherkinParam   = regexp.MustCompile(`&lt;[^&]+&gt;`)
)

// OpenInBrowser opens path with the platform's default handler.
func OpenInBrowser(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", path).Start()
	case "windows":
		return exec.Command("rundll32", "url.dll,FileProtocolHandler", path).Start()
	default:
		return exec.Command("xdg-open", path).Start()
	}
}

// gherkinInline escapes s, then marks quoted strings and <parameters>. The
// span wrapping happens after escaping, so content can never inject HTML.
func gherkinInline(s string) string {
	s = template.HTMLEscapeString(s)
	s = gherkinString.ReplaceAllString(s, `<span class="g-str">$0</span>`)
	s = gherkinParam.ReplaceAllString(s, `<span class="g-param">$0</span>`)
	return s
}

// RenderGherkinLine highlights one line inside a gherkin fence: comments,
// tags, table rows, section and step keywords, quoted strings, <parameters>.
func RenderGherkinLine(line string) string {
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

// RenderMarkdown covers exactly the subset the contract format uses:
// headings, paragraphs, dash lists, fenced code blocks (with Gherkin
// highlighting), inline code and bold. Everything is HTML-escaped; unknown
// constructs degrade to plain paragraphs.
func RenderMarkdown(md string) template.HTML {
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
				b.WriteString(RenderGherkinLine(line) + "\n")
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
