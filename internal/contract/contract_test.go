package contract

import (
	"strings"
	"testing"
)

const product = `# Product

### INT-001 — Application greets reliably

The application always produces its greeting.
`

const core = "# Core\n\n### REQ-001 — Emit the greeting\nClass: behavior\nMotivated by: INT-001\n\nThe application emits the greeting exactly once.\n\n```gherkin\nScenario: greeting\n  Given the app runs\n  Then the greeting is produced\n```\n"

func files(pairs ...string) map[string][]byte {
	out := map[string][]byte{}
	for i := 0; i < len(pairs); i += 2 {
		out[pairs[i]] = []byte(pairs[i+1])
	}
	return out
}

func TestParseValidContract(t *testing.T) {
	c, problems := Parse(files(ProductFile, product, "spec/core.md", core))
	if len(problems) != 0 {
		t.Fatalf("problems = %v", problems)
	}
	if len(c.Intents) != 1 || c.Intents["INT-001"].Title != "Application greets reliably" {
		t.Fatalf("intents = %v", c.Intents)
	}
	req := c.Requirements["REQ-001"]
	if req == nil || req.Class != ClassBehavior || !req.Gherkin || len(req.MotivatedBy) != 1 || req.MotivatedBy[0] != "INT-001" {
		t.Fatalf("requirement = %+v", req)
	}
}

func TestParseEmptyContractIsValid(t *testing.T) {
	if _, problems := Parse(nil); len(problems) != 0 {
		t.Fatalf("empty contract problems = %v", problems)
	}
}

func TestParsePlacementRules(t *testing.T) {
	_, problems := Parse(files(
		ProductFile, product+"\n### REQ-009 — misplaced\nClass: behavior\nMotivated by: INT-001\n",
		"spec/core.md", "### INT-002 — misplaced intent\n",
		"spec/notes.txt", "not markdown",
	))
	text := strings.Join(problems, "\n")
	for _, want := range []string{
		"REQ sections belong in spec domain files",
		"INT sections belong in spec/PRODUCT.md",
		"only Markdown files are allowed",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("problems miss %q:\n%s", want, text)
		}
	}
}

func TestParseRequirementValidation(t *testing.T) {
	_, problems := Parse(files(ProductFile, product, "spec/core.md",
		"### REQ-001 — no class\nMotivated by: INT-001\n\n```gherkin\nScenario: x\n```\n"+
			"### REQ-002 — bad class\nClass: quantum\nMotivated by: INT-001\n\n```gherkin\nScenario: x\n```\n"+
			"### REQ-003 — unmotivated\nClass: behavior\n\n```gherkin\nScenario: x\n```\n"+
			"### REQ-004 — dangling intent\nClass: behavior\nMotivated by: INT-999\n\n```gherkin\nScenario: x\n```\n"+
			"### REQ-005 — no scenario\nClass: behavior\nMotivated by: INT-001\n"))
	text := strings.Join(problems, "\n")
	for _, want := range []string{
		"REQ-001 is missing a `Class:` line",
		"REQ-002 has unknown class quantum",
		"REQ-003 is missing a `Motivated by: INT-NNN` line",
		"REQ-004 is motivated by unknown intent INT-999",
		"REQ-005 is missing a ```gherkin scenario block",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("problems miss %q:\n%s", want, text)
		}
	}
}

func TestParseGherkinOptionalForStructuralClasses(t *testing.T) {
	c, problems := Parse(files(ProductFile, product, "spec/perf.md",
		"### REQ-010 — p95 under 200ms\nClass: performance\nMotivated by: INT-001\n\nThe endpoint answers within 200ms at p95.\n"))
	if len(problems) != 0 {
		t.Fatalf("performance without gherkin must not be a problem: %v", problems)
	}
	if len(c.Warnings) != 1 || !strings.Contains(c.Warnings[0], "REQ-010") {
		t.Fatalf("warnings = %v", c.Warnings)
	}
}

func TestParseDuplicatesAcrossFiles(t *testing.T) {
	_, problems := Parse(files(ProductFile, product,
		"spec/a.md", core,
		"spec/b.md", strings.ReplaceAll(core, "# Core", "# B")))
	if len(problems) != 1 || !strings.Contains(problems[0], "duplicate requirement REQ-001") {
		t.Fatalf("problems = %v", problems)
	}
}

func TestParseDecisions(t *testing.T) {
	c, problems := Parse(files(ProductFile, product, DecisionsFile,
		"### DEC-001 — Resource-scoped authorization\nStatus: accepted\n\nAuthorization stays per-resource.\n"+
			"### DEC-002 — Old approach\nStatus: superseded by DEC-001\n"+
			"### DEC-003 — Broken\n\nNo status here.\n"+
			"### DEC-004 — Dangling\nStatus: superseded by DEC-999\n"))
	text := strings.Join(problems, "\n")
	if !strings.Contains(text, "DEC-003 is missing a `Status:` line") {
		t.Errorf("missing-status problem absent:\n%s", text)
	}
	if !strings.Contains(text, "DEC-004 is superseded by unknown decision DEC-999") {
		t.Errorf("dangling-supersession problem absent:\n%s", text)
	}
	if c.Decisions["DEC-001"].Status != "accepted" {
		t.Errorf("DEC-001 = %+v", c.Decisions["DEC-001"])
	}
	if d := c.Decisions["DEC-002"]; d.Status != "superseded" || d.SupersededBy != "DEC-001" {
		t.Errorf("DEC-002 = %+v", d)
	}
}

func TestParseRequirementsNeedProductFile(t *testing.T) {
	_, problems := Parse(files("spec/core.md", core))
	joined := strings.Join(problems, "\n")
	if !strings.Contains(joined, "spec/PRODUCT.md: missing") {
		t.Fatalf("problems = %v", problems)
	}
}

func TestConstraintExtraction(t *testing.T) {
	body := "### REQ-020 — Timeout bounds\nClass: invariant\nMotivated by: INT-001\n\n```gherkin\nScenario: x\n```\n\n```telos-constraint\nvars: { timeout_min: int }\nassert: timeout_min: >=5 & <=30\n```\n"
	c, problems := Parse(files(ProductFile, product, "spec/core.md", body))
	if len(problems) != 0 {
		t.Fatalf("problems = %v", problems)
	}
	if got := c.Requirements["REQ-020"].Constraint; !strings.Contains(got, "timeout_min: >=5 & <=30") {
		t.Fatalf("constraint = %q", got)
	}
}

func TestReqRefs(t *testing.T) {
	refs := ReqRefs([]byte("// asserts REQ-002 and REQ-001, REQ-002 again\r\n"))
	if len(refs) != 2 || refs[0] != "REQ-002" || refs[1] != "REQ-001" {
		t.Fatalf("refs = %v", refs)
	}
}
