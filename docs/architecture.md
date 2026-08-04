# Architecture

Telos has four layers:

1. One public `telos` Skill owns the conversation, phase routing, and the single human approval.
2. Three specialized agents own challenging (`telos-challenger`), implementation (`telos-implementer`), and independent audit (`telos-verifier`).
3. The dependency-free Go CLI is the only mutation authority. It owns validation, hashing, patch application, and state.
4. Git owns history and recovery; provider hooks block direct-write paths and surface the human gate.

## Three invariants

Everything is recomputable from the working tree; `telos verify` re-derives all three on every run:

1. `rootHash(spec/**) == state.spec.root` — otherwise the spec is **pending** and must go through review and approval (adoption path; never restored by an agent).
2. `rootHash(code) == state.code.root` — otherwise the project is **corrupted** and must be recovered through Git or a deliberately human-gated `telos init` re-baseline (no adoption path).
3. Every `RULE-NNN` is proven by a witnessed red-green cycle: a file matching `test_files` references it, the broker saw that test fail on a green baseline before any implementation existed (`state.red` seals the exact failing bytes), the sealed test did not change on the way to green, and the configured `test_commands` pass. A rule with no referencing test is `TELOS_RULE_NOT_IMPLEMENTED`; a sealed red awaiting its green witness is `TELOS_RED_PENDING`.

Root hashes are SHA-256 over sorted `path + NUL + content-hash + LF` records of LF-normalized content — the same construction for both trees, over two disjoint sets of paths, which is what makes "spec pending" and "code corrupted" mechanically distinguishable states with opposite exits.

## State

`.telos/state.json` is the only internal file and is committed to Git:

```json
{ "version": 1,
  "spec": { "root": "…", "files": { "spec/PRODUCT.md": "…" } },
  "code": { "root": "…", "files": { "app.go": "…" } },
  "review": "…",
  "green": "…",
  "red": { "RULE-004": { "tests": { "auth_test.go": "…" } } } }
```

Every field records a decision or a witnessed observation, never derivable content: `spec.root` is the approved root, `code.root` the declared one, `review` the digest presented to the human, `green` the last code root at which the broker saw the suite pass, and `red` seals, per unproven rule, the exact test bytes the broker saw fail. Red evidence is cleared by the apply that witnesses the suite green, and swept on approval for rules the spec no longer contains.

Because the state travels with the tree it describes, merges, pulls, and CI checkouts stay coherent by construction. A merge conflict on `state.json` is the correct signal that two branches diverged; the human resolves it in Git and, if needed, re-baselines through the gated `telos init`.

Phases are derived, never stored: `corrupted` > `spec_pending` > `awaiting_approval` (recorded review digest matches the current spec) > `implementing` (rules without tagged tests, or sealed red evidence awaiting its green witness) > `clean`. There is no flow object and no completion command — green `telos verify` is completion.

## The single human gate

`telos spec review` validates the spec structure, records `review = rootHash(spec/**)`, and returns the exact pending content. Any later spec change makes the digest stale by recomputation. `telos spec approve --review <digest>` requires the triple equality `digest == state.review == rootHash(spec/**)`; it is checked independently by `telos guard` (which answers the command with an `ask` permission decision naming digest and files, or denies a stale one outright) and by the command itself. Approval snapshots `state.spec` and clears the review.

Three other commands are gated. `telos init` inside an already-initialized project, because re-baselining adopts the current tree and must never happen on an agent's initiative. `telos apply` while the project is clean: a patch that no pending spec change motivates is a claim of behavior preservation (refactor, test hardening) that only the human can accept — the prompt names the cited rules and patched files, and declining it routes the change through the spec cycle. And `telos apply --expect-pass`, the adoption claim: a rule documenting behavior the code already has can never be witnessed failing, so the human is prompted before a test that passes immediately counts as proof. Spec-driven applies stay silent; their human decision already happened at approval. This keeps refactors possible without spec churn while ensuring no code change ever happens without a named human decision: an accepted bug is spec evidence and must strengthen the contract, not slip through the refactor door.

## The broker

`telos spec put` is the only write path for `spec/**` (path-validated, Markdown only). `telos apply` is the only write path for code: it requires both trees clean, cited rules that exist in the approved spec, applies the Git patch transactionally, then validates the **post-image** — every touched file must match an untraced pattern or carry a `telos:` annotation of existing rules intersecting the cited `--rule` references, or the patch is reversed. Patches may not touch `spec/**`, `telos.toml`, `.telos/**`, provider directories, or the managed instruction files: the broker protects its own hooks.

The broker is also the arbiter of the red-green cycle. The first patch citing an unproven rule must be test-only and add a reference to that rule; the broker runs the suite and requires failure — on a green baseline, witnessed once and cached in `state.green`, so the red is attributable to the new test alone (`TELOS_BASELINE_RED` enforces one cycle at a time). A test the suite already passes is reversed (`TELOS_RED_EXPECTED`). The witnessed red seals the citing test files: any later patch touching them is refused (`TELOS_TEST_SEALED`) unless it is another test-only patch the suite fails again, which re-seals the rewritten bytes — only implementation patches may turn red into green. While red evidence is pending every apply runs the suite; the run that passes is the green witness that proves the sealed rules and lifts their seals. Test references are policed on every post-image, so a rule cannot become referenced outside its own cycle, and `--expect-pass` (human-gated) is the single, explicit path for rules that document existing behavior.

Spec structure is validated with the same rules everywhere: OBJ headings only in `spec/PRODUCT.md`, RULE headings only in domain files, ids globally unique, every rule tracing to an existing objective and containing a Gherkin block. Referential coherence is mechanical; semantic coherence is the challenger's job, locked in by the human gate.

## Provider boundary

Codex and Claude Code receive the same Skill content and equivalent agent roles through their native layouts. Provider hooks call `telos guard` (PreToolUse, stdin/stdout JSON): Edit/Write/apply_patch are denied, Bash is denied unless the first command is the `telos` binary (single line or heredoc, no shell chaining), and the gated commands answer `ask`. Silence is allow. No provider adapter contains integrity logic.
