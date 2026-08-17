package policy

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/hugues31/telos-sdd/internal/coded"
)

func writePolicy(t *testing.T, root, name, content string) {
	t.Helper()
	dir := filepath.Join(root, "policies")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestLoadKernelOnly(t *testing.T) {
	root := t.TempDir()
	eff, err := Load(root)
	if err != nil {
		t.Fatal(err)
	}
	if !eff.Evidence["security"].RedGreen || eff.Evidence["security"].Adversarial {
		t.Fatalf("security defaults = %+v", eff.Evidence["security"])
	}
	if !eff.Evidence["performance"].Benchmark {
		t.Fatalf("performance defaults = %+v", eff.Evidence["performance"])
	}
	if eff.Hash == "" || len(eff.Protected) == 0 {
		t.Fatalf("eff = %+v", eff)
	}
	// Hash is stable.
	again, err := Load(root)
	if err != nil || again.Hash != eff.Hash {
		t.Fatalf("hash unstable: %s vs %s (%v)", again.Hash, eff.Hash, err)
	}
}

func TestProjectStrengthens(t *testing.T) {
	root := t.TempDir()
	writePolicy(t, root, "policy.cue", `package telospolicy

evidence: security: adversarial: true
escalation: project: [{min_confidence: 0.8, proposed_severity: "blocking", action: "block"}]
protected: "payments/**": true
`)
	eff, err := Load(root)
	if err != nil {
		t.Fatal(err)
	}
	if !eff.Evidence["security"].Adversarial {
		t.Fatal("strengthening did not take")
	}
	found := false
	for _, p := range eff.Protected {
		if p == "payments/**" {
			found = true
		}
	}
	if !found {
		t.Fatalf("protected = %v", eff.Protected)
	}
	// Deterministic escalation: a critic's 0.9-confidence blocking proposal
	// now blocks; a 0.5 one does not.
	if eff.Escalate("blocking", 0.9) != "block" {
		t.Fatal("escalation rule did not fire")
	}
	if eff.Escalate("blocking", 0.5) != "" {
		t.Fatal("escalation fired below its confidence floor")
	}
	// The kernel rule still applies (strictest-wins union).
	if eff.Escalate("blocking", 0.96) != "block" {
		t.Fatal("union lost the project rule")
	}

	// A different policy hashes differently.
	base, _ := Load(t.TempDir())
	if base.Hash == eff.Hash {
		t.Fatal("different policies must hash differently")
	}
}

func TestWeakeningIsAConflict(t *testing.T) {
	root := t.TempDir()
	writePolicy(t, root, "policy.cue", `package telospolicy

evidence: security: red_green: false
`)
	_, err := Load(root)
	e, ok := coded.As(err)
	if !ok || e.Code != "TELOS_POLICY_WEAKENS_KERNEL" {
		t.Fatalf("err = %v", err)
	}
}

func TestInvalidPolicy(t *testing.T) {
	root := t.TempDir()
	writePolicy(t, root, "policy.cue", "package telospolicy\n\nevidence: security: unknown_field: true\n")
	_, err := Load(root)
	e, ok := coded.As(err)
	if !ok || (e.Code != "TELOS_POLICY_INVALID" && e.Code != "TELOS_POLICY_WEAKENS_KERNEL") {
		t.Fatalf("err = %v", err)
	}
}
