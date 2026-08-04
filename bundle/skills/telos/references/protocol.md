# Telos agent protocol

## Repository model

- `spec/PRODUCT.md` — vision, measurable objectives as `### OBJ-NNN — Title` sections, constraints, non-goals. Objectives live only here.
- `spec/<domain>.md` — normative rules as `### RULE-NNN — Title` sections. Every rule carries a `Traces: OBJ-NNN` line and at least one ` ```gherkin ` scenario block. Rules live only in domain files.
- OBJ and RULE ids are unique across the repository and never reused after deletion.
- `telos.toml` (root) is human-owned configuration: `test_commands`, `test_files`, `infra` patterns. Never write it.
- `.telos/state.json` records the approved spec root and declared code root. Only the CLI writes it.

## Writing spec and code

Provider hooks permit shell execution only when the first command is the `telos` binary. Stream content with a heredoc; never chain another shell command before or after Telos.

- Spec: `telos spec put --file spec/<name>.md --json` over stdin (full file content). `--delete` removes a file.
- Code: `telos apply --rule RULE-NNN --json` with a Git unified patch over stdin. Repeat `--rule` when the patch serves several rules. The patch may not touch `spec/**`, `telos.toml`, `.telos/**`, `.claude/**`, `.codex/**`, `.agents/**`, `CLAUDE.md`, or `AGENTS.md`.

Annotation contract enforced on the patch post-image:

- Every touched file that does not match an `infra` pattern must contain, within its first 10 lines, a comment line `telos: RULE-NNN [RULE-NNN ...]` whose rules exist and intersect the cited `--rule` references.
- A rule counts as implemented only when a file matching `test_files` references its id and the configured `test_commands` pass. Give every rule a real, asserting test tagged with its id.

## Error codes → action

- `TELOS_SPEC_INVALID` — fix the listed structural problems with `telos spec put`; the gate never sees an invalid spec.
- `TELOS_NOTHING_PENDING` — the spec already matches its approved state; no review needed.
- `TELOS_APPROVAL_STALE` — the spec changed after review. Re-run `telos spec review` and present the new content before approving.
- `TELOS_SPEC_UNAPPROVED` — pending spec changes (possibly a direct human edit). Route through the challenger, review, and approval. Never revert the human's edit.
- `TELOS_CODE_CORRUPTED` — code changed outside the broker. Stop and report; the human recovers via Git or deliberately re-baselines with `telos init`. Never adopt the write.
- `TELOS_ANNOTATION_MISSING` / `TELOS_ANNOTATION_ORPHAN` — annotate the listed files against real rules, or have the human classify them as infra in `telos.toml`.
- `TELOS_ANNOTATION_MISMATCH` — the patch was reversed; resubmit with correct annotations intersecting the cited rules.
- `TELOS_RULE_NOT_IMPLEMENTED` — the spec is ahead: add tagged tests and implementation for the listed rules.
- `TELOS_TESTS_FAILED` — make the configured test commands pass without weakening assertions.
- `TELOS_TRACEABILITY_GAP` — a cited rule does not exist in the approved spec; fix the citation or return to the spec cycle.
- Permission prompt declined on `spec approve` — the user refused the approval. Return to challenging the spec; do not retry unchanged.

## Semantic defects

If a rule proves impossible, contradictory, or wrong during implementation, do not work around it. Stop, report the defect, and restart the cycle: correct the spec, obtain fresh review and approval, then implement from the corrected contract.
