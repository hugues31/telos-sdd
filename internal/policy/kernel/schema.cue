package telospolicy

// Closed definitions: a project policy cannot smuggle unknown fields.

#EvidenceClassRule: {
	red_green:   bool
	adversarial: bool
	benchmark:   bool
	mutation:    bool
}

#EscalationRule: {
	min_confidence:    float & >=0 & <=1
	proposed_severity: "info" | "minor" | "major" | "blocking"
	action:            "annotate" | "require_human" | "block"
}
