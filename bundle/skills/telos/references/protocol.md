# Telos agent protocol

## Repository model

- `spec/PRODUCT.md` — vision, measurable objectives as `### OBJ-NNN — Title` sections, constraints, non-goals. Objectives live only here.
- `spec/<domain>.md` — normative rules as `### RULE-NNN — Title` sections. Every rule carries a `Traces: OBJ-NNN` line and at least one ` ```gherkin ` scenario block. Rules live only in domain files.
- OBJ and RULE ids are unique across the repository and never reused after deletion.
- `telos.toml` (root) is human-owned configuration: `test_commands`, `test_files`, `untraced` patterns. Never write it.
- `.telos/state.json` records the approved spec root and declared code root. Only the CLI writes it.

## Writing spec and code

Provider hooks permit shell execution only when the first command is the `telos` binary. Stream content with a heredoc; never chain another shell command before or after Telos.

- Spec: `telos spec put --file spec/<name>.md --json` over stdin (full file content). `--delete` removes a file.
- Code: `telos apply --rule RULE-NNN --json` with a Git unified patch over stdin. Repeat `--rule` when the patch serves several rules. The patch may not touch `spec/**`, `telos.toml`, `.telos/**`, `.claude/**`, `.codex/**`, `.agents/**`, `CLAUDE.md`, or `AGENTS.md`.

Annotation contract enforced on the patch post-image:

- Every touched file that does not match an `untraced` pattern must contain, within its first 10 lines, a comment line `telos: RULE-NNN [RULE-NNN ...]` whose rules exist and intersect the cited `--rule` references.
- A rule counts as implemented only when a file matching `test_files` references its id and the configured `test_commands` pass. Give every rule a real, asserting test tagged with its id.

## Test-first proof

A rule is proven by a witnessed red-green cycle, enforced mechanically by `telos apply`:

- The first patch citing an unproven rule must touch only `test_files` matches and add a test referencing the rule. The broker runs `test_commands` on a green baseline and requires failure; the witnessed red seals the exact test bytes in `.telos/state.json`. Work one rule's cycle at a time — a second red is not attributable while the suite is already failing.
- Sealed tests may not change until their rule is proven. The only exception is a test-only patch the suite fails again: a legitimate rewrite re-seals through red. Only implementation patches may turn the suite green; the apply that witnesses green proves the sealed rules and lifts their seals.
- Announce the witnessed red to the user before implementing: the failing output is the evidence the implementation must answer.
- A rule that documents behavior the code already has can never be witnessed failing. Submit its test with `--expect-pass`: the guard raises a permission prompt for that adoption claim and the suite must pass with the documentation test in place.
- Test references are policed on the post-image: a patch may not introduce references to rules outside its own witnessed cycle.

## Error codes → action

- `TELOS_SPEC_INVALID` — fix the listed structural problems with `telos spec put`; the gate never sees an invalid spec.
- `TELOS_NOTHING_PENDING` — the spec already matches its approved state; no review needed.
- `TELOS_APPROVAL_STALE` — the spec changed after review. Re-run `telos spec review` and present the new content before approving.
- `TELOS_SPEC_UNAPPROVED` — pending spec changes (possibly a direct human edit). Route through the challenger, review, and approval. Never revert the human's edit.
- `TELOS_CODE_CORRUPTED` — code changed outside the broker. Stop and report; the human recovers via Git or deliberately re-baselines with `telos init`. Never adopt the write.
- `TELOS_ANNOTATION_MISSING` / `TELOS_ANNOTATION_ORPHAN` — annotate the listed files against real rules, or have the human declare them untraced in `telos.toml`.
- `TELOS_ANNOTATION_MISMATCH` — the patch was reversed; resubmit with correct annotations intersecting the cited rules.
- `TELOS_RULE_NOT_IMPLEMENTED` — the spec is ahead: start the listed rules' red cycles with failing test-only patches.
- `TELOS_TEST_FIRST` — an unproven rule needs its witnessed failing test before any implementation, and no patch may introduce test references outside its own cycle; submit a test-only patch citing the rule.
- `TELOS_BASELINE_RED` — a new failing test is only attributable on a green baseline; finish or revert the in-flight cycle first.
- `TELOS_RED_EXPECTED` — the submitted test passes, so it proves nothing: strengthen it until it fails, or claim adoption with `--expect-pass` if the rule documents existing behavior.
- `TELOS_TEST_SEALED` — the patch touches a witnessed failing test: fix the implementation instead, or rewrite the test through another witnessed red with a test-only patch.
- `TELOS_RED_PENDING` — a witnessed failing test awaits its green witness; continue implementing through `telos apply`.
- `TELOS_RED_STALE` — sealed tests no longer match their red evidence. Stop and report; the state was tampered with.
- `TELOS_TESTS_FAILED` — make the configured test commands pass without weakening assertions.
- `TELOS_TRACEABILITY_GAP` — a cited rule does not exist in the approved spec; fix the citation or return to the spec cycle.
- Permission prompt declined on `spec approve` — the user refused the approval. Return to challenging the spec; do not retry unchanged.
- Permission prompt declined on `apply` — the user judged the patch is not behavior-preserving. Return to the spec cycle: strengthen the rule that should have forbidden the defect, then implement from the approved contract.
- Permission prompt declined on `apply --expect-pass` — the user rejected the adoption claim: the behavior is new, so it must enter through a witnessed failing test, or the rule itself is wrong and returns to the spec cycle.

## Reported bugs

An accepted bug is evidence about the specification. When the user reports wrong or missing behavior, start in `spec/` even if no existing scenario is violated — that only proves the contract was too weak to forbid the defect. Strengthen the rule or scenario, obtain review and approval, then implement. `telos apply` without a pending spec change is reserved for behavior-preserving work (refactors, test hardening) and raises a human permission prompt naming that claim.

## Semantic defects

If a rule proves impossible, contradictory, or wrong during implementation, do not work around it. Stop, report the defect, and restart the cycle: correct the spec, obtain fresh review and approval, then implement from the corrected contract.
