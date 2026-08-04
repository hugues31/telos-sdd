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
3. Every `RULE-NNN` is referenced by a file matching `test_files` **and** the configured `test_commands` pass — otherwise the spec is ahead: `TELOS_RULE_NOT_IMPLEMENTED`.

Root hashes are SHA-256 over sorted `path + NUL + content-hash + LF` records of LF-normalized content — the same construction for both trees, over two disjoint sets of paths, which is what makes "spec pending" and "code corrupted" mechanically distinguishable states with opposite exits.

## State

`.telos/state.json` is the only internal file and is committed to Git:

```json
{ "version": 1,
  "spec": { "root": "…", "files": { "spec/PRODUCT.md": "…" } },
  "code": { "root": "…", "files": { "app.go": "…" } },
  "review": "…" }
```

Because the state travels with the tree it describes, merges, pulls, and CI checkouts stay coherent by construction. A merge conflict on `state.json` is the correct signal that two branches diverged; the human resolves it in Git and, if needed, re-baselines through the gated `telos init`.

Phases are derived, never stored: `corrupted` > `spec_pending` > `awaiting_approval` (recorded review digest matches the current spec) > `implementing` (rules without tagged tests) > `clean`. There is no flow object and no completion command — green `telos verify` is completion.

## The single human gate

`telos spec review` validates the spec structure, records `review = rootHash(spec/**)`, and returns the exact pending content. Any later spec change makes the digest stale by recomputation. `telos spec approve --review <digest>` requires the triple equality `digest == state.review == rootHash(spec/**)`; it is checked independently by `telos guard` (which answers the command with an `ask` permission decision naming digest and files, or denies a stale one outright) and by the command itself. Approval snapshots `state.spec` and clears the review.

The only other gated command is `telos init` inside an already-initialized project, because re-baselining adopts the current tree and must never happen on an agent's initiative.

## The broker

`telos spec put` is the only write path for `spec/**` (path-validated, Markdown only). `telos apply` is the only write path for code: it requires both trees clean, cited rules that exist in the approved spec, applies the Git patch transactionally, then validates the **post-image** — every touched file must match an untraced pattern or carry a `telos:` annotation of existing rules intersecting the cited `--rule` references, or the patch is reversed. Patches may not touch `spec/**`, `telos.toml`, `.telos/**`, provider directories, or the managed instruction files: the broker protects its own hooks.

Spec structure is validated with the same rules everywhere: OBJ headings only in `spec/PRODUCT.md`, RULE headings only in domain files, ids globally unique, every rule tracing to an existing objective and containing a Gherkin block. Referential coherence is mechanical; semantic coherence is the challenger's job, locked in by the human gate.

## Provider boundary

Codex and Claude Code receive the same Skill content and equivalent agent roles through their native layouts. Provider hooks call `telos guard` (PreToolUse, stdin/stdout JSON): Edit/Write/apply_patch are denied, Bash is denied unless the first command is the `telos` binary (single line or heredoc, no shell chaining), and the two gated commands answer `ask`. Silence is allow. No provider adapter contains integrity logic.
