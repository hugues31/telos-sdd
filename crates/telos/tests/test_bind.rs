//! End-to-end tests for `telos test`: the sealed red/green witness, the
//! change it is journalled into, and every gate that stands before it.
//!
//! Two properties carry most of what is asserted here:
//!
//! - **The witness is sealed to the bytes it was taken on.** The `run` line
//!   the command appends names the test's blob oid, and the test recomputes
//!   that oid with `git hash-object` rather than reading it back out of the
//!   file it is checking.
//! - **The drift carve-out** (D6). Writing the test function into a *sealed*
//!   test file drifts the project before any journal line claims it, so
//!   `telos test` has to admit exactly that path -- and only that path.
//!
//! The runner every test that actually runs one is configured with is `git
//! hash-object .fake-green`: it exits 0 while the marker file exists and
//! non-zero once it is deleted, which is a deterministic, cross-platform
//! red/green switch that needs no test framework of its own.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

use common::{telos, with_fixture, with_fixture_mut};

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The `[test] cmd` the fixtures below install: green while [`MARKER`]
/// exists, red once it is gone.
const RUNNER: &str = "git hash-object .fake-green";

/// The file [`RUNNER`] hashes -- deleting it is what turns a run red.
const MARKER: &str = ".fake-green";

/// The corpus' sealed test file, the one every witness here is taken on.
const BILLING_TEST: &str = "tests/billing.rs";

/// The scenario the staged `edit intent INT-0017` allocates: the corpus'
/// highest scenario is `SCN-0107`, so the next one is this.
const SCN: &str = "SCN-0108";

/// The test function name appended to [`BILLING_TEST`], and therefore the
/// `name` half of the discovered locator (D4).
const TEST_FN: &str = "scn_0108_x";

/// The exact `TELOS_DRIFT_DETECTED` hint, frozen by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

// --- fixtures ---------------------------------------------------------------

/// The sealed corpus with a runner wired up and its marker in place.
///
/// Both happen *before* the seal: the harness seals by running `telos change
/// reconcile --full`, which runs `[test] cmd` once, so a marker written
/// afterwards would come too late and the fixture would never seal.
fn fixture_with_runner() -> TempDir {
    with_fixture_mut(|root| {
        fs::write(root.join(MARKER), "marker\n").unwrap();
        let path = root.join("telos/telos.toml");
        let src = fs::read_to_string(&path).unwrap();
        assert!(
            src.contains("cmd = \"\""),
            "the corpus no longer ships an empty `[test] cmd`: {src}"
        );
        fs::write(
            &path,
            src.replace("cmd = \"\"", &format!("cmd = \"{RUNNER}\"")),
        )
        .unwrap();
    })
}

/// `telos change open`, asserting the id every test below assumes.
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

/// `SCN-0091` exactly as the corpus declares it: re-supplied unchanged so
/// that its emitted fragment is identical and D7 exempts it from owing a
/// witness of its own.
fn unchanged_scn_0091() -> Value {
    json!({
        "id": "SCN-0091",
        "title": "a newly issued invoice is open",
        "given": [{"notion": "Customer", "fields": {"name": "ACME"}}],
        "when": {"notion": "InvoiceIssued", "fields": {}},
        "then": ["Invoice.state == open"]
    })
}

/// Stages `edit intent INT-0017` growing one brand-new scenario, and returns
/// the id the allocator minted for it.
fn stage_new_scenario(dir: &Path) -> String {
    let payload = json!({
        "scenarios": [
            unchanged_scn_0091(),
            {
                "title": "an issued invoice starts with nothing paid",
                "given": [{"notion": "Invoice",
                           "fields": {"state": "open", "balance": "0.00 EUR"}}],
                "when": {"notion": "InvoiceIssued", "fields": {}},
                "then": ["Invoice.state == open"]
            }
        ]
    })
    .to_string();

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

    let envelope = json_stdout(&out);
    let ids = envelope["result"]["scenario_ids"]
        .as_array()
        .expect("`edit intent` reports the scenario ids it allocated")
        .clone();
    assert_eq!(ids.len(), 1, "expected exactly one new scenario: {ids:?}");
    ids[0]
        .as_str()
        .expect("a scenario id is a string")
        .to_string()
}

fn approve(dir: &Path) -> Value {
    let out = telos(dir, &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    json_stdout(&out)
}

/// Appends the test function the scenario's witness will be discovered
/// through to the *sealed* `tests/billing.rs` -- which is what drifts the
/// project on exactly the path the command is about to claim.
fn append_test_fn(dir: &Path) {
    let path = dir.join(BILLING_TEST);
    let mut src = fs::read_to_string(&path).unwrap();
    src.push_str(&format!("\nfn {TEST_FN}() {{}}\n"));
    fs::write(&path, src).unwrap();
}

/// A project one `telos test` away from its first witness: `CHG-0001`
/// approved with a brand-new `SCN-0108` staged on `INT-0017`, and the
/// matching test function written into the sealed test file.
fn approved_with_a_drifted_test() -> TempDir {
    let tmp = fixture_with_runner();
    open_change(tmp.path());
    assert_eq!(stage_new_scenario(tmp.path()), SCN);
    approve(tmp.path());
    append_test_fn(tmp.path());
    tmp
}

/// `git hash-object <path>` in `dir`: the blob oid a sealed witness must
/// name, computed independently of anything telos wrote.
fn blob_oid(dir: &Path, path: &str) -> String {
    let out = Command::new("git")
        .args(["hash-object", path])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git hash-object {path} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn stderr(out: &std::process::Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Runs `telos <args> --json` and returns the parsed envelope.
fn run_json(dir: &Path, args: &[&str]) -> Value {
    let out = telos(dir, args).output().unwrap();
    json_stdout(&out)
}

/// The `error` object of a command that must have failed.
fn error_of(dir: &Path, args: &[&str]) -> Value {
    let out = telos(dir, args).output().unwrap();
    assert!(
        !out.status.success(),
        "expected `telos {}` to fail: {}",
        args.join(" "),
        stderr(&out)
    );
    json_stdout(&out)["error"].clone()
}

/// The whole change file, as text.
fn change_file(dir: &Path) -> String {
    fs::read_to_string(dir.join("telos/changes/CHG-0001.tel")).unwrap()
}

// --- the happy paths --------------------------------------------------------

/// The precondition the carve-out exists for: writing the test function into
/// a sealed file drifts the project *before* any journal line claims it.
#[test]
fn writing_the_test_into_a_sealed_file_drifts_the_project_first() {
    let tmp = approved_with_a_drifted_test();

    let envelope = run_json(tmp.path(), &["status", "--json"]);

    assert_eq!(envelope["result"]["state"], json!("drifted"));
    assert_eq!(
        envelope["result"]["drift"],
        json!({ "paths": [BILLING_TEST], "suggestion": "telos adopt" })
    );
}

/// The Annex C result, key for key, on a green run -- carve-out included:
/// the command succeeds even though the project was drifted on the test
/// file it just claimed.
#[test]
fn test_records_a_green_witness_with_the_annex_c_result() {
    let tmp = approved_with_a_drifted_test();

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
                "command": RUNNER,
            },
            "error": null,
            "next_actions": ["telos change reconcile CHG-0001"]
        })
    );
}

/// The same flow with the marker deleted: the runner exits non-zero, and the
/// witness sealed is `red` -- the one an implementation has yet to turn.
#[test]
fn test_records_a_red_witness_when_the_runner_fails() {
    let tmp = approved_with_a_drifted_test();
    fs::remove_file(tmp.path().join(MARKER)).unwrap();

    let out = telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["witness"], json!("red"));
    assert_eq!(envelope["result"]["command"], json!(RUNNER));
    assert_eq!(
        envelope["next_actions"],
        json!([format!("telos test {SCN}")])
    );
}

/// The journal line, byte for byte, with the oid computed independently.
#[test]
fn test_appends_the_exact_journal_line_to_the_owning_change() {
    let tmp = approved_with_a_drifted_test();
    fs::remove_file(tmp.path().join(MARKER)).unwrap();

    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    let oid = blob_oid(tmp.path(), BILLING_TEST);
    assert!(
        change_file(tmp.path()).contains(&format!(
            "  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\"\n"
        )),
        "{}",
        change_file(tmp.path())
    );
}

/// The journal is append-only evidence: the red/green pair of one scenario
/// is two lines in the order they were taken, never one line rewritten.
#[test]
fn a_second_run_appends_rather_than_replacing_the_first() {
    let tmp = approved_with_a_drifted_test();

    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();
    fs::remove_file(tmp.path().join(MARKER)).unwrap();
    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    let oid = blob_oid(tmp.path(), BILLING_TEST);
    let runs: Vec<String> = change_file(tmp.path())
        .lines()
        .filter(|line| line.starts_with("  run "))
        .map(str::to_string)
        .collect();
    assert_eq!(
        runs,
        vec![
            format!("  run  {SCN} green \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\""),
            format!("  run  {SCN} red \"{BILLING_TEST}::{TEST_FN}\" \"{oid}\""),
        ]
    );
}

/// D5's transition: the first journalled run takes an `approved` change to
/// `implementing`, which is what the grammar requires of a change that has a
/// journal at all -- and the project, whose only drift is now claimed, is
/// back to `changing`.
#[test]
fn test_moves_the_owner_to_implementing_and_the_project_to_changing() {
    let tmp = approved_with_a_drifted_test();

    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    let envelope = run_json(tmp.path(), &["status", "--json"]);
    assert_eq!(envelope["result"]["state"], json!("changing"));
    assert_eq!(envelope["result"]["drift"], Value::Null);
    assert_eq!(
        envelope["result"]["changes"],
        json!([{ "id": "CHG-0001", "status": "implementing", "obligations": ["reconcile"] }])
    );
}

/// D1's whole point: journalling is digest-inert, so the change that was
/// approved before its first run is not stale after it.
#[test]
fn journalling_a_run_leaves_the_approval_fresh() {
    let tmp = approved_with_a_drifted_test();
    // Read back rather than re-approved: the project is drifted on the test
    // file at this point, and `approve` is gated on that (D17).
    let approved_digest = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"])["result"]
        ["approved_digest"]
        .clone();

    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    let envelope = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"]);
    assert_eq!(envelope["result"]["stale"], json!(false));
    assert_eq!(envelope["result"]["approved_digest"], approved_digest);
    assert_eq!(envelope["result"]["status"], json!("implementing"));
}

/// D13, advanced here because `telos test` is what makes it reachable:
/// re-approving a change that has already started being implemented
/// recalculates the digest and *keeps* `implementing` -- writing `approved`
/// over a journalled change would produce a file the grammar refuses.
#[test]
fn approving_an_implementing_change_keeps_it_implementing() {
    let tmp = approved_with_a_drifted_test();
    telos(tmp.path(), &["test", SCN, "--json"])
        .output()
        .unwrap();

    let envelope = approve(tmp.path());

    assert_eq!(envelope["result"]["status"], json!("implementing"));
    assert!(
        change_file(tmp.path()).contains("  status implementing\n"),
        "{}",
        change_file(tmp.path())
    );
    // And the file still parses -- which is what the ruling is about.
    assert_eq!(
        run_json(tmp.path(), &["change", "list", "--json"])["result"]["changes"][0]["status"],
        json!("implementing")
    );
}

/// A re-approval after implementation has started reviews newly staged work
/// without losing the journal-derived lifecycle state: it refreshes the ops
/// digest, clears staleness, and leaves reconciliation as the next step.
#[test]
fn reapproving_after_implementation_refreshes_the_digest_but_keeps_implementing() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();
    telos(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    let first_digest = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        ["result"]["approved_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let out = telos(
        tmp.path(),
        &[
            "edit",
            "intent",
            BOUND_INTENT,
            "--change",
            "CHG-0001",
            "--json",
        ],
    )
    .write_stdin(r#"{"telos":"The implementation was re-reviewed."}"#)
    .output()
    .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));

    let stale = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"]);
    assert_eq!(stale["result"]["status"], json!("implementing"));
    assert_eq!(stale["result"]["stale"], json!(true));
    assert_eq!(stale["result"]["approved_digest"], json!(first_digest));

    let reapproval = approve(tmp.path());
    let refreshed_digest = reapproval["result"]["digest"].as_str().unwrap().to_owned();
    assert_eq!(reapproval["result"]["status"], json!("implementing"));
    assert_ne!(refreshed_digest, first_digest);

    let fresh = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"]);
    assert_eq!(fresh["result"]["status"], json!("implementing"));
    assert_eq!(fresh["result"]["stale"], json!(false));
    assert_eq!(fresh["result"]["approved_digest"], json!(refreshed_digest));
    assert_eq!(
        fresh["next_actions"],
        json!(["telos change reconcile CHG-0001"])
    );

    let reconciled = run_json(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"]);
    assert_eq!(reconciled["ok"], json!(true), "{reconciled}");
}

// --- the gates, in the order D6 freezes them --------------------------------

/// An id no change and no spec file declares: the same `unknown scenario`
/// shape `show` answers with, nearest existing id included.
#[test]
fn test_on_an_unknown_scenario_names_the_nearest_id() {
    let tmp = fixture_with_runner();

    let error = error_of(tmp.path(), &["test", "SCN-9999", "--json"]);

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(error["message"], json!("unknown scenario `SCN-9999`"));
    assert_eq!(error["hint"], json!("closest is SCN-0107"));
}

/// D5: a scenario the sealed spec declares but no open change stages is
/// nobody's to witness.
#[test]
fn test_on_a_scenario_no_change_implements_is_refused() {
    let tmp = fixture_with_runner();

    let error = error_of(tmp.path(), &["test", "SCN-0107", "--json"]);

    assert_eq!(error["code"], json!("TELOS_CHANGE_STATE_INVALID"));
    assert_eq!(
        error["message"],
        json!("no open change is implementing SCN-0107")
    );
    assert_eq!(
        error["hint"],
        json!("stage it into a change and approve it first")
    );
}

/// The owner exists but has not been reviewed: the M2 wording, reused.
#[test]
fn test_on_a_drafted_owner_asks_for_the_approval_first() {
    let tmp = fixture_with_runner();
    open_change(tmp.path());
    assert_eq!(stage_new_scenario(tmp.path()), SCN);

    let error = error_of(tmp.path(), &["test", SCN, "--json"]);

    assert_eq!(error["code"], json!("TELOS_CHANGE_STATE_INVALID"));
    assert_eq!(
        error["message"],
        json!("change CHG-0001 is not approved; approve it first")
    );
    assert_eq!(
        error["hint"],
        json!("run `telos change diff CHG-0001` then `telos change approve CHG-0001`")
    );
}

/// No runner wired up at all -- the corpus' own state (D13 ships `[test]
/// cmd` empty), reached only once the owner and its status have checked out.
#[test]
fn test_without_a_configured_runner_names_the_missing_setting() {
    let tmp = with_fixture();
    open_change(tmp.path());
    assert_eq!(stage_new_scenario(tmp.path()), SCN);
    approve(tmp.path());

    let error = error_of(tmp.path(), &["test", SCN, "--json"]);

    assert_eq!(error["code"], json!("TELOS_TEST_NOT_FOUND"));
    assert_eq!(
        error["message"],
        json!("no `[test] cmd` is configured in telos/telos.toml")
    );
    assert_eq!(
        error["hint"],
        json!("set [test] cmd, e.g. `cargo test {filter}`")
    );
}

/// D4: nothing the `[tests]` globs cover holds the convention.
#[test]
fn test_without_a_matching_test_function_names_the_convention() {
    let tmp = fixture_with_runner();
    open_change(tmp.path());
    assert_eq!(stage_new_scenario(tmp.path()), SCN);
    approve(tmp.path());

    let error = error_of(tmp.path(), &["test", SCN, "--json"]);

    assert_eq!(error["code"], json!("TELOS_TEST_NOT_FOUND"));
    assert_eq!(
        error["message"],
        json!("no file matched by the [tests] globs contains `scn_0108`")
    );
    assert_eq!(
        error["hint"],
        json!(
            "name the test after the scenario id (`scn_0108_…`) in a file the [tests] \
             globs cover, or pass `--file <path>`"
        )
    );
}

/// An explicit path bypasses convention discovery, but it never turns a
/// missing file into a runner error. Its no-hint form is part of the frozen
/// `TELOS_TEST_NOT_FOUND` family.
#[test]
fn test_with_an_absent_explicit_file_is_the_exact_no_hint_error() {
    let tmp = fixture_with_runner();
    open_change(tmp.path());
    assert_eq!(stage_new_scenario(tmp.path()), SCN);
    approve(tmp.path());

    let error = error_of(
        tmp.path(),
        &["test", SCN, "--file", "tests/missing.rs", "--json"],
    );

    assert_eq!(error["code"], json!("TELOS_TEST_NOT_FOUND"));
    assert_eq!(
        error["message"],
        json!("the file passed with --file does not exist: `tests/missing.rs`")
    );
    assert_eq!(error["hint"], Value::Null);
}

/// D4: two files hold the convention, so discovery refuses rather than
/// picking one, and names `--file` as the way out.
#[test]
fn test_with_the_convention_in_two_files_refuses_to_choose() {
    let tmp = approved_with_a_drifted_test();
    // A brand-new, unbound code file: never sealed, so it adds a second
    // discovery hit without adding any drift of its own.
    fs::write(
        tmp.path().join("tests/extra.rs"),
        format!("fn {TEST_FN}_too() {{}}\n"),
    )
    .unwrap();

    let error = error_of(tmp.path(), &["test", SCN, "--json"]);

    assert_eq!(error["code"], json!("TELOS_TEST_NOT_FOUND"));
    assert_eq!(
        error["message"],
        json!(
            "`scn_0108` appears in more than one test file: `tests/billing.rs`, `tests/extra.rs`"
        )
    );
    assert_eq!(error["hint"], json!("pass `--file <path>` to pick one"));
}

/// `--file` is the explicit answer to that: it wins outright, and the
/// locator still picks up the function name the file happens to hold.
#[test]
fn test_with_an_explicit_file_picks_it_and_still_names_the_function() {
    let tmp = approved_with_a_drifted_test();
    fs::write(
        tmp.path().join("tests/extra.rs"),
        format!("fn {TEST_FN}_too() {{}}\n"),
    )
    .unwrap();

    let out = telos(tmp.path(), &["test", SCN, "--file", BILLING_TEST, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out)["result"]["test"],
        json!(format!("{BILLING_TEST}::{TEST_FN}"))
    );
}

/// The carve-out is exactly one path wide (D6): drift anywhere else is still
/// damage nobody claimed, and the command refuses before running anything.
#[test]
fn test_refuses_unclaimed_drift_outside_the_test_file() {
    let tmp = approved_with_a_drifted_test();
    let invoice = tmp.path().join("telos/notions/Invoice.tel");
    let mut src = fs::read_to_string(&invoice).unwrap();
    src.push('\n');
    fs::write(&invoice, src).unwrap();

    let error = error_of(tmp.path(), &["test", SCN, "--json"]);

    assert_eq!(error["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(
        error["message"],
        json!("the project has drifted from its seal")
    );
    assert_eq!(error["hint"], json!(DRIFT_HINT));
    // Nothing was written: the change is still approved, with no journal.
    assert!(
        !change_file(tmp.path()).contains("  run "),
        "{}",
        change_file(tmp.path())
    );
}

// --- --all ------------------------------------------------------------------

/// `--all` runs every scenario the open, approved changes owe a witness for
/// -- which here is the new one only: `SCN-0091` was re-staged byte-
/// identical, so D7 exempts it.
#[test]
fn test_all_runs_every_scenario_that_owes_a_witness() {
    let tmp = approved_with_a_drifted_test();

    let out = telos(tmp.path(), &["test", "--all", "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "test",
            "result": { "runs": [{
                "scenario": SCN,
                "witness": "green",
                "test": format!("{BILLING_TEST}::{TEST_FN}"),
                "change": "CHG-0001",
                "command": RUNNER,
            }]},
            "error": null,
            "next_actions": []
        })
    );
}

#[test]
fn test_all_without_an_active_change_is_refused() {
    let tmp = fixture_with_runner();

    let error = error_of(tmp.path(), &["test", "--all", "--json"]);

    assert_eq!(error["code"], json!("TELOS_CHANGE_STATE_INVALID"));
    assert_eq!(
        error["message"],
        json!("no open change is implementing any scenario")
    );
    assert_eq!(error["hint"], json!("run `telos change list`"));
}

/// clap's own contract: a scenario or `--all`, never both and never neither
/// (exit 2, a usage error rather than an envelope).
#[test]
fn test_requires_exactly_one_of_a_scenario_and_all() {
    let tmp = fixture_with_runner();

    for args in [vec!["test"], vec!["test", SCN, "--all"]] {
        let out = telos(tmp.path(), &args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "expected a usage error for `telos {}`",
            args.join(" ")
        );
    }
}

// =============================================================================
// `telos bind`: the same shape of gates (D5, D6), and the dedup that makes
// it idempotent -- `telos test` never checks a run against what is already
// in the journal, but a re-bind of the identical pair must answer with the
// one line already on disk, not a second copy of it.
// =============================================================================

/// The new, unbound code file the happy-path bind tests below claim: never
/// part of the seal, so writing it introduces no drift of its own (`status`
/// only ever compares the sealed spec files and `lock.code`, and a brand
/// new file is in neither).
const NEW_CODE_FILE: &str = "src/billing/new.rs";

/// The corpus' already-bound code file -- `telos/bindings.tel` implements
/// `INT-0042` with it -- so editing it after the seal is exactly the drift
/// the carve-out tests below claim through `telos bind`.
const INVOICE_CODE: &str = "src/billing/invoice.rs";

/// The intent every happy-path bind below targets: `active` with one
/// scenario and one binding already in the sealed corpus, so a no-op `edit
/// intent` is enough to make an open change its owner (D5) without
/// changing its content.
const BOUND_INTENT: &str = "INT-0042";

/// An intent the sealed corpus declares but no change stages: the owner
/// gate (D5) has nothing to find.
const UNOWNED_INTENT: &str = "INT-0017";

/// Stages a no-op `edit intent INT-0042` into `CHG-0001`. An empty patch
/// payload keeps every field exactly as `patch_intent`'s base default
/// leaves it, so the only effect of this call is making `CHG-0001` the
/// intent's owner (D5).
fn edit_int_0042(dir: &Path) {
    let out = telos(
        dir,
        &[
            "edit",
            "intent",
            BOUND_INTENT,
            "--change",
            "CHG-0001",
            "--json",
        ],
    )
    .write_stdin("{}")
    .output()
    .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
}

/// A project one `telos bind` away from its first binding: `CHG-0001`
/// approved, owning `INT-0042` through a no-op edit.
fn approved_owning_int_0042() -> TempDir {
    let tmp = with_fixture();
    open_change(tmp.path());
    edit_int_0042(tmp.path());
    approve(tmp.path());
    tmp
}

/// A valid payload for a brand-new intent, built entirely on the corpus'
/// own notions (`Invoice`'s `cancelled` state, unused by any sealed
/// intent): what the overlay-only-intent test below stages before binding
/// to the id the allocator hands back.
fn new_intent_payload() -> String {
    json!({
        "title": "Invoices can be cancelled", "status": "active",
        "telos": "Customers must be able to void an invoice raised in error.",
        "statement": { "template": "event-driven", "when": "PaymentReceived",
                       "on": "Invoice", "action": "set Invoice.state = cancelled" },
        "refines": [], "requires": [], "excludes": [],
        "scenarios": [
          { "title": "a payment cancels a disputed invoice",
            "given": [ {"notion": "Invoice", "fields": {"state": "open", "balance": "50.00 EUR"}} ],
            "when":  {"notion": "PaymentReceived", "fields": {"amount": "50.00 EUR"}},
            "then":  ["Invoice.state == cancelled"] } ]
    })
    .to_string()
}

/// Stages `add intent` into `change` and returns the id the allocator
/// minted -- an intent the sealed spec has never heard of, which is the
/// whole point of the overlay-only test.
fn stage_new_intent(dir: &Path, change: &str) -> String {
    let out = telos(dir, &["add", "intent", "--change", change, "--json"])
        .write_stdin(new_intent_payload())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    json_stdout(&out)["result"]["id"]
        .as_str()
        .expect("`add intent` reports the allocated id")
        .to_string()
}

/// `telos change open`, not asserting which id comes back -- unlike
/// [`open_change`], usable once `CHG-0001` is no longer guaranteed to be
/// the next one (a fixture that already reconciled and closed an earlier
/// change).
fn open_change_any(dir: &Path, motivation: &str) -> String {
    let out = telos(dir, &["change", "open", motivation, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    json_stdout(&out)["result"]["id"]
        .as_str()
        .expect("`change open` reports the allocated id")
        .to_string()
}

/// `telos change approve <id>`, for a change id other than `CHG-0001`.
fn approve_id(dir: &Path, id: &str) {
    let out = telos(dir, &["change", "approve", id, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
}

/// A project with a second, approved change whose **only** op is `remove
/// intent <id>` -- and returns that id.
///
/// The intent itself comes from a first change (`add intent`) that was
/// reconciled and sealed before the removal is even staged, so by
/// construction the removal shares its change with no companion `add`/
/// `edit` op for the same id: exactly the shape the `owner_of` regression
/// needs, and not reachable by editing one of the corpus' own intents (both
/// are cross-referenced -- `INT-0042` `requires INT-0017` and is itself
/// `implements`-bound -- so removing either one outright is refused by the
/// overlay's own referential-integrity check, several steps before
/// `telos bind` is ever reached).
fn approved_removal_of_a_fresh_intent(dir: &Path) -> String {
    let adding = open_change_any(dir, "a short-lived intent");
    let new_intent = stage_new_intent(dir, &adding);
    approve_id(dir, &adding);
    let out = telos(dir, &["change", "reconcile", &adding, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));

    let removing = open_change_any(dir, "removing it again");
    let out = telos(
        dir,
        &[
            "remove",
            "intent",
            &new_intent,
            "--change",
            &removing,
            "--json",
        ],
    )
    .output()
    .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    approve_id(dir, &removing);

    new_intent
}

/// Appends a comment to the corpus' already-bound, sealed code file --
/// which drifts the project on exactly the path the carve-out tests bind.
fn append_to_invoice_code(dir: &Path) {
    let path = dir.join(INVOICE_CODE);
    let mut src = fs::read_to_string(&path).unwrap();
    src.push_str("// touched by the implementer\n");
    fs::write(&path, src).unwrap();
}

// --- the happy path ----------------------------------------------------------

/// The Annex C result, key for key, binding a brand-new file.
#[test]
fn bind_records_a_new_file_with_the_annex_c_result() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    let out = telos(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "bind",
            "result": {
                "change": "CHG-0001",
                "path": NEW_CODE_FILE,
                "intent": BOUND_INTENT,
            },
            "error": null,
            "next_actions": ["telos change reconcile CHG-0001"]
        })
    );
}

/// The journal line, byte for byte (Annex A's padding group).
#[test]
fn bind_appends_the_exact_journal_line() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    telos(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    assert!(
        change_file(tmp.path())
            .contains(&format!("  bind \"{NEW_CODE_FILE}\" -> {BOUND_INTENT}\n")),
        "{}",
        change_file(tmp.path())
    );
}

/// D5's transition: the first journalled bind takes an `approved` change to
/// `implementing`, and the project -- which has no drift at all here -- is
/// `changing`.
#[test]
fn bind_moves_the_owner_to_implementing() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    telos(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    let envelope = run_json(tmp.path(), &["status", "--json"]);
    assert_eq!(envelope["result"]["state"], json!("changing"));
    assert_eq!(
        envelope["result"]["changes"],
        json!([{ "id": "CHG-0001", "status": "implementing", "obligations": ["reconcile"] }])
    );
}

/// D1's whole point: journalling a bind is digest-inert, so the change that
/// was approved before it is not stale after.
#[test]
fn bind_leaves_the_approval_fresh() {
    let tmp = approved_owning_int_0042();
    let approved_digest = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"])["result"]
        ["approved_digest"]
        .clone();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    telos(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    let envelope = run_json(tmp.path(), &["change", "diff", "CHG-0001", "--json"]);
    assert_eq!(envelope["result"]["stale"], json!(false));
    assert_eq!(envelope["result"]["approved_digest"], approved_digest);
    assert_eq!(envelope["result"]["status"], json!("implementing"));
}

/// Unlike a run, a bind is deduplicated (Annex C): the identical pair
/// journalled twice is one line, and the second call answers with exactly
/// the same result as the first.
#[test]
fn rebinding_the_same_pair_is_idempotent() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    let first = run_json(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"]);
    let second = run_json(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"]);

    assert_eq!(first, second);
    let content = change_file(tmp.path());
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| line.starts_with("  bind "))
        .collect();
    assert_eq!(
        lines,
        vec![format!("  bind \"{NEW_CODE_FILE}\" -> {BOUND_INTENT}")]
    );
}

// --- the carve-out -------------------------------------------------------

/// The precondition the carve-out exists for: editing the already-bound,
/// sealed source file drifts the project *before* any journal line claims
/// it.
#[test]
fn editing_the_bound_file_drifts_the_project_first() {
    let tmp = approved_owning_int_0042();
    append_to_invoice_code(tmp.path());

    let envelope = run_json(tmp.path(), &["status", "--json"]);

    assert_eq!(envelope["result"]["state"], json!("drifted"));
    assert_eq!(
        envelope["result"]["drift"],
        json!({ "paths": [INVOICE_CODE], "suggestion": "telos adopt" })
    );
}

/// The carve-out itself: `telos bind` succeeds even though the project was
/// drifted on the exact path it just claimed, and the project settles at
/// `changing` rather than `drifted`.
#[test]
fn bind_admits_the_drift_of_the_path_it_claims() {
    let tmp = approved_owning_int_0042();
    append_to_invoice_code(tmp.path());

    let out = telos(tmp.path(), &["bind", INVOICE_CODE, BOUND_INTENT, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    let envelope = run_json(tmp.path(), &["status", "--json"]);
    assert_eq!(envelope["result"]["state"], json!("changing"));
    assert_eq!(envelope["result"]["drift"], Value::Null);
}

// --- the gates, in the order the flow freezes them --------------------------

/// An id no change and no spec file declares: the same `unknown intent`
/// shape `show`/`telos test` answer with, nearest existing id included.
#[test]
fn bind_on_an_unknown_intent_names_the_nearest_id() {
    let tmp = with_fixture();

    let error = error_of(tmp.path(), &["bind", NEW_CODE_FILE, "INT-9999", "--json"]);

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(error["message"], json!("unknown intent `INT-9999`"));
    assert_eq!(error["hint"], json!("closest is INT-0042"));
}

/// D5: an intent the sealed spec declares but no open change claims is
/// nobody's to bind.
#[test]
fn bind_on_an_intent_no_change_owns_is_refused() {
    let tmp = with_fixture();

    let error = error_of(
        tmp.path(),
        &["bind", NEW_CODE_FILE, UNOWNED_INTENT, "--json"],
    );

    assert_eq!(error["code"], json!("TELOS_CHANGE_STATE_INVALID"));
    assert_eq!(
        error["message"],
        json!("no open change is implementing INT-0017")
    );
    assert_eq!(
        error["hint"],
        json!("stage it into a change and approve it first")
    );
}

/// The owner exists but has not been reviewed: the M2 wording, reused.
#[test]
fn bind_on_a_drafted_owner_asks_for_the_approval_first() {
    let tmp = with_fixture();
    open_change(tmp.path());
    edit_int_0042(tmp.path());

    let error = error_of(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"]);

    assert_eq!(error["code"], json!("TELOS_CHANGE_STATE_INVALID"));
    assert_eq!(
        error["message"],
        json!("change CHG-0001 is not approved; approve it first")
    );
    assert_eq!(
        error["hint"],
        json!("run `telos change diff CHG-0001` then `telos change approve CHG-0001`")
    );
}

/// A path that does not exist on disk: the M2 seal wording, reused
/// verbatim, no hint.
#[test]
fn bind_of_a_missing_path_names_it() {
    let tmp = approved_owning_int_0042();

    let error = error_of(
        tmp.path(),
        &["bind", "src/billing/missing.rs", BOUND_INTENT, "--json"],
    );

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    assert_eq!(
        error["message"],
        json!("binding references `src/billing/missing.rs`, which does not exist")
    );
    assert_eq!(error["hint"], Value::Null);
}

/// An absolute path is not repo-relative -- refused before any change or
/// file is even looked at.
#[test]
fn bind_of_an_absolute_path_is_refused() {
    let tmp = with_fixture();

    let error = error_of(tmp.path(), &["bind", "/abs/x.rs", BOUND_INTENT, "--json"]);

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        error["message"],
        json!("cannot parse `/abs/x.rs` as a repository-relative path")
    );
    assert_eq!(error["hint"], Value::Null);
}

/// A path under `telos/` is refused with the grammar's own wording -- the
/// same message `parse_change_file` would answer with if this line reached
/// disk and were read back.
#[test]
fn bind_of_a_path_under_telos_is_refused() {
    let tmp = with_fixture();

    let error = error_of(
        tmp.path(),
        &["bind", "telos/bindings.tel", BOUND_INTENT, "--json"],
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        error["message"],
        json!("a journal line cannot name a path under telos/")
    );
    assert_eq!(
        error["hint"],
        json!(
            "journal lines name code and test files; the spec tree is written by ops and by reconcile"
        )
    );
}

/// The carve-out is exactly one path wide (D6): drift anywhere else is
/// still damage nobody claimed, and the command refuses before writing
/// anything.
#[test]
fn bind_refuses_unclaimed_drift_elsewhere() {
    let tmp = approved_owning_int_0042();
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();
    let invoice_notion = tmp.path().join("telos/notions/Invoice.tel");
    let mut src = fs::read_to_string(&invoice_notion).unwrap();
    src.push('\n');
    fs::write(&invoice_notion, src).unwrap();

    let error = error_of(tmp.path(), &["bind", NEW_CODE_FILE, BOUND_INTENT, "--json"]);

    assert_eq!(error["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(
        error["message"],
        json!("the project has drifted from its seal")
    );
    assert_eq!(error["hint"], json!(DRIFT_HINT));
    // Nothing was written: the change is still approved, with no journal.
    assert!(
        !change_file(tmp.path()).contains("  bind "),
        "{}",
        change_file(tmp.path())
    );
}

// --- an intent only the open change's overlay knows about -------------------

/// D5's ownership rule reaches an intent `add intent` allocated a moment
/// ago just as well as one the sealed spec has always known: the overlay is
/// part of "known" and part of "owned" alike.
#[test]
fn bind_on_an_intent_only_the_open_change_knows_about() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let new_intent = stage_new_intent(tmp.path(), "CHG-0001");
    approve(tmp.path());
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    let out = telos(tmp.path(), &["bind", NEW_CODE_FILE, &new_intent, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json_stdout(&out)["result"],
        json!({
            "change": "CHG-0001",
            "path": NEW_CODE_FILE,
            "intent": new_intent,
        })
    );
}

// --- an intent a change only *removes* is never its owner --------------------

/// The review's regression, pinned at the CLI layer: a change whose only op
/// on an intent *removes* it must never be treated as that intent's owner.
///
/// Today the failure mode this guards is masked one layer up -- once the
/// removal is staged, `require_known`'s fold withdraws the id from the
/// known set the same way it withdraws any removed id, so `telos bind`
/// answers `TELOS_REFERENCE_UNKNOWN` before ownership is even consulted.
/// That is asserted here, as the current, observable behaviour; the
/// ownership predicate itself is pinned directly, independent of
/// `require_known`, by `owner_of_never_selects_a_change_that_only_removes_
/// the_intent` in `bind.rs`'s own unit tests -- the pin that survives a
/// future `require_known` that stops withdrawing removed ids for some
/// unrelated reason.
#[test]
fn bind_to_an_intent_a_change_only_removes_is_unknown_not_owned() {
    let tmp = with_fixture();
    let removed_intent = approved_removal_of_a_fresh_intent(tmp.path());
    fs::write(tmp.path().join(NEW_CODE_FILE), "// new\n").unwrap();

    let error = error_of(
        tmp.path(),
        &["bind", NEW_CODE_FILE, &removed_intent, "--json"],
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        error["message"],
        json!(format!("unknown intent `{removed_intent}`"))
    );
    assert_eq!(error["hint"], json!("closest is INT-0042"));
}
