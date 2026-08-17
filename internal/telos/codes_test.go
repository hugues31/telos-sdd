package telos

import (
	"regexp"
	"testing"
)

func TestCodesRegistry(t *testing.T) {
	if len(Codes) == 0 {
		t.Fatal("Codes registry is empty")
	}
	namePattern := regexp.MustCompile(`^TELOS_[A-Z][A-Z_]*$`)
	seen := map[string]bool{}
	for _, c := range Codes {
		if !namePattern.MatchString(c.Name) {
			t.Errorf("code %q does not match %s", c.Name, namePattern)
		}
		if seen[c.Name] {
			t.Errorf("duplicate code %q", c.Name)
		}
		seen[c.Name] = true
		if c.AgentAction == "" {
			t.Errorf("code %q has no agent action", c.Name)
		}
	}
	for _, dropped := range []string{
		"TELOS_ANNOTATION_MISSING", "TELOS_ANNOTATION_ORPHAN", "TELOS_ANNOTATION_MISMATCH",
		"TELOS_CODE_CORRUPTED", "TELOS_SPEC_INVALID", "TELOS_SPEC_UNAPPROVED",
		"TELOS_RULE_NOT_IMPLEMENTED", "TELOS_TRACEABILITY_GAP", "TELOS_STATE_MISSING",
	} {
		if seen[dropped] {
			t.Errorf("V1 code %q must not appear in the V2 registry", dropped)
		}
	}
	for _, required := range []string{
		"TELOS_STATE_CORRUPTED", "TELOS_BASE_STALE", "TELOS_CERTIFICATE_INVALID",
		"TELOS_APPROVAL_REQUIRED", "TELOS_FINDING_BLOCKING", "TELOS_INDEX_STALE",
		"TELOS_POLICY_WEAKENS_KERNEL", "TELOS_CONSTRAINT_UNSAT",
	} {
		if !seen[required] {
			t.Errorf("missing required V2 code %q", required)
		}
	}
}
