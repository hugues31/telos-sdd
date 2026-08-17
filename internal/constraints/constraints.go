// Package constraints checks the formalized subset of the contract: the
// ```telos-constraint blocks requirements may carry. Tier 1 (always
// available) uses CUE unification — an invalid block is a contract problem,
// and a provably unsatisfiable union blocks certification. Tier 2 (optional
// external z3, for cross-variable arithmetic) arrives at M9. Formalization
// is incremental and optional per requirement; absence is never an error.
package constraints

import (
	"sort"

	"cuelang.org/go/cue"
	"cuelang.org/go/cue/cuecontext"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
)

// Check validates every telos-constraint block and their conjunction.
func Check(c contract.Contract) error {
	var reqs []string
	for id, req := range c.Requirements {
		if req.Constraint != "" {
			reqs = append(reqs, id)
		}
	}
	if len(reqs) == 0 {
		return nil
	}
	sort.Strings(reqs)

	ctx := cuecontext.New()
	var unified cue.Value
	first := true
	for _, id := range reqs {
		v := ctx.CompileString(c.Requirements[id].Constraint, cue.Filename(id))
		if v.Err() != nil {
			return coded.New("TELOS_CONTRACT_INVALID", id+" carries an invalid telos-constraint block: "+v.Err().Error())
		}
		if err := v.Validate(); err != nil {
			return coded.WithPaths("TELOS_CONSTRAINT_UNSAT", id+"'s constraint is unsatisfiable on its own: "+err.Error(), []string{id})
		}
		if first {
			unified, first = v, false
		} else {
			unified = unified.Unify(v)
		}
	}
	if err := unified.Validate(); err != nil {
		return coded.WithPaths("TELOS_CONSTRAINT_UNSAT", "the formalized requirements are provably contradictory; a human must resolve them: "+err.Error(), reqs)
	}
	return nil
}
