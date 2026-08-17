package constraints

import (
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/smt"
)

var (
	smtLine    = regexp.MustCompile(`(?m)^\s*//\s*smt:\s*(.+)$`)
	fieldLine  = regexp.MustCompile(`(?m)^\s*([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.+)$`)
	boundExpr  = regexp.MustCompile(`^(<=|>=|<|>|==)?\s*(-?[0-9]+)$`)
	unsafeName = regexp.MustCompile(`[^A-Za-z0-9_]`)
)

// Assertions extracts the tier-2 subset from a requirement's constraint
// block: explicit `// smt: <linear expr>` lines, plus single-variable
// integer bounds (`name: >=5 & <=30`) so the solver sees the same ranges
// CUE enforces.
func Assertions(c contract.Contract) ([]smt.Assertion, map[string]string) {
	var reqs []string
	for id, req := range c.Requirements {
		if req.Constraint != "" {
			reqs = append(reqs, id)
		}
	}
	sort.Strings(reqs)

	var out []smt.Assertion
	names := map[string]string{} // assertion name → REQ id
	for _, id := range reqs {
		block := c.Requirements[id].Constraint
		prefix := unsafeName.ReplaceAllString(id, "_")
		n := 0
		add := func(expr string) {
			name := prefix + "_a" + itoa(n)
			n++
			names[name] = id
			out = append(out, smt.Assertion{Name: name, Expr: expr})
		}
		for _, m := range smtLine.FindAllStringSubmatch(block, -1) {
			add(strings.TrimSpace(m[1]))
		}
		for _, m := range fieldLine.FindAllStringSubmatch(block, -1) {
			field, rhs := m[1], m[2]
			if field == "vars" || field == "assert" || field == "scope" {
				continue
			}
			for _, part := range strings.Split(rhs, "&") {
				b := boundExpr.FindStringSubmatch(strings.TrimSpace(part))
				if b == nil {
					continue
				}
				op := b[1]
				if op == "" {
					op = "=="
				}
				add(field + " " + op + " " + b[2])
			}
		}
	}
	return out, names
}

// CheckSMT runs the tier-2 satisfiability check when z3 is available. An
// absent solver or an unknown/timeout verdict is a non-result: it never
// blocks and never satisfies anything.
func CheckSMT(c contract.Contract, timeout time.Duration) error {
	assertions, names := Assertions(c)
	if len(assertions) == 0 || !smt.Available() {
		return nil
	}
	script, err := smt.Script(assertions)
	if err != nil {
		return nil // outside the conservative grammar: tier-1 only, by design
	}
	result, err := smt.CheckSat(script, timeout)
	if err != nil || result.Status != smt.Unsat {
		return nil
	}
	seen := map[string]bool{}
	var culprits []string
	for _, name := range result.Core {
		if req, ok := names[name]; ok && !seen[req] {
			seen[req] = true
			culprits = append(culprits, req)
		}
	}
	if len(culprits) == 0 {
		for _, req := range names {
			if !seen[req] {
				seen[req] = true
				culprits = append(culprits, req)
			}
		}
	}
	sort.Strings(culprits)
	return coded.WithPaths("TELOS_CONSTRAINT_UNSAT", "the formalized requirements are provably contradictory (z3 unsat core); a human must resolve them", culprits)
}

func itoa(v int) string {
	if v == 0 {
		return "0"
	}
	var digits []byte
	for v > 0 {
		digits = append([]byte{byte('0' + v%10)}, digits...)
		v /= 10
	}
	return string(digits)
}
