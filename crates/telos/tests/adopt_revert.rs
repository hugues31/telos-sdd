//! End-to-end tests for `telos adopt` and `telos revert`: the two exits from
//! drift (spec §6, D7).
//!
//! Both commands start from the same place -- a project whose working tree no
//! longer matches its seal -- and go opposite ways. `adopt` keeps the bytes
//! and routes them back through the protocol, turning each drifted path into
//! a staged op of a change that then follows the ordinary
//! diff/approve/reconcile loop. `revert` keeps the seal and throws the bytes
//! away, restoring every sealed path from the git object store and deleting
//! what was never sealed.
//!
//! Two things are worth knowing before reading further:
//!
//! - **`adopt` never reads a blob, `revert` always does.** `adopt` only
//!   hashes the tree (`git hash-object`, no object store involved), which is
//!   why every `adopt` test here runs on an uncommitted fixture. `revert`
//!   restores sealed *content*, which only exists in the object store once
//!   somebody committed it -- hence the `commit` helper, and the last test of
//!   the file, which proves the exact refusal a caller gets when they did
//!   not.
//! - **After `adopt` the project is `changing`, not `coherent`.** The drift
//!   is claimed by the change that adopted it (D5), so it stops being drift
//!   and becomes the change in progress. It is the reconcile, not the adopt,
//!   that reseals.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use common::{telos, with_fixture};

// --- the corpus paths these tests drift ------------------------------------

const INVOICE: &str = "telos/notions/Invoice.tel";
/// Named by `Invoice`'s `rel issued-to` and by INT-0017's scenario: the
/// corpus' most-referenced notion, hence the one whose deletion the post-
/// state model must refuse.
const CUSTOMER: &str = "telos/notions/Customer.tel";
const CON_0003: &str = "telos/constraints/CON-0003.tel";
const ROGUE: &str = "telos/notions/Rogue.tel";
const CONFIG: &str = "telos/telos.toml";
const CODE: &str = "src/billing/invoice.rs";
const LOCK: &str = "telos/telos.lock";

/// A valid notion, in canonical form, created outside the protocol -- the
/// `Untracked` half of drift.
const ROGUE_TEL: &str = "notion Rogue entity {\n  \
    def  \"A notion created outside the protocol.\"\n  \
    attr label string\n\
}\n";

/// The exact `TELOS_PARSE_ERROR` hint `adopt` attaches to a drifted file it
/// cannot read (D7).
const PARSE_HINT: &str = "fix the file or run `telos revert`";

/// The exact `TELOS_GIT_ERROR` hint `revert` attaches when the sealed blob is
/// not in the object store.
const SEALED_HINT: &str = "the sealed content is not in the git object store; commit the sealed state or restore the file by hand";

// --- plumbing ---------------------------------------------------------------

fn run(dir: &Path, args: &[&str]) -> Value {
    let out = telos(dir, args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run telos {args:?}: {e}"));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn run_ok(dir: &Path, args: &[&str]) -> Value {
    let envelope = run(dir, args);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected `telos {}` to succeed, got: {envelope}",
        args.join(" ")
    );
    envelope
}

fn run_err(dir: &Path, args: &[&str], code: &str) -> Value {
    let envelope = run(dir, args);
    assert_eq!(
        envelope["ok"],
        json!(false),
        "expected `telos {}` to fail with {code}, got: {envelope}",
        args.join(" ")
    );
    assert_eq!(
        envelope["error"]["code"],
        json!(code),
        "expected `telos {}` to fail with {code}, got: {envelope}",
        args.join(" ")
    );
    envelope["error"].clone()
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir for {rel}: {e}"));
    }
    fs::write(&path, content).unwrap_or_else(|e| panic!("write {rel}: {e}"));
}

/// Appends `suffix` to a file, out of protocol -- the `Modified` half of
/// drift.
fn append(dir: &Path, rel: &str, suffix: &str) {
    let content = format!("{}{suffix}", read(dir, rel));
    write(dir, rel, &content);
}

fn delete(dir: &Path, rel: &str) {
    fs::remove_file(dir.join(rel)).unwrap_or_else(|e| panic!("delete {rel}: {e}"));
}

fn state(dir: &Path) -> Value {
    run_ok(dir, &["status", "--json"])["result"].clone()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Commits the whole fixture, which is what puts the *sealed* blobs in the
/// object store -- the precondition `revert` has and `adopt` has not.
fn commit(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", "seal the billing corpus"]);
}

/// `git hash-object` of a working-tree file: the OID the seal records for it.
fn hash_object(dir: &Path, rel: &str) -> String {
    let out = Command::new("git")
        .args(["hash-object", rel])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git hash-object {rel}: {e}"));
    assert!(out.status.success(), "git hash-object {rel} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The `{op, entity, key}` descriptors `change diff` reports, in staged
/// order.
fn diff_ops(dir: &Path, id: &str) -> Vec<Value> {
    let result = run_ok(dir, &["change", "diff", id, "--json"])["result"].clone();
    result["ops"]
        .as_array()
        .expect("`change diff` reports an ops array")
        .iter()
        .map(|op| json!({ "op": op["op"], "entity": op["entity"], "key": op["key"] }))
        .collect()
}

/// The ordinary loop, from a change `adopt` just produced to a sealed
/// project.
fn approve_and_reconcile(dir: &Path, id: &str) {
    run_ok(dir, &["change", "diff", id, "--json"]);
    run_ok(dir, &["change", "approve", id, "--json"]);
    run_ok(dir, &["change", "reconcile", id, "--json"]);
}

// --- adopt: a modified entity file ------------------------------------------

/// The golden of Annex E, and the canonicalization proof: a byte appended to
/// a sealed notion is adopted as an `edit` op carrying the *parsed* entity,
/// so what the reconcile writes back is the canonical form -- the stray
/// newline is gone, not sealed.
#[test]
fn adopting_a_modified_notion_stages_an_edit_and_reconcile_canonicalizes_it() {
    let tmp = with_fixture();
    let dir = tmp.path();
    let sealed = read(dir, INVOICE);

    append(dir, INVOICE, "\n");
    assert_eq!(state(dir)["state"], json!("drifted"));

    let envelope = run_ok(dir, &["adopt", "--json"]);

    // The whole envelope of Annex E: the command names itself, the result is
    // the golden shape, the next actions are the loop that follows.
    assert_eq!(envelope["command"], json!("adopt"));
    assert_eq!(
        envelope["result"],
        json!({ "change": "CHG-0001", "ops": 1, "paths": [INVOICE] })
    );
    assert_eq!(
        envelope["next_actions"],
        json!([
            "telos change diff CHG-0001",
            "telos change approve CHG-0001"
        ])
    );
    // D5: the adopted path is claimed now, so it is no longer drift.
    assert_eq!(state(dir)["state"], json!("changing"));
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![json!({ "op": "edit", "entity": "notion", "key": "Invoice" })]
    );

    approve_and_reconcile(dir, "CHG-0001");

    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(
        read(dir, INVOICE),
        sealed,
        "reconcile must write the canonical bytes, not the drifted ones"
    );
}

// --- adopt: a deleted entity file -------------------------------------------

/// A `.tel` file deleted by hand is adopted as a `remove` op whose identity
/// comes from the path -- there is no content left to parse.
#[test]
fn adopting_a_deleted_constraint_stages_a_remove_and_reconcile_drops_it() {
    let tmp = with_fixture();
    let dir = tmp.path();

    delete(dir, CON_0003);

    let result = run_ok(dir, &["adopt", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-0001", "ops": 1, "paths": [CON_0003] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![json!({ "op": "remove", "entity": "constraint", "key": "CON-0003" })]
    );

    approve_and_reconcile(dir, "CHG-0001");

    let state = state(dir);
    assert_eq!(state["state"], json!("coherent"));
    assert_eq!(state["coverage"]["constraints"], json!(0));
    assert!(
        !dir.join(CON_0003).exists(),
        "the constraint file must stay deleted"
    );
    assert!(
        !read(dir, LOCK).contains("CON-0003"),
        "the new seal must not still hold the removed constraint"
    );
}

// --- adopt: an untracked entity file ----------------------------------------

/// A spec file that exists but was never sealed is adopted as an `add`.
#[test]
fn adopting_an_untracked_notion_stages_an_add() {
    let tmp = with_fixture();
    let dir = tmp.path();

    write(dir, ROGUE, ROGUE_TEL);

    let result = run_ok(dir, &["adopt", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-0001", "ops": 1, "paths": [ROGUE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![json!({ "op": "add", "entity": "notion", "key": "Rogue" })]
    );

    approve_and_reconcile(dir, "CHG-0001");

    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, ROGUE), ROGUE_TEL);
    assert!(
        read(dir, LOCK).contains(ROGUE),
        "the new seal must hold the adopted notion"
    );
}

// --- adopt: a file the model holds no entity for (gate 4 of the reconcile) ---

/// `telos.toml` is spec, but not an entity: it is adopted as an `accept` of
/// its current bytes, and the reconcile's fourth gate re-hashes them before
/// sealing them (D7).
#[test]
fn adopting_a_modified_telos_toml_stages_an_accept_and_reconcile_seals_it() {
    let tmp = with_fixture();
    let dir = tmp.path();

    append(dir, CONFIG, "\n# adopted out of protocol\n");
    let adopted_oid = hash_object(dir, CONFIG);
    assert!(
        !read(dir, LOCK).contains(&adopted_oid),
        "the fixture's seal already holds the drifted OID; the test proves nothing"
    );

    let result = run_ok(dir, &["adopt", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-0001", "ops": 1, "paths": [CONFIG] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![json!({ "op": "accept", "entity": "file", "key": CONFIG })]
    );
    assert!(
        read(dir, "telos/changes/CHG-0001.tel").contains(&adopted_oid),
        "the accept op must carry the OID of the bytes it accepted"
    );

    approve_and_reconcile(dir, "CHG-0001");

    assert_eq!(state(dir)["state"], json!("coherent"));
    assert!(
        read(dir, LOCK).contains(&adopted_oid),
        "the new seal must hold the accepted OID"
    );
}

/// The fourth gate refusing: an `accept` is an approval of *specific* bytes,
/// so bytes that moved after the approval stop the reconcile instead of
/// sealing content nobody reviewed.
#[test]
fn a_file_changed_after_it_was_accepted_fails_the_accept_gate() {
    let tmp = with_fixture();
    let dir = tmp.path();

    append(dir, CONFIG, "\n# adopted out of protocol\n");
    run_ok(dir, &["adopt", "--json"]);
    run_ok(dir, &["change", "approve", "CHG-0001", "--json"]);

    // The same path drifts again, after the review.
    append(dir, CONFIG, "# and again\n");

    let error = run_err(
        dir,
        &["change", "reconcile", "CHG-0001", "--json"],
        "TELOS_INTEGRITY_VIOLATION",
    );

    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(CONFIG),
        "the refusal must name the path: {message}"
    );
    assert!(
        dir.join("telos/changes/CHG-0001.tel").exists(),
        "a refused reconcile must not delete the change"
    );
}

/// A bound code file is not spec at all, and is adopted the same way: an
/// `accept` of its current bytes.
#[test]
fn adopting_a_modified_code_file_stages_an_accept() {
    let tmp = with_fixture();
    let dir = tmp.path();

    append(dir, CODE, "// adopted\n");
    let adopted_oid = hash_object(dir, CODE);

    let result = run_ok(dir, &["adopt", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-0001", "ops": 1, "paths": [CODE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![json!({ "op": "accept", "entity": "file", "key": CODE })]
    );

    approve_and_reconcile(dir, "CHG-0001");

    assert_eq!(state(dir)["state"], json!("coherent"));
    assert!(
        read(dir, LOCK).contains(&adopted_oid),
        "the new seal must hold the accepted code OID"
    );
}

/// The one drift `adopt` cannot capture: a deleted file that carries no
/// entity has no bytes to accept and no identity to remove, so it is
/// `revert`'s business.
#[test]
fn adopting_the_deletion_of_a_bound_code_file_is_refused() {
    let tmp = with_fixture();
    let dir = tmp.path();

    delete(dir, CODE);

    let error = run_err(dir, &["adopt", "--json"], "TELOS_INTEGRITY_VIOLATION");

    assert_eq!(
        error,
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": "cannot adopt: bound file `src/billing/invoice.rs` was deleted",
            "hint": "restore it with `telos revert`, or remove its binding"
        })
    );
    assert!(
        !dir.join("telos/changes/CHG-0001.tel").exists(),
        "a refused adopt must write nothing"
    );
}

/// A `.tel` file whose declared entity belongs at another path. Adopting it
/// would stage an op claiming `Other.tel` and leave `Rogue.tel` drifted --
/// an `adopt` that reports success on a project it did not capture.
#[test]
fn adopting_a_file_that_declares_another_entity_is_refused() {
    let tmp = with_fixture();
    let dir = tmp.path();

    write(
        dir,
        ROGUE,
        &ROGUE_TEL.replace("notion Rogue", "notion Other"),
    );

    let error = run_err(dir, &["adopt", "--json"], "TELOS_INTEGRITY_VIOLATION");

    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(ROGUE) && message.contains("telos/notions/Other.tel"),
        "the refusal must name both the file and where its entity belongs: {message}"
    );
    assert!(
        !dir.join("telos/changes/CHG-0001.tel").exists(),
        "a refused adopt must write nothing"
    );
}

/// The safety net under the idempotent overlay (D7), end to end: `adopt`
/// plans a `remove` for any deleted entity file without complaint, so what
/// stops a deletion the rest of the spec still depends on is the *post-state
/// model* -- and it does, on both routes into the command.
///
/// The `--into` half matters on its own: it allocates no id, so it never
/// loads a model before the validation does, which makes
/// `validate_ops_idempotent` provably the thing that refuses.
#[test]
fn adopting_a_delta_the_post_state_model_refuses_writes_nothing() {
    let tmp = with_fixture();
    let dir = tmp.path();

    // Opened while the project is still coherent, and left empty.
    run_ok(dir, &["change", "open", "unrelated work", "--json"]);

    delete(dir, CUSTOMER);
    assert_eq!(state(dir)["state"], json!("drifted"));

    // (a) Into an existing change: the validation is the only model load.
    let error = run_err(
        dir,
        &["adopt", "--into", "CHG-0001", "--json"],
        "TELOS_REFERENCE_UNKNOWN",
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("unknown notion `Customer`"),
        "the refusal must name the reference that no longer resolves: {error}"
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        Vec::<Value>::new(),
        "a refused adopt must leave the target change untouched"
    );

    // (b) Into a new change: same verdict, and no change file to show for it.
    run_err(dir, &["adopt", "--json"], "TELOS_REFERENCE_UNKNOWN");
    assert!(
        !dir.join("telos/changes/CHG-0002.tel").exists(),
        "a refused adopt must not leave a change behind"
    );
}

// --- adopt: the refusals ----------------------------------------------------

#[test]
fn adopt_on_a_coherent_project_is_refused() {
    let tmp = with_fixture();

    let error = run_err(
        tmp.path(),
        &["adopt", "--json"],
        "TELOS_CHANGE_STATE_INVALID",
    );

    assert_eq!(
        error["message"],
        json!("nothing to adopt: the project has not drifted")
    );
}

#[test]
fn revert_on_a_coherent_project_is_refused() {
    let tmp = with_fixture();

    let error = run_err(
        tmp.path(),
        &["revert", "--json"],
        "TELOS_CHANGE_STATE_INVALID",
    );

    assert_eq!(
        error["message"],
        json!("nothing to revert: the project has not drifted")
    );
}

/// A drifted `.tel` file that no longer parses cannot become an op: `adopt`
/// says so, names the other exit, and writes nothing.
#[test]
fn adopting_an_unparseable_spec_file_is_refused_and_writes_nothing() {
    let tmp = with_fixture();
    let dir = tmp.path();

    write(dir, INVOICE, "this is not a notion at all\n");

    let error = run_err(dir, &["adopt", "--json"], "TELOS_PARSE_ERROR");

    assert_eq!(error["hint"], json!(PARSE_HINT));
    assert!(
        error["message"].as_str().unwrap().contains(INVOICE),
        "the refusal must name the file: {error}"
    );
    assert!(
        !dir.join("telos/changes/CHG-0001.tel").exists(),
        "a refused adopt must write nothing"
    );
}

// --- adopt --into -----------------------------------------------------------

/// `--into` appends the adopted ops to a change that already holds some,
/// instead of opening a new one.
#[test]
fn adopt_into_an_existing_change_appends_its_ops() {
    let tmp = with_fixture();
    let dir = tmp.path();

    run_ok(dir, &["change", "open", "tighten the constraint", "--json"]);
    let mut cmd = telos(
        dir,
        &[
            "edit",
            "constraint",
            "CON-0003",
            "--change",
            "CHG-0001",
            "--json",
        ],
    );
    let out = cmd
        .write_stdin(json!({ "title": "Hexagonal boundaries, tightened" }).to_string())
        .output()
        .expect("failed to stage the edit");
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(envelope["ok"], json!(true), "staging failed: {envelope}");

    // Only now does the project drift, on a path CHG-0001 does not claim.
    append(dir, INVOICE, "\n");
    assert_eq!(state(dir)["state"], json!("drifted"));

    let result = run_ok(dir, &["adopt", "--into", "CHG-0001", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-0001", "ops": 1, "paths": [INVOICE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-0001"),
        vec![
            json!({ "op": "edit", "entity": "constraint", "key": "CON-0003" }),
            json!({ "op": "edit", "entity": "notion", "key": "Invoice" }),
        ],
        "the adopted op is appended after the ops already staged"
    );

    approve_and_reconcile(dir, "CHG-0001");
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert!(read(dir, CON_0003).contains("Hexagonal boundaries, tightened"));
}

// --- revert -----------------------------------------------------------------

/// The golden of Annex E: what was sealed comes back, what was never sealed
/// goes away, and the project is coherent again -- byte for byte.
#[test]
fn revert_restores_a_modified_file_and_deletes_an_untracked_one() {
    let tmp = with_fixture();
    let dir = tmp.path();
    commit(dir);
    let sealed = read(dir, INVOICE);

    append(dir, INVOICE, "\n");
    write(dir, ROGUE, ROGUE_TEL);
    assert_eq!(state(dir)["state"], json!("drifted"));

    let envelope = run_ok(dir, &["revert", "--json"]);

    assert_eq!(envelope["command"], json!("revert"));
    assert_eq!(
        envelope["result"],
        json!({ "restored": [INVOICE], "deleted": [ROGUE] })
    );
    assert_eq!(envelope["next_actions"], json!(["telos status"]));
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, INVOICE), sealed);
    assert!(!dir.join(ROGUE).exists(), "the untracked file must be gone");
}

/// A deleted file is restored from the same place a modified one is: the
/// blob its OID names. Covers both halves of the seal -- a spec file and a
/// bound code file.
#[test]
fn revert_restores_deleted_spec_and_code_files() {
    let tmp = with_fixture();
    let dir = tmp.path();
    commit(dir);
    let sealed_constraint = read(dir, CON_0003);
    let sealed_code = read(dir, CODE);

    delete(dir, CON_0003);
    delete(dir, CODE);

    let result = run_ok(dir, &["revert", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "restored": [CODE, CON_0003], "deleted": [] })
    );
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, CON_0003), sealed_constraint);
    assert_eq!(read(dir, CODE), sealed_code);
}

/// `revert` restores *content*, and content only lives in the object store
/// once somebody committed it. A fixture that was sealed but never committed
/// gets the one refusal that says exactly that.
#[test]
fn revert_without_the_sealed_blob_in_the_object_store_is_refused() {
    let tmp = with_fixture();
    let dir = tmp.path();

    append(dir, INVOICE, "\n");

    let error = run_err(dir, &["revert", "--json"], "TELOS_GIT_ERROR");

    assert_eq!(error["hint"], json!(SEALED_HINT));
}
