---
name: telos
description: Route every Telos repository request through persisted plans, approved task scope, execution and recovery.
---

# Telos router

Start with `telos status --json`. Every versioned path is governed: code, tests,
configuration, dependencies, assets, CI, documentation and specifications.
Use the CLI for all writes under `telos/`; generated work records are governed
by their own journal and do not require a recursive plan.

- For a new request, load and invoke `telos-brainstormer`, then
  `telos-challenger`. Use a small plan for a small change; do not bypass planning.
- For existing work, run `telos plan resume <PLN-id> --json`. Load the owning
  skill from the saved phase. Reuse the brief and decisions rather than
  restarting brainstorming. A fresh agent needs no prior chat transcript.
- For an approved ready task, load and invoke `telos-implementer`. Start it
  before modifying repository files. The approved revision authorizes its
  exact task deltas; covered changes require no additional human approval.
- For new scope, pause, invoke the brainstormer for the new question only,
  then the challenger. A new revision needs approval before execution.
- For `TELOS_RECOVERY_REQUIRED`, run `telos recover` and inspect the recovered
  plan. `TELOS_RECOVERY_CONFLICT` preserves external bytes: resolve the named
  conflict before retrying. Never erase the recovery journal to continue.
- For unplanned changes, inspect their origin. Use an explicit recovery plan
  to adopt or revert them, or restore unrelated work with existing permission.
  Do not silently assign them to an approved task or discard user work.

Honor `TELOS_PLAN_VERSION_STALE` by refreshing the plan and `TELOS_APPROVAL_STALE`
by reviewing a new revision. Reuse a mutation's `--request-id` when retrying a
lost response. An interrupted runner has an unknown outcome: inspect it before
an explicit retry, and never infer passing evidence from a missing result.

Use bounded model queries to resolve references and domain boundaries. A
context-map relation provides a published contract, not permission to modify
its supplier. Preserve scenario witness, sealed-test and ownership gates.
