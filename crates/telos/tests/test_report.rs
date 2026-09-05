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
const SCN_0107: &str = "scn_0107_full_payment_settles_the_invoice";
const SCN_0091: &str = "scn_0091_issued_invoice_is_open";
const NOT_EXECUTED_HINT: &str = "make the runner execute the test named after `scn_0108` and write the report, then run `telos test SCN-0108` again";

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
    let out = telos(
        dir,
        &["change", "open", "Invoices can be settled", "--json"],
    )
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
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
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
    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();
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

    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

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

    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["witness"], json!("red"));
    assert_eq!(envelope["result"]["evidence"], json!("report"));
    assert_eq!(envelope["result"]["executed"], json!(1));
    assert_eq!(
        envelope["next_actions"],
        json!([format!("telos test {SCN}")])
    );
    let oid = blob_oid(tmp.path(), BILLING_TEST);
    assert!(change_file(tmp.path()).contains(&format!(
        "  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" report\n"
    )));
}

#[test]
fn an_error_child_counts_as_red() {
    let tmp = approved_with_report();
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "error")]));

    let envelope = json_stdout(
        &telos(tmp.path(), &["test", SCN, "--json"])
            .output()
            .unwrap(),
    );

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

    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let error = json_stdout(&out)["error"].clone();
    assert_eq!(error["code"], json!("TELOS_TEST_NOT_EXECUTED"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.starts_with(&format!(
            "the report at `{REPORT}` is not valid JUnit XML: "
        )),
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

    let out = telos(tmp.path(), &["test", "--all", "--json"])
        .output()
        .unwrap();

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
    // SCN-0109 remains staged (its `scenario` block is naturally present)
    // but must have gained no `run` journal line of its own.
    assert!(!change.contains("run  SCN-0109"), "{change}");
    assert!(change.contains("status implementing"), "{change}");
}

// --- reconcile ------------------------------------------------------------

/// Red then green through the report, on the same bytes.
fn witness_pair_through_the_report(tmp: &TempDir) {
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "failed")]));
    let red = json_stdout(
        &telos(tmp.path(), &["test", SCN, "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(red["result"]["witness"], json!("red"), "{red}");
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "passed")]));
    let green = json_stdout(
        &telos(tmp.path(), &["test", SCN, "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(green["result"]["witness"], json!("green"), "{green}");
}

/// What gate 11 must find: every impacted target's scenario passed.
fn impacted_all_passed() -> String {
    junit_report(&[
        (SCN_0091, "passed"),
        (TEST_FN, "passed"),
        (SCN_0107, "passed"),
    ])
}

const RECONCILE_HINT: &str = "run the configured executable with the displayed arguments and inspect the report, then reconcile again";

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
        &junit_report(&[
            (SCN_0091, "skipped"),
            (TEST_FN, "passed"),
            (SCN_0107, "passed"),
        ]),
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
        &junit_report(&[
            (SCN_0091, "failed"),
            (TEST_FN, "passed"),
            (SCN_0107, "passed"),
        ]),
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

/// A pre-run failure -- here, a stale report that cannot be removed because
/// something else occupies its path -- is `run_proof`'s own `TELOS_INTERNAL`,
/// not the `TELOS_INTEGRITY_VIOLATION` gate 11 raises for a run that
/// actually completed and failed.
#[test]
fn gate_11_surfaces_run_proofs_own_error_for_an_unremovable_stale_report() {
    let tmp = approved_with_report();
    witness_pair_through_the_report(&tmp);
    write_report_fixture(tmp.path(), &impacted_all_passed());
    fs::remove_file(tmp.path().join(REPORT)).unwrap();
    fs::create_dir(tmp.path().join(REPORT)).unwrap();

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success(), "{}", stderr(&out));
    let error = json_stdout(&out)["error"].clone();
    assert_eq!(error["code"], json!("TELOS_INTERNAL"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("failed to remove the stale report"),
        "{message}"
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
    let (body, _) = src
        .rsplit_once("}\n")
        .expect("a change file ends its block");
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

/// Appends one hand-written exit-status red run line after an already-intact
/// report pair, the way a stray leftover from before the report was
/// configured would look.
fn append_exit_status_red_line(dir: &Path) {
    let path = dir.join("telos/changes/CHG-0001.tel");
    let oid = blob_oid(dir, BILLING_TEST);
    let src = fs::read_to_string(&path).unwrap();
    let (body, _) = src
        .rsplit_once("}\n")
        .expect("a change file ends its block");
    fs::write(
        &path,
        format!(
            "{body}\n  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" exit-status\n}}\n"
        ),
    )
    .unwrap();
}

/// With a report configured, only `report` runs are judged: an intact
/// report pair still seals even when the journal also carries a leftover
/// exit-status red for the same scenario and bytes.
#[test]
fn gate_8_ignores_an_extra_exit_status_run_when_the_report_pair_is_intact() {
    let tmp = approved_with_report();
    witness_pair_through_the_report(&tmp);
    append_exit_status_red_line(tmp.path());
    write_report_fixture(tmp.path(), &impacted_all_passed());

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
}

/// Hand-writes a lone exit-status red witness -- no matching green -- the
/// way a red taken before the report was configured would look.
fn write_exit_status_red_line(dir: &Path) {
    let path = dir.join("telos/changes/CHG-0001.tel");
    let oid = blob_oid(dir, BILLING_TEST);
    let src = fs::read_to_string(&path)
        .unwrap()
        .replace("status approved", "status implementing");
    let (body, _) = src
        .rsplit_once("}\n")
        .expect("a change file ends its block");
    fs::write(
        &path,
        format!(
            "{body}\n  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\" exit-status\n}}\n"
        ),
    )
    .unwrap();
}

/// An exit-status red followed by a report-backed green, on the same bytes,
/// still refuses: the filtered (report-only) verdict is `MissingRed`, and
/// the unfiltered journal's exit-status run is what gate 8 names.
#[test]
fn gate_8_refuses_an_exit_status_red_followed_by_a_report_backed_green() {
    let tmp = approved_with_report();
    write_exit_status_red_line(tmp.path());
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "passed")]));
    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
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

/// A failed testcase for an active scenario keeps the `TELOS_INTEGRITY_
/// VIOLATION` gate `--full` always had, unlike a `NotExecuted` verdict.
#[test]
fn full_reconcile_keeps_the_integrity_violation_for_a_failed_active_scenario() {
    let tmp = with_report_fixture("strict");
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "failed"), (SCN_0107, "passed")]),
    );

    let out = telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out)["error"],
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": format!(
                "the test run for `the whole suite` failed: `{}`",
                display("").trim_end()
            ),
            "hint": "run the configured executable with the displayed arguments, then reconcile again",
        })
    );
}

// --- rebuild status -------------------------------------------------------

#[test]
fn rebuild_status_judges_each_scenario_by_the_report() {
    let tmp = with_report_fixture("strict");
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(SCN_0091, "passed"), (SCN_0107, "skipped")]),
    );

    let out = telos(tmp.path(), &["rebuild", "status", "--json"])
        .output()
        .unwrap();

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

    let out = telos(tmp.path(), &["rebuild", "status", "--json"])
        .output()
        .unwrap();

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

/// Both new scenarios use one frozen file. The journal may interleave their
/// independent red/green cycles; gate 8 judges each pair on those exact bytes.
#[test]
fn grouped_reds_then_grouped_greens_reconcile_on_one_frozen_file() {
    let tmp = with_report_fixture("strict");
    open_change(tmp.path());
    assert_eq!(
        stage_new_scenarios(tmp.path(), 2),
        vec![SCN.to_string(), "SCN-0109".to_string()]
    );
    approve(tmp.path());
    append_test_fns(tmp.path(), &[TEST_FN, "scn_0109_x"]);
    let proof_bytes = fs::read(tmp.path().join(BILLING_TEST)).unwrap();
    for verdict in ["failed", "passed"] {
        write_report_fixture(
            tmp.path(),
            &junit_report(&[(TEST_FN, verdict), ("scn_0109_x", verdict)]),
        );
        for scenario in [SCN, "SCN-0109"] {
            let out = telos(tmp.path(), &["test", scenario, "--json"])
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", stderr(&out));
            let result = json_stdout(&out)["result"].clone();
            assert_eq!(
                result["witness"],
                if verdict == "failed" { "red" } else { "green" }
            );
            assert_eq!(result["evidence"], "report");
            assert_eq!(result["executed"], 1);
        }
    }
    assert_eq!(
        fs::read(tmp.path().join(BILLING_TEST)).unwrap(),
        proof_bytes
    );
    write_report_fixture(
        tmp.path(),
        &junit_report(&[
            ("scn_0091_issued_invoice_is_open", "passed"),
            ("scn_0107_full_payment_settles_the_invoice", "passed"),
            (TEST_FN, "passed"),
            ("scn_0109_x", "passed"),
        ]),
    );
    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(json_stdout(&out)["result"]["witness_warnings"], json!([]));
    telos(tmp.path(), &["check", "--sealed"]).assert().success();
}

#[test]
fn adding_a_grouped_test_after_the_first_red_invalidates_the_shared_file() {
    let tmp = with_report_fixture("strict");
    open_change(tmp.path());
    stage_new_scenarios(tmp.path(), 2);
    approve(tmp.path());
    append_test_fns(tmp.path(), &[TEST_FN]);
    write_report_fixture(tmp.path(), &junit_report(&[(TEST_FN, "failed")]));
    telos(tmp.path(), &["test", SCN, "--json"])
        .assert()
        .success();
    append_test_fns(tmp.path(), &["scn_0109_x"]);
    write_report_fixture(tmp.path(), &junit_report(&[("scn_0109_x", "failed")]));
    telos(tmp.path(), &["test", "SCN-0109", "--json"])
        .assert()
        .success();
    write_report_fixture(
        tmp.path(),
        &junit_report(&[(TEST_FN, "passed"), ("scn_0109_x", "passed")]),
    );
    for scenario in [SCN, "SCN-0109"] {
        telos(tmp.path(), &["test", scenario, "--json"])
            .assert()
            .success();
    }
    let before = fs::read(tmp.path().join("telos/telos.lock")).unwrap();
    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_stdout(&out)["error"]["code"], "TELOS_TEST_SEALED");
    assert_eq!(
        fs::read(tmp.path().join("telos/telos.lock")).unwrap(),
        before
    );
}
