# Task 2 implementer report — T9–T10

## Scope and RED evidence

Implemented only the Task 2 host integration surface in `codex/m3-agent-hosts`, then rebased it onto Task 1 (`15615cba0274fd0144cd813e332e7df8e510d5e8`) so the generated workflows expose `telos context`.

The recorded no-skill pressure baselines were used as the behavioral RED witnesses:

- Router: skipped `telos status --json`, accepted manual `telos/` drift, and rationalized implementation before adopt/revert, approval, witnessed TDD, binding, and reconcile.
- Challenger: broadened context, accepted self-approval, and crossed into application-code implementation before human digest approval.
- Implementer: chose code-first, changed the test after red, and considered direct `.tel` edits instead of preserving the approved delta and same-byte witness.

The first executable `agent_init` RED was run before production changes. Result: **11 failed, 2 passed**. Expected failures were Clap rejecting the absent `--agents`, the absent internal guard entry point, missing artifacts, and missing JSON preflight. A second focused RED for structurally invalid hook configuration failed because `telos/` had already been created; the preflight was then extended before rendering.

## Decisions

- `AgentHost` is a typed Clap `ValueEnum`; `--agents` is comma-delimited and normalized with deterministic set ordering. The successful init envelope remains unchanged.
- The three `SKILL.md` files under `crates/telos/assets/skills/` are the sole canonical sources and are copied byte-for-byte to Claude and Codex locations.
- Host formats follow the official documentation current on 2026-08-20:
  - Claude project skills: `.claude/skills/<name>/SKILL.md`; `PreToolUse` uses `hookSpecificOutput.permissionDecision` and supports `ask`.
  - Codex project skills: `.agents/skills/<name>/SKILL.md`; project instructions use root `AGENTS.md`; hooks use `.codex/hooks.json`; rules use `prefix_rule(... decision = "prompt")`.
- Mandatory ruling applied: the Codex guard never emits `ask`. It permits approve/adopt/revert to reach static native prompt rules. The challenger displays `result.digest` immediately before invoking the approval command, with the digest repeated in tool-call context. Claude emits `ask` directly. No digest CLI flag was invented.
- The preventive policy is Rust in the `telos` executable (`telos agent-guard --host ...`); generated projects require neither Python nor Node.
- Existing JSON is syntax- and shape-checked before any Telos or host write. Merge code preserves unrelated keys/hooks and normalizes the owned hook to one occurrence. `AGENTS.md` and `telos.rules` use owned markers and preserve surrounding content.

## Changes

- Added opt-in Claude/Codex parsing and hidden hook dispatch in `cli.rs`.
- Added preflight and requested-host rendering to `init` without changing its result envelope.
- Added canonical router, challenger, and implementer skills with literal state/error routing, explicit command order, phase boundaries, and stop conditions derived from the pressure failures.
- Added Claude settings/skill renderer and Codex skills/AGENTS/hooks/rules renderer.
- Added a common allow/deny/ask policy covering file tools, `apply_patch`, direct Bash mutations, read-only inspection, and native decision paths.
- Added `agent_init.rs` coverage for opt-in selection, duplicates, Clap rejection before writes, byte identity/frontmatter, pressure-derived instructions, guard decisions, official host output shapes, Rust rule semantics, merge preservation, owned-entry uniqueness, and syntax/shape preflight safety.

## Commands and results

All shell commands were invoked through `rtk`.

- Initial RED: `cargo test -p telos --test agent_init` → **11 failed, 2 passed**.
- Structural-preflight RED: focused `agent_init` test → **1 failed**, because partial `telos/` existed.
- Focused GREEN before rebase: `cargo test -p telos --test agent_init` → **14 passed**.
- Pre-rebase Telos test sweep: `cargo test -p telos --tests` → **224 passed, 2 ignored**.
- Rebase: `git rebase 15615cba0274fd0144cd813e332e7df8e510d5e8` → successful, no conflict; `context` wiring retained.
- Fresh post-rebase gate (single successful chain):
  - `cargo fmt --check` → exit 0.
  - `cargo test -p telos --test agent_init` → **14 passed**.
  - `cargo test -p telos --test cli_m1` → **14 passed**.
  - `cargo test --workspace` → **865 passed, 2 ignored**.

## Commits

- Pre-rebase implementation commit: `3474f6e` (`feat(cli): generate agent host skills and guards`).
- Rebased implementation commit: `b75ce434dcef7ddc3bf1e0312c62a0b819088c00`.

## Self-review and residual risks

Auto-review checked the complete diff against the brief, verified `git diff --check`, confirmed the Task 1 `Context` variant/module/dispatch survived the rebase, and found no out-of-scope source changes.

Residual operational risks are explicit:

- Hooks are preventive guardrails, not a security boundary; Codex documents that specialized tool paths may opt out.
- Project-local Codex hooks/rules require the project `.codex/` layer to be trusted, and new hook definitions require native trust review.
- Generated hook commands require the `telos` binary to be available on the host `PATH`.
- Bash mutation recognition is intentionally conservative and covers direct writes; drift detection remains the authoritative safety net for indirect or external writes.
