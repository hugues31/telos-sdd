package contract

import (
	"strings"
	"testing"
)

const addDelta = "<!-- telos:op add file: spec/core.md -->\n" +
	"### REQ-001 — Emit the greeting\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: x\n  Given y\n  Then z\n```\n"

func TestParseDeltaOps(t *testing.T) {
	ops, err := ParseDelta([]byte(addDelta +
		"\n<!-- telos:op replace file: spec/core.md -->\n### REQ-002 — Replacement\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: r\n```\n" +
		"\n<!-- telos:op remove id: REQ-003 -->\n"))
	if err != nil {
		t.Fatal(err)
	}
	if len(ops) != 3 {
		t.Fatalf("ops = %v", ops)
	}
	if ops[0].Kind != OpAdd || ops[0].ID != "REQ-001" || ops[0].File != "spec/core.md" {
		t.Fatalf("op0 = %+v", ops[0])
	}
	if ops[1].Kind != OpReplace || ops[1].ID != "REQ-002" {
		t.Fatalf("op1 = %+v", ops[1])
	}
	if ops[2].Kind != OpRemove || ops[2].ID != "REQ-003" {
		t.Fatalf("op2 = %+v", ops[2])
	}
}

func TestParseDeltaRejections(t *testing.T) {
	cases := []struct{ name, body, want string }{
		{"content before marker", "stray text\n" + addDelta, "before the first"},
		{"content without marker", "just text\n", "outside any"},
		{"two sections in one op", "<!-- telos:op add file: spec/a.md -->\n### REQ-001 — a\n### REQ-002 — b\n", "exactly one"},
		{"add with id attr", "<!-- telos:op add id: REQ-001 -->\n### REQ-001 — a\n", "takes `file:`"},
		{"remove with file attr", "<!-- telos:op remove file: spec/a.md -->\n", "takes `id:`"},
		{"remove with body", "<!-- telos:op remove id: REQ-001 -->\nleftover\n", "unexpected content"},
		{"target outside spec", "<!-- telos:op add file: src/a.md -->\n### REQ-001 — a\n", "under spec/"},
	}
	for _, c := range cases {
		if _, err := ParseDelta([]byte(c.body)); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: err = %v, want containing %q", c.name, err, c.want)
		}
	}
}

func TestParseDeltaTemplateCommentsAreEmpty(t *testing.T) {
	ops, err := ParseDelta([]byte("<!-- Describe the contract delta with telos:op markers. -->\n\n<!-- guidance -->\n"))
	if err != nil || ops != nil {
		t.Fatalf("template delta = %v, %v", ops, err)
	}
}

func TestFoldAddReplaceRemove(t *testing.T) {
	base := map[string][]byte{
		ProductFile:    []byte(product),
		"spec/core.md": []byte(core),
	}

	// Add a new requirement to a new file.
	ops, err := ParseDelta([]byte("<!-- telos:op add file: spec/auth.md -->\n### REQ-010 — Locked out\nClass: security\nMotivated by: INT-001\n\n```gherkin\nScenario: lock\n```\n"))
	if err != nil {
		t.Fatal(err)
	}
	folded, err := Fold(base, ops)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(folded["spec/auth.md"]), "REQ-010") {
		t.Fatalf("folded auth.md = %q", folded["spec/auth.md"])
	}
	if c, problems := Parse(folded); len(problems) != 0 || c.Requirements["REQ-010"] == nil {
		t.Fatalf("folded contract invalid: %v", problems)
	}

	// Replace the existing requirement.
	ops, _ = ParseDelta([]byte("<!-- telos:op replace file: spec/core.md -->\n### REQ-001 — Emit the greeting twice\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: twice\n```\n"))
	folded, err = Fold(base, ops)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(folded["spec/core.md"]), "twice") || strings.Contains(string(folded["spec/core.md"]), "exactly once") {
		t.Fatalf("replace did not swap the section: %q", folded["spec/core.md"])
	}

	// Remove the only requirement of a file: the file disappears.
	ops, _ = ParseDelta([]byte("<!-- telos:op remove id: REQ-001 -->\n"))
	folded, err = Fold(base, ops)
	if err != nil {
		t.Fatal(err)
	}
	if _, exists := folded["spec/core.md"]; exists && strings.Contains(string(folded["spec/core.md"]), "REQ-001") {
		t.Fatalf("remove left the section: %q", folded["spec/core.md"])
	}

	// The base is never mutated.
	if !strings.Contains(string(base["spec/core.md"]), "exactly once") {
		t.Fatal("Fold mutated its base input")
	}
}

func TestFoldErrors(t *testing.T) {
	base := map[string][]byte{ProductFile: []byte(product), "spec/core.md": []byte(core)}
	cases := []struct{ name, delta, want string }{
		{"add duplicates", "<!-- telos:op add file: spec/other.md -->\n### REQ-001 — dup\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: d\n```\n", "already exists"},
		{"replace missing id", "<!-- telos:op replace file: spec/core.md -->\n### REQ-099 — none\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: n\n```\n", "not found"},
		{"replace missing file", "<!-- telos:op replace file: spec/none.md -->\n### REQ-001 — x\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: n\n```\n", "does not exist"},
		{"remove missing id", "<!-- telos:op remove id: REQ-404 -->\n", "not found"},
	}
	for _, c := range cases {
		ops, err := ParseDelta([]byte(c.delta))
		if err != nil {
			t.Fatalf("%s: parse: %v", c.name, err)
		}
		if _, err := Fold(base, ops); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: err = %v, want containing %q", c.name, err, c.want)
		}
	}
}
