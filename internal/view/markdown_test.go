package view

import (
	"strings"
	"testing"
)

func TestRenderMarkdownEscapesHTML(t *testing.T) {
	out := string(RenderMarkdown("# Title <script>alert(1)</script>\n\nBody with <img src=x onerror=y> and `code <b>`.\n\n- item <i>\n\n```\n<script>raw</script>\n```\n"))
	if strings.Contains(out, "<script>") || strings.Contains(out, "<img") || strings.Contains(out, "<i>") || strings.Contains(out, "<b>") {
		t.Fatalf("unescaped HTML leaked:\n%s", out)
	}
	for _, want := range []string{"<h1>", "<p>", "<ul>", "<li>", "<code>", "<pre class=\"code\">"} {
		if !strings.Contains(out, want) {
			t.Errorf("output misses %s:\n%s", want, out)
		}
	}
}

func TestRenderMarkdownGherkinHighlighting(t *testing.T) {
	out := string(RenderMarkdown("```gherkin\nScenario: greeting\n  Given the app runs with \"flag\"\n  Then <param> is produced\n  # comment\n  | a | b |\n```\n"))
	for _, want := range []string{`g-sec`, `g-kw`, `g-str`, `g-param`, `g-com`, `g-pipe`, `data-lang="gherkin"`} {
		if !strings.Contains(out, want) {
			t.Errorf("gherkin output misses %s:\n%s", want, out)
		}
	}
	if strings.Contains(out, "<param>") {
		t.Fatal("gherkin parameter leaked unescaped")
	}
}
