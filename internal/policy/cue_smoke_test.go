package policy

import (
	"testing"

	"cuelang.org/go/cue"
	"cuelang.org/go/cue/cuecontext"
)

// TestCUESmoke pins the unification semantics KERNEL-008 relies on: a value
// satisfying its constraints validates, and a project value conflicting with
// a kernel floor is a unification error, not an override.
func TestCUESmoke(t *testing.T) {
	ctx := cuecontext.New()

	good := ctx.CompileString("timeout: >=5 & <=30\ntimeout: 10")
	if err := good.Validate(cue.Concrete(true)); err != nil {
		t.Fatalf("valid constraint failed to validate: %v", err)
	}

	// The kernel floor pattern: a concrete `true` cannot be weakened to false.
	kernel := ctx.CompileString("require: red_green: true")
	project := ctx.CompileString("require: red_green: false")
	if err := kernel.Unify(project).Validate(); err == nil {
		t.Fatal("weakening a concrete kernel floor must be a unification conflict")
	}

	// Strengthening a default is allowed.
	kernelDefault := ctx.CompileString("require: mutation: bool | *false")
	stronger := ctx.CompileString("require: mutation: true")
	if err := kernelDefault.Unify(stronger).Validate(cue.Concrete(true)); err != nil {
		t.Fatalf("strengthening a kernel default must unify: %v", err)
	}
}
