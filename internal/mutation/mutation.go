// Package mutation hardens test honesty: it generates small AST mutants of
// changed Go code and runs the suite against each via `go test -overlay` —
// the tree under proof is never touched. A surviving mutant means the tests
// cannot tell the mutated program from the real one: that is a signal about
// the tests, reported for triage through the normal finding taxonomy.
package mutation

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// operators maps each mutable token to its mutation. Four families:
// conditional boundary, negation, arithmetic, logical.
var operators = map[token.Token]token.Token{
	token.LSS: token.LEQ, token.LEQ: token.LSS,
	token.GTR: token.GEQ, token.GEQ: token.GTR,
	token.EQL: token.NEQ, token.NEQ: token.EQL,
	token.ADD: token.SUB, token.SUB: token.ADD,
	token.MUL: token.QUO, token.QUO: token.MUL,
	token.LAND: token.LOR, token.LOR: token.LAND,
}

// Mutant is one candidate mutation of one file.
type Mutant struct {
	File     string `json:"file"`
	Line     int    `json:"line"`
	Operator string `json:"operator"` // "< -> <="
	Source   []byte `json:"-"`
}

// Caps bounds the runtime cost.
type Caps struct {
	PerFile   int           // max mutants per file (default 12)
	Total     int           // max mutants per run (default 100)
	PerMutant time.Duration // per-mutant suite timeout (default 60s)
}

func (c *Caps) defaults() {
	if c.PerFile <= 0 {
		c.PerFile = 12
	}
	if c.Total <= 0 {
		c.Total = 100
	}
	if c.PerMutant <= 0 {
		c.PerMutant = 60 * time.Second
	}
}

// Generate produces mutants of one Go source file, skipping test files and
// generated files. Deterministic: mutants come out in position order.
func Generate(filePath string, src []byte) []Mutant {
	if strings.HasSuffix(filePath, "_test.go") || strings.Contains(string(src), "Code generated") {
		return nil
	}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, filePath, src, 0)
	if err != nil {
		return nil
	}
	var mutants []Mutant
	ast.Inspect(file, func(n ast.Node) bool {
		bin, ok := n.(*ast.BinaryExpr)
		if !ok {
			return true
		}
		replacement, ok := operators[bin.Op]
		if !ok {
			return true
		}
		pos := fset.Position(bin.OpPos)
		before := bin.Op.String()
		after := replacement.String()
		mutated := append([]byte(nil), src...)
		copy(mutated[pos.Offset:], after)
		if len(before) != len(after) {
			mutated = append(mutated[:pos.Offset], append([]byte(after), src[pos.Offset+len(before):]...)...)
		}
		mutants = append(mutants, Mutant{File: filePath, Line: pos.Line, Operator: before + " -> " + after, Source: mutated})
		return true
	})
	sort.SliceStable(mutants, func(i, j int) bool { return mutants[i].Line < mutants[j].Line })
	return mutants
}

// Outcome summarizes a mutation run.
type Outcome struct {
	Sites     int      `json:"sites"`
	Killed    int      `json:"killed"`
	Survived  int      `json:"survived"`
	Score     float64  `json:"score"`
	Survivors []Mutant `json:"survivors,omitempty"`
}

// Run executes the suite against each mutant through a build overlay in dir
// (a checkout of the tree under proof). The real files are never modified.
func Run(dir string, testArgs []string, mutants []Mutant, caps Caps) (Outcome, error) {
	caps.defaults()
	perFile := map[string]int{}
	var outcome Outcome
	tmp, err := os.MkdirTemp("", "telos-mutants-*")
	if err != nil {
		return outcome, err
	}
	defer os.RemoveAll(tmp)

	for i, m := range mutants {
		if outcome.Sites >= caps.Total {
			break
		}
		if perFile[m.File] >= caps.PerFile {
			continue
		}
		perFile[m.File]++
		outcome.Sites++

		mutatedPath := filepath.Join(tmp, fmt.Sprintf("mutant-%d.go", i))
		if err := os.WriteFile(mutatedPath, m.Source, 0o644); err != nil {
			return outcome, err
		}
		overlay := filepath.Join(tmp, fmt.Sprintf("overlay-%d.json", i))
		payload, _ := json.Marshal(map[string]map[string]string{
			"Replace": {filepath.Join(dir, filepath.FromSlash(m.File)): mutatedPath},
		})
		if err := os.WriteFile(overlay, payload, 0o644); err != nil {
			return outcome, err
		}

		args := append([]string{"test", "-overlay", overlay, "-count=1"}, testArgs...)
		cmd := exec.Command("go", args...)
		cmd.Dir = dir
		done := make(chan error, 1)
		if err := cmd.Start(); err != nil {
			return outcome, err
		}
		go func() { done <- cmd.Wait() }()
		select {
		case err := <-done:
			if err != nil {
				outcome.Killed++ // the suite noticed the mutant
			} else {
				outcome.Survived++
				m.Source = nil
				outcome.Survivors = append(outcome.Survivors, m)
			}
		case <-time.After(caps.PerMutant):
			_ = cmd.Process.Kill()
			<-done
			outcome.Killed++ // a hung mutant counts as detected
		}
	}
	if outcome.Sites > 0 {
		outcome.Score = float64(outcome.Killed) / float64(outcome.Sites)
	}
	return outcome, nil
}
