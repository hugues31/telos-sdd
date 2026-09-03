# Report-Backed Test Evidence Design

**Date:** 2026-09-03
**Status:** Approved in conversation
**Issues:** #9 (green witness on a zero-test run), #10 (red witness on a
compile or infrastructure failure)

## Objective

Make every green Telos records mean "a test named after the scenario
executed and passed", and every red mean "that test executed and failed",
instead of "the runner exited 0" and "the runner exited non-zero". The
mechanism is a structured JUnit XML report the runner writes and Telos
reads. It is opt-in per project; without a report the current
exit-status behaviour remains, but it is now labelled as such wherever a
proof is reported or sealed.

The report governs every surface that executes the runner: `telos test`,
reconcile gate 11, `reconcile --full`, and `rebuild status`. One verdict
function in `telos-core` decides; each surface only translates its answer.

## Non-goals

- Parsing runner stdout/stderr. Output is not reproducible across
  machines; a report file is.
- Any report format other than JUnit XML. A `format` field can be added
  later; nothing in this design depends on its absence.
- Toolchain-specific adapters (Cargo exit codes, pytest plugins, …).
- Backward compatibility with journals, locks, or configuration written by
  Telos ≤ 0.12. The project has no compatibility commitment yet; formats
  change outright and the lock version is bumped.

## Configuration

### `[test] report`

```toml
[test]
cmd = "cargo nextest run --profile telos {filter}"
report = "target/nextest/telos/junit.xml"
```

- `report` is a repository-relative path, validated with the same rules as
  a code path: normalized components, `/` separators, no `.`/`..`, no
  leading `/`, no `\` or `:`, and never under `telos/`. A path under
  `telos/` is `TELOS_PARSE_ERROR` with message
  `` invalid [test] report: `<path>` is under the spec tree `` and hint
  `` write the report outside telos/, e.g. `target/telos-report.xml` ``.
  Any other invalid path is the existing `RepoPath::parse` error.
- Empty (the default) means no report: exit-status evidence.
- `TestCfg` gains `pub report: String` with `#[serde(default)]`. The
  canonical TOML always serializes `report` (as `cmd` is today), so a
  fresh `config` read shows `report = ""`.
- `Config::validate_self` validates the path and the placeholder rule
  below, so `config --change`, `change approve`, `change reconcile`, and
  every runner-executing surface refuse an incoherent `[test]` before
  anything runs.

### `{report}` in `cmd`

- `{report}` is substituted with the configured path and passed as
  argument data through the same direct-argv path as `{filter}`: it may be
  a whole argument or part of a word, and no shell ever sees it.
- `{report}` in a template whose `report` is empty is `TELOS_PARSE_ERROR`
  with message `` invalid [test] cmd: `{report}` is used but `[test] report` is not configured ``
  and hint `` set [test] report to the repository-relative path the runner writes its JUnit XML report to ``.
- The converse is allowed: a runner that always writes to a fixed path (a
  nextest profile, a pytest.ini `addopts`) needs no placeholder. Telos
  still deletes and reads the configured path.
- The `command` display string substitutes both placeholders literally.

### Ripples

- `op edit config` gains a `test_report "<path>"` line, emitted right after
  `test_cmd`, always present (empty string when unset). The parser accepts
  it in any position, like the other config fields. The `edit config`
  op digest therefore changes for every config op; there is no
  compatibility to preserve.
- `config` read/write JSON: `test` is `{"cmd": "...", "report": "..."}`,
  both keys always present. The write payload's `test.report` is optional
  and defaults to `""`.
- The exact-keys tables and representative envelopes in `contracts.md`
  are updated accordingly.

## Report verdict (`telos-core/src/report.rs`)

A new module reads a JUnit XML file with the `roxmltree` crate (pure Rust,
no transitive dependencies) and answers one question: what did the report
say about scenario `SCN-NNNN`?

### Matching

- Every `<testcase>` element anywhere in the document is considered,
  whether the root is `<testsuites>` or `<testsuite>`.
- A testcase matches the scenario when its `name` attribute contains the
  discovery pattern `scn_NNNN` at an identifier boundary: the byte before
  the match, if any, is not `[A-Za-z0-9_]`. This is the exact predicate
  `witness::identifier_at` already applies to test files, so what
  discovery selects and what the report proves are one rule. `classname`
  and every other attribute are ignored.

### Testcase status

- A child `<failure>` or `<error>` element: **failed**. `<error>` counts
  as failed because it means the test ran and raised; JUnit-family
  reporters (JUnit, NUnit, pytest fixtures) file exceptions there.
- A child `<skipped>` element: **skipped**.
- Otherwise: **passed**.

### Verdict for one scenario

Evaluated over the matching testcases only, in this order:

| Condition | Verdict |
|---|---|
| ≥ 1 failed | `Failed { passed, failed }` |
| 0 failed, ≥ 1 skipped | `NotExecuted(Skipped { skipped })` |
| 0 failed, 0 skipped, ≥ 1 passed | `Passed { passed }` |
| no matching testcase | `NotExecuted(NoTestcase)` |

A skipped testcase alongside a passed one is *not* green: a scenario with
one executed test and one `#[ignore]`d test is exactly the shape #9 warns
about, and the honest answer is to make the author look.

Two more `NotExecuted` reasons come from the execution cycle below:
`ReportMissing` and `ReportInvalid(error)`.

### Frozen reason wordings

Each reason renders to one sentence, reused verbatim by every surface:

| Reason | Wording |
|---|---|
| `ReportMissing` | `` the runner did not write the report at `<path>` `` |
| `ReportInvalid` | `` the report at `<path>` is not valid JUnit XML: <error> `` |
| `NoTestcase` | `` the report at `<path>` contains no testcase named after `scn_NNNN` `` |
| `Skipped` | `` <n> testcase(s) named after `scn_NNNN` were skipped in the report at `<path>` `` |

`<path>` is the configured `[test] report`; `<error>` is `roxmltree`'s
error display; `<n>` is the skipped count.

## Proof execution cycle (`telos-core/src/exec.rs`)

One function, `run_proof`, replaces the direct `run_shell_with_filter`
calls on every runner-executing surface:

1. If a report is configured, delete the report file if it exists. A
   deletion failure other than "not found" is `TELOS_INTERNAL` naming the
   path; nothing runs.
2. Run the template with `{filter}` and `{report}` substituted, exactly as
   `run_shell_with_filter` does today.
3. If a report is configured, read and parse it. Absent is
   `ReportMissing`; unreadable or malformed is `ReportInvalid`.

The result carries the display `command`, the raw exit `status`, and the
evidence: either `ExitStatus` (no report configured) or `Report(parsed)`.
With a report, the exit status is diagnostic only: a runner that exits 1
because an unrelated test failed still yields green when the scenario's
testcase passed, and one that exits 0 having run nothing yields
`NotExecuted`.

The scenario verdict is computed by the caller from the parsed report, so
a `--full` run can ask one report about every active scenario.

## Surfaces

### `telos test`

Gate 5 ("a configured runner") also validates the `[test]` section via
`Config::validate_self` on the effective configuration.

| Evidence | Outcome |
|---|---|
| exit-status, exit 0 | green, as today |
| exit-status, exit ≠ 0 | red, as today |
| report, `Passed` | green |
| report, `Failed` | red |
| report, `NotExecuted(reason)` | `TELOS_TEST_NOT_EXECUTED`; nothing is journalled |

The `TELOS_TEST_NOT_EXECUTED` error's message is the reason wording; its
hint is `` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again ``,
with the scenario's own pattern and id substituted (`scn_0108`, `SCN-0108`).

The result object gains two keys, always present:

```json
{"scenario":"SCN-0108","witness":"green",
 "test":"tests/billing.rs::scn_0108_x","change":"CHG-0001",
 "command":"cargo nextest run --profile telos scn_0108_x",
 "evidence":"report","executed":1}
```

- `evidence` is `"report"` or `"exit-status"`.
- `executed` is the number of matching testcases that ran (passed plus
  failed) under `report`, and `null` under `exit-status`.

The human line appends `, 1 test executed` or `, exit status only`.

`test --all` runs every target in order as today. A `NotExecuted` verdict
aborts the loop with the error above; runs already journalled survive,
consistent with the existing "each run is journalled as it is taken"
rule.

### Journal `run` line

The line gains a mandatory final evidence word:

```text
run  SCN-0001 red "tests/billing.rs::scn_0001_x" "e69de29b…" exit-status
run  SCN-0001 green "tests/billing.rs::scn_0001_x" "e69de29b…" report
```

- `TestRun` gains `pub evidence: Evidence` with `Evidence::ExitStatus |
  Evidence::Report`; `as_str` yields `exit-status` / `report`.
- The parser reads it with `listed_word`, expected as
  `` `exit-status` or `report` ``, the same shape as `red`/`green`.
- The emitter always writes it. The canonical `JOURNAL_EXAMPLE` golden is
  updated.

### Reconcile gate 8 (sealed witness)

When the effective `[test] report` is non-empty, `witness_verdict`
considers only `report` runs of the scenario. When that verdict is not
`Intact` and the journal holds at least one `exit-status` run for the
scenario, the gate reports, instead of the verdict's own wording:

- code `TELOS_TEST_NOT_EXECUTED`
- message `` scenario SCN-0108's witness was taken by exit status; `[test] report` is configured ``
- hint `` run `telos test SCN-0108` again to record a report-backed red and green ``

Otherwise the existing `TELOS_SCENARIO_RED_EXPECTED` / `TELOS_TEST_SEALED`
wordings apply unchanged. Under `advisory`, the message joins
`witness_warnings` like the others. With no report configured, gate 8 is
unchanged and every run counts.

### Reconcile gate 11 (tests)

Per distinct `proves` target of the impacted scenarios, one `run_proof`,
then one verdict per impacted scenario the target proves, in scenario-id
order:

| Verdict | Refusal |
|---|---|
| `Failed` | `TELOS_INTEGRITY_VIOLATION`, existing message `` the test run for `<target>` failed: `<command>` `` and hint |
| `NotExecuted(reason)` | `TELOS_TEST_NOT_EXECUTED`, message `` the test run for `<target>` did not execute SCN-NNNN: <reason> ``, hint `` run the configured executable with the displayed arguments and inspect the report, then reconcile again `` |

Without a report, gate 11 is unchanged. `tests_run` still counts runner
invocations.

### `reconcile --full`

One `run_proof` with an empty `{filter}` when at least one intent is
active, as today. With a report, every active scenario that has at least
one `proves` binding is then judged against that single report, in
scenario-id order, with the same two refusals as gate 11 where `<target>`
is `the whole suite`. A draft-only model still invokes nothing.

### `rebuild status`

The global per-target pass runs `run_proof` once per distinct target and
caches its evidence and display command. Each scenario row then derives
its test rows from the cache:

- exit-status evidence: `green` iff exit 0, as today;
- report evidence: `green` iff the verdict for *that row's scenario* is
  `Passed`.

A target shared by two scenarios is still invoked once; the per-scenario
verdicts may differ, which is the point. Row keys stay exactly `test`,
`green`, `command`. A missing, unsafe, or stale target is still a red row
without running anything.

## Lock and `status`

### `telos.lock`

`LOCK_VERSION` becomes `3`. The file gains one mandatory line after
`spec_digest`:

```toml
version = 3
tool = "telos 0.13.0"
spec_digest = "sha256:…"
proof_evidence = "exit-status"
```

- `proof_evidence` is `"report"` when the sealing configuration's
  `[test] report` was non-empty, `"exit-status"` otherwise. `init`, a
  per-change reconcile (effective configuration), and `--full` all write
  it through `seal` / `lock_from_maps`.
- Reading requires the key; an unknown value is `TELOS_PARSE_ERROR`
  naming it. A `version = 2` lock is refused like `version = 1` today,
  with hint `` run `telos reconcile --full` to regenerate telos.lock ``
  (`--full` never reads the lock, so the hint is actionable).
- `Lock` gains `pub proof_evidence: Evidence`: the same two-variant enum
  the journal's `TestRun` carries, defined once in `telos-core`.

### `status`

`result` gains `proof_evidence`, read from the lock, always present:

```json
{"state":"coherent","changes":[],"drift":null,
 "proof_evidence":"exit-status","coverage":{…}}
```

The human summary gains a `proof evidence: exit-status` line. `status`
reports what the seal proved, not what the configuration says now; the
two differ only between turning the report on and the next reconcile.

## Error contract

`TELOS_TEST_NOT_EXECUTED` is the eighteenth frozen code. Emission cases:

| Emission | When | Hint |
|---|---|---|
| `TELOS_TEST_NOT_EXECUTED` | `telos test` with a report configured: the report is missing, invalid, names no testcase for the scenario, or every such testcase was skipped (message: the reason wording). Nothing is journalled. | `` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again `` |
| `TELOS_TEST_NOT_EXECUTED` | Gate 8 under strict policy with a report configured: the scenario's only witnesses were taken by exit status. | `` run `telos test SCN-NNNN` again to record a report-backed red and green `` |
| `TELOS_TEST_NOT_EXECUTED` | Gate 11 or `--full` with a report configured: a run's report does not prove an impacted (resp. active) scenario (message `` the test run for `<target>` did not execute SCN-NNNN: <reason> ``). | `` run the configured executable with the displayed arguments and inspect the report, then reconcile again `` |

The `test` and `status` result schemas, the `config` schemas, the `op edit
config` grammar, the lock format, the eleven-gates table, the `--full`
section, the `rebuild status` section, and the proof execution matrix in
`contracts.md` are all updated to match this document. The contract test
that pins the code count moves from 17 to 18.

## Documentation

- `docs/contracts.md`: every item above, plus a short "wiring a report"
  note in the `test` section listing one emitter per mainstream stack
  (`cargo nextest` junit profile, `pytest --junitxml={report}`,
  `gotestsum --junitfile {report}` behind a runner script, `jest-junit`,
  `phpunit --log-junit {report}`, `dotnet test --logger "junit;LogFilePath={report}"`)
  and the advice to keep the report path out of the `[code]`/`[tests]`
  globs and in `.gitignore`.
- `crates/telos/assets/skills/telos-implementer/SKILL.md`: step 3 states
  that a compile error, a missing dependency, or a runner that ran zero
  tests is not a red; without a report the implementer must run the
  runner directly and check the executed count. `TELOS_TEST_NOT_EXECUTED`
  joins the stop conditions: fix the test selection or the runner, never
  weaken the test.
- `crates/telos/assets/skills/telos/SKILL.md`: the new code routes to the
  implementer.
- `README.md`: one sentence introducing `[test] report`.
- `demo/`: unchanged (`cargo test` has no JUnit emitter; the demo keeps
  exit-status evidence).
- `telos test`, `exec.rs`, and `test.rs` module docs drop the "nothing
  detects a zero-test run" limitation and describe the report instead.

## Testing

Unit, in `telos-core`:

- `report.rs`: fixtures shaped like `cargo nextest`, `pytest`, and
  `jest-junit` output; a `<testsuite>` root; matching at identifier
  boundaries (`descn_0001x` and `xscn_0001` do not match, `scn_0001_x`
  does); `<failure>`, `<error>`, `<skipped>`; pass + skip is
  `NotExecuted`; no match; malformed XML.
- `exec.rs`: `{report}` substitution in every quote context; `{report}`
  without a configured report; the delete-run-read cycle with a runner
  that writes, and one that does not.
- `config.rs`: `report` round-trips; the two `validate_self` refusals.
- `parser.rs` / `emit.rs`: the `run` line with each evidence word, the
  updated `JOURNAL_EXAMPLE`, an unknown evidence word.
- `witness.rs`: `witness_verdict` filtered to report runs.
- `lock.rs`: `proof_evidence` renders, reads, rejects an unknown value,
  and a `version = 2` lock is refused with the new hint.

Integration, in `crates/telos/tests`, with a cross-platform fake runner
(a `sh` script on Unix, a `.bat` on Windows) that copies a prepared XML
file to `{report}` when the fixture exists and exits non-zero without
writing otherwise:

- `telos test`: green with `executed: 1`; red on a `<failure>`; each of the
  four `NotExecuted` cases leaves the journal untouched and returns the
  frozen envelope; exit 0 with no report file is `NotExecuted`; the exact
  journal line carries `report`; `--all` aborts on the first
  `NotExecuted` and keeps earlier runs.
- Gate 8: an exit-status red followed by enabling the report is refused
  with the frozen wording under strict and warned under advisory.
- Gate 11 and `--full`: a skipped test refuses the seal with
  `TELOS_TEST_NOT_EXECUTED`; a failing one keeps the existing violation.
- `rebuild status`: a skipped test is a red row; a shared target proving
  two scenarios reports them independently.
- `status`: `proof_evidence` reflects the lock; a fresh `init` seals
  `exit-status`; a reconcile with a report seals `report`.
- Contract pins: 18 codes, updated `test`/`status`/`config` envelopes and
  the `edit config` op bytes.

The existing exit-status fixtures (`git hash-object .fake-green`) keep
covering the no-report path unchanged, apart from the new keys and the
`exit-status` word on journal lines.
