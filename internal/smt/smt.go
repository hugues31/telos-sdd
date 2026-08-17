// Package smt is the optional tier-2 constraint checker: if a z3 binary is
// on PATH, cross-variable integer systems (linear sums plus simple products)
// extracted from telos-constraint blocks are checked for satisfiability. z3 is NEVER
// required — absence is explicit and non-blocking, and unknown/timeout is a
// non-result that neither blocks nor satisfies anything.
package smt

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
	"time"
)

// Status is the solver verdict.
type Status string

const (
	Sat     Status = "sat"
	Unsat   Status = "unsat"
	Unknown Status = "unknown"
)

// Result is one solver run.
type Result struct {
	Status Status   `json:"status"`
	Core   []string `json:"core,omitempty"` // named assertions of the unsat core
}

// Available reports whether z3 can be found.
func Available() bool {
	_, err := exec.LookPath("z3")
	return err == nil
}

// Assertion is one named SMT assertion in the conservative grammar:
// comparisons between linear integer sums (ints, vars, int*var).
type Assertion struct {
	Name string
	Expr string // infix, e.g. "attempts * window <= 150"
}

var (
	comparison = regexp.MustCompile(`^(.*?)(<=|>=|==|!=|<|>)(.*)$`)
	token      = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)
	number     = regexp.MustCompile(`^-?[0-9]+$`)
	varRef     = regexp.MustCompile(`[A-Za-z_][A-Za-z0-9_]*`)
)

// translate converts one infix comparison into SMT-LIB, or fails for
// anything outside the conservative grammar (which then stays tier-1 only).
func translate(expr string) (string, error) {
	m := comparison.FindStringSubmatch(strings.TrimSpace(expr))
	if m == nil {
		return "", fmt.Errorf("not a comparison: %q", expr)
	}
	left, err := translateSum(m[1])
	if err != nil {
		return "", err
	}
	right, err := translateSum(m[3])
	if err != nil {
		return "", err
	}
	op := m[2]
	switch op {
	case "==":
		return "(= " + left + " " + right + ")", nil
	case "!=":
		return "(not (= " + left + " " + right + "))", nil
	default:
		return "(" + op + " " + left + " " + right + ")", nil
	}
}

// translateSum handles `term (+|-) term ...` where term is int, var, or
// int*var / var*int.
func translateSum(s string) (string, error) {
	s = strings.TrimSpace(s)
	parts := splitTop(s, "+-")
	var terms []string
	for _, p := range parts {
		t, err := translateTerm(strings.TrimSpace(strings.TrimLeft(p, "+")))
		if err != nil {
			return "", err
		}
		if strings.HasPrefix(strings.TrimSpace(p), "-") {
			t = "(- 0 " + t + ")"
		}
		terms = append(terms, t)
	}
	switch len(terms) {
	case 0:
		return "", fmt.Errorf("empty expression")
	case 1:
		return terms[0], nil
	default:
		return "(+ " + strings.Join(terms, " ") + ")", nil
	}
}

func splitTop(s, ops string) []string {
	var parts []string
	start := 0
	for i, r := range s {
		if i > 0 && strings.ContainsRune(ops, r) {
			parts = append(parts, s[start:i])
			start = i
		}
	}
	parts = append(parts, s[start:])
	return parts
}

func translateTerm(s string) (string, error) {
	s = strings.TrimSpace(s)
	if factors := strings.Split(s, "*"); len(factors) == 2 {
		a, b := strings.TrimSpace(factors[0]), strings.TrimSpace(factors[1])
		aOK := number.MatchString(a) || token.MatchString(a)
		bOK := number.MatchString(b) || token.MatchString(b)
		if aOK && bOK {
			return "(* " + a + " " + b + ")", nil
		}
		return "", fmt.Errorf("unsupported product %q", s)
	}
	if number.MatchString(s) || token.MatchString(s) {
		return s, nil
	}
	return "", fmt.Errorf("unsupported term %q", s)
}

// Script builds an SMT-LIB script with named assertions over Int constants.
func Script(assertions []Assertion) (string, error) {
	vars := map[string]bool{}
	var asserts []string
	for _, a := range assertions {
		smtExpr, err := translate(a.Expr)
		if err != nil {
			return "", fmt.Errorf("%s: %w", a.Name, err)
		}
		for _, v := range varRef.FindAllString(a.Expr, -1) {
			vars[v] = true
		}
		asserts = append(asserts, "(assert (! "+smtExpr+" :named "+a.Name+"))")
	}
	var b strings.Builder
	// No set-logic: z3 auto-detects, and simple variable products (the
	// design's canonical example) fall outside pure QF_LIA.
	b.WriteString("(set-option :produce-unsat-cores true)\n")
	var names []string
	for v := range vars {
		names = append(names, v)
	}
	sortStrings(names)
	for _, v := range names {
		b.WriteString("(declare-const " + v + " Int)\n")
	}
	for _, a := range asserts {
		b.WriteString(a + "\n")
	}
	b.WriteString("(check-sat)\n(get-unsat-core)\n")
	return b.String(), nil
}

// CheckSat runs the script through z3 with a timeout.
func CheckSat(script string, timeout time.Duration) (Result, error) {
	return checkSatWith("z3", script, timeout)
}

// checkSatWith is the seam tests use to substitute a fake solver.
func checkSatWith(binary, script string, timeout time.Duration) (Result, error) {
	seconds := int(timeout.Seconds())
	if seconds < 1 {
		seconds = 1
	}
	cmd := exec.Command(binary, "-in", fmt.Sprintf("-T:%d", seconds))
	cmd.Stdin = strings.NewReader(script)
	out, _ := cmd.CombinedOutput() // z3 exits nonzero on unsat cores in some builds; parse output regardless
	text := strings.TrimSpace(string(out))
	lines := strings.Split(text, "\n")
	if len(lines) == 0 {
		return Result{Status: Unknown}, nil
	}
	switch strings.TrimSpace(lines[0]) {
	case "sat":
		return Result{Status: Sat}, nil
	case "unsat":
		result := Result{Status: Unsat}
		if len(lines) > 1 {
			core := strings.Trim(strings.TrimSpace(strings.Join(lines[1:], " ")), "()")
			for _, name := range strings.Fields(core) {
				result.Core = append(result.Core, name)
			}
		}
		return result, nil
	default:
		return Result{Status: Unknown}, nil
	}
}

func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}
