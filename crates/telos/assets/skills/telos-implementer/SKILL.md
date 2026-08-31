---
name: telos-implementer
description: Implement an approved Telos change scenario by scenario with sealed same-byte red/green evidence, bindings, and reconciliation.
---

# Telos implementer

Never alter the approved delta and never edit any path under `telos/` manually. If the delta is wrong, stale, incomplete, or blocked by a constraint, stop and return it to the challenger for a fresh diff and human approval.

Work one scenario at a time in this order:

1. Run `telos pack <intent-id> --json`; record its literal `result.owner` and use only this bounded pack and targeted source/test files. Context-map mappings expose supplier contracts only: never inspect or modify supplier internals without a separately approved supplier change.
2. Add the smallest test named with its scenario id (`scn_NNNN`) and run `telos test SCN-NNNN --file <test-path> --json`.
3. Require literal `result.witness == "red"` — that seals the red witness. A crash, missing test, unrelated failure, or green first run is not a valid red.
4. Freeze the same test bytes. Do not edit the test after the sealed red, weaken assertions, or replace it with an easier test.
5. Make the minimum application-code change that satisfies the scenario.
6. Run `telos test SCN-NNNN --file <same-test-path> --json` again and require literal `result.witness == "green"` for the same test bytes.
7. Run `telos bind <code-path> <INT-id> --json` for each implementation file. Every production path must stay owned by one context. If a split is required, stop so the challenger can stage and obtain approval for that boundary design; never redesign the approved delta during implementation. Repeat from the bounded pack for the next scenario.
8. Run `telos change reconcile <CHG-id> --json` only after every impacted scenario has a sealed same-byte red/green pair and every legitimate code file is bound.

Stop conditions, by code:

- `TELOS_SCENARIO_RED_EXPECTED`: stop and create a genuine failing test before code.
- `TELOS_TEST_SEALED`: stop; the test changed after red, so restore its sealed bytes or deliberately begin a new red witness. Never continue to green with changed bytes.
- `TELOS_APPROVAL_STALE`: stop and return to the challenger.
- `TELOS_ORPHAN_CODE`: bind a necessary file, or remove code that is not needed.
- `TELOS_CONTEXT_BOUNDARY_VIOLATION`: stop and return boundary design to the challenger.
- `TELOS_CONSTRAINT_FAILED`: stop and report the constraint instead of bypassing it.
- `TELOS_FILE_CLAIMED`: stop rather than editing another change's file.

Do not reconcile partially, modify tests to fit an implementation, self-approve, or directly repair `.tel` files. A failed stop condition returns to the owning phase.
