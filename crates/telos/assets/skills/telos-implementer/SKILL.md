---
name: telos-implementer
description: Implement an approved Telos change with sealed same-byte red/green evidence, individual or grouped scenario execution, bindings, and reconciliation.
---

# Telos implementer

Never alter the approved delta and never edit any path under `telos/` manually. If the delta is wrong, stale, incomplete, or blocked by a constraint, stop and return it to the challenger for a fresh diff and human approval.

Use the following cycle for one scenario, or the grouped red phase below when several scenarios of the same approved intent share one implementation:

1. Run `telos pack <intent-id> --json`; record its literal `result.owner` and use only this bounded pack and targeted source/test files. Context-map mappings expose supplier contracts only: never inspect or modify supplier internals without a separately approved supplier change.
2. Add the smallest test named with its scenario id (`scn_NNNN`) and run `telos test SCN-NNNN --file <test-path> --json`.
3. Require literal `result.witness == "red"` — that seals the red witness. A compile error, a missing dependency, or a runner that executed zero tests is not a red: with `[test] report` configured Telos refuses those as `TELOS_TEST_NOT_EXECUTED`; without a report (`result.evidence == "exit-status"`) run the runner directly and confirm the count of executed tests is at least one before trusting the verdict. A crash, missing test, unrelated failure, or green first run is not a valid red.
4. Freeze the same test bytes. Do not edit the test after the sealed red, weaken assertions, or replace it with an easier test.
5. Make the minimum application-code change that satisfies the scenario.
6. Run `telos test SCN-NNNN --file <same-test-path> --json` again and require literal `result.witness == "green"` for the same test bytes. With a report, require `result.executed >= 1`.
7. Run `telos bind <code-path> <INT-id> --json` for each implementation file. Every production path must stay owned by one context. If a split is required, stop so the challenger can stage and obtain approval for that boundary design; never redesign the approved delta during implementation. Repeat from the bounded pack for the next scenario.
8. Run `telos change reconcile <CHG-id> --json` only after every new or changed active scenario has a sealed same-byte red/green pair and every legitimate code file is bound.

## Grouped red phase

Within the same already approved intent and change, a common implementation may satisfy several independent scenarios at once. Grouping their execution is routine ordering and requires no new human decision.

- Select the group from the bounded pack. Write **all** its tests before recording the first red, and finish formatting any shared test file first: the witness seals the entire file, not just one function.
- Run `telos test` for each grouped scenario and confirm a genuine assertion failure for that scenario. Every scenario needs its own red; a different failing test, compilation failure, skipped test or empty selection pays nothing.
- Freeze every proof file in the group. Implement the common behavior once, then record a green for every scenario on those same test bytes. Bind the implementation files and reconcile under the existing gates. Existing impacted scenarios are re-run by reconcile; unchanged scenarios do not require fabricated new reds.

Example: an approved win-detection intent contains `SCN-0001` (horizontal win) and `SCN-0002` (vertical win). Write both assertions in `tests/test_wins.py`, then:

```console
telos test SCN-0001 --file tests/test_wins.py --json
telos test SCN-0002 --file tests/test_wins.py --json
```

Both must genuinely fail before implementing the shared directional scan. After implementing it once, repeat the two commands and require green for each, with the test file unchanged throughout. The same ordering works with separate test files. Do not add the second test to a shared file after sealing the first test's red.

## Already satisfied before red

Report a green first run honestly. Never temporarily break working behavior, weaken an assertion, or manufacture a historical red. If the approved scenario is already satisfied and has no genuine red, stop and return that fact to the challenger. Under the existing strict policy it cannot acquire a strict red/green pair. The human may explicitly approve a configuration change to the existing advisory policy, which reports witness warnings; this changes the project's assurance policy and is never an automatic implementation workaround. If strict policy remains required, report the blocked witness. Do not use full reconciliation to bypass it or rewrite an existing proof.

Stop conditions, by code:

- `TELOS_SCENARIO_RED_EXPECTED`: stop and create a genuine failing test before code.
- `TELOS_TEST_SEALED`: stop; the test changed after red, so restore its sealed bytes or deliberately begin a new red witness. Never continue to green with changed bytes.
- `TELOS_TEST_NOT_EXECUTED`: stop; make the runner execute the scenario's test and write the report (fix the test name, the filter, the build, or the runner), never weaken or skip the test.
- `TELOS_APPROVAL_STALE`: stop and return to the challenger.
- `TELOS_ORPHAN_CODE`: bind a necessary file, or remove code that is not needed.
- `TELOS_CONTEXT_BOUNDARY_VIOLATION`: stop and return boundary design to the challenger.
- `TELOS_CONSTRAINT_FAILED`: stop and report the constraint instead of bypassing it.
- `TELOS_FILE_CLAIMED`: stop rather than editing another change's file.

Do not reconcile partially, modify tests to fit an implementation, self-approve, or directly repair `.tel` files. A failed stop condition returns to the owning phase.
