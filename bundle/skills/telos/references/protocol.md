# Telos agent protocol

## State machine

`discovery → intent_draft → intent_review → contract_draft → contract_review → ready_to_implement → implementing → complete`

Always follow `next_actions` returned by `telos inspect --json`. Treat JSON error codes as authoritative.

## Draft formats

- Intent success criteria use `### CRIT-NNN — Name` headings.
- Resolve every intent ambiguity before review and write exactly `None.` under `## Open questions`.
- Each spec rule uses `### RULE-NNN — Name` followed by `Traces: CRIT-NNN`.
- Scenario IDs are globally unique `SCN-NNN` identifiers in a flow.
- A test plan declares one coverage entry per rule and category. Status is `covered`, requiring a scenario with the category tag, or `not_applicable`, requiring a concrete rationale.

Required categories: `positive`, `negative`, `boundary`, `authorization`, `state-transition`, `retry-idempotency`, `concurrency`, `failure-recovery`, and `prohibited-side-effect`.

Write Markdown bodies with `telos artifact put --id <id> --json` over stdin. Write JSON plans with `telos test-plan put --spec <id> --json` over stdin. Do not include TOML frontmatter in an artifact body.

Provider hooks permit shell execution only when the first command is the `telos` binary. Use a heredoc to stream an artifact, plan, patch, or verifier evidence; do not chain another shell command before or after Telos.

## Implementation

Create a Git-compatible unified patch outside the repository or stream it directly. Apply it with:

`telos change apply --flow <flow> --rule RULE-NNN --scenario SCN-NNN --json`

Repeat `--rule` and `--scenario` when a patch covers multiple contract elements. Do not patch `.telos/**` or generated `features/**`.

## Failures

- `TELOS_APPROVAL_STALE`: show the changed review again and obtain fresh approval.
- Permission prompt declined on `intent seal`, `contract seal`, `change complete`, or `repair --restore`: the user refused the approval. Return to the matching review or inspection step; do not retry the command unchanged.
- Guard denial of a seal (`Telos human gate: … digest is missing or stale`): re-run the matching review command and present the new content before sealing again.
- `TELOS_TRACEABILITY_GAP` or `TELOS_CONTRACT_INVALID`: return to the responsible contract agent.
- `TELOS_INTEGRITY_UNDECLARED_CHANGE`: report the project as corrupted. Do not continue or adopt the write. Use read-only `telos repair --json`, ask for approval, then restore through the CLI.

For a semantic contract defect, reverse declared implementation patches with `telos change abort --flow <flow> --reason "..." --json`, then create an immutable successor with `telos artifact revise --id <id> --reason "..." --json`.
