//! End-to-end tests for `telos change reconcile`: the transaction that turns
//! a reviewed delta into spec files on disk and a fresh seal (`CHG-NNNN`),
//! and the delta-less reseal that proves a whole project from scratch
//! (`--full`, D12 -- last section).
//!
//! The shape of this file follows the frozen gate order of the module docs
//! -- drift, status, digest, accept OIDs, overlay, rule 5, the sealed code
//! coverage, the red witness, constraint checks, tests, and only then the
//! writes (D6). Each gate gets a test that proves it refuses *and* that the
//! refusal cost nothing: no `.tel` file appeared, the lock did not move, the
//! change file is still there.
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
    settle_intent_payload_with_status("draft")
}

fn active_settle_intent_payload() -> String {
    settle_intent_payload_with_status("active")
}

fn settle_intent_payload_with_status(status: &str) -> String {
    json!({
        "title": "Invoices can be settled", "status": status,
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

fn approved_active_three_op_change() -> tempfile::TempDir {
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
        &active_settle_intent_payload(),
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
           status draft\n  \
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

/// A new source file no binding covers -- the shape of in-flight work under
/// the `[code]` globs.
const IN_FLIGHT: &str = "src/billing/payment.rs";

/// Opens a change on the corpus, stages one `telos` reword of `intent` into
/// it, approves it, and hands back its id.
fn open_and_approve_edit(dir: &Path, intent: &str, telos_text: &str) -> String {
    let id = ok_result(dir, &["change", "open", MOTIVATION, "--json"])["id"]
        .as_str()
        .expect("`change open` answers with the allocated id")
        .to_string();
    stage(
        dir,
        &["edit", "intent", intent, "--change", &id, "--json"],
        &json!({ "telos": telos_text }).to_string(),
    );
    approve_id(dir, &id);
    id
}

/// The in-flight exemption: a file another open change has bound in its
/// journal is somebody's declared work in progress, not an orphan, so an
/// unrelated reconcile is not held hostage to it.
///
/// And the exemption dies with the claim: abandon that change and the very
/// same file is rule 5's business again, with the frozen wording.
#[test]
fn code_another_open_change_claims_is_no_orphan_until_that_change_is_gone() {
    let tmp = with_fixture();

    // A binds a brand new source file -- no green needed, a `bind` line is
    // already a claim.
    let a = open_and_approve_edit(tmp.path(), "INT-0017", "A's reworded telos.");
    fs::write(tmp.path().join(IN_FLIGHT), "// A's work in progress\n").unwrap();
    ok_result(tmp.path(), &["bind", IN_FLIGHT, "INT-0017", "--json"]);

    // B is unrelated, and reconciles even though A's file is unbound *here*:
    // A's journal is not folded into B's model.
    let b = open_and_approve_edit(tmp.path(), "INT-0042", "B's reworded telos.");
    assert_eq!(reconcile_id_ok(tmp.path(), &b)["ops_applied"], json!(1));
    assert!(
        !read(tmp.path(), LOCK).contains(IN_FLIGHT),
        "an exempted file is not sealed by the reconcile that tolerated it"
    );

    ok_result(tmp.path(), &["change", "abandon", &a, "--json"]);
    let c = open_and_approve_edit(tmp.path(), "INT-0017", "C's reworded telos.");

    assert_eq!(
        reconcile_id_err(tmp.path(), &c),
        json!({
            "code": "TELOS_ORPHAN_CODE",
            "message": format!(
                "`{IN_FLIGHT}` matches the [code] globs but no `implements` binding covers it"
            ),
            "hint": ORPHAN_HINT
        })
    );
}

/// The exemption covers what *another* change claims and what this change's
/// own **journal** declares -- never the paths its own ops target. This is
/// the sharpest case of that: a delta `adopt` built from a hand-edited
/// `bindings.tel` *and* from the very code file that edit orphaned, so the
/// orphan is one of the change's own `accept` targets. Rule 5 still refuses
/// it -- adopting a code file without binding it is an orphan, as in M2.
#[test]
fn a_code_file_this_changes_own_accept_op_targets_is_still_an_orphan() {
    let tmp = with_fixture();
    drop_the_implements_line(tmp.path());
    // The bound file drifts too, so `adopt` captures both: unlike the ledger
    // attack, the `[code]` globs stay, so rule 5 has something to say -- and
    // it says it about a path this very delta stages.
    fs::write(
        tmp.path().join("src/billing/invoice.rs"),
        "// edited out of protocol\n",
    )
    .unwrap();

    let adopted = ok_result(tmp.path(), &["adopt", "--json"]);
    assert_eq!(adopted["ops"], json!(2), "{adopted}");
    let a = adopted["change"]
        .as_str()
        .expect("`adopt` answers with the change that captured the drift")
        .to_string();
    approve_id(tmp.path(), &a);

    assert_eq!(
        reconcile_id_err(tmp.path(), &a),
        json!({
            "code": "TELOS_ORPHAN_CODE",
            "message": "`src/billing/invoice.rs` matches the [code] globs but \
                        no `implements` binding covers it",
            "hint": ORPHAN_HINT
        })
    );
}

// --- gate 7: the sealed code coverage (D9) -----------------------------------

/// The frozen `TELOS_INTEGRITY_VIOLATION` hint of Annex F, gate 7's row.
const COVERAGE_HINT: &str = "the bindings shrank outside this change; reconcile or abandon the change that claims telos/bindings.tel, or restore them with `telos revert`";

/// Hand-edits `telos/bindings.tel` down to its `proves` line, out of
/// protocol. A sealed spec file, so this is drift.
fn drop_the_implements_line(dir: &Path) {
    let bindings = read(dir, BINDINGS);
    let kept: String = bindings
        .lines()
        .filter(|line| !line.starts_with("implements"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_ne!(
        kept, bindings,
        "the corpus no longer ships an `implements` line to drop"
    );
    fs::write(dir.join(BINDINGS), kept).unwrap();
}

/// Hand-edits the corpus into the ledger attack's starting position: the
/// `implements` line dropped, *and* the `[code]` globs emptied so that rule
/// 5 has nothing left to say about the file that just lost its binding --
/// which is what leaves gate 7 as the only thing standing in the way.
fn stage_the_ledger_attack(dir: &Path) {
    drop_the_implements_line(dir);

    let toml = read(dir, "telos/telos.toml");
    let emptied = toml.replace("globs = [\"src/**/*.rs\"]", "globs = []");
    assert_ne!(emptied, toml, "the corpus no longer ships the [code] globs");
    fs::write(dir.join("telos/telos.toml"), emptied).unwrap();
}

/// The attack, captured: the hand-edits adopted into change A (two `accept`
/// ops, one of them on `telos/bindings.tel`), and an unrelated change B
/// approved next to it.
fn ledger_attack() -> (tempfile::TempDir, String, String) {
    let tmp = with_fixture();
    stage_the_ledger_attack(tmp.path());

    let adopted = ok_result(tmp.path(), &["adopt", "--json"]);
    assert_eq!(
        adopted["ops"],
        json!(2),
        "adopt must capture both hand-edits: {adopted}"
    );
    let a = adopted["change"]
        .as_str()
        .expect("`adopt` answers with the change that captured the drift")
        .to_string();

    let b = open_and_approve_edit(tmp.path(), "INT-0017", "B's reworded telos.");
    (tmp, a, b)
}

/// D9, the M2 residual: B never touched the bindings, but sealing its delta
/// would quietly drop a code path the previous seal held -- so it is refused,
/// with the frozen wording naming the path.
#[test]
fn reconcile_refuses_a_shrink_of_the_sealed_code_coverage() {
    let (tmp, _a, b) = ledger_attack();

    assert_eq!(
        reconcile_id_err(tmp.path(), &b),
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": "sealing would drop `src/billing/invoice.rs` from the code table: \
                        no binding covers it and this change does not stage telos/bindings.tel",
            "hint": COVERAGE_HINT
        })
    );
    assert!(
        read(tmp.path(), LOCK).contains("src/billing/invoice.rs"),
        "a refused reconcile must not move the seal"
    );
}

/// The other half: the change that *stages* `telos/bindings.tel` is the one
/// that was reviewed for the shrink, so it goes through -- and B, whose
/// refusal was about a coverage nobody had approved dropping, goes through
/// right after it.
#[test]
fn the_change_that_stages_the_bindings_file_may_shrink_the_coverage() {
    let (tmp, a, b) = ledger_attack();

    approve_id(tmp.path(), &a);
    assert_eq!(reconcile_id_ok(tmp.path(), &a)["ops_applied"], json!(2));

    let lock = read(tmp.path(), LOCK);
    assert!(
        !lock.contains("src/billing/invoice.rs"),
        "the reviewed shrink must really drop the path:\n{lock}"
    );
    assert_eq!(reconcile_id_ok(tmp.path(), &b)["ops_applied"], json!(1));
}

/// §7.4: `--full` is total proof of the tree on disk and reads no previous
/// lock at all, so there is no coverage to compare against and nothing for
/// this gate to refuse -- on the very tree that refuses B.
#[test]
fn a_full_reseal_is_exempt_from_the_coverage_gate() {
    let (tmp, _a, _b) = ledger_attack();

    assert_eq!(reconcile_full_ok(tmp.path())["full"], json!(true));

    let lock = read(tmp.path(), LOCK);
    assert!(
        !lock.contains("src/billing/invoice.rs"),
        "a full reseal seals the tree as it stands:\n{lock}"
    );
}

// --- gate 9: constraint checks (D11) -----------------------------------------

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

// --- gate 10: tests (D10) -----------------------------------------------------

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

#[test]
fn reconcile_runner_preserves_an_embedded_quoted_filter() {
    let tmp = approved_int_0042_edit(with_fixture_mut(|root| {
        set_test_cmd_to(root, "git hash-object \"prefix-{filter}-suffix\"");
        // The fixture's initial `reconcile --full` substitutes an empty
        // filter, so its composed whole-suite target must exist as well.
        fs::write(root.join("prefix--suffix"), "").unwrap();
    }));
    fs::write(
        tmp.path()
            .join("prefix-scn_0107_full_payment_settles_the_invoice-suffix"),
        "",
    )
    .unwrap();

    assert_eq!(reconcile_ok(tmp.path())["tests_run"], json!(1));
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
    complete_unsealed_for_full(tmp.path());
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
                "tests_run": 1,
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
    complete_unsealed_for_full(tmp.path());
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

#[test]
fn full_reconcile_refuses_an_active_scenario_without_a_proof_before_writing_lock() {
    let tmp = unsealed_fixture();

    let envelope = reconcile_full(tmp.path());

    assert_eq!(envelope["ok"], json!(false), "got {envelope}");
    assert_eq!(
        envelope["error"],
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": "active scenario SCN-0091 has no `proves` binding",
            "hint": "record a green proof for SCN-0091 through an approved change before reconciling"
        })
    );
    assert!(
        !tmp.path().join(LOCK).exists(),
        "a structurally incomplete project must not receive a lock"
    );
}

#[test]
fn ordinary_reconcile_refuses_an_active_scenario_without_a_proof_before_any_write() {
    let tmp = approved_active_three_op_change();
    let change_before = read(tmp.path(), CHG_0001);
    let lock_before = read(tmp.path(), LOCK);

    let envelope = reconcile(tmp.path());

    assert_eq!(envelope["ok"], json!(false), "got {envelope}");
    assert_eq!(
        envelope["error"],
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": "active scenario SCN-0001 has no `proves` binding",
            "hint": "record a green proof for SCN-0001 through an approved change before reconciling"
        })
    );
    assert_eq!(read(tmp.path(), CHG_0001), change_before);
    assert_eq!(read(tmp.path(), LOCK), lock_before);
    assert!(!tmp.path().join("telos/intents/INT-0001.tel").exists());
}

#[test]
fn full_reconcile_requires_a_runner_when_active_scenarios_all_have_proofs() {
    let tmp = unsealed_fixture();
    let bindings = tmp.path().join(BINDINGS);
    let mut source = fs::read_to_string(&bindings).unwrap();
    source.push_str("proves     \"tests/billing.rs\" -> SCN-0091\n");
    fs::write(bindings, source).unwrap();

    let envelope = reconcile_full(tmp.path());

    assert_eq!(envelope["ok"], json!(false), "got {envelope}");
    assert_eq!(
        envelope["error"],
        json!({
            "code": "TELOS_TEST_NOT_FOUND",
            "message": "no `[test] cmd` is configured in telos/telos.toml",
            "hint": "set [test] cmd, e.g. `cargo test {filter}`"
        })
    );
    assert!(!tmp.path().join(LOCK).exists());
}

fn complete_unsealed_for_full(dir: &Path) {
    let bindings_path = dir.join(BINDINGS);
    let bindings = fs::read_to_string(&bindings_path).unwrap();
    if !bindings.contains("-> SCN-0091") {
        let (implements, rest) = bindings.split_once('\n').unwrap();
        fs::write(
            bindings_path,
            format!("{implements}\nproves     \"tests/billing.rs\" -> SCN-0091\n{rest}"),
        )
        .unwrap();
    }
    set_test_cmd_to(dir, "git --version");
}

#[test]
fn full_reconcile_without_active_intents_needs_no_runner_or_test_run() {
    let tmp = fresh();
    fs::remove_file(tmp.path().join(LOCK)).unwrap();

    let result = reconcile_full_ok(tmp.path());

    assert_eq!(result["tests_run"], json!(0));
}

#[test]
fn full_reconcile_treats_a_whitespace_runner_as_absent_for_draft_only_intents() {
    let tmp = unsealed_fixture();
    for intent in ["INT-0017", "INT-0042"] {
        let path = tmp.path().join(format!("telos/intents/{intent}.tel"));
        let source = fs::read_to_string(&path).unwrap();
        fs::write(path, source.replace("status active", "status draft")).unwrap();
    }
    set_test_cmd_to(tmp.path(), " \t ");

    let result = reconcile_full_ok(tmp.path());

    assert_eq!(result["tests_run"], json!(0));
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
    /// Every scenario id `add intent` allocated, in payload order -- read
    /// straight out of `result.scenario_ids`, never derived from another id
    /// by arithmetic.
    scenarios: Vec<String>,
}

impl Feature {
    /// The first (and, for most fixtures here, only) scenario of the delta.
    fn scenario(&self) -> &str {
        &self.scenarios[0]
    }
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
/// `Feature.scenarios` is `result.scenario_ids` verbatim, in payload order,
/// so a caller that stages three reads all three from the envelope. No id is
/// hardcoded anywhere, and none is *computed* from another either -- the
/// allocator's answers are read back as they come (§14's anti-goal).
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
        scenarios: added["result"]["scenario_ids"]
            .as_array()
            .expect("`add intent` reports the scenario ids it allocated")
            .iter()
            .map(|id| id.as_str().expect("a scenario id is a string").to_string())
            .collect(),
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

    write_test_file(tmp.path(), &[feature.scenario()], "");
    witness(tmp.path(), feature.scenario(), "red");
    set_marker(tmp.path());
    witness(tmp.path(), feature.scenario(), "green");
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
            test_fn(feature.scenario()),
            feature.scenario()
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

// --- gate 8: the sealed red witness (D7), strict ----------------------------

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
            "message": format!("scenario {} has no sealed red witness", feature.scenario()),
            "hint": format!(
                "run `telos test {}` to record a red witness before implementing",
                feature.scenario()
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

/// The red-only state, and the convergent answer it must get.
///
/// A change that has journalled a red run and nothing else owns a test file
/// under the `[tests]` globs that no `proves` binding covers yet -- a red run
/// asserts none, by construction (D2). Rule 5 would call that an orphan and
/// send the caller off to `telos bind`, which is exactly the wrong next step:
/// the honest verdict is the witness gate's, one gate later, and it names the
/// green that is missing. So a path this change's own journal declares is
/// exempt from rule 5, and the refusal is the one the caller can act on.
#[test]
fn a_red_only_change_is_told_about_its_missing_green_not_about_an_orphan() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );
    write_test_file(tmp.path(), &[feature.scenario()], "");
    witness(tmp.path(), feature.scenario(), "red");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_SCENARIO_RED_EXPECTED",
            "message": format!(
                "scenario {} has a red witness but no green run on the same bytes",
                feature.scenario()
            ),
            "hint": format!(
                "run `telos test {}` again once the implementation is in place",
                feature.scenario()
            ),
        })
    );
}

/// The other side of that exemption, and the asymmetry it creates: `--full`
/// proves the *disk*, and a journal nobody has reconciled yet is not on it.
///
/// So the very red-only state the test above gets a witness verdict for is
/// refused by a full reseal of the same tree, one command apart -- `--full`
/// is *stricter* here than a per-change reconcile, deliberately (§7.4). The
/// remedy is to finish or abandon the change, not to reach for `--full`.
#[test]
fn a_full_reseal_refuses_the_in_flight_file_a_change_reconcile_exempts() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );
    write_test_file(tmp.path(), &[feature.scenario()], "");
    witness(tmp.path(), feature.scenario(), "red");

    let envelope = reconcile_full(tmp.path());

    assert_eq!(envelope["ok"], json!(false), "got {envelope}");
    assert_eq!(
        envelope["error"],
        json!({
            "code": "TELOS_ORPHAN_CODE",
            "message": format!(
                "`{TEST_FILE}` matches the [tests] globs but no `proves` binding covers it"
            ),
            "hint": ORPHAN_HINT
        })
    );
}

/// A red witness with no green on the same bytes -- and, with it, the
/// convergence property the gate's ordering exists for: **two** scenarios owe
/// a witness here, and the refusal names the lower-numbered one.
///
/// Three scenarios, each playing a distinct part:
///
/// - the **first** is witnessed red *and* green, so it is `Intact` -- and its
///   `proves`, folded from the green, is what keeps rule 5 (gate 6) from
///   firing on `tests/billing.rs` before this gate is reached at all;
/// - the **second** has a red and no green: `MissingGreen`, the verdict this
///   test is named for;
/// - the **third** was never run at all: `MissingRed`, a *different* verdict,
///   and the one that would surface if the gate reported the last failure, an
///   arbitrary one, or a list.
///
/// So the assertion below pins two things at once: the frozen red-without-
/// green wording, and that the first failing scenario in id order is the one
/// -- and the only one -- reported.
#[test]
fn a_red_witness_with_no_green_names_the_first_failing_scenario() {
    let tmp = configured("strict");
    let feature = approved_feature(
        tmp.path(),
        vec![
            scenario_payload("a full payment settles the invoice"),
            scenario_payload("a partial payment leaves the invoice open"),
            scenario_payload("an overpayment settles the invoice too"),
        ],
    );
    let [witnessed, no_green, never_run] = match feature.scenarios.as_slice() {
        [a, b, c] => [a.clone(), b.clone(), c.clone()],
        other => panic!("`add intent` allocated {} scenario ids", other.len()),
    };

    // The third scenario deliberately gets no test function and no run.
    write_test_file(tmp.path(), &[&witnessed, &no_green], "");
    witness(tmp.path(), &witnessed, "red");
    witness(tmp.path(), &no_green, "red");
    set_marker(tmp.path());
    witness(tmp.path(), &witnessed, "green");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_SCENARIO_RED_EXPECTED",
            "message": format!(
                "scenario {no_green} has a red witness but no green run on the same bytes"
            ),
            "hint": format!(
                "run `telos test {no_green}` again once the implementation is in place"
            ),
        }),
        "the gate must report {no_green}, not {never_run} and not both"
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
    write_test_file(tmp.path(), &[feature.scenario()], "");
    witness(tmp.path(), feature.scenario(), "red");
    write_test_file(tmp.path(), &[feature.scenario()], "// second thoughts\n");
    set_marker(tmp.path());
    witness(tmp.path(), feature.scenario(), "green");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(
        error,
        json!({
            "code": "TELOS_TEST_SEALED",
            "message": format!(
                "the test file `{TEST_FILE}` changed after the red witness for {} was sealed",
                feature.scenario()
            ),
            "hint": format!(
                "the red witness is invalid; run `telos test {}` again on the current \
                 bytes before reconciling",
                feature.scenario()
            ),
        })
    );
}

/// And the same the other way round: a red/green pair taken on one set of
/// bytes says nothing about the bytes on disk at reconcile time.
#[test]
fn a_test_edited_after_its_green_is_refused_as_sealed() {
    let (tmp, feature) = implemented();
    write_test_file(tmp.path(), &[feature.scenario()], "// touched afterwards\n");

    let error = reconcile_id_err(tmp.path(), &feature.change);

    assert_eq!(error["code"], json!("TELOS_TEST_SEALED"));
    assert_eq!(
        error["message"],
        json!(format!(
            "the test file `{TEST_FILE}` changed after the red witness for {} was sealed",
            feature.scenario()
        ))
    );
}

// --- advisory (D7): the same verdicts, as warnings ---------------------------

/// Advisory relaxes red-witness discipline, never the structural requirement
/// that an active scenario have a proof before it can be sealed.
#[test]
fn advisory_still_refuses_an_active_scenario_without_a_proof() {
    let tmp = configured("advisory");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    assert_eq!(
        reconcile_id_err(tmp.path(), &feature.change),
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": format!("active scenario {} has no `proves` binding", feature.scenario()),
            "hint": format!(
                "record a green proof for {} through an approved change before reconciling",
                feature.scenario()
            )
        })
    );
    assert!(
        !tmp.path()
            .join(format!("telos/intents/{}.tel", feature.intent))
            .exists()
    );
}

#[test]
fn a_staged_config_glob_is_effective_before_reconcile_writes_it() {
    let tmp = fresh();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src/unbound.rs"), "fn unbound() {}\n").unwrap();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &["config", "--change", "CHG-0001", "--json"],
        r#"{"code":{"globs":["src/**/*.rs"]},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"strict"},"agents":{"hosts":[]}}"#,
    );
    approve(tmp.path());

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], "TELOS_ORPHAN_CODE");
    assert!(read(tmp.path(), "telos/telos.toml").contains("globs = []"));
}

fn stage_corpus_config(dir: &Path, test_cmd: &str) {
    stage(
        dir,
        &["config", "--change", "CHG-0001", "--json"],
        &json!({
            "code": {"globs": ["src/**/*.rs"]},
            "tests": {"globs": ["tests/**/*.rs"]},
            "test": {"cmd": test_cmd},
            "policy": {"tdd": "strict"},
            "agents": {"hosts": []}
        })
        .to_string(),
    );
}

#[test]
fn config_edit_revalidates_scoped_constraints_before_writing() {
    let tmp = with_fixture_mut(|root| {
        fs::write(root.join(".constraint-green"), "green\n").unwrap();
        let path = root.join("telos/constraints/CON-0003.tel");
        let source = fs::read_to_string(&path).unwrap();
        let source = source.replace("scope global", "scope INT-0017").replace(
            "check \"git --version\"",
            "check \"git hash-object .constraint-green\"",
        );
        fs::write(path, source).unwrap();
    });
    fs::remove_file(tmp.path().join(".constraint-green")).unwrap();
    open_change(tmp.path());
    stage_corpus_config(tmp.path(), "git --version");
    approve(tmp.path());
    let config_before = read(tmp.path(), "telos/telos.toml");
    let lock_before = read(tmp.path(), LOCK);

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], json!("TELOS_CONSTRAINT_FAILED"));
    assert_eq!(
        error["message"],
        json!("CON-0003 check failed: `git hash-object .constraint-green`")
    );
    assert_eq!(read(tmp.path(), "telos/telos.toml"), config_before);
    assert_eq!(read(tmp.path(), LOCK), lock_before);
    assert!(tmp.path().join(CHG_0001).exists());
}

#[test]
fn config_edit_runs_each_distinct_proof_once_with_the_staged_runner() {
    const RUN_LOG: &str = ".config-proof-runs";
    const JOURNAL_RUNNER: &str =
        "git config --file .config-proof-runs --add runs.filter proof-{filter}";

    let tmp = with_fixture_mut(|root| {
        set_test_cmd_to(root, JOURNAL_RUNNER);
        let path = root.join(BINDINGS);
        let mut source = fs::read_to_string(&path).unwrap();
        source.push_str("proves     \"tests/billing.rs\" -> SCN-0107\n");
        fs::write(path, source).unwrap();
    });
    fs::remove_file(tmp.path().join(RUN_LOG)).unwrap();
    open_change(tmp.path());
    stage_corpus_config(tmp.path(), JOURNAL_RUNNER);
    approve(tmp.path());

    let result = reconcile_ok(tmp.path());

    assert_eq!(
        result["tests_run"],
        json!(2),
        "three bindings collapse to two distinct proof targets"
    );
    assert_eq!(
        read(tmp.path(), RUN_LOG),
        "[runs]\n\tfilter = proof-tests/billing.rs\n\tfilter = proof-scn_0107_full_payment_settles_the_invoice\n"
    );
}

#[test]
fn config_edit_treats_a_whitespace_runner_as_absent_for_draft_only_intents() {
    let tmp = with_fixture_mut(|root| {
        for intent in ["INT-0017", "INT-0042"] {
            let path = root.join(format!("telos/intents/{intent}.tel"));
            let source = fs::read_to_string(&path).unwrap();
            fs::write(path, source.replace("status active", "status draft")).unwrap();
        }
        set_test_cmd_to(root, " \t ");
    });
    open_change(tmp.path());
    stage_corpus_config(tmp.path(), " \t ");
    approve(tmp.path());

    let result = reconcile_ok(tmp.path());

    assert_eq!(result["tests_run"], json!(0));
}

#[test]
fn config_edit_cannot_remove_the_runner_from_a_project_with_active_obligations() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_corpus_config(tmp.path(), "");
    approve(tmp.path());
    let config_before = read(tmp.path(), "telos/telos.toml");
    let lock_before = read(tmp.path(), LOCK);

    let error = reconcile_err(tmp.path());

    assert_eq!(error["code"], json!("TELOS_TEST_NOT_FOUND"));
    assert_eq!(read(tmp.path(), "telos/telos.toml"), config_before);
    assert_eq!(read(tmp.path(), LOCK), lock_before);
    assert!(tmp.path().join(CHG_0001).exists());
}

#[test]
fn whitespace_runner_reports_missing_runner_before_missing_red_witness() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        &json!({
            "scenarios": [
                {
                    "id": "SCN-0091",
                    "title": "a newly issued invoice is open",
                    "given": [{"notion": "Customer", "fields": {"name": "ACME"}}],
                    "when": {"notion": "InvoiceIssued", "fields": {}},
                    "then": ["Invoice.state == open"]
                },
                {
                    "title": "an issued invoice starts with nothing paid",
                    "given": [{
                        "notion": "Invoice",
                        "fields": {"state": "open", "balance": "0.00 EUR"}
                    }],
                    "when": {"notion": "InvoiceIssued", "fields": {}},
                    "then": ["Invoice.state == open"]
                }
            ]
        })
        .to_string(),
    );
    stage_corpus_config(tmp.path(), "git hash-object {filter}");
    approve(tmp.path());
    fs::write(tmp.path().join(test_fn("SCN-0108")), "green\n").unwrap();
    fs::write(
        tmp.path().join("tests/whitespace_runner.rs"),
        format!("fn {}() {{}}\n", test_fn("SCN-0108")),
    )
    .unwrap();
    witness(tmp.path(), "SCN-0108", "green");
    let change_path = tmp.path().join(CHG_0001);
    let change = fs::read_to_string(&change_path).unwrap();
    let whitespace = change.replace(
        "test_cmd   \"git hash-object {filter}\"",
        "test_cmd   \"   \"",
    );
    assert_ne!(whitespace, change, "the staged runner was not found");
    fs::write(change_path, whitespace).unwrap();
    approve(tmp.path());
    let config_before = read(tmp.path(), "telos/telos.toml");
    let lock_before = read(tmp.path(), LOCK);

    let error = reconcile_err(tmp.path());

    assert_eq!(
        error,
        json!({
            "code": "TELOS_TEST_NOT_FOUND",
            "message": "no `[test] cmd` is configured in telos/telos.toml",
            "hint": "set [test] cmd, e.g. `cargo test {filter}`"
        })
    );
    assert_eq!(read(tmp.path(), "telos/telos.toml"), config_before);
    assert_eq!(read(tmp.path(), LOCK), lock_before);
    assert!(tmp.path().join(CHG_0001).exists());
}

#[test]
fn telos_test_uses_the_approved_config_staged_by_the_owning_change() {
    let tmp = fresh();
    open_change(tmp.path());
    stage(
        tmp.path(),
        &["config", "--change", "CHG-0001", "--json"],
        &json!({
            "code": {"globs": ["src/**/*.rs"]},
            "tests": {"globs": ["tests/**/*.rs"]},
            "test": {"cmd": RUNNER},
            "policy": {"tdd": "strict"},
            "agents": {"hosts": []}
        })
        .to_string(),
    );
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
        &active_settle_intent_payload(),
    );
    approve(tmp.path());
    write_test_file(tmp.path(), &["SCN-0001"], "");

    let result = ok_result(tmp.path(), &["test", "SCN-0001", "--json"]);

    assert_eq!(result["witness"], json!("red"));
    assert_eq!(result["command"], json!(RUNNER));
}

/// Human mode surfaces the same structural refusal on stderr.
#[test]
fn advisory_prints_the_structural_refusal_in_human_mode() {
    let tmp = configured("advisory");
    let feature = approved_feature(
        tmp.path(),
        vec![scenario_payload("a full payment settles the invoice")],
    );

    let out = telos(tmp.path(), &["change", "reconcile", &feature.change])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!(
            "active scenario {} has no `proves` binding",
            feature.scenario()
        )),
        "the structural refusal is not in the human error:\n{stderr}"
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
