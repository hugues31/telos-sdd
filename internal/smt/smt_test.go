package smt

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestScriptEmission(t *testing.T) {
	script, err := Script([]Assertion{
		{Name: "REQ_001_a0", Expr: "attempts * window <= 150"},
		{Name: "REQ_002_a0", Expr: "window >= 20"},
		{Name: "REQ_003_a0", Expr: "attempts >= 10"},
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"(declare-const attempts Int)",
		"(declare-const window Int)",
		"(assert (! (<= (* attempts window) 150) :named REQ_001_a0))",
		"(check-sat)",
		"(get-unsat-core)",
	} {
		if !strings.Contains(script, want) {
			t.Errorf("script misses %q:\n%s", want, script)
		}
	}
}

func TestTranslateRejectsOutsideGrammar(t *testing.T) {
	if _, err := Script([]Assertion{{Name: "a", Expr: "not an expression"}}); err == nil {
		t.Fatal("non-comparison must be rejected")
	}
	if _, err := Script([]Assertion{{Name: "a", Expr: "x / y <= 10"}}); err == nil {
		t.Fatal("division is outside the grammar")
	}
}

// fakeSolver writes an executable script that prints a canned z3 answer.
func fakeSolver(t *testing.T, output string) string {
	t.Helper()
	if runtime.GOOS == "windows" {
		t.Skip("sh-based fake solver")
	}
	path := filepath.Join(t.TempDir(), "fake-z3")
	script := "#!/bin/sh\ncat > /dev/null\nprintf '" + output + "'\n"
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestCheckSatParsing(t *testing.T) {
	cases := []struct {
		output string
		status Status
		core   int
	}{
		{`sat\n`, Sat, 0},
		{`unsat\n(REQ_001_a0 REQ_002_a0)\n`, Unsat, 2},
		{`unknown\n`, Unknown, 0},
		{`timeout gibberish\n`, Unknown, 0},
	}
	for _, c := range cases {
		bin := fakeSolver(t, c.output)
		result, err := checkSatWith(bin, "(check-sat)", time.Second)
		if err != nil {
			t.Fatal(err)
		}
		if result.Status != c.status || len(result.Core) != c.core {
			t.Errorf("output %q → %+v, want %s/%d", c.output, result, c.status, c.core)
		}
	}
}

// TestRealZ3 exercises the actual solver when present; absence skips, never
// fails — z3 is optional by design.
func TestRealZ3(t *testing.T) {
	if !Available() {
		t.Skip("z3 not installed (optional)")
	}
	script, _ := Script([]Assertion{
		{Name: "a0", Expr: "x >= 10"},
		{Name: "a1", Expr: "x <= 5"},
	})
	result, err := CheckSat(script, 5*time.Second)
	if err != nil || result.Status != Unsat {
		t.Fatalf("contradiction → %+v, %v", result, err)
	}
}
