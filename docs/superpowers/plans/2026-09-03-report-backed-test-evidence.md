# Report-Backed Test Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `telos test`, reconcile gate 11, `reconcile --full`, and `rebuild status` decide green/red from an optional JUnit XML report the runner writes, refuse to record a verdict when the scenario's test did not execute, and label every seal with the kind of evidence it carries.

**Architecture:** A new `telos-core::report` module parses JUnit XML and answers "what did this report say about SCN-NNNN". `telos-core::exec::run_proof` wraps the runner in a delete-run-read cycle and hands back a `ProofRun` whose `verdict(scenario)` every surface translates. The journal `run` line, `telos.lock`, and the `status`/`test` JSON results carry an `Evidence` word (`exit-status` | `report`) so a seal says how it was proven. A new frozen error code `TELOS_TEST_NOT_EXECUTED` covers every "the test did not run" outcome.

**Tech Stack:** Rust 2024 workspace (`crates/telos-core`, `crates/telos`), `roxmltree` for XML, `serde`/`toml`, `assert_cmd` integration tests driving the real `telos` binary against the `billing` corpus.

**Spec:** `docs/superpowers/specs/2026-09-03-report-backed-test-evidence-design.md`

## Global Constraints

- No backward compatibility with journals, locks, or configuration written by Telos ≤ 0.12: formats change outright, `LOCK_VERSION` becomes `3`.
- Every frozen wording in the spec is reproduced byte for byte in code, in `docs/contracts.md`, and in tests.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` must pass after every task (CI runs exactly these).
- Runner templates are parsed into a direct argv; `{report}` is data, never shell-evaluated, exactly like `{filter}`.
- Commit after every task with a conventional message and the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Work happens on branch `feat/test-report-evidence` (already created, spec committed).
- Prefix shell commands with `rtk` (the repo's token-saving wrapper): `rtk cargo test …`, `rtk git add …`.
- Windows CI runs `cargo test --workspace`; the fake runner has a `.bat` variant. If the Windows job fails on the report integration tests only, gate those tests with `#[cfg(unix)]` (the existing self-rewriting-runner test sets that precedent) and say so in the PR.

## File map

| File | Responsibility after this plan |
|---|---|
| `crates/telos-core/src/model/change.rs` | `Evidence` enum; `TestRun.evidence`; fixtures and `JOURNAL_EXAMPLE` golden |
| `crates/telos-core/src/syntax/parser.rs` | `run` line evidence word; `test_report` config field |
| `crates/telos-core/src/emit.rs` | `run` line evidence word; `test_report` config line (column width 11) |
| `crates/telos-core/src/config.rs` | `TestCfg.report`, `TestCfg::report_path`, `TestCfg::evidence`, `validate_test_cfg` |
| `crates/telos-core/src/report.rs` (new) | JUnit parsing, `ReportVerdict`, `NotExecuted` reasons and wordings |
| `crates/telos-core/src/witness.rs` | `names_scenario` (public boundary match); unchanged verdict logic |
| `crates/telos-core/src/exec.rs` | `{report}` placeholder; `run_proof`, `ProofRun`, `ProofVerdict` |
| `crates/telos-core/src/error.rs` | `TelosTestNotExecuted` |
| `crates/telos-core/src/lock.rs` | `LOCK_VERSION = 3`, `proof_evidence`, public `render` |
| `crates/telos-core/src/reconcile.rs` | gate 8 report-only witnesses; gate 11 and `--full` verdicts; lock evidence |
| `crates/telos/src/commands/test.rs` | verdict from `run_proof`; `evidence`/`executed` keys; `TELOS_TEST_NOT_EXECUTED` |
| `crates/telos/src/commands/rebuild.rs` | per-scenario report verdicts |
| `crates/telos/src/commands/status.rs` | `proof_evidence` key |
| `crates/telos/src/commands/config.rs` | `test.report` payload field |
| `crates/telos/src/commands/init.rs` | uses `Lock::render` instead of its own copy |
| `crates/telos/tests/common/mod.rs` | fake runner, report fixture builders |
| `crates/telos/tests/test_report.rs` (new) | report-backed `telos test`, gates 8/11, `--full`, `rebuild status`, `status` |
| `docs/contracts.md` | every frozen wording and schema above |
| `crates/telos/assets/skills/*/SKILL.md`, `README.md` | agent guidance and the one-line feature mention |

---

### Task 1: `Evidence` on the journal `run` line

**Files:**
- Modify: `crates/telos-core/src/model/change.rs` (after `impl Witness`, ~line 436; `TestRun` ~446; fixtures ~871-925; test ~1505)
- Modify: `crates/telos-core/src/model/mod.rs` (re-export)
- Modify: `crates/telos-core/src/syntax/parser.rs` (~line 90 consts; import at ~23-27; `journal_line` ~1992; tests ~4120-4145)
- Modify: `crates/telos-core/src/emit.rs` (`emit_journal_entry` ~713; tests ~1213-1265)
- Modify: `crates/telos-core/src/witness.rs` (test helper `run` ~524)
- Modify: `crates/telos-core/tests/change_file.rs` (literal `run` lines at 159, 160, 173, 175, 464, 465, 533)
- Modify: `crates/telos/src/commands/test.rs:397` (temporary `Evidence::ExitStatus`)
- Modify: `crates/telos/tests/test_bind.rs` (~line 410 journal line literal) and any other CLI test asserting a `run  SCN-…` line

**Interfaces:**
- Produces: `telos_core::model::Evidence { ExitStatus, Report }` with `as_str(self) -> &'static str` (`"exit-status"` / `"report"`); `TestRun { scenario, witness, test, oid, evidence: Evidence }`; fixtures `change::fixtures::run(witness)` (exit-status) and `run_with(witness, evidence)`.

- [ ] **Step 1: Write the failing parser and emitter tests**

In `crates/telos-core/src/syntax/parser.rs`, in the journal tests near `a_run_line_carries_the_scenario_the_verdict_the_test_and_the_oid`, change that test's source line to end with ` exit-status` and its expected `TestRun` to carry `evidence: Evidence::ExitStatus`, then add:

```rust
#[test]
fn a_run_line_carries_its_evidence_word() {
    let src = journalled("  run  SCN-0107 green \"tests/billing.rs::scn_0107\" \"cafe\" report\n");
    let JournalEntry::Run(run) = &parse(&src).journal[0] else {
        panic!("a run line");
    };
    assert_eq!(run.evidence, Evidence::Report);
}
```

Also append ` exit-status` before `\n` in `a_test_locator_may_be_a_bare_path`'s source.

In `crates/telos-core/src/emit.rs`, test `a_run_line_is_the_keyword_the_scenario_the_verdict_the_test_and_the_oid`: append ` exit-status` before `\n` in both expected strings. Add:

```rust
#[test]
fn a_report_backed_run_ends_in_the_report_word() {
    assert!(
        emit_journal_entry(&run_with(Witness::Green, Evidence::Report)).ends_with("\" report\n")
    );
}
```

In `a_test_reference_without_a_name_is_emitted_as_a_bare_path` add `evidence: Evidence::ExitStatus,` to the `TestRun` literal.

In `crates/telos-core/tests/change_file.rs`, append ` exit-status` before `\n` (or before the closing quote where there is no `\n`) on every literal `run` line listed above, and add:

```rust
#[test]
fn a_run_line_without_a_known_evidence_word_is_rejected() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status implementing\n",
        "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        "\n",
        "  run  SCN-0107 red \"tests/billing.rs\" \"cafe\" maybe\n",
        "}\n",
    );
    assert_reports(&parse_err(src), "expected `exit-status` or `report`, found `maybe`");
}

#[test]
fn a_run_line_without_any_evidence_word_is_rejected() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status implementing\n",
        "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        "\n",
        "  run  SCN-0107 red \"tests/billing.rs\" \"cafe\"\n",
        "}\n",
    );
    assert_reports(&parse_err(src), "expected `exit-status` or `report`");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos-core journal 2>&1 | tail -20`
Expected: compile errors (`Evidence` and `run_with` do not exist).

- [ ] **Step 3: Add `Evidence` to the model**

In `crates/telos-core/src/model/change.rs`, right after `impl Witness { … }`:

```rust
/// How a run's verdict was decided.
///
/// `ExitStatus` is the runner's exit code alone: 0 read as green, anything
/// else as red, with nothing saying whether a test ran at all. `Report` is a
/// JUnit XML report the runner wrote, in which a testcase named after the
/// scenario passed or failed -- the only reading under which green means
/// "executed and passed". The word is part of the `run` line so a reconcile
/// can tell the two apart (`docs/contracts.md`, gate 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    ExitStatus,
    Report,
}

impl Evidence {
    /// The keyword this evidence is written as on a `run` line, in
    /// `telos.lock`'s `proof_evidence`, and in `--json` results.
    pub fn as_str(self) -> &'static str {
        match self {
            Evidence::ExitStatus => "exit-status",
            Evidence::Report => "report",
        }
    }
}
```

Add `pub evidence: Evidence,` as the last field of `TestRun`, and extend its doc: "`evidence` says whether the verdict came from the exit status or a report."

In `fixtures`: add `evidence: Evidence::ExitStatus,` to `run(witness)`'s literal and add

```rust
/// One run of the example's scenario, on either verdict and either kind
/// of evidence.
pub(crate) fn run_with(witness: Witness, evidence: Evidence) -> JournalEntry {
    JournalEntry::Run(TestRun {
        scenario: ScenarioId(1),
        witness,
        test: billing_test(),
        oid: run_oid(),
        evidence,
    })
}
```

Append ` exit-status` to both `run` lines of `JOURNAL_EXAMPLE`. Add `evidence: Evidence::ExitStatus,` to the `TestRun` literal in the test `runs_for_ignores_other_scenarios_and_bind_lines` (~line 1505).

In `crates/telos-core/src/model/mod.rs`, add `Evidence` to the `pub use change::{…}` list.

- [ ] **Step 4: Parse and emit the word**

`crates/telos-core/src/syntax/parser.rs`: add `Evidence` to the `use crate::model::{…}` import. Next to `WITNESSES`/`WITNESS_WORDS` add:

```rust
const EVIDENCES: [(&str, Evidence); 2] = [
    ("exit-status", Evidence::ExitStatus),
    ("report", Evidence::Report),
];
const EVIDENCE_WORDS: &str = "`exit-status` or `report`";
```

In `journal_line`, after `let oid = self.expect_str(…)?;` and before `self.end_of_field()?;`, add `let evidence = self.evidence()?;` and put `evidence` into the `TestRun` literal. Next to `fn witness`, add:

```rust
/// The two kinds of evidence a run may carry.
fn evidence(&mut self) -> Result<Evidence, Diagnostic> {
    self.listed_word(EVIDENCE_WORDS, &EVIDENCES)
}
```

`crates/telos-core/src/emit.rs`, `emit_journal_entry`: the `run` arm becomes

```rust
w!(
    out,
    "{} {} {} {} {}\n",
    run.scenario,
    run.witness.as_str(),
    quote(&run.test.to_string()),
    quote(&run.oid.0),
    run.evidence.as_str()
);
```

and its doc comment gains: "The evidence word closes the line: `exit-status` or `report`."

`crates/telos-core/src/witness.rs` test helper `run(...)`: add `evidence: Evidence::ExitStatus,` and import `Evidence`.

`crates/telos/src/commands/test.rs:397`: add `evidence: Evidence::ExitStatus,` to the `TestRun` literal and `Evidence` to the `telos_core::model::{…}` import (Task 6 replaces this).

- [ ] **Step 5: Update the CLI journal-line literals**

Run: `rtk grep -rn 'run  SCN\|run  {SCN}' crates/telos/tests`
For every hit that asserts the exact bytes of a `run` line (e.g. `test_bind.rs` `test_appends_the_exact_journal_line_to_the_owning_change`), append ` exit-status` before the closing `\n` of the expected line. Hits that only search for `run  SCN-0108 red` as a substring need no change.

- [ ] **Step 6: Run the whole workspace**

Run: `rtk cargo test --workspace 2>&1 | tail -30`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
rtk git add -A crates && rtk git commit -m "feat(journal): record the kind of evidence on every run line

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: `[test] report` configuration and the `{report}` rule

**Files:**
- Modify: `crates/telos-core/src/config.rs` (`TestCfg` ~132; `validate_self` ~63; tests)
- Modify: `crates/telos-core/src/emit.rs` (config op ~661-690; `width` module ~73-90)
- Modify: `crates/telos-core/src/syntax/parser.rs` (`config_op` ~1786)
- Modify: `crates/telos/src/commands/config.rs` (`PayloadTest`, `TestCfg` literal)
- Modify: `crates/telos/tests/config.rs`, `crates/telos/tests/contracts.rs` (~1067-1130), `crates/telos/src/commands/init.rs` (~1265 test bytes)
- Modify: `docs/contracts.md` (`config` section ~1011-1060)

**Interfaces:**
- Produces: `TestCfg { cmd: String, report: String }`; `TestCfg::report_path(&self) -> Result<Option<RepoPath>, TelosError>`; `TestCfg::evidence(&self) -> Evidence`; `config::validate_test_cfg(&TestCfg) -> Result<(), TelosError>` (called by `Config::validate_self`).

- [ ] **Step 1: Write the failing config tests**

In `crates/telos-core/src/config.rs` tests, extend `full_toml_round_trips_every_field` with `report = "target/telos-report.xml"` under `[test]` and `assert_eq!(config.test.report, "target/telos-report.xml");`. Extend `empty_toml_yields_every_default` with `assert_eq!(config.test.report, "");`. Add:

```rust
#[test]
fn a_report_under_the_spec_tree_is_refused() {
    let config = Config {
        test: TestCfg {
            cmd: "runner {filter}".to_string(),
            report: "telos/report.xml".to_string(),
        },
        ..Config::default()
    };
    let error = config.validate_self().unwrap_err();
    assert_eq!(error.code, ErrorCode::TelosParseError);
    assert_eq!(
        error.message,
        "invalid [test] report: `telos/report.xml` is under the spec tree"
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("write the report outside telos/, e.g. `target/telos-report.xml`")
    );
}

#[test]
fn a_report_placeholder_without_a_report_is_refused() {
    let config = Config {
        test: TestCfg {
            cmd: "runner --junit {report} {filter}".to_string(),
            report: String::new(),
        },
        ..Config::default()
    };
    let error = config.validate_self().unwrap_err();
    assert_eq!(error.code, ErrorCode::TelosParseError);
    assert_eq!(
        error.message,
        "invalid [test] cmd: `{report}` is used but `[test] report` is not configured"
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("set [test] report to the repository-relative path the runner writes its JUnit XML report to")
    );
}

#[test]
fn report_path_and_evidence_follow_the_report_field() {
    let unset = TestCfg::default();
    assert_eq!(unset.report_path().unwrap(), None);
    assert_eq!(unset.evidence(), Evidence::ExitStatus);

    let set = TestCfg {
        cmd: String::new(),
        report: "target/telos-report.xml".to_string(),
    };
    assert_eq!(
        set.report_path().unwrap(),
        Some(RepoPath::new("target/telos-report.xml"))
    );
    assert_eq!(set.evidence(), Evidence::Report);
    assert!(
        TestCfg {
            cmd: String::new(),
            report: "../escape.xml".to_string()
        }
        .report_path()
        .is_err()
    );
}
```

Add `use crate::ids::RepoPath; use crate::model::Evidence;` to the test module imports as needed.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos-core config:: 2>&1 | tail -20`
Expected: compile errors (`report` field, `report_path`, `evidence` missing).

- [ ] **Step 3: Implement the field and its validation**

In `crates/telos-core/src/config.rs`:

```rust
/// `[test]`: the command used to run the test suite, and the JUnit XML
/// report it writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TestCfg {
    #[serde(default)]
    pub cmd: String,
    /// `[test] report = "..."` -- the repository-relative path of the JUnit
    /// XML report the runner writes. Empty means no report: verdicts are
    /// read from the exit status alone.
    #[serde(default)]
    pub report: String,
}

impl TestCfg {
    /// The configured report as a validated repository path, `None` when
    /// unset. Validity is the code-path rule: normalized, `/`-separated,
    /// nothing under `telos/`.
    pub fn report_path(&self) -> Result<Option<RepoPath>, TelosError> {
        if self.report.is_empty() {
            return Ok(None);
        }
        let path = RepoPath::parse(self.report.clone())?;
        if path.first_component() == Some("telos") {
            return Err(TelosError::new(
                ErrorCode::TelosParseError,
                format!("invalid [test] report: `{}` is under the spec tree", self.report),
            )
            .hint("write the report outside telos/, e.g. `target/telos-report.xml`"));
        }
        Ok(Some(path))
    }

    /// The kind of evidence runs under this configuration produce.
    pub fn evidence(&self) -> Evidence {
        if self.report.is_empty() {
            Evidence::ExitStatus
        } else {
            Evidence::Report
        }
    }
}

/// `[test] report` must be a code path, and `{report}` in `cmd` requires it.
pub(crate) fn validate_test_cfg(test: &TestCfg) -> Result<(), TelosError> {
    if test.report_path()?.is_none() && test.cmd.contains("{report}") {
        return Err(TelosError::new(
            ErrorCode::TelosParseError,
            "invalid [test] cmd: `{report}` is used but `[test] report` is not configured",
        )
        .hint("set [test] report to the repository-relative path the runner writes its JUnit XML report to"));
    }
    Ok(())
}
```

Add `use crate::ids::RepoPath; use crate::model::Evidence;` at the top. In `Config::validate_self`, add `validate_test_cfg(&self.test)?;` after the two `compile_globs` calls, and extend its doc: "The `[test]` section is validated here too: the report path and the `{report}` placeholder rule."

- [ ] **Step 4: Emit and parse `test_report` in `op edit config`**

`crates/telos-core/src/emit.rs`: in `mod width` add `/// \`code_glob\`, \`test_glob\`, \`test_cmd\`, \`test_report\`, \`tdd\`, \`agent_host\`.\n pub const CONFIG: usize = 11;`. Rewrite the `StagedOp::EditConfig` arm so every field goes through `keyword`:

```rust
StagedOp::EditConfig(config) => {
    let mut config = config.clone();
    config.normalize();
    let mut out = String::from("op edit config {\n");
    for glob in &config.code.globs {
        keyword(&mut out, 1, "code_glob", width::CONFIG);
        w!(out, "{}\n", quote(glob));
    }
    for glob in &config.tests.globs {
        keyword(&mut out, 1, "test_glob", width::CONFIG);
        w!(out, "{}\n", quote(glob));
    }
    keyword(&mut out, 1, "test_cmd", width::CONFIG);
    w!(out, "{}\n", quote(&config.test.cmd));
    keyword(&mut out, 1, "test_report", width::CONFIG);
    w!(out, "{}\n", quote(&config.test.report));
    keyword(&mut out, 1, "tdd", width::CONFIG);
    w!(
        out,
        "{}\n",
        match config.policy.tdd {
            crate::config::TddPolicy::Strict => "strict",
            crate::config::TddPolicy::Advisory => "advisory",
        }
    );
    for host in &config.agents.hosts {
        keyword(&mut out, 1, "agent_host", width::CONFIG);
        w!(
            out,
            "{}\n",
            match host {
                crate::config::AgentHost::Claude => "claude",
                crate::config::AgentHost::Codex => "codex",
            }
        );
    }
    out.push_str("}\n");
    out
}
```

(Keep whatever the existing arm does after the hosts loop — read it first; only the field lines change. `keyword(out, 1, …)` writes the two-space indent, the word padded to 11, then one space, so `code_glob   "…"`, `test_report "…"`, `tdd         strict`.)

`crates/telos-core/src/syntax/parser.rs` `config_op`: after the `test_cmd` branch add

```rust
} else if self.at_kw("test_report") {
    self.advance();
    config.test.report = self.expect_str("a test report path")?.node;
```

- [ ] **Step 5: Accept `report` in the CLI payload**

`crates/telos/src/commands/config.rs`:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadTest {
    cmd: String,
    #[serde(default)]
    report: String,
}
```

and in `stage`: `test: TestCfg { cmd: payload.test.cmd, report: payload.test.report },`.

- [ ] **Step 6: Update the pinned bytes and the contract**

Run: `rtk cargo test --workspace 2>&1 | grep -E "^test .*FAILED|panicked" | head -30`

Fix each pin to the new canonical form:
- `crates/telos/tests/config.rs`: every expected TOML gains `report = ""` (or the configured value) right after the `cmd = …` line; the `op edit config` bytes at ~line 150 become `"op edit config {\n    code_glob   \"src/**/*.rs\"\n    test_glob   \"tests/**/*.rs\"\n    test_cmd    \"cargo test {filter}\"\n    test_report \"\"\n    tdd         advisory\n    agent_host  claude\n    agent_host  codex\n  }"`; the `source.replace("code_glob  \"src/**/*.rs\"", …)` at ~274 becomes `"code_glob   \"src/**/*.rs\""` → `"code_glob   \"[\""`; every JSON `"test": {"cmd": …}` gains `"report": ""`.
- `crates/telos/tests/contracts.rs` ~1085 and ~1118: `"test": {"cmd": "cargo test {filter}", "report": ""}`.
- `crates/telos/src/commands/init.rs` ~1265: the expected bytes gain `report = \"\"\n` after `cmd = \"\"\n` (init writes the canonical TOML through `emit_config`).
- Any other test asserting `[test]\ncmd = …` bytes or the `test_cmd   ` op line: same rule.
- The padding is `keyword`'s: each field name padded to 11 columns plus one space. If an assertion shows a one-space difference, the emitter is right and the literal in the test is what to fix.

`docs/contracts.md`, `### config [--change CHG-NNNN]`: in both JSON examples change `"test": {"cmd": "cargo test {filter}"}` to `"test": {"cmd": "cargo test {filter}", "report": ""}`. After the sentence ending "JSON output is the complete typed configuration:" add a paragraph:

```markdown
`test.report` is the repository-relative path of the JUnit XML report the
runner writes (`""` when unset — exit-status evidence; see the `test`
section). It must be a code path outside `telos/`: a path under the spec
tree is `TELOS_PARSE_ERROR` with message
`` invalid [test] report: `<path>` is under the spec tree `` and hint
`` write the report outside telos/, e.g. `target/telos-report.xml` ``. A
`{report}` placeholder in `test.cmd` requires it: otherwise
`TELOS_PARSE_ERROR` with message
`` invalid [test] cmd: `{report}` is used but `[test] report` is not configured ``
and hint
`` set [test] report to the repository-relative path the runner writes its JUnit XML report to ``.
Both checks run wherever the configuration is validated (the validation
matrix below), so no surface executes a runner under an incoherent `[test]`.
On the write side `test.report` is optional and defaults to `""`.
```

In the "Write mode reads one complete JSON object" paragraph, change "Partial objects, unknown fields" to "Partial objects (`test.report` excepted), unknown fields".

In the `change` section describing the `edit config` op line format, if any example shows `test_cmd   "…"`, update it to the width-11 form and add a `test_report ""` line after it. Run `rtk grep -n "test_cmd" docs/contracts.md` to find them.

- [ ] **Step 7: Run the whole workspace**

Run: `rtk cargo test --workspace 2>&1 | tail -30`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
rtk git add -A crates docs && rtk git commit -m "feat(config): add [test] report and the {report} placeholder rule

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: The report module and the `TELOS_TEST_NOT_EXECUTED` code

**Files:**
- Create: `crates/telos-core/src/report.rs`
- Modify: `crates/telos-core/src/lib.rs` (`pub mod report;`)
- Modify: `crates/telos-core/Cargo.toml` (`roxmltree`)
- Modify: `crates/telos-core/src/witness.rs` (`pub fn names_scenario`)
- Modify: `crates/telos-core/src/error.rs` (variant + frozen test)
- Modify: `crates/telos/tests/contracts.rs` (~430-475: live set and counts)
- Modify: `docs/contracts.md` (~112-140: prose count and canonical table)

**Interfaces:**
- Produces: `report::Report::parse(xml: &str) -> Result<Report, String>`; `Report::verdict(&self, ScenarioId) -> ReportVerdict`; `ReportVerdict { Passed { passed: u32 }, Failed { passed: u32, failed: u32 }, NotExecuted(NotExecuted) }`; `NotExecuted { ReportMissing, ReportInvalid(String), NoTestcase, Skipped(u32) }` with `message(&self, report: &RepoPath, scenario: ScenarioId) -> String`; `witness::names_scenario(name: &str, ScenarioId) -> bool`; `ErrorCode::TelosTestNotExecuted`.

- [ ] **Step 1: Add the dependency and the module skeleton**

Run: `rtk cargo add roxmltree -p telos-core` (pins the current 0.x line in `crates/telos-core/Cargo.toml`; `Cargo.lock` updates). Add `pub mod report;` to `crates/telos-core/src/lib.rs` (alphabetical, after `rebuild`).

- [ ] **Step 2: Write the failing report tests**

Create `crates/telos-core/src/report.rs` with the tests first (the implementation follows in Step 4):

```rust
//! JUnit XML reports: what the runner wrote, read for one scenario.
//!
//! The report is the one runner artifact telos parses. Its stdout is not
//! reproducible across machines; a JUnit file is a stable, structured
//! artifact nearly every runner can emit, and it is the only reading under
//! which a green verdict means "the scenario's test executed and passed"
//! rather than "the process exited 0" (`docs/contracts.md`, `test`).

use crate::ids::{RepoPath, ScenarioId};
use crate::witness::{names_scenario, scenario_pattern};

#[cfg(test)]
mod tests {
    use super::*;

    fn nextest() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="1" errors="0">
  <testsuite name="billing::tests" tests="3" failures="1" errors="0" skipped="0">
    <testcase name="scn_0091_issued_invoice_is_open" classname="billing::tests" time="0.001"/>
    <testcase name="scn_0107_full_payment_settles_the_invoice" classname="billing::tests" time="0.002">
      <failure message="assertion failed"><![CDATA[left: "open" right: "settled"]]></failure>
    </testcase>
    <testcase name="unrelated_helper_test" classname="billing::tests" time="0.000"/>
  </testsuite>
</testsuites>
"#
    }

    fn pytest() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8"?><testsuites><testsuite name="pytest" errors="1" failures="0" skipped="1" tests="3" time="0.03"><testcase classname="tests.test_billing" name="scn_0108_cancel_open_invoice" time="0.001"><skipped type="pytest.skip" message="not yet"/></testcase><testcase classname="tests.test_billing" name="scn_0109_refund" time="0.001"><error message="fixture error">boom</error></testcase><testcase classname="tests.test_billing" name="scn_0110_close" time="0.001"/></testsuite></testsuites>"#
    }

    fn jest_junit() -> &'static str {
        r#"<testsuite name="billing" tests="2" failures="0" errors="0" skipped="0">
  <testcase classname="billing cancel" name="scn_0108_cancel_open_invoice closes it" time="0.01"/>
  <testcase classname="billing cancel" name="scn_0108_cancel_open_invoice keeps the balance" time="0.01"/>
</testsuite>"#
    }

    #[test]
    fn a_passed_testcase_named_after_the_scenario_is_passed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(91)),
            ReportVerdict::Passed { passed: 1 }
        );
    }

    #[test]
    fn a_failure_child_is_failed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(107)),
            ReportVerdict::Failed { passed: 0, failed: 1 }
        );
    }

    #[test]
    fn an_error_child_is_failed_too() {
        let report = Report::parse(pytest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(109)),
            ReportVerdict::Failed { passed: 0, failed: 1 }
        );
    }

    #[test]
    fn a_skipped_testcase_is_not_executed() {
        let report = Report::parse(pytest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::Skipped(1))
        );
    }

    #[test]
    fn a_pass_next_to_a_skip_is_still_not_executed() {
        let xml = r#"<testsuite><testcase name="scn_0108_a"/><testcase name="scn_0108_b"><skipped/></testcase></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::Skipped(1))
        );
    }

    #[test]
    fn a_failure_outranks_a_skip() {
        let xml = r#"<testsuite><testcase name="scn_0108_a"><failure/></testcase><testcase name="scn_0108_b"><skipped/></testcase><testcase name="scn_0108_c"/></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::Failed { passed: 1, failed: 1 }
        );
    }

    #[test]
    fn no_matching_testcase_is_not_executed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::NoTestcase)
        );
    }

    #[test]
    fn a_testsuite_root_counts_every_matching_case() {
        let report = Report::parse(jest_junit()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::Passed { passed: 2 }
        );
    }

    #[test]
    fn matching_respects_the_identifier_boundary() {
        let xml = r#"<testsuite><testcase name="descn_0108x"/><testcase name="xscn_0108"/><testcase name="test::scn_0108_y"/></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::Passed { passed: 1 }
        );
    }

    #[test]
    fn malformed_xml_is_an_error_carrying_the_parser_message() {
        let error = Report::parse("<testsuites><testcase name=\"scn_0001\"").unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn every_reason_has_its_frozen_wording() {
        let report = RepoPath::new("target/telos-report.xml");
        let scenario = ScenarioId(108);
        assert_eq!(
            NotExecuted::ReportMissing.message(&report, scenario),
            "the runner did not write the report at `target/telos-report.xml`"
        );
        assert_eq!(
            NotExecuted::ReportInvalid("unexpected end of stream".to_string())
                .message(&report, scenario),
            "the report at `target/telos-report.xml` is not valid JUnit XML: unexpected end of stream"
        );
        assert_eq!(
            NotExecuted::NoTestcase.message(&report, scenario),
            "the report at `target/telos-report.xml` contains no testcase named after `scn_0108`"
        );
        assert_eq!(
            NotExecuted::Skipped(2).message(&report, scenario),
            "2 testcase(s) named after `scn_0108` were skipped in the report at `target/telos-report.xml`"
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `rtk cargo test -p telos-core report:: 2>&1 | tail -20`
Expected: compile errors (`Report`, `ReportVerdict`, `NotExecuted`, `names_scenario` missing).

- [ ] **Step 4: Implement the module and the boundary match**

In `crates/telos-core/src/witness.rs`, next to `scenario_pattern`, add:

```rust
/// Whether `name` -- a test function, a JUnit `testcase` name -- is named
/// after `scenario`: the `scn_NNNN` pattern occurs in it at an identifier
/// boundary, the same predicate discovery applies to test files. One rule
/// for what discovery selects and what a report proves.
pub fn names_scenario(name: &str, scenario: ScenarioId) -> bool {
    identifier_at(name.as_bytes(), scenario_pattern(scenario).as_bytes()).is_some()
}
```

Above the tests in `crates/telos-core/src/report.rs` add:

```rust
/// One parsed JUnit report: every `testcase` it holds, wherever nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    cases: Vec<TestCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCase {
    name: String,
    status: CaseStatus,
}

/// A `testcase`'s outcome, read from its child elements: `failure` or
/// `error` is failed (the test ran and raised), `skipped` is skipped,
/// nothing is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseStatus {
    Passed,
    Failed,
    Skipped,
}

/// What a report says about one scenario, over the testcases named after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportVerdict {
    /// At least one passed, none failed, none skipped.
    Passed { passed: u32 },
    /// At least one failed.
    Failed { passed: u32, failed: u32 },
    /// Nothing proves the scenario ran.
    NotExecuted(NotExecuted),
}

/// Why a run proved nothing about a scenario. Each reason renders to one
/// frozen sentence ([`NotExecuted::message`]) that every surface reuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotExecuted {
    /// No file at the configured report path after the run.
    ReportMissing,
    /// The file exists but is not readable JUnit XML; carries the parser's
    /// own message.
    ReportInvalid(String),
    /// No `testcase` is named after the scenario.
    NoTestcase,
    /// Testcases named after the scenario exist, none failed, and this
    /// many were skipped.
    Skipped(u32),
}

impl Report {
    /// Parses JUnit XML. Every `testcase` element anywhere in the document
    /// counts, so a `testsuites` root and a bare `testsuite` root read the
    /// same. The error is `roxmltree`'s message, kept for the
    /// `ReportInvalid` wording.
    pub fn parse(xml: &str) -> Result<Report, String> {
        let document = roxmltree::Document::parse(xml).map_err(|error| error.to_string())?;
        let cases = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "testcase")
            .map(|node| TestCase {
                name: node.attribute("name").unwrap_or_default().to_string(),
                status: case_status(node),
            })
            .collect();
        Ok(Report { cases })
    }

    /// The verdict for `scenario`, in the frozen order: any failure is
    /// `Failed`; otherwise any skip is `NotExecuted(Skipped)` -- a skipped
    /// twin next to a passed test is exactly the shape a zero-test green
    /// hides in; otherwise any pass is `Passed`; otherwise `NoTestcase`.
    pub fn verdict(&self, scenario: ScenarioId) -> ReportVerdict {
        let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
        for case in self
            .cases
            .iter()
            .filter(|case| names_scenario(&case.name, scenario))
        {
            match case.status {
                CaseStatus::Passed => passed += 1,
                CaseStatus::Failed => failed += 1,
                CaseStatus::Skipped => skipped += 1,
            }
        }
        if failed > 0 {
            ReportVerdict::Failed { passed, failed }
        } else if skipped > 0 {
            ReportVerdict::NotExecuted(NotExecuted::Skipped(skipped))
        } else if passed > 0 {
            ReportVerdict::Passed { passed }
        } else {
            ReportVerdict::NotExecuted(NotExecuted::NoTestcase)
        }
    }
}

fn case_status(node: roxmltree::Node) -> CaseStatus {
    let mut status = CaseStatus::Passed;
    for child in node.children().filter(|child| child.is_element()) {
        match child.tag_name().name() {
            "failure" | "error" => return CaseStatus::Failed,
            "skipped" => status = CaseStatus::Skipped,
            _ => {}
        }
    }
    status
}

impl NotExecuted {
    /// The frozen sentence for this reason, naming the report path and the
    /// scenario's `scn_NNNN` pattern (`docs/contracts.md`).
    pub fn message(&self, report: &RepoPath, scenario: ScenarioId) -> String {
        let pattern = scenario_pattern(scenario);
        match self {
            NotExecuted::ReportMissing => {
                format!("the runner did not write the report at `{report}`")
            }
            NotExecuted::ReportInvalid(error) => {
                format!("the report at `{report}` is not valid JUnit XML: {error}")
            }
            NotExecuted::NoTestcase => {
                format!("the report at `{report}` contains no testcase named after `{pattern}`")
            }
            NotExecuted::Skipped(count) => format!(
                "{count} testcase(s) named after `{pattern}` were skipped in the report at `{report}`"
            ),
        }
    }
}
```

- [ ] **Step 5: Run the report tests**

Run: `rtk cargo test -p telos-core report:: 2>&1 | tail -20`
Expected: all pass.

- [ ] **Step 6: Add the frozen error code**

`crates/telos-core/src/error.rs`: after `TelosTestNotFound` add

```rust
/// A run proved nothing about the scenario: with `[test] report`
/// configured, the report is missing, invalid, names no testcase for the
/// scenario, or every such testcase was skipped -- or a sealed witness was
/// taken by exit status while a report is configured.
TelosTestNotExecuted,
```

In `error_code_serialization_is_frozen`, change the comment count to "eighteen" and add

```rust
assert_eq!(
    serde_json::to_string(&ErrorCode::TelosTestNotExecuted)?,
    "\"TELOS_TEST_NOT_EXECUTED\""
);
```

`crates/telos/tests/contracts.rs`: add `"TELOS_TEST_NOT_EXECUTED",` to the `live` list; change both `17` assertions to `18`; in the test doc comment "an accidental eighteenth code" becomes "an accidental nineteenth code".

`docs/contracts.md`: "The seventeen codes below are stable." → "The eighteen codes below are stable."; add a `| \`TELOS_TEST_NOT_EXECUTED\` |` row at the end of the canonical table. (Emission rows come with the surfaces in Tasks 6 and 7.)

- [ ] **Step 7: Run the workspace and commit**

Run: `rtk cargo test --workspace 2>&1 | tail -20`
Expected: all green.

```bash
rtk git add -A Cargo.lock crates docs && rtk git commit -m "feat(report): read JUnit reports for one scenario and freeze TELOS_TEST_NOT_EXECUTED

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: `{report}` and the delete-run-read cycle in `exec.rs`

**Files:**
- Modify: `crates/telos-core/src/exec.rs` (whole file: placeholders, `run_proof`, tests)
- Modify: `crates/telos-core/src/reconcile.rs` (~1316-1326 and ~1350: `substitute_filter` call sites)
- Modify: `crates/telos/src/commands/rebuild.rs` (~line 234: `substitute_filter` call site)

**Interfaces:**
- Consumes: `TestCfg::report_path`, `TestCfg::evidence`, `report::{Report, NotExecuted, ReportVerdict}`, `model::Evidence`.
- Produces: `exec::substitute_placeholders(cmd: &str, filter: &str, report: &str) -> String` (replaces `substitute_filter`); `exec::run_proof(test: &TestCfg, filter: &str, repo_root: &Path) -> Result<ProofRun, TelosError>`; `ProofRun { command: String, status: i32, evidence: ProofEvidence }` with `kind(&self) -> Evidence`, `report_path(&self) -> Option<&RepoPath>`, `verdict(&self, ScenarioId) -> ProofVerdict`; `ProofEvidence { ExitStatus, Report { path: RepoPath, parsed: Result<Report, NotExecuted> } }`; `ProofVerdict { Green { executed: Option<u32> }, Red { executed: Option<u32> }, NotExecuted(NotExecuted) }`. `run_shell_with_filter` stays until Task 8 removes it.

- [ ] **Step 1: Write the failing tests**

In `crates/telos-core/src/exec.rs`'s `filter_rewrite_tests` module add:

```rust
#[test]
fn the_report_placeholder_is_data_in_every_quote_context() {
    assert_eq!(
        parse_runner_template(
            "runner --junit={report} \"{report}\" {filter}",
            "scn_0108_x",
            "target/telos report.xml",
        )
        .unwrap(),
        [
            "runner",
            "--junit=target/telos report.xml",
            "target/telos report.xml",
            "scn_0108_x",
        ]
    );
}

#[test]
fn an_empty_report_creates_no_empty_argument() {
    assert_eq!(
        parse_runner_template("runner {report} {filter}", "scn_0108_x", "").unwrap(),
        ["runner", "scn_0108_x"]
    );
}

#[test]
fn substitution_displays_both_placeholders_literally() {
    assert_eq!(
        substitute_placeholders("runner {report} {filter}", "", "out.xml"),
        "runner out.xml"
    );
}
```

Add a second test module:

```rust
#[cfg(test)]
mod run_proof_tests {
    use super::*;
    use crate::config::TestCfg;
    use crate::ids::ScenarioId;
    use crate::report::{NotExecuted, Report};

    fn report_run(parsed: Result<Report, NotExecuted>, status: i32) -> ProofRun {
        ProofRun {
            command: "runner".to_string(),
            status,
            evidence: ProofEvidence::Report {
                path: RepoPath::new("telos-report.xml"),
                parsed,
            },
        }
    }

    #[test]
    fn exit_status_evidence_reads_zero_as_green_and_anything_else_as_red() {
        let green = ProofRun {
            command: "runner".to_string(),
            status: 0,
            evidence: ProofEvidence::ExitStatus,
        };
        let red = ProofRun { status: 3, ..green.clone() };
        assert_eq!(green.kind(), Evidence::ExitStatus);
        assert_eq!(green.verdict(ScenarioId(1)), ProofVerdict::Green { executed: None });
        assert_eq!(red.verdict(ScenarioId(1)), ProofVerdict::Red { executed: None });
    }

    #[test]
    fn report_evidence_outranks_the_exit_status() {
        let passed = Report::parse(r#"<testsuite><testcase name="scn_0001_x"/></testsuite>"#).unwrap();
        let failed = Report::parse(
            r#"<testsuite><testcase name="scn_0001_x"><failure/></testcase><testcase name="scn_0001_y"/></testsuite>"#,
        )
        .unwrap();
        assert_eq!(
            report_run(Ok(passed), 1).verdict(ScenarioId(1)),
            ProofVerdict::Green { executed: Some(1) }
        );
        assert_eq!(
            report_run(Ok(failed), 0).verdict(ScenarioId(1)),
            ProofVerdict::Red { executed: Some(2) }
        );
        assert_eq!(
            report_run(Err(NotExecuted::ReportMissing), 0).verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportMissing)
        );
        assert_eq!(report_run(Err(NotExecuted::ReportMissing), 0).kind(), Evidence::Report);
    }

    #[test]
    fn without_a_report_run_proof_reads_the_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let test = TestCfg { cmd: "git --version".to_string(), report: String::new() };
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert_eq!(run.command, "git --version");
        assert_eq!(run.status, 0);
        assert_eq!(run.evidence, ProofEvidence::ExitStatus);
    }

    #[test]
    fn a_stale_report_is_deleted_before_the_run_and_its_absence_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("telos-report.xml"), "<testsuite/>").unwrap();
        let test = TestCfg {
            cmd: "git --version".to_string(),
            report: "telos-report.xml".to_string(),
        };
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert!(!tmp.path().join("telos-report.xml").exists());
        assert_eq!(
            run.verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportMissing)
        );
        assert_eq!(run.report_path(), Some(&RepoPath::new("telos-report.xml")));
    }

    #[cfg(unix)]
    fn writer(dir: &std::path::Path, body: &str) -> TestCfg {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("writer");
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s' '{body}' > \"$1\"\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        TestCfg {
            cmd: "./writer {report} {filter}".to_string(),
            report: "out/report.xml".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_report_the_runner_writes_is_read_after_the_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("out")).unwrap();
        let test = writer(tmp.path(), r#"<testsuite><testcase name="scn_0001_x"/></testsuite>"#);
        let run = run_proof(&test, "scn_0001_x", tmp.path()).unwrap();
        assert_eq!(run.command, "./writer out/report.xml scn_0001_x");
        assert_eq!(run.verdict(ScenarioId(1)), ProofVerdict::Green { executed: Some(1) });
    }

    #[cfg(unix)]
    #[test]
    fn an_unparseable_report_is_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("out")).unwrap();
        let test = writer(tmp.path(), "<testsuite><testcase");
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert!(matches!(
            run.verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportInvalid(_))
        ));
    }
}
```

(`tempfile` is already a dev-dependency of `telos-core`.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos-core exec:: 2>&1 | tail -20`
Expected: compile errors.

- [ ] **Step 3: Generalize the placeholders**

In `crates/telos-core/src/exec.rs`:

- Rename `substitute_filter` to `substitute_placeholders(cmd: &str, filter: &str, report: &str) -> String` returning `cmd.replace("{filter}", filter).replace("{report}", report).trim_end().to_string()`; update its doc ("Replaces every `{filter}` and `{report}` …").
- `parse_runner_template(template: &str, filter: &str, report: &str)`: add `const REPORT: &str = "{report}";` and, right after the `{filter}` branch inside the loop:

```rust
if rest.starts_with(REPORT) {
    if !report.is_empty() {
        word.push_str(report);
        in_word = true;
    }
    index += REPORT.len();
    continue;
}
```

- `run_shell_with_filter` passes `""` as the report for both calls (it is on its way out).
- Update the two existing `parse_runner_template(...)` test call sites in `filter_rewrite_tests` to pass a third `""` argument.

Call sites outside the file: in `crates/telos-core/src/reconcile.rs` replace `substitute_filter(cmd, &filter)` (in `run_tests`) with `substitute_placeholders(cmd, &filter, &ws.config.test.report)` and `substitute_filter(cmd, "")` (in `run_full_tests`) with `substitute_placeholders(cmd, "", &ws.config.test.report)`; fix the `use crate::exec::{…}` import. In `crates/telos/src/commands/rebuild.rs` replace `substitute_filter(&runner, filter)` with `substitute_placeholders(&runner, filter, &input.ws.config.test.report)` and its import.

- [ ] **Step 4: Implement `run_proof`**

Add to `crates/telos-core/src/exec.rs` (imports: `use crate::config::TestCfg; use crate::ids::{RepoPath, ScenarioId}; use crate::model::Evidence; use crate::report::{NotExecuted, Report, ReportVerdict};`):

```rust
/// One execution of `[test] cmd` for one filter: the frozen display
/// command, the raw exit status, and the evidence the configuration asked
/// for. Built by [`run_proof`]; read through [`ProofRun::verdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRun {
    /// The template with `{filter}` and `{report}` literally substituted --
    /// diagnostic display, not a shell-replay contract.
    pub command: String,
    /// The runner's exit status (`-1` when killed by a signal).
    pub status: i32,
    pub evidence: ProofEvidence,
}

/// What a run left behind to judge it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofEvidence {
    /// No `[test] report`: the exit status is all there is.
    ExitStatus,
    /// `[test] report` is configured: the report read after the run, or the
    /// reason it could not be.
    Report {
        path: RepoPath,
        parsed: Result<Report, NotExecuted>,
    },
}

/// A run's verdict for one scenario. `executed` is the number of testcases
/// named after the scenario that ran (passed plus failed) under report
/// evidence, `None` under exit-status evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofVerdict {
    Green { executed: Option<u32> },
    Red { executed: Option<u32> },
    NotExecuted(NotExecuted),
}

impl ProofRun {
    /// Which kind of evidence this run carries.
    pub fn kind(&self) -> Evidence {
        match self.evidence {
            ProofEvidence::ExitStatus => Evidence::ExitStatus,
            ProofEvidence::Report { .. } => Evidence::Report,
        }
    }

    /// The configured report path, when there is one.
    pub fn report_path(&self) -> Option<&RepoPath> {
        match &self.evidence {
            ProofEvidence::ExitStatus => None,
            ProofEvidence::Report { path, .. } => Some(path),
        }
    }

    /// The verdict for `scenario`. With a report the exit status is
    /// diagnostic only: a runner that exits 1 over an unrelated failure is
    /// still green when the scenario's testcase passed, and one that exits
    /// 0 having run nothing is not executed.
    pub fn verdict(&self, scenario: ScenarioId) -> ProofVerdict {
        match &self.evidence {
            ProofEvidence::ExitStatus if self.status == 0 => {
                ProofVerdict::Green { executed: None }
            }
            ProofEvidence::ExitStatus => ProofVerdict::Red { executed: None },
            ProofEvidence::Report { parsed: Err(reason), .. } => {
                ProofVerdict::NotExecuted(reason.clone())
            }
            ProofEvidence::Report { parsed: Ok(report), .. } => match report.verdict(scenario) {
                ReportVerdict::Passed { passed } => ProofVerdict::Green {
                    executed: Some(passed),
                },
                ReportVerdict::Failed { passed, failed } => ProofVerdict::Red {
                    executed: Some(passed + failed),
                },
                ReportVerdict::NotExecuted(reason) => ProofVerdict::NotExecuted(reason),
            },
        }
    }
}

/// Runs `[test] cmd` once for `filter` under the delete-run-read cycle:
/// a stale report is removed first so nothing can be read from a previous
/// run, the template runs as a direct argv, and the report (when one is
/// configured) is read back. The runner not writing it is evidence in its
/// own right (`NotExecuted::ReportMissing`), never an error here.
///
/// The template is parsed with `cmd` as configured; callers have already
/// refused an empty one. A report path that fails validation or a stale
/// report that cannot be removed are `TELOS_PARSE_ERROR` / `TELOS_INTERNAL`
/// respectively, before anything runs.
pub fn run_proof(test: &TestCfg, filter: &str, repo_root: &Path) -> Result<ProofRun, TelosError> {
    let report = test.report_path()?;
    let report_str = report.as_ref().map(RepoPath::as_str).unwrap_or("");
    let command = substitute_placeholders(&test.cmd, filter, report_str);
    validate_filter_data(filter)?;
    let argv = parse_runner_template(&test.cmd, filter, report_str)?;

    let report_file = report.as_ref().map(|path| absolute(repo_root, path));
    if let Some(file) = &report_file {
        remove_stale_report(file)?;
    }

    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(repo_root)
        .output()
        .map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to spawn the test runner displayed as `{command}`: {e}"),
            )
        })?;
    let status = run_result(output).status;

    let evidence = match (report, report_file) {
        (Some(path), Some(file)) => ProofEvidence::Report {
            path,
            parsed: read_report(&file),
        },
        _ => ProofEvidence::ExitStatus,
    };
    Ok(ProofRun {
        command,
        status,
        evidence,
    })
}

/// `repo_root` joined with a validated repository path, component by
/// component, so the OS separator is never guessed from the `/` form.
fn absolute(repo_root: &Path, path: &RepoPath) -> std::path::PathBuf {
    let mut absolute = repo_root.to_path_buf();
    for component in path.as_str().split('/') {
        absolute.push(component);
    }
    absolute
}

fn remove_stale_report(file: &Path) -> Result<(), TelosError> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to remove the stale report {}: {e}", file.display()),
        )),
    }
}

fn read_report(file: &Path) -> Result<Report, NotExecuted> {
    match std::fs::read_to_string(file) {
        Ok(xml) => Report::parse(&xml).map_err(NotExecuted::ReportInvalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(NotExecuted::ReportMissing),
        Err(e) => Err(NotExecuted::ReportInvalid(e.to_string())),
    }
}
```

Update the module doc's first lines: "Cross-OS command execution: the platform shell `check` runs through, and the proof cycle `[test] cmd` runs under -- `{filter}`/`{report}` substitution, stale-report removal, and the report read-back."

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test --workspace 2>&1 | tail -20`
Expected: all green (the CLI still uses `run_shell_with_filter`, untouched).

- [ ] **Step 6: Commit**

```bash
rtk git add -A crates && rtk git commit -m "feat(exec): run proofs under a delete-run-read report cycle with {report}

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: `telos.lock` v3 with `proof_evidence`, and `status`

**Files:**
- Modify: `crates/telos-core/src/lock.rs` (struct, `read`, `render`, `seal`, `RawLock`, tests)
- Modify: `crates/telos-core/src/reconcile.rs` (`lock_from_maps` ~500 and its two callers; test helpers `lock_of` ~1585, `previous_with_tool` ~1798)
- Modify: `crates/telos/src/commands/init.rs` (~656-700: use `Lock::render`)
- Modify: `crates/telos/src/commands/status.rs`
- Modify: `crates/telos-core/tests/git_oids.rs` (~273, ~415-440), `crates/telos-core/tests/path_safety.rs` (~65)
- Modify: `crates/telos/tests/status_check.rs` (~73), `crates/telos/tests/change_flow.rs` (~193), other full `status` pins
- Modify: `docs/contracts.md` (`status` section ~217-266)

**Interfaces:**
- Consumes: `TestCfg::evidence`, `model::Evidence`.
- Produces: `Lock { version, tool, sealed_by, spec_digest, proof_evidence: Evidence, spec, code }`; `LOCK_VERSION = 3`; `pub fn Lock::render(&self) -> String`; `reconcile::lock_from_maps(spec, code, sealed_by, evidence: Evidence)`; `status` result key `proof_evidence`.

- [ ] **Step 1: Write the failing lock tests**

In `crates/telos-core/src/lock.rs` tests, replace `read_rejects_the_v1_lock_format_with_an_actionable_hint` with:

```rust
#[test]
fn read_rejects_an_older_lock_format_with_an_actionable_hint() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("telos.lock");
    std::fs::write(
        &path,
        concat!(
            "version = 2\n",
            "tool = \"telos 0.12.0\"\n",
            "spec_digest = \"sha256:old\"\n",
            "\n[spec]\n",
            "\n[code]\n",
        ),
    )
    .unwrap();

    let error = Lock::read(&path).unwrap_err();
    assert_eq!(error.code, ErrorCode::TelosParseError);
    assert!(error.message.contains("lock format version 2"));
    assert_eq!(
        error.hint.as_deref(),
        Some("run `telos reconcile --full` to regenerate telos.lock")
    );
}

#[test]
fn render_writes_proof_evidence_right_after_the_digest() {
    let lock = Lock {
        version: LOCK_VERSION,
        tool: "telos 0.13.0".to_string(),
        sealed_by: None,
        spec_digest: Lock::compute_digest(&BTreeMap::new()),
        proof_evidence: Evidence::Report,
        spec: BTreeMap::new(),
        code: BTreeMap::new(),
    };
    assert_eq!(
        lock.render(),
        format!(
            "version = 3\ntool = \"telos 0.13.0\"\nspec_digest = \"{}\"\nproof_evidence = \"report\"\n\n[spec]\n\n[code]\n",
            Lock::compute_digest(&BTreeMap::new())
        )
    );
}

#[test]
fn read_requires_a_known_proof_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("telos.lock");
    for (body, needle) in [
        (
            "version = 3\ntool = \"telos 0.13.0\"\nspec_digest = \"sha256:x\"\n\n[spec]\n\n[code]\n",
            "proof_evidence",
        ),
        (
            "version = 3\ntool = \"telos 0.13.0\"\nspec_digest = \"sha256:x\"\nproof_evidence = \"vibes\"\n\n[spec]\n\n[code]\n",
            "invalid `proof_evidence` value `vibes`",
        ),
    ] {
        std::fs::write(&path, body).unwrap();
        let error = Lock::read(&path).unwrap_err();
        assert_eq!(error.code, ErrorCode::TelosParseError);
        assert!(error.message.contains(needle), "{}", error.message);
    }
}
```

Add `use crate::model::Evidence;` to the test module.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos-core lock:: 2>&1 | tail -20`
Expected: compile errors (`proof_evidence`, `render` private).

- [ ] **Step 3: Implement the lock change**

In `crates/telos-core/src/lock.rs`:
- `pub const LOCK_VERSION: u32 = 3;`
- `Lock` gains, after `spec_digest`: `/// How every proof this seal rests on was judged: \`report\` when the sealing configuration had a \`[test] report\`, \`exit-status\` otherwise. pub proof_evidence: Evidence,` (import `crate::model::Evidence`).
- Version mismatch hint becomes `"run `telos reconcile --full` to regenerate telos.lock"`.
- `RawLock` gains `proof_evidence: String` (no default). After `sealed_by` parsing in `read`, add:

```rust
let proof_evidence = match raw.proof_evidence.as_str() {
    "exit-status" => Evidence::ExitStatus,
    "report" => Evidence::Report,
    other => {
        return Err(TelosError::new(
            ErrorCode::TelosParseError,
            format!(
                "{}: invalid `proof_evidence` value `{other}`; expected `exit-status` or `report`",
                path.display()
            ),
        ));
    }
};
```

and put `proof_evidence` into the returned `Lock`.
- `render` becomes `pub fn render(&self) -> String` and writes `writeln!(out, "proof_evidence = {}", quote(self.proof_evidence.as_str())).unwrap();` right after the `spec_digest` line. Update `write`'s doc to list `proof_evidence` after `spec_digest`.
- `seal` sets `proof_evidence: ws.config.test.evidence(),` and its doc gains "and the kind of evidence `ws`'s `[test]` section produces".
- Module doc: mention the `proof_evidence` line.

In `crates/telos-core/src/reconcile.rs`: `lock_from_maps(spec, code, sealed_by, proof_evidence: Evidence)` sets the field; `reconcile_change` passes `effective_ws.config.test.evidence()`; `reconcile_full` passes `ws.config.test.evidence()`; test helpers `lock_of` and `previous_with_tool` add `proof_evidence: Evidence::ExitStatus`. Import `Evidence`.

In `crates/telos/src/commands/init.rs`: `compute_lock_bytes` returns `Ok(seal(&ws, &model, git, None)?.render().into_bytes())`; delete `render_lock`. Run `rtk grep -n "quote_lock" crates/telos/src/commands/init.rs`; if `render_lock` was its only caller, delete `quote_lock` too (clippy `-D warnings` would flag dead code otherwise). Remove the now-unused `use std::fmt::Write` if the compiler reports it.

Fix the `Lock { … }` literals in `crates/telos-core/tests/git_oids.rs` (~273: `version: LOCK_VERSION`, `proof_evidence: Evidence::ExitStatus`), `crates/telos-core/tests/path_safety.rs` (~65: same), and the hand-written TOML in `git_oids.rs` `lock_read_tolerates_reformatted_toml` (`version = 3`, add `proof_evidence = "exit-status"`, assert `lock.version == 3` and `lock.proof_evidence == Evidence::ExitStatus`).

- [ ] **Step 4: Expose it in `status`**

`crates/telos/src/commands/status.rs`: in `result` add `"proof_evidence": project.lock.proof_evidence.as_str(),` between `"drift"` and `"coverage"`. In `human_summary`, take `evidence: &str` as an extra parameter and push `format!("proof evidence: {evidence}")` right before the coverage line; pass `project.lock.proof_evidence.as_str()` (bind it before `project.state` is moved out).

Update the pinned envelopes: `crates/telos/tests/status_check.rs` (~73) and `crates/telos/tests/change_flow.rs` (~193) gain `"proof_evidence": "exit-status",` after `"drift": null,`. Then run `rtk cargo test -p telos 2>&1 | grep -E "FAILED|panicked" | head` and fix every other full-`status`-result pin the same way.

`docs/contracts.md`, `status` `result` schema: add `"proof_evidence": "exit-status",` after `"drift": null,` and this bullet after the `drift` bullet:

```markdown
- `proof_evidence` — `"exit-status"` or `"report"`, read from `telos.lock`:
  how every proof the current seal rests on was judged. `"report"` means the
  sealing configuration had a `[test] report`, so each sealed green is a
  testcase named after its scenario that executed and passed; `"exit-status"`
  means the runner's exit code alone was read, which cannot distinguish a
  zero-test run from green. It reports what the seal proved, not what the
  configuration says now: the two differ only between turning the report on
  and the next reconcile.
```

Add a short `### telos.lock` subsection to the `status` section (before `### result schema`):

```markdown
### `telos.lock`

Format version `3`: `version`, `tool`, optional `sealed_by`, `spec_digest`,
`proof_evidence` (`"exit-status"` | `"report"`, required), then the `[spec]`
and `[code]` tables. A lock of any other version is `TELOS_PARSE_ERROR` with
hint `` run `telos reconcile --full` to regenerate telos.lock `` — `--full`
never reads the lock, so the hint is always actionable. `init`, a per-change
reconcile (from the effective configuration), and `--full` all write
`proof_evidence`.
```

- [ ] **Step 5: Run the workspace and commit**

Run: `rtk cargo test --workspace 2>&1 | tail -20`
Expected: all green.

```bash
rtk git add -A crates docs && rtk git commit -m "feat(lock): seal the kind of proof evidence and report it in status

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: `telos test` judges by the report

**Files:**
- Modify: `crates/telos/src/commands/test.rs` (module doc; `require_runner` ~306; `RunReport` ~322; `journal_run` ~355-415; `run_result`, `human_line`)
- Modify: `crates/telos/tests/common/mod.rs` (fake runner and report fixtures)
- Create: `crates/telos/tests/test_report.rs`
- Modify: `crates/telos/tests/test_bind.rs` (~312-337 canonical result gains two keys)
- Modify: `docs/contracts.md` (`test` section ~407-460; error emission table ~141-183)

**Interfaces:**
- Consumes: `exec::{run_proof, ProofRun, ProofVerdict}`, `report::NotExecuted`, `witness::scenario_pattern`, `model::Evidence`, `config::TestCfg`, `ErrorCode::TelosTestNotExecuted`.
- Produces: `test` result keys `evidence` and `executed`; test helpers `common::{REPORT, REPORT_FIXTURE, REPORT_SILENT, FAKE_RUNNER_TEMPLATE, install_fake_runner, write_report_fixture, junit_report, with_report_fixture}`.

- [ ] **Step 1: Add the fake runner and report fixtures to the test harness**

Append to `crates/telos/tests/common/mod.rs`:

```rust
/// The report path every report-backed fixture configures, at the repo root
/// so no `[code]`/`[tests]` glob ever matches it.
pub const REPORT: &str = "telos-report.xml";
/// The file the fake runner copies to `{report}`; tests rewrite it between
/// runs to script what "the runner" reports.
pub const REPORT_FIXTURE: &str = ".report-fixture.xml";
/// Marker: while it exists the fake runner exits 0 without writing a report.
pub const REPORT_SILENT: &str = ".report-silent";
/// The `[test] cmd` the report fixtures install.
pub const FAKE_RUNNER_TEMPLATE: &str = if cfg!(windows) {
    "./fake-runner.bat {report} {filter}"
} else {
    "./fake-runner {report} {filter}"
};

/// Installs a runner that copies [`REPORT_FIXTURE`] to its first argument
/// and exits 0, exits 0 without writing when [`REPORT_SILENT`] exists, and
/// exits 101 without writing (a compile error, a network failure) when the
/// fixture is absent. A shell script on Unix, a batch file on Windows.
pub fn install_fake_runner(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("fake-runner");
        fs::write(
            &script,
            concat!(
                "#!/bin/sh\n",
                "# telos fake runner: $1 is the report path telos asked for.\n",
                "if test -f .report-silent; then exit 0; fi\n",
                "if test -f .report-fixture.xml; then cp .report-fixture.xml \"$1\" && exit 0; fi\n",
                "exit 101\n",
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(windows)]
    {
        fs::write(
            root.join("fake-runner.bat"),
            concat!(
                "@echo off\r\n",
                "if exist .report-silent exit /b 0\r\n",
                "if exist .report-fixture.xml (\r\n",
                "  copy /Y .report-fixture.xml \"%~1\" >nul\r\n",
                "  exit /b 0\r\n",
                ")\r\n",
                "exit /b 101\r\n",
            ),
        )
        .unwrap();
    }
}

/// Scripts the next runner report.
pub fn write_report_fixture(root: &Path, xml: &str) {
    fs::write(root.join(REPORT_FIXTURE), xml).unwrap();
}

/// A JUnit report with one `testcase` per `(name, status)`, `status` being
/// one of `passed`, `failed`, `error`, `skipped`.
pub fn junit_report(cases: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n  <testsuite name=\"billing\">\n",
    );
    for (name, status) in cases {
        let body = match *status {
            "passed" => "",
            "failed" => "<failure message=\"assertion failed\">left != right</failure>",
            "error" => "<error message=\"panicked\">boom</error>",
            "skipped" => "<skipped/>",
            other => panic!("unknown testcase status `{other}`"),
        };
        xml.push_str(&format!(
            "    <testcase name=\"{name}\" classname=\"billing\" time=\"0.01\">{body}</testcase>\n"
        ));
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    xml
}

/// The corpus' two sealed scenarios, both passed: what the sealing
/// `reconcile --full` of a report fixture must find in the report.
pub fn sealed_scenarios_passed() -> String {
    junit_report(&[
        ("scn_0091_issued_invoice_is_open", "passed"),
        ("scn_0107_full_payment_settles_the_invoice", "passed"),
    ])
}

/// [`with_fixture`] with the fake runner installed, `[test] report` set to
/// [`REPORT`], `[policy] tdd` set to `policy`, and a report proving both
/// sealed scenarios in place before the seal.
pub fn with_report_fixture(policy: &str) -> TempDir {
    with_fixture_mut(|root| {
        install_fake_runner(root);
        write_report_fixture(root, &sealed_scenarios_passed());
        // The corpus test file is a placeholder; give the sealed SCN-0107
        // target a real function so `rebuild status` can resolve it.
        fs::write(
            root.join("tests/billing.rs"),
            "fn scn_0107_full_payment_settles_the_invoice() {}\n",
        )
        .unwrap();
        let config = root.join("telos/telos.toml");
        let src = fs::read_to_string(&config).unwrap();
        assert!(
            src.contains("cmd = \"\""),
            "the corpus no longer ships an empty `[test] cmd`: {src}"
        );
        let src = src
            .replace(
                "cmd = \"\"",
                &format!("cmd = \"{FAKE_RUNNER_TEMPLATE}\"\nreport = \"{REPORT}\""),
            )
            .replace("tdd = \"strict\"", &format!("tdd = \"{policy}\""));
        fs::write(&config, src).unwrap();
    })
}
```

- [ ] **Step 2: Write the failing integration tests**

Create `crates/telos/tests/test_report.rs`:

```rust
//! `telos test` under `[test] report`: the verdict is the report's, and a
//! run that proves nothing records nothing.
//!
//! The runner is `common::install_fake_runner`, scripted through the
//! `.report-fixture.xml` file: whatever a test writes there is what "the
//! runner" reports next.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

use common::{
    FAKE_RUNNER_TEMPLATE, REPORT, REPORT_FIXTURE, REPORT_SILENT, junit_report, telos,
    with_report_fixture, write_report_fixture,
};

const BILLING_TEST: &str = "tests/billing.rs";
const SCN: &str = "SCN-0108";
const TEST_FN: &str = "scn_0108_x";
const SCN_0091: &str = "scn_0091_issued_invoice_is_open";
const SCN_0107: &str = "scn_0107_full_payment_settles_the_invoice";
const NOT_EXECUTED_HINT: &str =
    "make the runner execute the test named after `scn_0108` and write the report, then run `telos test SCN-0108` again";

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn stderr(out: &std::process::Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The display command `telos test` reports for `filter`.
fn display(filter: &str) -> String {
    FAKE_RUNNER_TEMPLATE
        .replace("{report}", REPORT)
        .replace("{filter}", filter)
}

fn open_change(dir: &Path) {
    let out = telos(dir, &["change", "open", "Invoices can be settled", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(json_stdout(&out)["result"]["id"], json!("CHG-0001"));
}

fn unchanged_scn_0091() -> Value {
    json!({
        "id": "SCN-0091",
        "title": "a newly issued invoice is open",
        "given": [{"notion": "Customer", "fields": {"name": "ACME"}}],
        "when": {"notion": "InvoiceIssued", "fields": {}},
        "then": ["Invoice.state == open"]
    })
}

fn new_scenario(title: &str) -> Value {
    json!({
        "title": title,
        "given": [{"notion": "Invoice", "fields": {"state": "open", "balance": "0.00 EUR"}}],
        "when": {"notion": "InvoiceIssued", "fields": {}},
        "then": ["Invoice.state == open"]
    })
}

/// Stages `edit intent INT-0017` with `count` brand-new scenarios and
/// returns the ids the allocator minted, ascending.
fn stage_new_scenarios(dir: &Path, count: usize) -> Vec<String> {
    let mut scenarios = vec![unchanged_scn_0091()];
    for n in 0..count {
        scenarios.push(new_scenario(&format!("new scenario {n}")));
    }
    let payload = json!({ "scenarios": scenarios }).to_string();
    let out = telos(
        dir,
        &["edit", "intent", "INT-0017", "--change", "CHG-0001", "--json"],
    )
    .write_stdin(payload)
    .output()
    .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    json_stdout(&out)["result"]["scenario_ids"]
        .as_array()
        .expect("`edit intent` reports the scenario ids it allocated")
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect()
}

fn approve(dir: &Path) {
    let out = telos(dir, &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
}

fn append_test_fns(dir: &Path, names: &[&str]) {
    let path = dir.join(BILLING_TEST);
    let mut src = fs::read_to_string(&path).unwrap();
    for name in names {
        src.push_str(&format!("\nfn {name}() {{}}\n"));
    }
    fs::write(&path, src).unwrap();
}

/// A report project one `telos test` away from its first witness:
/// `CHG-0001` approved with `SCN-0108` staged on `INT-0017`, and
/// `scn_0108_x` written into the sealed test file.
fn approved_with_report() -> TempDir {
    let tmp = with_report_fixture("strict");
    open_change(tmp.path());
    assert_eq!(stage_new_scenarios(tmp.path(), 1), vec![SCN.to_string()]);
    approve(tmp.path());
    append_test_fns(tmp.path(), &[TEST_FN]);
    tmp
}

fn blob_oid(dir: &Path, path: &str) -> String {
    let out = Command::new("git")
        .args(["hash-object", path])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git hash-object {path} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn change_file(dir: &Path) -> String {
    fs::read_to_string(dir.join("telos/changes/CHG-0001.tel")).unwrap()
}

fn not_executed_envelope(message: String) -> Value {
    json!({
        "ok": false,
        "command": "test",
        "result": null,
        "error": {
            "code": "TELOS_TEST_NOT_EXECUTED",
            "message": message,
            "hint": NOT_EXECUTED_HINT,
        },
        "next_actions": []
    })
}

/// Runs `telos test SCN-0108 --json`, asserts the frozen not-executed
/// envelope, and that the change file gained no journal line and kept its
/// `approved` status.
fn assert_not_executed(tmp: &TempDir, message: String) {
    let out = telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap();
    assert!(!out.status.success(), "{}", stderr(&out));
    assert_eq!(json_stdout(&out), not_executed_envelope(message));
    let change = change_file(tmp.path());
    assert!(!change.contains("run  "), "{change}");
    assert!(change.contains("status approved"), "{change}");
}

// --- the verdict is the report's -------------------------------------------

#[test]
fn a_passed_testcase_named_after_the_scenario_is_green_with_one_executed() {
    let tmp = approved_with_report();
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(TEST_FN, "passed"), (SCN_0107, "passed")]),
    );

    let out = telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "test",
            "result": {
                "scenario": SCN,
                "witness": "green",
                "test": format!("{BILLING_TEST}::{TEST_FN}"),
                "change": "CHG-0001",
                "command": display(TEST_FN),
                "evidence": "report",
                "executed": 1,
            },
            "error": null,
            "next_actions": ["telos change reconcile CHG-0001"]
        })
    );
    let oid = blob_oid(tmp.path(), BILLING_TEST);
    assert!(
        change_file(tmp.path()).contains(&format!(
            "  run  {SCN} green \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" report\n"
        )),
        "{}",
        change_file(tmp.path())
    );
}

#[test]
fn a_failed_testcase_is_red_even_though_the_runner_exits_zero() {
    let tmp = approved_with_report();
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(TEST_FN, "failed"), (SCN_0107, "passed")]),
    );

    let out = telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["witness"], json!("red"));
    assert_eq!(envelope["result"]["evidence"], json!("report"));
    assert_eq!(envelope["result"]["executed"], json!(1));
    assert_eq!(envelope["next_actions"], json!([format!("telos test {SCN}")]));
    let oid = blob_oid(tmp.path(), BILLING_TEST);
    assert!(change_file(tmp.path()).contains(&format!(
        "  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" report\n"
    )));
}

#[test]
fn an_error_child_counts_as_red() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "error")]));

    let envelope = json_stdout(&telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap());

    assert_eq!(envelope["result"]["witness"], json!("red"));
}

#[test]
fn the_human_line_counts_the_executed_tests() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "passed")]));

    let out = telos(tmp.path(), &["test", SCN]).output().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{SCN} green: {BILLING_TEST}::{TEST_FN} (recorded in CHG-0001, 1 test executed)\n")
    );
}

// --- nothing executed, nothing recorded -----------------------------------

#[test]
fn a_skipped_testcase_is_not_executed_and_journals_nothing() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "skipped")]));

    assert_not_executed(
        &tmp,
        format!("1 testcase(s) named after `scn_0108` were skipped in the report at `{REPORT}`"),
    );
}

#[test]
fn a_pass_next_to_a_skip_is_not_executed() {
    let tmp = approved_with_report();
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(TEST_FN, "passed"), ("scn_0108_twin", "skipped")]),
    );

    assert_not_executed(
        &tmp,
        format!("1 testcase(s) named after `scn_0108` were skipped in the report at `{REPORT}`"),
    );
}

#[test]
fn a_report_without_the_scenario_is_not_executed() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), &junit_report(&[(SCN_0107, "passed")]));

    assert_not_executed(
        &tmp,
        format!("the report at `{REPORT}` contains no testcase named after `scn_0108`"),
    );
}

#[test]
fn a_runner_that_exits_zero_without_a_report_is_not_executed() {
    let tmp = approved_with_report();
    fs::write(tmp.path().join(REPORT_SILENT), "").unwrap();

    assert_not_executed(
        &tmp,
        format!("the runner did not write the report at `{REPORT}`"),
    );
}

/// The #10 case: a compile error or a dependency fetch failure is a
/// non-zero exit with no report, and it is not a red.
#[test]
fn a_runner_that_fails_without_a_report_is_not_executed_rather_than_red() {
    let tmp = approved_with_report();
    fs::remove_file(tmp.path().join(REPORT_FIXTURE)).unwrap();

    assert_not_executed(
        &tmp,
        format!("the runner did not write the report at `{REPORT}`"),
    );
}

/// The #9 case in its report-less shape: a report left by a previous run
/// must never be read again.
#[test]
fn a_stale_report_is_deleted_before_the_run() {
    let tmp = approved_with_report();
    fs::write(
        tmp.path().join(REPORT),
        junit_report(&[(TEST_FN, "passed")]),
    )
    .unwrap();
    fs::write(tmp.path().join(REPORT_SILENT), "").unwrap();

    assert_not_executed(
        &tmp,
        format!("the runner did not write the report at `{REPORT}`"),
    );
    assert!(!tmp.path().join(REPORT).exists());
}

#[test]
fn an_invalid_report_is_not_executed() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), "<testsuites><testcase name=\"scn_0108_x\"");

    let out = telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap();

    assert!(!out.status.success());
    let error = json_stdout(&out)["error"].clone();
    assert_eq!(error["code"], json!("TELOS_TEST_NOT_EXECUTED"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.starts_with(&format!("the report at `{REPORT}` is not valid JUnit XML: ")),
        "{message}"
    );
    assert_eq!(error["hint"], json!(NOT_EXECUTED_HINT));
    assert!(!change_file(tmp.path()).contains("run  "));
}

#[test]
fn test_all_stops_at_the_first_unexecuted_scenario_and_keeps_earlier_runs() {
    let tmp = with_report_fixture("strict");
    open_change(tmp.path());
    assert_eq!(
        stage_new_scenarios(tmp.path(), 2),
        vec![SCN.to_string(), "SCN-0109".to_string()]
    );
    approve(tmp.path());
    append_test_fns(tmp.path(), &[TEST_FN, "scn_0109_x"]);
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(TEST_FN, "passed"), ("scn_0109_x", "skipped")]),
    );

    let out = telos(tmp.path(), &["test", "--all", "--json"]).output().unwrap();

    assert!(!out.status.success(), "{}", stderr(&out));
    let error = json_stdout(&out)["error"].clone();
    assert_eq!(error["code"], json!("TELOS_TEST_NOT_EXECUTED"));
    assert_eq!(
        error["message"],
        json!(format!(
            "1 testcase(s) named after `scn_0109` were skipped in the report at `{REPORT}`"
        ))
    );
    let change = change_file(tmp.path());
    assert!(change.contains(&format!("run  {SCN} green")), "{change}");
    assert!(!change.contains("SCN-0109 "), "{change}");
    assert!(change.contains("status implementing"), "{change}");
}
```

In `crates/telos/tests/test_bind.rs`, `test_records_a_green_witness_with_the_canonical_result`: add `"evidence": "exit-status", "executed": null,` after `"command": RUNNER,`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `rtk cargo test -p telos --test test_report 2>&1 | tail -30`
Expected: the green test fails on the missing `evidence`/`executed` keys and the `report` journal word; the not-executed tests fail because a witness is recorded instead.

- [ ] **Step 4: Implement the verdict in `test.rs`**

In `crates/telos/src/commands/test.rs`:

Imports: replace `use telos_core::exec::run_shell_with_filter;` with `use telos_core::exec::{ProofVerdict, run_proof};`, add `use telos_core::config::TestCfg; use telos_core::report::NotExecuted; use telos_core::witness::{find_test_for, required_witnesses, scenario_pattern};` (merge with the existing `witness` import), and add `Evidence` to the `telos_core::model::{…}` list (already there from Task 1) plus `RepoPath` is already imported.

`require_runner` returns the configuration, validated:

```rust
/// `[test]`, validated, with the runner trimmed -- or the frozen
/// `TELOS_TEST_NOT_FOUND` for a project that never wired a runner up.
///
/// Trimmed, so that a `cmd = "   "` is the same "no runner" as `cmd = ""`.
/// `validate_self` runs first: a `{report}` without a report, or a report
/// under `telos/`, is refused before anything could execute.
fn require_runner(ws: &telos_core::workspace::Workspace) -> Result<TestCfg, TelosError> {
    ws.config.validate_self()?;
    let cmd = ws.config.test.cmd.trim();
    if cmd.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(TestCfg {
        cmd: cmd.to_string(),
        report: ws.config.test.report.clone(),
    })
}
```

Rename the `cmd` locals in `one` and `every` to `runner` and pass `&runner` to `journal_run`. `RunReport` gains `evidence: Evidence,` and `/// Testcases that ran, under report evidence. executed: Option<u32>,`. `journal_run(project, change, scenario, test, runner: &TestCfg)`:

```rust
let execution = run_proof(runner, &filter, &project.ws.repo_root)?;
let command = execution.command.clone();

let after = project.git.blob_oids(std::slice::from_ref(&test.path))?;
if after.get(&test.path) != Some(&oid) {
    return Err(TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("the test file {} changed while its test was running", test.path),
    )
    .hint("restore the intended test bytes and run `telos test` again"));
}

let (witness, executed) = match execution.verdict(scenario) {
    ProofVerdict::Green { executed } => (Witness::Green, executed),
    ProofVerdict::Red { executed } => (Witness::Red, executed),
    ProofVerdict::NotExecuted(reason) => {
        let report = execution
            .report_path()
            .expect("a not-executed verdict comes from a configured report");
        return Err(not_executed(scenario, report, &reason));
    }
};
let evidence = execution.kind();

change.journal.push(JournalEntry::Run(TestRun {
    scenario,
    witness,
    test: test.clone(),
    oid,
    evidence,
}));
```

and return `evidence` and `executed` in the `RunReport`. Add:

```rust
/// The frozen `TELOS_TEST_NOT_EXECUTED` for a run that proved nothing: the
/// reason's own sentence, and a hint naming the scenario's pattern.
fn not_executed(scenario: ScenarioId, report: &RepoPath, reason: &NotExecuted) -> TelosError {
    TelosError::new(
        ErrorCode::TelosTestNotExecuted,
        reason.message(report, scenario),
    )
    .hint(format!(
        "make the runner execute the test named after `{}` and write the report, then run `telos test {scenario}` again",
        scenario_pattern(scenario)
    ))
}
```

`run_result` adds `"evidence": run.evidence.as_str(), "executed": run.executed,`. `human_line` becomes:

```rust
fn human_line(run: &RunReport) -> String {
    let evidence = match run.executed {
        Some(1) => "1 test executed".to_string(),
        Some(n) => format!("{n} tests executed"),
        None => "exit status only".to_string(),
    };
    format!(
        "{} {}: {} (recorded in {}, {evidence})",
        run.scenario,
        run.witness.as_str(),
        run.test,
        run.change
    )
}
```

Update the docs: in the module doc replace the "**Nothing detects a run that executed zero tests.**" bullet with:

```rust
//! - **A run that executed no test records nothing.** With `[test] report`
//!   configured the verdict is the report's ([`run_proof`]): a testcase
//!   named after the scenario passed or failed, and anything else --
//!   report missing, invalid, no such testcase, only skipped ones -- is
//!   `TELOS_TEST_NOT_EXECUTED` with no journal line. Without a report the
//!   exit status alone decides, which cannot tell a zero-test run from
//!   green; the run line says so (`exit-status`) and so does the seal.
```

In `journal_run`'s doc add: "The verdict is judged only after the post-run hash check: a runner that rewrote its proof is refused before the report is even read." In `every`'s doc, add: "A `TELOS_TEST_NOT_EXECUTED` verdict aborts the loop like a discovery error would, after the runs already taken were journalled."

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test -p telos --test test_report --test test_bind --test contracts 2>&1 | tail -30`
Expected: all pass (if the Windows batch variant cannot be verified locally, note it for CI).

- [ ] **Step 6: Document the contract**

`docs/contracts.md`, `## test <SCN-id|--all> [--file <path>]`: replace the first paragraph's last two sentences ("A non-zero runner exit is **red evidence** … zero-test run from green.") with:

```markdown
Without `[test] report`, a non-zero runner exit is **red evidence, not a
command failure**: the command still exits zero and records the exact blob
OID of the test file that was run, and a zero exit records green. That
reading cannot distinguish a zero-test run from green, and the run line, the
seal and the result say so (`exit-status`). With `[test] report` configured
the verdict is the report's — see "Report-backed evidence" below — and a run
that proves nothing is `TELOS_TEST_NOT_EXECUTED` with no journal line.
```

Replace the single-run result example with:

```json
{"scenario":"SCN-0108","witness":"red|green",
 "test":"tests/billing.rs::scn_0108_x","change":"CHG-0001",
 "command":"cargo nextest run --profile telos scn_0108_x",
 "evidence":"report|exit-status","executed":1}
```

followed by: "`evidence` says how the verdict was decided. `executed` is the number of testcases named after the scenario that ran (passed plus failed) under `report`, and `null` under `exit-status`. The journal line ends in the same evidence word: `` run  SCN-0108 green "tests/billing.rs::scn_0108_x" "<oid>" report ``."

Add, before `### Display and runner-template execution`:

```markdown
### Report-backed evidence

`[test] report = "<path>"` names the JUnit XML report the runner writes,
repository-relative and outside `telos/`. `{report}` in `[test] cmd` is
substituted with that path as argument data exactly like `{filter}`; a
runner that always writes to a fixed path needs no placeholder. Before every
run Telos deletes the report if it exists; after the run it reads it back.
The exit status is then diagnostic only.

A `testcase` is named after the scenario when its `name` attribute contains
`scn_NNNN` at an identifier boundary — the same predicate as discovery.
`classname` is ignored. A testcase with a `failure` or `error` child is
failed; with a `skipped` child, skipped; otherwise passed. Over the
testcases named after the scenario, in this order: any failed → **red**;
otherwise any skipped → not executed; otherwise any passed → **green**;
otherwise not executed. Every `testcase` in the document counts, whether the
root is `testsuites` or `testsuite`.

"Not executed" is `TELOS_TEST_NOT_EXECUTED`, nothing is journalled, and the
message is one of four frozen sentences (`<path>` the configured report,
`scn_NNNN` the scenario's pattern):

| Reason | Message |
|---|---|
| no file at the report path after the run | `` the runner did not write the report at `<path>` `` |
| unreadable or malformed XML | `` the report at `<path>` is not valid JUnit XML: <parser error> `` |
| no testcase named after the scenario | `` the report at `<path>` contains no testcase named after `scn_NNNN` `` |
| testcases named after the scenario, none failed, `<n>` skipped | `` <n> testcase(s) named after `scn_NNNN` were skipped in the report at `<path>` `` |

The hint is always
`` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again ``.
A compile error, a missing dependency, or a runner that selected nothing all
land here rather than as red or green. Under `--all` the first such verdict
aborts the loop; runs already taken stay journalled.

Wiring a report: `cargo nextest run --profile <p> {filter}` with a junit
profile, `pytest --junitxml={report} -k {filter}`, `gotestsum --junitfile
{report} -- -run {filter}` (behind a runner script, since pipes are refused),
`jest --ci --reporters=jest-junit -t {filter}` with `JEST_JUNIT_OUTPUT_FILE`,
`phpunit --log-junit {report} --filter {filter}`, `dotnet test --logger
"junit;LogFilePath={report}" --filter {filter}`. Keep the report path out of
the `[code]`/`[tests]` globs and in `.gitignore`.
```

In the error emission table, add after the `TELOS_TEST_NOT_FOUND` row:

```markdown
| `TELOS_TEST_NOT_EXECUTED` | `telos test` with `[test] report` configured: the report is missing, invalid, names no testcase for the scenario, or every such testcase was skipped (message: one of the four sentences in the `test` section). Nothing is journalled; under `--all` the loop stops there. | `` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again `` |
```

- [ ] **Step 7: Run the workspace and commit**

Run: `rtk cargo test --workspace 2>&1 | tail -20`
Expected: all green.

```bash
rtk git add -A crates docs && rtk git commit -m "feat(test): judge the witness by the JUnit report and refuse unexecuted runs

Closes #9, closes #10

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Reconcile gates 8 and 11, and `--full`

**Files:**
- Modify: `crates/telos-core/src/reconcile.rs` (module doc items 8 and 11 ~48-65; `check_witnesses` ~950-1015; `run_tests` ~1292-1330; `run_full_tests` ~1340-1355; `reconcile_full` call ~327)
- Modify: `crates/telos/tests/test_report.rs` (new tests)
- Modify: `docs/contracts.md` (gates table ~624-640; gate 8 paragraph ~660-666; `--full` ~692-712; error table)

**Interfaces:**
- Consumes: `exec::{run_proof, substitute_placeholders, ProofRun, ProofVerdict}`, `model::Evidence`, `Change::runs_for`, `TestCfg::evidence`.
- Produces: `reconcile::run_full_tests(ws, model)`; private `require_proven(run, target, scenarios)` and `test_not_executed(target, scenario, reason)`.

- [ ] **Step 1: Write the failing integration tests**

Append to `crates/telos/tests/test_report.rs`:

```rust
// --- reconcile ------------------------------------------------------------

/// Red then green through the report, on the same bytes.
fn witness_pair_through_the_report(tmp: &TempDir) {
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "failed")]));
    let red = json_stdout(&telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap());
    assert_eq!(red["result"]["witness"], json!("red"), "{red}");
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "passed")]));
    let green = json_stdout(&telos(tmp.path(), &["test", SCN, "--json"]).output().unwrap());
    assert_eq!(green["result"]["witness"], json!("green"), "{green}");
}

/// What gate 11 must find: every impacted target's scenario passed.
fn impacted_all_passed() -> String {
    junit_report(&[(SCN_0091, "passed"), (TEST_FN, "passed"), (SCN_0107, "passed")])
}

const RECONCILE_HINT: &str =
    "run the configured executable with the displayed arguments and inspect the report, then reconcile again";

#[test]
fn reconcile_reproves_every_impacted_scenario_in_the_report_and_seals_report_evidence() {
    let tmp = approved_with_report();
    witness_pair_through_the_report(&tmp);
    write_report_fixture(tmp.path(), &impacted_all_passed());

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    // Three distinct targets: `tests/billing.rs` (SCN-0091), the sealed
    // SCN-0107 test (INT-0042 requires INT-0017, so it is a dependent), and
    // the journalled SCN-0108 test.
    assert_eq!(json_stdout(&out)["result"]["tests_run"], json!(3));
    let lock = fs::read_to_string(tmp.path().join("telos/telos.lock")).unwrap();
    assert!(lock.contains("\nproof_evidence = \"report\"\n"), "{lock}");
    let status = json_stdout(&telos(tmp.path(), &["status", "--json"]).output().unwrap());
    assert_eq!(status["result"]["proof_evidence"], json!("report"));
    assert_eq!(status["result"]["state"], json!("coherent"));
}

#[test]
fn gate_11_refuses_an_impacted_scenario_the_report_skipped() {
    let tmp = approved_with_report();
    witness_pair_through_the_report(&tmp);
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "skipped"), (TEST_FN, "passed"), (SCN_0107, "passed")]),
    );

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert_eq!(
        json_stdout(&out)["error"],
        json!({
            "code": "TELOS_TEST_NOT_EXECUTED",
            "message": format!(
                "the test run for `tests/billing.rs` did not execute SCN-0091: 1 testcase(s) named after `scn_0091` were skipped in the report at `{REPORT}`"
            ),
            "hint": RECONCILE_HINT,
        })
    );
    assert!(tmp.path().join("telos/changes/CHG-0001.tel").exists());
}

#[test]
fn gate_11_keeps_the_integrity_violation_for_a_failed_impacted_test() {
    let tmp = approved_with_report();
    witness_pair_through_the_report(&tmp);
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "failed"), (TEST_FN, "passed"), (SCN_0107, "passed")]),
    );

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let error = json_stdout(&out)["error"].clone();
    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    assert_eq!(
        error["message"],
        json!(format!(
            "the test run for `tests/billing.rs` failed: `{}`",
            display("tests/billing.rs")
        ))
    );
}

/// Hand-writes an exit-status red/green pair into the change file, the way
/// a journal taken before the report was configured would look.
fn journal_exit_status_pair(dir: &Path) {
    let path = dir.join("telos/changes/CHG-0001.tel");
    let oid = blob_oid(dir, BILLING_TEST);
    let src = fs::read_to_string(&path)
        .unwrap()
        .replace("status approved", "status implementing");
    let (body, _) = src.rsplit_once("}\n").expect("a change file ends its block");
    fs::write(
        &path,
        format!(
            "{body}\n  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" exit-status\n  run  {SCN} green \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" exit-status\n}}\n"
        ),
    )
    .unwrap();
}

const EXIT_STATUS_WITNESS: &str =
    "scenario SCN-0108's witness was taken by exit status; `[test] report` is configured";

#[test]
fn gate_8_refuses_an_exit_status_witness_when_a_report_is_configured() {
    let tmp = approved_with_report();
    journal_exit_status_pair(tmp.path());
    write_report_fixture(tmp.path(), &impacted_all_passed());

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert_eq!(
        json_stdout(&out)["error"],
        json!({
            "code": "TELOS_TEST_NOT_EXECUTED",
            "message": EXIT_STATUS_WITNESS,
            "hint": "run `telos test SCN-0108` again to record a report-backed red and green",
        })
    );
}

#[test]
fn gate_8_warns_about_an_exit_status_witness_under_advisory_policy() {
    let tmp = with_report_fixture("advisory");
    open_change(tmp.path());
    assert_eq!(stage_new_scenarios(tmp.path(), 1), vec![SCN.to_string()]);
    approve(tmp.path());
    append_test_fns(tmp.path(), &[TEST_FN]);
    journal_exit_status_pair(tmp.path());
    write_report_fixture(tmp.path(), &impacted_all_passed());

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out)["result"]["witness_warnings"],
        json!([EXIT_STATUS_WITNESS])
    );
}

#[test]
fn full_reconcile_judges_every_active_scenario_against_one_report() {
    let tmp = with_report_fixture("strict");
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "passed"), (SCN_0107, "skipped")]),
    );

    let out = telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert_eq!(
        json_stdout(&out)["error"],
        json!({
            "code": "TELOS_TEST_NOT_EXECUTED",
            "message": format!(
                "the test run for `the whole suite` did not execute SCN-0107: 1 testcase(s) named after `scn_0107` were skipped in the report at `{REPORT}`"
            ),
            "hint": RECONCILE_HINT,
        })
    );

    write_report_fixture(tmp.path(), &common::sealed_scenarios_passed());
    let out = telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(json_stdout(&out)["result"]["tests_run"], json!(1));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos --test test_report reconcile 2>&1 | tail -30`
Expected: `reconcile_reproves…` passes by exit status but the `gate_11_refuses…`, `gate_8_…`, and `full_reconcile_…` tests fail (the seal goes through or the wrong code comes back).

- [ ] **Step 3: Implement gate 8**

In `crates/telos-core/src/reconcile.rs` `check_witnesses`, after `let required = …; if required.is_empty() { return Ok(Vec::new()); }`:

```rust
// With a report configured, only report-backed runs can pay a witness: an
// exit-status red may be a compile error, an exit-status green a zero-test
// run. Filter them out before judging, and name the situation when it is
// what made the verdict fail.
let report_required = ws.config.test.evidence() == Evidence::Report;
let judged: Vec<JournalEntry> = change
    .journal
    .iter()
    .filter(|entry| {
        !report_required
            || !matches!(entry, JournalEntry::Run(run) if run.evidence == Evidence::ExitStatus)
    })
    .cloned()
    .collect();
```

Then judge `witness_verdict(&judged, scenario, &current)` and add a guard arm right after `WitnessVerdict::Intact => continue,`:

```rust
_ if report_required
    && change
        .runs_for(scenario)
        .any(|run| run.evidence == Evidence::ExitStatus) =>
{
    (
        ErrorCode::TelosTestNotExecuted,
        format!(
            "scenario {scenario}'s witness was taken by exit status; `[test] report` is configured"
        ),
        format!("run `telos test {scenario}` again to record a report-backed red and green"),
    )
}
```

(`test_paths`/`current` keep being computed from the whole journal.) Import `Evidence`. Extend the function doc with a "# With a report configured, only report-backed runs count" paragraph saying the above.

- [ ] **Step 4: Implement gate 11 and `--full`**

Replace `run_tests`:

```rust
fn run_tests(
    ws: &Workspace,
    model: &TelosModel,
    impacted: &BTreeSet<NodeRef>,
) -> Result<u32, TelosError> {
    let cmd = &ws.config.test.cmd;
    if cmd.trim().is_empty() {
        return Ok(0);
    }

    let scenarios = impacted_scenarios(model, impacted);
    // Keyed by the target's rendered form: that both deduplicates two
    // scenarios proved by one test and orders the runs deterministically.
    // Each target carries the impacted scenarios it proves, so one run can
    // be judged once per scenario.
    let mut targets: BTreeMap<String, (&TestRef, BTreeSet<ScenarioId>)> = BTreeMap::new();
    for binding in &model.bindings {
        if let Binding::Proves { test, scenario } = binding
            && scenarios.contains(&scenario.node)
        {
            targets
                .entry(test.to_string())
                .or_insert_with(|| (test, BTreeSet::new()))
                .1
                .insert(scenario.node);
        }
    }

    let mut tests_run = 0;
    for (rendered, (test, proved)) in targets {
        let filter = test
            .name
            .clone()
            .unwrap_or_else(|| test.path.as_str().to_string());
        let run = match run_proof(&ws.config.test, &filter, &ws.repo_root) {
            Ok(run) => run,
            Err(_) => {
                let command = substitute_placeholders(cmd, &filter, &ws.config.test.report);
                return Err(test_failed(&rendered, &command));
            }
        };
        tests_run += 1;
        require_proven(&run, &rendered, proved)?;
    }
    Ok(tests_run)
}

/// Judges one run for every scenario its target proves, ascending. A red
/// is the integrity refusal gate 11 always had; a run that proves nothing
/// is `TELOS_TEST_NOT_EXECUTED`, naming the target, the scenario and the
/// reason. Under exit-status evidence every scenario reads the same, so
/// the loop answers as the single check used to.
fn require_proven(
    run: &ProofRun,
    target: &str,
    scenarios: impl IntoIterator<Item = ScenarioId>,
) -> Result<(), TelosError> {
    for scenario in scenarios {
        match run.verdict(scenario) {
            ProofVerdict::Green { .. } => {}
            ProofVerdict::Red { .. } => return Err(test_failed(target, &run.command)),
            ProofVerdict::NotExecuted(reason) => {
                let report = run
                    .report_path()
                    .expect("a not-executed verdict comes from a configured report");
                return Err(test_not_executed(
                    target,
                    scenario,
                    &reason.message(report, scenario),
                ));
            }
        }
    }
    Ok(())
}

/// The frozen `TELOS_TEST_NOT_EXECUTED` of gates 11 and `--full`.
fn test_not_executed(target: &str, scenario: ScenarioId, reason: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosTestNotExecuted,
        format!("the test run for `{target}` did not execute {scenario}: {reason}"),
    )
    .hint("run the configured executable with the displayed arguments and inspect the report, then reconcile again")
}
```

Replace `run_full_tests` (and its call in `reconcile_full` with `run_full_tests(ws, &model)?`):

```rust
/// A full reseal runs `[test] cmd` once with `{filter}` empty -- the whole
/// suite -- and, with a report configured, judges every active scenario that
/// has a proof against that one report, ascending. Without a report the
/// exit status decides as before. The caller skips this for a draft-only
/// model; an empty `cmd` reports zero runs.
fn run_full_tests(ws: &Workspace, model: &TelosModel) -> Result<u32, TelosError> {
    let cmd = &ws.config.test.cmd;
    if cmd.trim().is_empty() {
        return Ok(0);
    }

    let run = match run_proof(&ws.config.test, "", &ws.repo_root) {
        Ok(run) => run,
        Err(_) => {
            let command = substitute_placeholders(cmd, "", &ws.config.test.report);
            return Err(test_failed("the whole suite", &command));
        }
    };
    match run.kind() {
        Evidence::ExitStatus if run.status == 0 => {}
        Evidence::ExitStatus => return Err(test_failed("the whole suite", &run.command)),
        Evidence::Report => require_proven(&run, "the whole suite", active_proved_scenarios(model))?,
    }
    Ok(1)
}

/// Every scenario of an active intent that has at least one `proves`
/// binding, ascending.
fn active_proved_scenarios(model: &TelosModel) -> BTreeSet<ScenarioId> {
    let proved: BTreeSet<ScenarioId> = model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Proves { scenario, .. } => Some(scenario.node),
            _ => None,
        })
        .collect();
    model
        .intents
        .values()
        .filter(|intent| intent.status == IntentStatus::Active)
        .flat_map(|intent| intent.scenarios.iter().map(|scenario| scenario.id))
        .filter(|id| proved.contains(id))
        .collect()
}
```

Fix the imports (`use crate::exec::{ProofRun, ProofVerdict, run_proof, substitute_placeholders};`, `Evidence`, `IntentStatus` is already imported). Update the module doc: item 8 gains "With `[test] report` configured, only report-backed runs pay a witness; an exit-status pair is `TELOS_TEST_NOT_EXECUTED`."; item 11 gains "Each run is judged per scenario the target proves: red is the integrity refusal, a report that does not prove the scenario is `TELOS_TEST_NOT_EXECUTED`. `--full` judges every active proved scenario against its single run."

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test --workspace 2>&1 | tail -30`
Expected: all green.

- [ ] **Step 6: Document the gates**

`docs/contracts.md`:
- Gates table row 8: `` `TELOS_SCENARIO_RED_EXPECTED` or `TELOS_TEST_SEALED` under strict policy; `TELOS_TEST_NOT_EXECUTED` when `[test] report` is configured and the scenario's witnesses were taken by exit status; warnings under advisory policy ``.
- Row 11: `` `TELOS_INTEGRITY_VIOLATION`, `` the test run for `<target>` failed ``; with `[test] report`, `TELOS_TEST_NOT_EXECUTED`, `` the test run for `<target>` did not execute SCN-NNNN: <reason> `` ``.
- After the "Gate 8 is strict versus advisory" paragraph add:

```markdown
With `[test] report` configured, gate 8 reads only `report` runs. When the
filtered verdict is not intact and the journal holds an `exit-status` run
for the scenario, the refusal is `TELOS_TEST_NOT_EXECUTED` with message
`` scenario SCN-NNNN's witness was taken by exit status; `[test] report` is configured ``
and hint `` run `telos test SCN-NNNN` again to record a report-backed red and green ``
(a warning under `advisory`). Gate 11 runs each impacted target once and
judges the run for every impacted scenario the target proves, in scenario-id
order: a red keeps the integrity refusal above; a report that does not prove
the scenario is `TELOS_TEST_NOT_EXECUTED` with message
`` the test run for `<target>` did not execute SCN-NNNN: <reason> `` — `<reason>`
one of the four sentences of the `test` section — and hint
`` run the configured executable with the displayed arguments and inspect the report, then reconcile again ``.
The seal records `proof_evidence = "report"` from the effective
configuration.
```

- In `#### --full`, after "Gate 11 invokes `[test] cmd` with `{filter}` empty exactly once …" add: "With `[test] report` configured that single run's report is judged for every active scenario that has a `proves` binding, in scenario-id order, with the same two refusals as gate 11 and `<target>` being `the whole suite`."
- Error table, add after the `telos test` row from Task 6:

```markdown
| `TELOS_TEST_NOT_EXECUTED` | Gate 8 under strict policy with `[test] report` configured: the scenario's witnesses were taken by exit status (message `` scenario SCN-NNNN's witness was taken by exit status; `[test] report` is configured ``). A warning under advisory. | `` run `telos test SCN-NNNN` again to record a report-backed red and green `` |
| `TELOS_TEST_NOT_EXECUTED` | Gate 11 or `--full` with `[test] report` configured: a run's report does not prove an impacted (respectively active) scenario (message `` the test run for `<target>` did not execute SCN-NNNN: <reason> ``, `<target>` being `the whole suite` under `--full`). | `` run the configured executable with the displayed arguments and inspect the report, then reconcile again `` |
```

- [ ] **Step 7: Run the contract tests and commit**

Run: `rtk cargo test -p telos --test contracts 2>&1 | tail -10`
Expected: all pass (the gates table is pinned by prose tests; if a pinned row text changed, align the test with the new wording).

```bash
rtk git add -A crates docs && rtk git commit -m "feat(reconcile): judge gates 8, 11 and --full by the report

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: `rebuild status` reads each scenario in the report

**Files:**
- Modify: `crates/telos/src/commands/rebuild.rs` (`status` ~223-290; `require_runner` ~330-340)
- Modify: `crates/telos/tests/common/mod.rs` (extract `configure_report`)
- Modify: `crates/telos/tests/test_report.rs` (new tests)
- Modify: `crates/telos-core/src/exec.rs` (delete `run_shell_with_filter` and `FilteredRun`)
- Modify: `docs/contracts.md` (`#### Status and real measurement` ~1323-1345; execution matrix ~1370-1382)

**Interfaces:**
- Consumes: `exec::{run_proof, substitute_placeholders, ProofRun, ProofVerdict}`, `TestCfg`.
- Produces: `common::configure_report(root: &Path, policy: &str)`.

- [ ] **Step 1: Write the failing tests**

In `crates/telos/tests/common/mod.rs`, extract the closure body of `with_report_fixture` into

```rust
/// Installs the fake runner and points `[test]` at it and at [`REPORT`],
/// with `[policy] tdd = <policy>`; the report proving both sealed scenarios
/// is written too. Call before the fixture seals.
pub fn configure_report(root: &Path, policy: &str) {
    // … the former closure body, verbatim …
}

pub fn with_report_fixture(policy: &str) -> TempDir {
    with_fixture_mut(|root| configure_report(root, policy))
}
```

Append to `crates/telos/tests/test_report.rs`:

```rust
// --- rebuild status -------------------------------------------------------

#[test]
fn rebuild_status_judges_each_scenario_by_the_report() {
    let tmp = with_report_fixture("strict");
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "passed"), (SCN_0107, "skipped")]),
    );

    let out = telos(tmp.path(), &["rebuild", "status", "--json"]).output().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out)["result"],
        json!({
            "scenarios_green": 1,
            "scenarios_total": 2,
            "scenarios": [
                {"id": "SCN-0091", "green": true, "tests": [{
                    "test": "tests/billing.rs",
                    "green": true,
                    "command": display("tests/billing.rs"),
                }]},
                {"id": "SCN-0107", "green": false, "tests": [{
                    "test": format!("tests/billing.rs::{SCN_0107}"),
                    "green": false,
                    "command": display(SCN_0107),
                }]}
            ]
        })
    );
}

#[test]
fn rebuild_status_runs_a_shared_target_once_and_judges_it_per_scenario() {
    let tmp = common::with_fixture_mut(|root| {
        common::configure_report(root, "strict");
        fs::write(
            root.join("telos/contexts/billing/bindings.tel"),
            "implements \"src/billing/invoice.rs\" -> INT-0042\n\
             proves     \"tests/billing.rs\" -> SCN-0091\n\
             proves     \"tests/billing.rs\" -> SCN-0107\n",
        )
        .unwrap();
    });
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "passed"), (SCN_0107, "skipped")]),
    );

    let out = telos(tmp.path(), &["rebuild", "status", "--json"]).output().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let result = json_stdout(&out)["result"].clone();
    assert_eq!(result["scenarios_green"], json!(1));
    assert_eq!(result["scenarios"][0]["green"], json!(true));
    assert_eq!(result["scenarios"][1]["green"], json!(false));
    assert_eq!(
        result["scenarios"][1]["tests"][0]["command"],
        json!(display("tests/billing.rs"))
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p telos --test test_report rebuild 2>&1 | tail -20`
Expected: both fail (SCN-0107 reads green from the exit status).

- [ ] **Step 3: Implement**

In `crates/telos/src/commands/rebuild.rs`:

```rust
fn require_runner(ws: &Workspace) -> Result<TestCfg, TelosError> {
    ws.config.validate_self()?;
    if ws.config.test.cmd.trim().is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(ws.config.test.clone())
}
```

In `status`, the global pass caches the run itself:

```rust
let runner = require_runner(&input.ws)?;
// … first_scenario_by_proof unchanged …
let mut outcomes = BTreeMap::<TestRef, (Option<ProofRun>, String)>::new();
for (test, scenario) in first_scenario_by_proof {
    let filter = test.name.as_deref().unwrap_or_else(|| test.path.as_str());
    let command = substitute_placeholders(&runner.cmd, filter, &runner.report);
    let run = if proof_resolves(&input.ws, scenario, &test)? {
        Some(run_proof(&runner, filter, &input.ws.repo_root)?)
    } else {
        None
    };
    outcomes.insert(test, (run, command));
}
```

and each row reads its own scenario's verdict:

```rust
for test in &proofs {
    let (run, command) = outcomes
        .get(test)
        .expect("every scenario proof was executed in the global pass");
    let target_green = run
        .as_ref()
        .is_some_and(|run| matches!(run.verdict(*scenario), ProofVerdict::Green { .. }));
    green &= target_green;
    tests.push(json!({
        "test": test.to_string(),
        "green": target_green,
        "command": command,
    }));
}
```

Imports: `use telos_core::config::TestCfg; use telos_core::exec::{ProofRun, ProofVerdict, run_proof, substitute_placeholders};`. Update the module doc: "Under `[test] report` a target is still run once, and its report is judged once per scenario the target proves."

Then in `crates/telos-core/src/exec.rs` delete `run_shell_with_filter` and `FilteredRun` (no caller remains: check with `rtk grep -rn "run_shell_with_filter\|FilteredRun" crates docs`; update any doc comment that still names them to say `run_proof`).

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test --workspace 2>&1 | tail -20`
Expected: all green.

- [ ] **Step 5: Document**

`docs/contracts.md`, `#### Status and real measurement`: after "A scenario is green iff it has at least one proof and **all** proof targets are safe, present, resolvable, and exit zero." add: "With `[test] report` configured, "exit zero" becomes "the run's report gives the row's scenario a green verdict" (the `test` section's rule); a target shared by several scenarios is still run once, and its cached report is judged once per scenario, so two rows on one target may differ. A run that proves nothing is a red row, not a command failure." In the `### Proof and constraint execution matrix` introduction (or right after the table) add: "Every scenario/test execution above is judged by the exit status without `[test] report` and by the report with it (`test` section)."

- [ ] **Step 6: Commit**

```bash
rtk git add -A crates docs && rtk git commit -m "feat(rebuild): judge scenario progress by the report

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Guidance, module docs, final verification, pull request

**Files:**
- Modify: `crates/telos/assets/skills/telos-implementer/SKILL.md`
- Modify: `crates/telos/assets/skills/telos/SKILL.md`
- Modify: `crates/telos/tests/agent_init.rs` (~181-201: pin the new lines)
- Modify: `README.md` (`## A typical development loop`)
- Modify: `docs/contracts.md` (final consistency pass)

- [ ] **Step 1: Pin the new skill rules in the agent tests**

In `crates/telos/tests/agent_init.rs`, test `skill_pressure_rules_pin_order_and_stop_conditions`, add (next to the existing implementer assertions; read the test to find the `implementer` variable):

```rust
assert!(implementer.contains("A compile error, a missing dependency, or a runner that executed zero tests is not a red"));
assert!(implementer.contains("`TELOS_TEST_NOT_EXECUTED`: stop; make the runner execute the scenario's test"));
assert!(router.contains("`TELOS_TEST_NOT_EXECUTED`: route to the implementer"));
```

Run: `rtk cargo test -p telos --test agent_init skill_pressure 2>&1 | tail -5` — Expected: FAIL (the lines do not exist yet).

- [ ] **Step 2: Write the guidance**

`crates/telos/assets/skills/telos-implementer/SKILL.md`, step 3 becomes:

```markdown
3. Require literal `result.witness == "red"` — that seals the red witness. A compile error, a missing dependency, or a runner that executed zero tests is not a red: with `[test] report` configured Telos refuses those as `TELOS_TEST_NOT_EXECUTED`; without a report (`result.evidence == "exit-status"`) run the runner directly and confirm the count of executed tests is at least one before trusting the verdict. A crash, missing test, unrelated failure, or green first run is not a valid red.
```

Step 6 gains, after "for the same test bytes": " With a report, require `result.executed >= 1`."

Add to the stop conditions, after `TELOS_TEST_SEALED`:

```markdown
- `TELOS_TEST_NOT_EXECUTED`: stop; make the runner execute the scenario's test and write the report (fix the test name, the filter, the build, or the runner), never weaken or skip the test.
```

`crates/telos/assets/skills/telos/SKILL.md`, after the `TELOS_TEST_SEALED` bullet:

```markdown
- `TELOS_TEST_NOT_EXECUTED`: route to the implementer; the scenario's test did not execute (missing report, skipped or unselected test, build failure) and must, before any witness or seal.
```

`README.md`, after the "Build and prove (strict TDD)." block's closing paragraph ("Staging commands accept structured input…"), add:

```markdown
Set `[test] report` in `telos/telos.toml` to the JUnit XML file your runner
writes (and `{report}` in `[test] cmd` to tell it where): every green then
means a test named after the scenario executed and passed, a run that
executed nothing is refused, and `telos status` reports `proof_evidence`.
```

- [ ] **Step 3: Final consistency pass over `docs/contracts.md`**

Run: `rtk grep -n "zero-test\|seventeen\|exit status alone\|run_shell_with_filter" docs/contracts.md crates/telos/src crates/telos-core/src`
Every hit must either describe the exit-status mode explicitly as such or be updated. In particular the `test` section sentence "The test command does not parse test runner output, so it cannot distinguish a zero-test run from green." must be gone (Task 6 replaced it) and no code comment may still claim nothing detects a zero-test run.

- [ ] **Step 4: Full verification**

Run, and paste the tail of each into the PR body if anything is notable:

```bash
rtk cargo fmt --all --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo test --workspace
```

Expected: fmt clean, clippy clean, every test green.

- [ ] **Step 5: Commit and open the pull request**

```bash
rtk git add -A && rtk git commit -m "docs(skills): teach agents the report-backed witness and TELOS_TEST_NOT_EXECUTED

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
rtk git push -u origin feat/test-report-evidence
```

Then:

```bash
gh pr create --title "feat(test): report-backed witnesses (closes #9, #10)" --body "$(cat <<'BODY'
## Summary

- `[test] report` names a JUnit XML report the runner writes (`{report}` in `[test] cmd` passes the path). `telos test`, reconcile gate 11, `reconcile --full` and `rebuild status` then judge each scenario by the testcase named after it, never by the exit status alone.
- A run that proves nothing (no report, invalid report, no matching testcase, skipped testcase) is the new frozen `TELOS_TEST_NOT_EXECUTED` and records nothing — this closes both the zero-test green of #9 and the compile-error red of #10.
- Every `run` line, `telos.lock` (`proof_evidence`, format version 3) and the `test`/`status` results say whether evidence came from the exit status or a report; gate 8 refuses exit-status witnesses once a report is configured.
- No compatibility with 0.12 journals/locks: run `telos reconcile --full` after upgrading.

Spec: `docs/superpowers/specs/2026-09-03-report-backed-test-evidence-design.md`.

Closes #9, closes #10.

## Test plan

- [ ] `cargo test --workspace` green on Linux (fake runner script) — Windows batch variant verified by CI
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`
- [ ] Manual: point a real `cargo nextest` junit profile at `[test] report`, take a red/green witness, `telos status` shows `proof_evidence: report`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
```

---

## Self-review notes

- Spec coverage: configuration (T2), report verdict and wordings (T3), execution cycle and `{report}` (T4), lock/status (T5), `telos test` incl. `--all` and JSON keys (T6), gate 8/11/`--full` (T7), `rebuild status` (T8), docs/skills/README (T6–T9), 18 codes (T3), journal word (T1).
- Types: `Evidence` (T1) is used by `TestCfg::evidence` (T2), `ProofRun::kind` (T4), `Lock.proof_evidence` (T5), gate 8 (T7). `NotExecuted`/`ReportVerdict` (T3) feed `ProofVerdict` (T4) which T6–T8 match on. `substitute_placeholders` (T4) replaces `substitute_filter` everywhere; `run_shell_with_filter` dies in T8.
