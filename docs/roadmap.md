# Roadmap

Deliberately out of v0.6, in rough priority order:

- **Dogfooding telos-sdd under Telos** — `telos init` on this repository
  once 0.6.0 is tagged; the three loops become the maintainers' daily
  drivers and the ergonomics bar ("the human never thinks about
  certificates") gets measured for real.
- **ATTESTED mode** — signed commits/tags or external attestation replacing
  the embedded-secret seal for hostile-ish environments.
- **Candidate overlay in the knowledge layer** — index and view over an
  unpromoted candidate (`?change=CHG-NNN`), context packs seeded from the
  live target contract instead of the certified one.
- **Language adapters beyond Go** — symbol provenance and closures for
  TypeScript/Python/Rust (tree-sitter); file-level fallback exists today.
- **Richer constraints** — a real `cross` grammar with CUE-visible
  formalization, SMT beyond integer arithmetic, model checking for selected
  stateful domains.
- **Policy-gated mutation scores** (`mutation.min_score` per class) and
  benchmark thresholds as first-class performance evidence.
- **Worktree-aware conflict assistance** — guided resolution for salvage
  `--into` and rebase conflicts instead of the preserved-stash escape hatch.
- **PR annotations** — surfacing the contract delta and per-requirement
  evidence on GitHub/GitLab reviews.
- **Certificate history compaction** — archive policy for `changes/` growth
  on long-lived repositories.
