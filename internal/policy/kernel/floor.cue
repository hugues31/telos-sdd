package telospolicy

// The kernel floor (KERNEL-008): minima are CONCRETE values, so a project
// writing a weaker value is a unification CONFLICT, not an override. Defaults
// (bool | *false) may only be strengthened to true.

evidence: {
	behavior: #EvidenceClassRule & {
		red_green:   true
		adversarial: bool | *false
		benchmark:   bool | *false
		mutation:    bool | *false
	}
	security: #EvidenceClassRule & {
		red_green:   true
		adversarial: bool | *false
		benchmark:   bool | *false
		mutation:    bool | *false
	}
	invariant: #EvidenceClassRule & {
		red_green:   true
		adversarial: bool | *false
		benchmark:   bool | *false
		mutation:    bool | *false
	}
	concurrency: #EvidenceClassRule & {
		red_green:   true
		adversarial: bool | *false
		benchmark:   bool | *false
		mutation:    bool | *false
	}
	performance: #EvidenceClassRule & {
		red_green:   bool | *false
		adversarial: bool | *false
		benchmark:   bool | *true
		mutation:    bool | *false
	}
	architecture: #EvidenceClassRule & {
		red_green:   bool | *false
		adversarial: bool | *false
		benchmark:   bool | *false
		mutation:    bool | *false
	}
}

// Escalation: kernel rules are closed and concrete; project rules extend
// under escalation.project. The evaluator applies kernel ∪ project with a
// strictest-wins action lattice (annotate < require_human < block).
escalation: {
	kernel: [...#EscalationRule] & [
		{min_confidence: 0.95, proposed_severity: "blocking", action: "require_human"},
	]
	project: [...#EscalationRule] | *[]
}

// Protected paths are a set-as-struct: keys can be added, never removed.
protected: {
	"spec/**":      true
	"telos.toml":   true
	"policies/**":  true
	".telos/**":    true
}
