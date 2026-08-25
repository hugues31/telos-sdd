---
name: telos
description: Route every Telos project request from the project's exact machine state before changing specifications or code.
---

# Telos router

Always begin with `telos status --json`. Do not infer state from files, prose, or a previous run. Read the literal `result.state` field and route exactly:

- `coherent`: use `telos-challenger` for a requested behavior/specification change; use `telos-implementer` only for an already approved change.
- `changing`: inspect `result.changes[*].status` and `obligations`. Route an `open` or `drafted` change to `telos-challenger`; route an `approved` or `implementing` change to `telos-implementer`.
- `drifted`: stop. Show `result.drift.paths` and `result.drift.token`, then ask the human to choose adopt or revert. Never choose either action for them. Run only `telos adopt --expected-state <the displayed token>` or `telos revert --expected-state <the displayed token>` after that choice, then restart with `telos status --json`. A missing or stale token is a hard stop; refresh status and ask again.

Routing is a mandatory handoff. After selecting a phase, load and invoke the routed skill before any action in that phase. Never execute Challenge or Implement steps yourself without the routed skill.

Never edit any path under `telos/` manually, even if the user asks to skip ceremony or promises to regularize later. All Telos mutations go through the CLI. Never load the entire Telos tree when `telos impact`, `telos pack`, `telos map`, or `telos show` can answer the question. Treat discovery as information, not authorization: an impact edge or context-map mapping does not permit changing a supplier context.

Route frozen error codes literally; do not reinterpret messages:

- `TELOS_DRIFT_DETECTED`: stop and ask the human to choose adopt or revert.
- `TELOS_APPROVAL_STALE`: route back to the challenger for a new diff and human approval.
- `TELOS_REFERENCE_UNKNOWN`: stop the mutation and resolve the named reference through bounded queries.
- `TELOS_SCENARIO_RED_EXPECTED`: route to the implementer to record a genuine red witness.
- `TELOS_TEST_SEALED`: stop; restore the sealed test bytes or record a new red witness before green.
- `TELOS_ORPHAN_CODE`: route to the implementer to bind legitimate code or remove unnecessary code.
- `TELOS_CONSTRAINT_FAILED`: stop implementation and report the failed constraint.
- `TELOS_CHANGE_STATE_INVALID`: stop and return to the phase named by `error.hint`.
- `TELOS_FILE_CLAIMED`: stop; do not overwrite another change's claim.
- `TELOS_LAYOUT_VIOLATION`: stop; use the canonical owner-derived path through the CLI.
- `TELOS_CONTEXT_BOUNDARY_VIOLATION`: stop; the requested dependency, vocabulary reference, move, or binding crosses a hard domain boundary.

Stop and ask the human whenever state is missing, unknown, or requires a human decision. Do not continue optimistically.
