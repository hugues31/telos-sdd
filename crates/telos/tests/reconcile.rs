//! End-to-end tests for `telos change reconcile`: the transaction that turns
//! a reviewed delta into spec files on disk and a fresh seal (`CHG-NNNN`),
//! and the delta-less reseal that proves a whole project from scratch
//! (`--full`, D12 -- last section).
//!
//! The shape of this file follows the frozen gate order of T10 -- drift,
//! status, digest, accept OIDs, overlay, rule 5, constraint checks, tests,
//! and only then the writes (D6). Each gate gets a test that proves it
//! refuses *and* that the refusal cost nothing: no `.tel` file appeared, the
//! lock did not move, the change file is still there.
//!
//! Two fixtures are used, deliberately: a freshly `init`ed repository (empty
//! globs, no constraint, no `[test] cmd`) for the pure transaction, and the
//! sealed `billing` corpus for everything that needs bindings, a global
//! constraint, or a scenario to run a test for. The corpus' commands are
//! `git` invocations (D9/D13) so the suite runs identically on the three
//! supported OSes.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use common::{repo, telos, unsealed_fixture, with_fixture, with_fixture_mut};

// --- plumbing --------------------------------------------------------------

const MOTIVATION: &str = "Invoices can be settled";
const CHG_0001: &str = "telos/changes/CHG-0001.tel";
const LOCK: &str = "telos/telos.lock";

/// The exact `TELOS_DRIFT_DETECTED` hint, frozen by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

/// The exact `TELOS_ORPHAN_CODE` hint, frozen by `docs/contracts.md`.
const ORPHAN_HINT: &str = "Bind it with `telos bind <path> <INT-id>`, or remove it from the `telos.toml` globs if it isn't spec-governed.";

/// The exact `TELOS_CONSTRAINT_FAILED` hint, frozen by `docs/contracts.md`.
const CONSTRAINT_HINT: &str = "Run the constraint's `check` command directly to see its output.";

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// A fresh git repository with an initialized, sealed and empty `telos/`.
fn fresh() -> tempfile::TempDir {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    tmp
}

fn open_change(dir: &Path) {
    telos(dir, &["change", "open", MOTIVATION])
        .assert()
        .success();
}

/// Stages one op, asserting it landed.
fn stage(dir: &Path, args: &[&str], payload: &str) {
    let mut cmd = telos(dir, args);
    let out = cmd.write_stdin(payload.to_string()).output().unwrap();
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected the staging to succeed, got {envelope}"
    );
}

fn approve(dir: &Path) {
    telos(dir, &["change", "approve", "CHG-0001"])
        .assert()
        .success();
}

/// Runs `telos change reconcile CHG-0001 --json` and returns the envelope.
fn reconcile(dir: &Path) -> Value {
    let out = telos(dir, &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();
    json_stdout(&out)
}

fn reconcile_ok(dir: &Path) -> Value {
    let envelope = reconcile(dir);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected the reconcile to succeed, got {envelope}"
    );
    envelope["result"].clone()
}

fn reconcile_err(dir: &Path) -> Value {
    let envelope = reconcile(dir);
    assert_eq!(
        envelope["ok"],
        json!(false),
        "expected the reconcile to fail, got {envelope}"
    );
    envelope["error"].clone()
}

// --- the payloads of the happy path (Annex D) -------------------------------

fn invoice_payload() -> String {
    json!({
        "name": "Invoice", "kind": "entity",
        "def": "A bill issued to a Customer for delivered work.",
        "attrs": [ {"name": "state", "type": "enum", "values": ["open", "settled"]} ]
    })
    .to_string()
}

fn payment_received_payload() -> String {
    json!({
        "name": "PaymentReceived", "kind": "event",
        "def": "A payment arrived for an invoice.",
        "attrs": [ {"name": "amount", "type": "money"} ]
    })
    .to_string()
}

fn settle_intent_payload() -> String {
    json!({
        "title": "Invoices can be settled", "status": "active",
        "telos": "Customers must see immediately that their debt is cleared.",
        "statement": { "template": "event-driven", "when": "PaymentReceived",
                       "on": "Invoice", "action": "set Invoice.state = settled" },
        "refines": [], "requires": [], "excludes": [],
        "scenarios": [
          { "title": "a full payment settles the invoice",
            "given": [ {"notion": "Invoice", "fields": {"state": "open"}} ],
            "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
            "then":  ["Invoice.state == settled"] } ]
    })
    .to_string()
}

/// A freshly initialized project carrying one approved, three-op change: two
/// notions and the intent that uses them.
fn approved_three_op_change() -> tempfile::TempDir {
    let tmp = fresh();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &invoice_payload(),
    );
    stage(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &payment_received_payload(),
    );
    stage(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
        &settle_intent_payload(),
    );
    approve(tmp.path());
    tmp
}

/// The corpus with one approved, one-op change: `INT-0042`'s `telos`
/// reworded. What makes it the interesting case is what the edit *reaches* --
/// SCN-0107, the corpus' only proved scenario (D10).
fn approved_int_0042_edit(fixture: tempfile::TempDir) -> tempfile::TempDir {
    open_change(fixture.path());
    stage(
        fixture.path(),
        &[
            "edit", "intent", "INT-0042", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "Customers must see their debt cleared -- reworded."}).to_string(),
    );
    approve(fixture.path());
    fixture
}

// --- the happy path ---------------------------------------------------------

/// The golden envelope of Annex E, on a fresh project with nothing to check
/// and no runner configured.
#[test]
fn reconcile_json_matches_the_golden_envelope() {
    let tmp = approved_three_op_change();

    let out = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?} -- {}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "change",
            "result": {
                "id": "CHG-0001",
                "full": false,
                "ops_applied": 3,
                "checks_run": 0,
                "tests_run": 0
            },
            "error": null,
            "next_actions": ["telos status"]
        })
    );
}

/// Every op's target lands as the emitter's own bytes -- reconcile re-emits
/// whole files from the model, it never edits text.
#[test]
fn reconcile_writes_the_canonical_spec_files_byte_for_byte() {
    let tmp = approved_three_op_change();

    reconcile_ok(tmp.path());

    assert_eq!(
        read(tmp.path(), "telos/notions/Invoice.tel"),
        "notion Invoice entity {\n  \
           def  \"A bill issued to a Customer for delivered work.\"\n  \
           attr state enum(open, settled)\n\
         }\n"
    );
    assert_eq!(
        read(tmp.path(), "telos/notions/PaymentReceived.tel"),
        "notion PaymentReceived event {\n  \
           def  \"A payment arrived for an invoice.\"\n  \
           attr amount money\n\
         }\n"
    );
    assert_eq!(
        read(tmp.path(), "telos/intents/INT-0001.tel"),
        "intent INT-0001 \"Invoices can be settled\" {\n  \
           status active\n  \
           telos  \"Customers must see immediately that their debt is cleared.\"\n  \
           statement event-driven {\n    \
             when   PaymentReceived on Invoice\n    \
             system shall set Invoice.state = settled\n  \
           }\n\
         \n  \
           scenario SCN-0001 \"a full payment settles the invoice\" {\n    \
             given Invoice { state: open }\n    \
             when  PaymentReceived { amount: \"120.00 EUR\" }\n    \
             then  Invoice.state == settled\n  \
           }\n\
         }\n"
    );
}

/// The change file is gone (D16: reconciled is deletion, the audit trail is
/// git) and `counters.toml` -- which is not a change -- stays.
#[test]
fn reconcile_leaves_only_the_counters_in_the_changes_directory() {
    let tmp = approved_three_op_change();

    reconcile_ok(tmp.path());

    let mut names: Vec<String> = fs::read_dir(tmp.path().join("telos/changes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["counters.toml".to_string()]);
    // D4: the ids this transaction burnt are never handed out again.
    assert_eq!(
        read(tmp.path(), "telos/changes/counters.toml"),
        "intent = 1\nscenario = 1\nconstraint = 0\nchange = 1\n"
    );
}

/// The new seal records the change that produced it, and the project is
/// `coherent` again -- nothing drifted, nothing open.
#[test]
fn reconcile_seals_the_lock_with_the_change_that_produced_it() {
    let tmp = approved_three_op_change();
    let before = read(tmp.path(), LOCK);

    reconcile_ok(tmp.path());

    let lock = read(tmp.path(), LOCK);
    assert_ne!(lock, before, "the seal must have moved");
    assert!(
        lock.contains("sealed_by = \"CHG-0001\"\n"),
        "lock does not record its change:\n{lock}"
    );
    for path in [
        "telos/intents/INT-0001.tel",
        "telos/notions/Invoice.tel",
        "telos/notions/PaymentReceived.tel",
    ] {
        assert!(lock.contains(path), "`{path}` is not sealed:\n{lock}");
    }

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["state"], json!("coherent"));
    assert_eq!(envelope["result"]["changes"], json!([]));
}

// --- gate 1: drift -----------------------------------------------------------

/// D17: unclaimed drift refuses the reconcile, and the message names the
/// path so a caller does not have to run `status` to find out which.
#[test]
fn reconcile_refuses_unclaimed_drift() {
    let tmp = approved_int_0042_edit(with_fixture());
    // Drifted *after* the approval, so `approve`'s own gate cannot catch it.
    let path = tmp.path().join("telos/notions/Invoice.tel");
    let mut content = fs::read_to_string(&path).unwrap();
    content.push('\n');
    fs::write(&path, content).unwrap();

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], json!("TELOS_DRIFT_DETECTED"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("telos/notions/Invoice.tel"),
        "the drifted path is not named: {message}"
    );
    assert_eq!(error["hint"], json!(DRIFT_HINT));
    assert!(
        tmp.path().join(CHG_0001).exists(),
        "a refused reconcile must not delete the change"
    );
}

/// D5's other half: a path *this* change claims is the change in progress,
/// not damage -- the reconcile goes through and overwrites it with the op's
/// canonical post-state, whatever the working tree had made of it.
#[test]
fn drift_on_the_reconciled_changes_own_claim_does_not_block_it() {
    let tmp = approved_int_0042_edit(with_fixture());
    let path = tmp.path().join("telos/intents/INT-0042.tel");
    // Still parseable (the overlay's base is read from disk), but no longer
    // the sealed bytes -- drift on a path this very change claims.
    let drifted = format!("{}\n", read(tmp.path(), "telos/intents/INT-0042.tel"));
    fs::write(&path, &drifted).unwrap();

    reconcile_ok(tmp.path());

    let written = read(tmp.path(), "telos/intents/INT-0042.tel");
    assert!(
        written.contains("Customers must see their debt cleared -- reworded."),
        "the op's post-state did not overwrite the drifted file:\n{written}"
    );
    assert!(
        !written.ends_with("}\n\n"),
        "the file kept the drifted trailing blank line instead of the canonical bytes"
    );
}

// --- gate 2: status ----------------------------------------------------------

/// Reconciling a change nobody approved is refused with the exact message
/// and hint of the frozen contract.
#[test]
fn reconcile_refuses_an_unapproved_change() {
    let tmp = fresh();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &invoice_payload(),
    );

    let error = reconcile_err(tmp.path());

    assert_eq!(
        error,
        json!({
            "code": "TELOS_CHANGE_STATE_INVALID",
            "message": "change CHG-0001 is not approved; approve it first",
            "hint": "run `telos change diff CHG-0001` then `telos change approve CHG-0001`"
        })
    );
    assert!(
        !tmp.path().join("telos/notions/Invoice.tel").exists(),
        "a refused reconcile must write nothing"
    );
}

// --- gate 3: digest ----------------------------------------------------------

/// D3: staging into an approved change is allowed, reconciling the result is
/// not -- the approval was taken against a delta that has since moved.
#[test]
fn reconcile_refuses_a_change_staged_into_after_its_approval() {
    let tmp = approved_three_op_change();
    stage(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &json!({"name": "Customer", "kind": "actor", "def": "A payer."}).to_string(),
    );

    let error = reconcile_err(tmp.path());

    assert_eq!(
        error,
        json!({
            "code": "TELOS_APPROVAL_STALE",
            "message": "the staged delta changed after approval",
            "hint": "re-approve with `telos change approve CHG-0001`"
        })
    );
    assert!(!tmp.path().join("telos/notions/Invoice.tel").exists());

    // Re-approving is idempotent (D16) and unblocks the same delta.
    approve(tmp.path());
    assert_eq!(reconcile_ok(tmp.path())["ops_applied"], json!(4));
}

// --- gate 6: rule 5, no code without telos -----------------------------------

/// D8: a file the `[code]` globs match and no `implements` binding covers
/// stops the transaction, with the frozen message and hint.
#[test]
fn reconcile_refuses_code_no_binding_covers() {
    let tmp = approved_int_0042_edit(with_fixture());
    fs::write(tmp.path().join("src/stray.rs"), "// nobody's code\n").unwrap();

    let error = reconcile_err(tmp.path());

    assert_eq!(
        error,
        json!({
            "code": "TELOS_ORPHAN_CODE",
            "message": "`src/stray.rs` matches the [code] globs but no `implements` binding covers it",
            "hint": ORPHAN_HINT
        })
    );
    assert!(tmp.path().join(CHG_0001).exists());
}

// --- gate 7: constraint checks (D11) -----------------------------------------

/// A constraint staged by this very change is in scope for its own reconcile:
/// the check runs, fails, and names both the constraint and the command.
/// Making the command succeed then makes the same reconcile go through.
#[test]
fn a_failing_constraint_check_refuses_the_reconcile_until_it_passes() {
    let tmp = fresh();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &["add", "constraint", "--change", "CHG-0001", "--json"],
        &json!({
            "kind": "architecture", "title": "Hexagonal boundaries",
            "rule": {"text": "Domain code must not import adapter modules."},
            "scope": "global", "check": "git hash-object missing-check-marker"
        })
        .to_string(),
    );
    approve(tmp.path());

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], json!("TELOS_CONSTRAINT_FAILED"));
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("CON-0001"), "{message}");
    assert!(
        message.contains("git hash-object missing-check-marker"),
        "{message}"
    );
    assert_eq!(error["hint"], json!(CONSTRAINT_HINT));
    assert!(
        !tmp.path().join("telos/constraints/CON-0001.tel").exists(),
        "a refused reconcile must write nothing"
    );

    fs::write(tmp.path().join("missing-check-marker"), "").unwrap();

    assert_eq!(
        reconcile_ok(tmp.path()),
        json!({
            "id": "CHG-0001",
            "full": false,
            "ops_applied": 1,
            "checks_run": 1,
            "tests_run": 0
        })
    );
}

// --- gate 8: tests (D10) ------------------------------------------------------

/// `{filter}` is substituted with the `proves` binding's test *name*, one
/// invocation per distinct `TestRef` of the impacted scenarios. The corpus'
/// edit reaches SCN-0107 through `verifies`, so exactly one test runs -- and
/// the corpus' own global constraint check runs alongside it.
#[test]
fn reconcile_runs_one_test_per_impacted_scenario() {
    let tmp = approved_int_0042_edit(with_fixture_mut(set_test_cmd));
    // `git hash-object <path>` succeeds exactly when the path exists: the
    // marker's name *is* the substituted filter, which is what proves the
    // substitution end to end.
    fs::write(
        tmp.path().join("scn_0107_full_payment_settles_the_invoice"),
        "",
    )
    .unwrap();

    assert_eq!(
        reconcile_ok(tmp.path()),
        json!({
            "id": "CHG-0001",
            "full": false,
            "ops_applied": 1,
            "checks_run": 1,
            "tests_run": 1
        })
    );
}

/// The same setup without the marker: the run fails, and the message carries
/// the substituted command verbatim.
#[test]
fn a_failing_test_run_refuses_the_reconcile_and_reports_the_substituted_command() {
    let tmp = approved_int_0042_edit(with_fixture_mut(set_test_cmd));

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("git hash-object scn_0107_full_payment_settles_the_invoice"),
        "the substituted command is not in the message: {message}"
    );
    assert!(tmp.path().join(CHG_0001).exists());
    assert!(
        !read(tmp.path(), "telos/intents/INT-0042.tel").contains("reworded"),
        "a refused reconcile must write nothing"
    );
}

/// The corpus ships `[test] cmd = ""` (D13); this puts a real, cross-OS
/// runner in its place before the fixture is sealed.
///
/// `git hash-object {filter}` is chosen so that both invocations the suite
/// needs are meaningful: with a filter it succeeds exactly when the named
/// file exists (which is how the substitution is proved end to end), and
/// with the empty filter of a `--full` run -- the one the fixture's own seal
/// performs -- it degenerates to a bare `git hash-object`, which hashes
/// nothing and exits 0.
fn set_test_cmd(root: &Path) {
    set_test_cmd_to(root, "git hash-object {filter}");
}

/// Rewrites the corpus' empty `[test] cmd` to `cmd`, before the seal.
fn set_test_cmd_to(root: &Path, cmd: &str) {
    let path = root.join("telos/telos.toml");
    let content = fs::read_to_string(&path).unwrap();
    let replaced = content.replace("cmd = \"\"", &format!("cmd = {cmd:?}"));
    assert_ne!(replaced, content, "the corpus no longer ships an empty cmd");
    fs::write(&path, replaced).unwrap();
}

// --- `reconcile --full` (D12) ------------------------------------------------
//
// The lock-merge exit, and the one legitimate way to seal a spec tree that
// exists but was never sealed. It re-proves everything from the files on
// disk, so it reads no lock, applies no op, and gates on no drift.

/// Runs `telos change reconcile --full --json` and returns the envelope.
fn reconcile_full(dir: &Path) -> Value {
    let out = telos(dir, &["change", "reconcile", "--full", "--json"])
        .output()
        .unwrap();
    json_stdout(&out)
}

fn reconcile_full_ok(dir: &Path) -> Value {
    let envelope = reconcile_full(dir);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected the full reconcile to succeed, got {envelope}"
    );
    envelope["result"].clone()
}

/// The golden envelope of a full reseal over the corpus, unsealed: no id, no
/// op, the corpus' one global constraint checked (`git --version`), and no
/// test run at all since the corpus ships `[test] cmd = ""` (D13).
#[test]
fn full_reconcile_seals_an_unsealed_project() {
    let tmp = unsealed_fixture();
    assert!(
        !tmp.path().join(LOCK).exists(),
        "the fixture starts unsealed"
    );

    let out = telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?} -- {}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "change",
            "result": {
                "id": null,
                "full": true,
                "ops_applied": 0,
                "checks_run": 1,
                "tests_run": 0
            },
            "error": null,
            "next_actions": ["telos status"]
        })
    );

    // D12: a full reseal is nobody's change, so the lock records none.
    let lock = read(tmp.path(), LOCK);
    assert!(
        !lock.contains("sealed_by"),
        "a full reseal must not claim a change:\n{lock}"
    );
    assert!(lock.contains("telos/intents/INT-0042.tel"), "{lock}");
    assert!(lock.contains("src/billing/invoice.rs"), "{lock}");

    let envelope = json_stdout(&telos(tmp.path(), &["status", "--json"]).output().unwrap());
    assert_eq!(envelope["result"]["state"], json!("coherent"));
}

/// The whole point of D12: the existing lock is treated as unreliable and
/// never read, so the merge conflict a `git merge` left in it is not an
/// obstacle -- it is the very reason to run this.
#[test]
fn full_reconcile_never_reads_the_existing_lock() {
    let tmp = unsealed_fixture();
    fs::write(
        tmp.path().join(LOCK),
        "<<<<<<< HEAD\nversion = 1\n=======\nnot even toml\n>>>>>>> theirs\n",
    )
    .unwrap();

    assert_eq!(reconcile_full_ok(tmp.path())["full"], json!(true));

    let lock = read(tmp.path(), LOCK);
    assert!(
        !lock.contains("<<<<<<<"),
        "the conflicted lock was not replaced:\n{lock}"
    );
    let envelope = json_stdout(&telos(tmp.path(), &["status", "--json"]).output().unwrap());
    assert_eq!(envelope["result"]["state"], json!("coherent"));
}

/// A full reseal proves the spec the same way a reconcile does: a reference
/// that resolves to nothing is refused, and nothing is sealed.
#[test]
fn full_reconcile_refuses_a_spec_that_does_not_resolve() {
    let tmp = unsealed_fixture();
    let path = tmp.path().join("telos/intents/INT-0042.tel");
    let content = read(tmp.path(), "telos/intents/INT-0042.tel");
    fs::write(
        &path,
        content.replace("requires INT-0017", "requires INT-9999"),
    )
    .unwrap();

    let envelope = reconcile_full(tmp.path());

    assert_eq!(envelope["ok"], json!(false), "got {envelope}");
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert!(
        !tmp.path().join(LOCK).exists(),
        "a refused full reconcile must seal nothing"
    );
}

/// D10's `--full` half: one invocation of `[test] cmd` with `{filter}`
/// substituted by nothing -- the whole suite, once -- however many scenarios
/// the spec proves.
#[test]
fn full_reconcile_runs_the_whole_suite_once() {
    let tmp = with_fixture_mut(|root| set_test_cmd_to(root, "git --version"));

    assert_eq!(
        reconcile_full_ok(tmp.path()),
        json!({
            "id": null,
            "full": true,
            "ops_applied": 0,
            "checks_run": 1,
            "tests_run": 1
        })
    );
}

/// D12: open changes are tolerated rather than reconciled or refused. They
/// stay open, their files untouched -- a full reseal is about the spec on
/// disk, not about anybody's staged delta.
#[test]
fn full_reconcile_leaves_open_changes_alone() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let before = read(tmp.path(), CHG_0001);

    assert_eq!(reconcile_full_ok(tmp.path())["ops_applied"], json!(0));

    assert_eq!(read(tmp.path(), CHG_0001), before, "the change file moved");
    let envelope = json_stdout(&telos(tmp.path(), &["status", "--json"]).output().unwrap());
    assert_eq!(envelope["result"]["state"], json!("changing"));
}

/// `--full` reseals *everything*; a change id says «this delta, and only
/// it». Asking for both is a contradiction clap refuses before any command
/// runs (exit 2, usage error -- not a domain error in the envelope).
#[test]
fn an_id_and_full_together_are_a_usage_error() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["change", "reconcile", "CHG-0001", "--full", "--json"],
    )
    .output()
    .unwrap();

    assert_eq!(out.status.code(), Some(2), "expected clap's usage exit");
}
