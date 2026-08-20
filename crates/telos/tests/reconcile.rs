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
                "tests_run": 0,
                "witness_warnings": []
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
            "tests_run": 0,
            "witness_warnings": []
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
            "tests_run": 1,
            "witness_warnings": []
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
                "tests_run": 0,
                "witness_warnings": []
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
            "tests_run": 1,
            "witness_warnings": []
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

// =============================================================================
// M3: the folded journal (D2), the sealed-witness gate (D7), and the advisory
// warnings.
//
// These drive the real feature loop end to end -- `open`, `add`, `approve`,
// `test` red, `test` green, `bind`, `reconcile` -- on a project that starts
// as a bare `telos init`. What they assert about reconcile is the M3 half of
// it: `telos/bindings.tel` is *derived* from the journal at reconcile and
// never written before it (D2), the witness discipline is a static gate over
// that journal (D7), and under `tdd = "advisory"` the same verdicts come back
// as warnings instead of refusals.
// =============================================================================

/// The runner every fixture below wires up: `git hash-object .fake-green`
/// exits 0 while [`MARKER`] exists and non-zero once it is deleted, which is
/// a deterministic, cross-OS red/green switch that needs no test framework of
/// its own (the same one `test_bind.rs` runs on).
const RUNNER: &str = "git hash-object .fake-green";

/// The file [`RUNNER`] hashes -- its absence is what makes a run red.
const MARKER: &str = ".fake-green";

/// The test file the witness protocol discovers by convention (D4), and the
/// code file the implementation is bound through.
const TEST_FILE: &str = "tests/billing.rs";
const CODE_FILE: &str = "src/billing.rs";

/// The derived bindings table (D2): written by reconcile, claimed by nobody.
const BINDINGS: &str = "telos/bindings.tel";

/// The ids the allocator mints for the delta [`approved_feature`] stages.
/// Captured from the command results rather than hardcoded -- the loop must
/// never make a caller know them in advance -- and carried here so the
/// assertions can name them.
struct Feature {
    change: String,
    intent: String,
    scenario: String,
}

/// Runs `telos <args>` and returns the parsed envelope, whatever its verdict.
fn run_json(dir: &Path, args: &[&str]) -> Value {
    json_stdout(&telos(dir, args).output().unwrap())
}

/// Runs a command that must succeed and returns its `result`.
fn ok_result(dir: &Path, args: &[&str]) -> Value {
    let envelope = run_json(dir, args);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected `telos {}` to succeed, got {envelope}",
        args.join(" ")
    );
    envelope["result"].clone()
}

fn approve_id(dir: &Path, id: &str) {
    ok_result(dir, &["change", "approve", id, "--json"]);
}

fn reconcile_id(dir: &Path, id: &str) -> Value {
    run_json(dir, &["change", "reconcile", id, "--json"])
}

fn reconcile_id_ok(dir: &Path, id: &str) -> Value {
    let envelope = reconcile_id(dir, id);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected the reconcile to succeed, got {envelope}"
    );
    envelope["result"].clone()
}

fn reconcile_id_err(dir: &Path, id: &str) -> Value {
    let envelope = reconcile_id(dir, id);
    assert_eq!(
        envelope["ok"],
        json!(false),
        "expected the reconcile to fail, got {envelope}"
    );
    envelope["error"].clone()
}

/// A freshly `init`ed project reconfigured **through the protocol**: globs
/// over `src/` and `tests/`, the [`RUNNER`], and `policy.tdd = tdd`.
///
/// `telos/telos.toml` is itself a sealed spec file, so writing it directly is
/// drift -- and `adopt` is the only way to stage a file telos models no
/// entity for (an `accept` op, gate 4's own subject). Going through it here
/// rather than seeding the config before the first seal is what makes this a
/// project a user could actually have produced, and it costs one change:
/// `CHG-0001` is the config change, so every helper below takes the change id
/// it works on rather than assuming one.
fn configured(tdd: &str) -> tempfile::TempDir {
    let tmp = fresh();
    fs::write(
        tmp.path().join("telos/telos.toml"),
        format!(
            "[code]\nglobs = [\"src/**/*.rs\"]\n\n\
             [tests]\nglobs = [\"tests/**/*.rs\"]\n\n\
             [test]\ncmd = \"{RUNNER}\"\n\n\
             [policy]\ntdd = \"{tdd}\"\n"
        ),
    )
    .unwrap();

    let adopted = ok_result(tmp.path(), &["adopt", "--json"]);
    let id = adopted["change"]
        .as_str()
        .expect("`adopt` answers with the change that captured the drift")
        .to_string();
    approve_id(tmp.path(), &id);
    assert_eq!(
        reconcile_id_ok(tmp.path(), &id)["ops_applied"],
        json!(1),
        "the config change is one `accept` op"
    );
    tmp
}

/// The scenario payload the intent below carries, `title` apart.
fn scenario_payload(title: &str) -> Value {
    json!({
        "title": title,
        "given": [ {"notion": "Invoice", "fields": {"state": "open"}} ],
        "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
        "then":  ["Invoice.state == settled"]
    })
}

/// Stages the delta every M3 test works from -- the two notions and the
/// intent carrying `scenarios` -- into a fresh change, and approves it.
///
/// `Feature.scenario` is the *first* scenario's id; a caller that stages two
/// derives the second from the same result. No id is hardcoded anywhere: the
/// allocator's answers are read back out of the envelopes (§14's anti-goal).
fn approved_feature(dir: &Path, scenarios: Vec<Value>) -> Feature {
    let change = ok_result(dir, &["change", "open", MOTIVATION, "--json"])["id"]
        .as_str()
        .expect("`change open` answers with the allocated id")
        .to_string();

    stage(
        dir,
        &["add", "notion", "--change", &change, "--json"],
        &invoice_payload(),
    );
    stage(
        dir,
        &["add", "notion", "--change", &change, "--json"],
        &payment_received_payload(),
    );

    let payload = json!({
        "title": "Invoices can be settled", "status": "active",
        "telos": "Customers must see immediately that their debt is cleared.",
        "statement": { "template": "event-driven", "when": "PaymentReceived",
                       "on": "Invoice", "action": "set Invoice.state = settled" },
        "refines": [], "requires": [], "excludes": [],
        "scenarios": scenarios
    })
    .to_string();
    let mut cmd = telos(dir, &["add", "intent", "--change", &change, "--json"]);
    let added = json_stdout(&cmd.write_stdin(payload).output().unwrap());
    assert_eq!(
        added["ok"],
        json!(true),
        "expected the intent to stage: {added}"
    );

    let feature = Feature {
        intent: added["result"]["id"].as_str().unwrap().to_string(),
        scenario: added["result"]["scenario_ids"][0]
            .as_str()
            .expect("`add intent` reports the scenario ids it allocated")
            .to_string(),
        change,
    };
    approve_id(dir, &feature.change);
    feature
}

/// The test function one scenario's witness is discovered through (D4).
fn test_fn(scenario: &str) -> String {
    format!(
        "scn_{}_a_full_payment_settles_the_invoice",
        scenario.trim_start_matches("SCN-")
    )
}

/// Writes `tests/billing.rs` holding one function per scenario, plus
/// `trailer` -- which is how a test moves the file's bytes between two runs
/// without changing what the runner does.
fn write_test_file(dir: &Path, scenarios: &[&str], trailer: &str) {
    fs::create_dir_all(dir.join("tests")).unwrap();
    let mut src = String::new();
    for scenario in scenarios {
        src.push_str(&format!("fn {}() {{}}\n", test_fn(scenario)));
    }
    src.push_str(trailer);
    fs::write(dir.join(TEST_FILE), src).unwrap();
}

/// `telos test <scenario>`, asserting the witness it sealed.
fn witness(dir: &Path, scenario: &str, expected: &str) {
    let result = ok_result(dir, &["test", scenario, "--json"]);
    assert_eq!(
        result["witness"],
        json!(expected),
        "expected a {expected} witness for {scenario}, got {result}"
    );
}

fn set_marker(dir: &Path) {
    fs::write(dir.join(MARKER), "").unwrap();
}

/// Writes the minimal domain file and binds it to `intent`.
fn write_and_bind_code(dir: &Path, intent: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join(CODE_FILE),
        "// Minimal domain code, named after the notions it implements.\n",
    )
    .unwrap();
    ok_result(dir, &["bind", CODE_FILE, intent, "--json"]);
}

/// A project one reconcile away from its first proved scenario: the delta
/// approved, the test written and witnessed red *then* green on the very same
/// bytes, the code written and bound.
fn implemented() -> (tempfile::TempDir, Feature) {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    write_test_file(tmp.path(), &[&feature.scenario], "");
    witness(tmp.path(), &feature.scenario, "red");
    set_marker(tmp.path());
    witness(tmp.path(), &feature.scenario, "green");
    write_and_bind_code(tmp.path(), &feature.intent);

    (tmp, feature)
}

// --- the happy path: the journal becomes the spec ---------------------------

/// The golden envelope of the whole feature loop: three ops applied, and the
/// one test the *folded* journal made visible actually run (D8's gate 10 over
/// the folded model -- without the fold there would be no `proves` binding,
/// hence no target, hence a dishonest `tests_run: 0`).
#[test]
fn reconciling_an_implemented_change_folds_its_journal_and_runs_its_test() {
    let (tmp, feature) = implemented();

    assert_eq!(
        reconcile_id_ok(tmp.path(), &feature.change),
        json!({
            "id": feature.change,
            "full": false,
            "ops_applied": 3,
            "checks_run": 0,
            "tests_run": 1,
            "witness_warnings": []
        })
    );
}

/// D2, byte for byte: `bindings.tel` is written by the reconcile, from the
/// journal, in the emitter's canonical order (`implements` first, then
/// `proves`) -- and it did not exist in that state one command earlier.
#[test]
fn reconcile_derives_the_bindings_file_from_the_journal() {
    let (tmp, feature) = implemented();
    assert_eq!(
        read(tmp.path(), BINDINGS),
        "",
        "`bindings.tel` stays empty for the whole life of the change (D2)"
    );

    reconcile_id_ok(tmp.path(), &feature.change);

    assert_eq!(
        read(tmp.path(), BINDINGS),
        format!(
            "implements \"{CODE_FILE}\" -> {}\nproves     \"{TEST_FILE}::{}\" -> {}\n",
            feature.intent,
            test_fn(&feature.scenario),
            feature.scenario
        )
    );
}

/// The seal that follows: both files the folded bindings reach are in
/// `[code]`, the project is coherent again, and the scenario counts as
/// proved.
#[test]
fn reconcile_seals_the_bound_code_and_test_files_and_leaves_it_coherent() {
    let (tmp, feature) = implemented();

    reconcile_id_ok(tmp.path(), &feature.change);

    let lock = read(tmp.path(), LOCK);
    for path in [CODE_FILE, TEST_FILE, BINDINGS] {
        assert!(lock.contains(path), "`{path}` is not sealed:\n{lock}");
    }

    let status = run_json(tmp.path(), &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
    assert_eq!(status["result"]["changes"], json!([]));
    assert_eq!(status["result"]["coverage"]["scenarios_proved"], json!(1));
}

/// The other half of "reconcile derives the bindings table": a change with
/// no journal at all derives exactly what was already there.
///
/// Every reconcile now re-emits `bindings.tel` from the folded model, so
/// this is what keeps that from being a change of behaviour for the M1/M2
/// projects that have no journal: the model holds what the sealed file said,
/// the emitter is what wrote that file in the first place, and the bytes come
/// back identical -- the lock included.
#[test]
fn a_reconcile_with_no_journal_leaves_the_bindings_file_byte_identical() {
    let tmp = approved_int_0042_edit(with_fixture());
    let before = read(tmp.path(), BINDINGS);

    reconcile_ok(tmp.path());

    assert_eq!(read(tmp.path(), BINDINGS), before);
}

// --- gate 7: the sealed red witness (D7), strict ----------------------------

/// The scenario is brand new and nothing was ever run for it: the frozen
/// `TELOS_SCENARIO_RED_EXPECTED` of Annex F, and not one byte written.
#[test]
fn a_scenario_with_no_run_at_all_is_refused_and_nothing_is_written() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_SCENARIO_RED_EXPECTED",
            "message": format!("scenario {} has no sealed red witness", feature.scenario),
            "hint": format!(
                "run `telos test {}` to record a red witness before implementing",
                feature.scenario
            ),
        })
    );
    assert!(
        !tmp.path()
            .join(format!("telos/intents/{}.tel", feature.intent))
            .exists(),
        "a refused reconcile must write nothing"
    );
    assert_eq!(read(tmp.path(), BINDINGS), "", "and derive nothing");
}

/// A red witness with no green on the same bytes: the implementation has not
/// happened yet, so the reconcile refuses -- naming the *first* scenario that
/// owes one, in id order, which is what makes a fixing agent converge.
///
/// Two scenarios, deliberately: the second is the one that owes a green,
/// while the first is fully witnessed and, through its `proves`, is what
/// keeps rule 5 (gate 6) from firing on `tests/billing.rs` before this gate
/// is reached at all.
#[test]
fn a_red_witness_with_no_green_on_the_same_bytes_is_refused() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![
            scenario_payload("a full payment settles the invoice"),
            scenario_payload("a partial payment leaves the invoice open"),
        ],
    );
    let second = format!(
        "SCN-{:04}",
        feature
            .scenario
            .trim_start_matches("SCN-")
            .parse::<u32>()
            .unwrap()
            + 1
    );
    write_test_file(tmp.path(), &[&feature.scenario, &second], "");
    witness(tmp.path(), &feature.scenario, "red");
    witness(tmp.path(), &second, "red");
    set_marker(tmp.path());
    witness(tmp.path(), &feature.scenario, "green");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_SCENARIO_RED_EXPECTED",
            "message": format!(
                "scenario {second} has a red witness but no green run on the same bytes"
            ),
            "hint": format!(
                "run `telos test {second}` again once the implementation is in place"
            ),
        })
    );
}

/// The witness is sealed to the bytes it was taken on: editing the test
/// between the red and the green invalidates the pair, whatever the green
/// says.
#[test]
fn a_test_edited_between_its_red_and_its_green_is_refused_as_sealed() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );
    write_test_file(tmp.path(), &[&feature.scenario], "");
    witness(tmp.path(), &feature.scenario, "red");
    write_test_file(tmp.path(), &[&feature.scenario], "// second thoughts\n");
    set_marker(tmp.path());
    witness(tmp.path(), &feature.scenario, "green");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_TEST_SEALED",
            "message": format!(
                "the test file `{TEST_FILE}` changed after the red witness for {} was sealed",
                feature.scenario
            ),
            "hint": format!(
                "the red witness is invalid; run `telos test {}` again on the current \
                 bytes before reconciling",
                feature.scenario
            ),
        })
    );
}

/// And the same the other way round: a red/green pair taken on one set of
/// bytes says nothing about the bytes on disk at reconcile time.
#[test]
fn a_test_edited_after_its_green_is_refused_as_sealed() {
    let (tmp, feature) = implemented();
    write_test_file(tmp.path(), &[&feature.scenario], "// touched afterwards\n");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(error["code"], json!("TELOS_TEST_SEALED"));
    assert_eq!(
        error["message"],
        json!(format!(
            "the test file `{TEST_FILE}` changed after the red witness for {} was sealed",
            feature.scenario
        ))
    );
}

// --- advisory (D7): the same verdicts, as warnings ---------------------------

/// `policy.tdd = "advisory"`: the reconcile goes through, and the verdict it
/// would have refused on comes back in `witness_warnings` -- the frozen
/// message, hint apart (a warning has nothing to remedy for the caller).
#[test]
fn advisory_reports_the_missing_witness_instead_of_refusing() {
    let tmp = configured("advisory");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    assert_eq!(
        reconcile_id_ok(tmp.path(), &feature.change),
        json!({
            "id": feature.change,
            "full": false,
            "ops_applied": 3,
            "checks_run": 0,
            "tests_run": 0,
            "witness_warnings": [
                format!("scenario {} has no sealed red witness", feature.scenario)
            ]
        })
    );
    // It really reconciled: the delta is on disk and the project is sealed.
    assert!(
        tmp.path()
            .join(format!("telos/intents/{}.tel", feature.intent))
            .exists()
    );
    let status = run_json(tmp.path(), &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
}

/// The same warning in human mode, where an advisory project's TDD debt has
/// to be visible at all.
#[test]
fn advisory_prints_the_warning_in_human_mode() {
    let tmp = configured("advisory");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    let out = telos(tmp.path(), &["change", "reconcile", &feature.change])
        .output()
        .unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "warning: scenario {} has no sealed red witness",
            feature.scenario
        )),
        "the advisory warning is not in the human output:\n{stdout}"
    );
}

// --- the fold is idempotent across reconciles -------------------------------

/// D2's deduplication, end to end: re-binding a pair the sealed
/// `bindings.tel` already holds folds to the same one line, so the derived
/// file comes back byte-identical -- and the scenario, re-staged unchanged,
/// owes no second witness (the fragment exemption `loop_merge` rests on).
#[test]
fn rebinding_a_pair_the_sealed_file_already_holds_leaves_it_unchanged() {
    let (tmp, feature) = implemented();
    reconcile_id_ok(tmp.path(), &feature.change);
    let sealed = read(tmp.path(), BINDINGS);

    let second = ok_result(tmp.path(), &["change", "open", "bind it again", "--json"])["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut cmd = telos(
        tmp.path(),
        &[
            "edit",
            "intent",
            &feature.intent,
            "--change",
            &second,
            "--json",
        ],
    );
    let edited = json_stdout(&cmd.write_stdin("{}").output().unwrap());
    assert_eq!(edited["ok"], json!(true), "{edited}");
    approve_id(tmp.path(), &second);
    ok_result(tmp.path(), &["bind", CODE_FILE, &feature.intent, "--json"]);

    assert_eq!(
        reconcile_id_ok(tmp.path(), &second)["witness_warnings"],
        json!([])
    );
    assert_eq!(read(tmp.path(), BINDINGS), sealed);
}
