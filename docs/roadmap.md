# Roadmap

Telos establishes the spec-first, git-native model: the versioned spec as source of truth, a single harness-enforced approval on the spec diff, brokered annotated implementation, and per-rule executable proof. Candidate follow-ups:

- external signatures and CI attestations for spec and code roots;
- privilege-separated local mutation broker;
- pull-request annotations surfacing the spec diff and per-rule coverage;
- semantic contradiction and completeness analysis across domain specs;
- mutation-testing adapters to harden the "tagged test really asserts the rule" guarantee;
- worktree-aware concurrent change coordination;
- reconstruction benchmarks that regenerate implementations from `spec/` alone.
