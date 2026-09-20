//! End-to-end tests for `telos adopt` and `telos revert`: the two exits from
//! drift by either capturing current bytes or restoring the seal.
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
//!   hashes the tree (`git hash-object`, no object store involved). `revert`
//!   restores sealed *content* from the object store -- and finds it there
//!   whether or not anybody committed, because the seal itself writes the
//!   objects it names. The revert section runs on committed and on
//!   never-committed fixtures alike, and its last test pins the one way a
//!   sealed blob can still be missing: git pruned it as unreachable.
//! - **After `adopt` the project is `changing`, not `coherent`.** The drift
//!   is claimed by the change that adopted it, so it stops being drift
//!   and becomes the change in progress. It is the reconcile, not the adopt,
//!   that reseals.
//! - **Recovery requires approved ownership.** An unrelated plan cannot
//!   authorize unresolved adopted bytes. The recovery plan must reconcile
//!   them first; a separately approved full recovery also records attribution.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use common::{telos, with_fixture};

#[test]
fn adopt_pairs_a_missing_and_untracked_owned_entity_into_a_move() {
    let tmp = with_fixture();
    let from = tmp
        .path()
        .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
    let to = tmp
        .path()
        .join("telos/contexts/billing/capabilities/settlement/intents/INT-0017.tel");
    let moved = fs::read_to_string(&from)
        .unwrap()
        .replace("in billing/invoicing", "in billing/settlement");
    fs::remove_file(&from).unwrap();
    fs::write(&to, moved).unwrap();

    let result = run_ok(tmp.path(), &["adopt", "--json"])["result"].clone();
    assert_eq!(result["ops"], json!(1));
    assert_eq!(
        result["paths"],
        json!([
            "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel",
            "telos/contexts/billing/capabilities/settlement/intents/INT-0017.tel"
        ])
    );
    let change = read(
        tmp.path(),
        "telos/changes/CHG-00000000-0000-0000-0000-000000000001.tel",
    );
    assert!(
        change.contains("op move from billing/invoicing to billing/settlement intent INT-0017")
    );
}

// --- the corpus paths these tests drift ------------------------------------

const INVOICE: &str = "telos/contexts/billing/notions/Invoice.tel";
/// Named by `Invoice`'s `rel issued-to` and by INT-0017's scenario: the
/// corpus' most-referenced notion, hence the one whose deletion the post-
/// state model must refuse.
const CUSTOMER: &str = "telos/contexts/billing/notions/Customer.tel";
const CON_0003: &str = "telos/contexts/billing/constraints/CON-0003.tel";
const ROGUE: &str = "telos/contexts/billing/notions/Rogue.tel";
const CONFIG: &str = "telos/telos.toml";
const CODE: &str = "src/billing/invoice.rs";
const LOCK: &str = "telos/telos.lock";

/// A valid notion, in canonical form, created outside the protocol -- the
/// `Untracked` half of drift.
const ROGUE_TEL: &str = "notion billing/Rogue entity {\n  \
    def  \"A notion created outside the protocol.\"\n  \
    attr label string\n\
}\n";

/// The exact `TELOS_PARSE_ERROR` hint `adopt` attaches to a drifted file it
/// cannot read.
const PARSE_HINT: &str = "fix the file or run `telos revert`";

/// The exact `TELOS_GIT_ERROR` hint `revert` attaches when the sealed blob is
/// not in the object store (pruned, or sealed by an older `telos`).
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
    common::finish_fixture_task(dir);
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

/// Commits the whole fixture. The seal already wrote its blobs to the
/// object store, so `revert` does not need this; the tests that call it
/// exercise a *committed* project, where the sealed objects are reachable
/// from `HEAD` rather than orphaned.
fn commit(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", "seal the billing corpus"]);
}

/// Stages one op into a change from a JSON payload on stdin, asserting the
/// envelope came back `ok`.
fn stage(dir: &Path, args: &[&str], payload: &str) -> Value {
    let out = telos(dir, args)
        .write_stdin(payload.to_string())
        .output()
        .unwrap_or_else(|e| panic!("failed to run telos {args:?}: {e}"));
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected `telos {}` to stage, got: {envelope}",
        args.join(" ")
    );
    envelope
}

/// The OID `telos.lock` currently records for `rel`, in whichever of its two
/// tables holds it -- `None` when the seal holds no record of the path at
/// all. Reads the rendered lock rather than a parse: the file's format is
/// frozen (`"<path>" = "<oid>"`, one entry per line), and what these tests
/// need to pin is exactly what a reader of the file would see.
fn lock_oid(dir: &Path, rel: &str) -> Option<String> {
    let prefix = format!("{rel:?} = ");
    read(dir, LOCK).lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|oid| oid.trim_matches('"').to_string())
    })
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

/// The golden of the result schema, and the canonicalization proof: a byte appended to
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

    // The whole envelope of the result schema: the command names itself, the result is
    // the golden shape, the next actions are the loop that follows.
    assert_eq!(envelope["command"], json!("adopt"));
    assert_eq!(
        envelope["result"],
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [INVOICE] })
    );
    let (plan, task) =
        telos_core::plans::store::for_change(dir, "CHG-00000000-0000-0000-0000-000000000001")
            .unwrap();
    assert_eq!(
        envelope["next_actions"],
        json!([
            "telos change diff CHG-00000000-0000-0000-0000-000000000001".to_string(),
            format!("telos plan task import {} {}", plan.id, task.definition.id),
            format!("telos plan diff {}", plan.id),
        ])
    );
    // The adopted path is claimed now, so it is no longer drift.
    assert_eq!(state(dir)["state"], json!("changing"));
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![json!({ "op": "edit", "entity": "notion", "key": "billing/Invoice" })]
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");

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
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [CON_0003] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![json!({ "op": "remove", "entity": "constraint", "key": "CON-0003" })]
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");

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
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [ROGUE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![json!({ "op": "add", "entity": "notion", "key": "billing/Rogue" })]
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");

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
/// sealing them.
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
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [CONFIG] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![json!({ "op": "accept", "entity": "file", "key": CONFIG })]
    );
    assert!(
        read(
            dir,
            "telos/changes/CHG-00000000-0000-0000-0000-000000000001.tel"
        )
        .contains(&adopted_oid),
        "the accept op must carry the OID of the bytes it accepted"
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");

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
    run_ok(
        dir,
        &[
            "change",
            "approve",
            "CHG-00000000-0000-0000-0000-000000000001",
            "--json",
        ],
    );

    // The same path drifts again, after the review.
    append(dir, CONFIG, "# and again\n");

    let error = run_err(
        dir,
        &[
            "change",
            "reconcile",
            "CHG-00000000-0000-0000-0000-000000000001",
            "--json",
        ],
        "TELOS_INTEGRITY_VIOLATION",
    );

    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(CONFIG),
        "the refusal must name the path: {message}"
    );
    assert!(
        dir.join("telos/changes/CHG-00000000-0000-0000-0000-000000000001.tel")
            .exists(),
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
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [CODE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![json!({ "op": "accept", "entity": "file", "key": CODE })]
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");

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
        prepared_change_is_empty(dir),
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
        &ROGUE_TEL.replace("billing/Rogue", "billing/Other"),
    );

    let error = run_err(dir, &["adopt", "--json"], "TELOS_INTEGRITY_VIOLATION");

    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(ROGUE) && message.contains("telos/contexts/billing/notions/Other.tel"),
        "the refusal must name both the file and where its entity belongs: {message}"
    );
    assert!(
        prepared_change_is_empty(dir),
        "a refused adopt must write nothing"
    );
}

/// The safety net under the idempotent overlay, end to end: `adopt`
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
        &[
            "adopt",
            "--into",
            "CHG-00000000-0000-0000-0000-000000000001",
            "--json",
        ],
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
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        Vec::<Value>::new(),
        "a refused adopt must leave the target change untouched"
    );

    // (b) Into a new change: same verdict, and no change file to show for it.
    run_err(dir, &["adopt", "--json"], "TELOS_REFERENCE_UNKNOWN");
    assert!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000002").is_empty(),
        "a refused adopt must leave the prepared task unchanged"
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

#[test]
fn adopt_refuses_a_drift_token_whose_scope_changed_during_review() {
    let tmp = with_fixture();
    let dir = tmp.path();
    append(dir, INVOICE, "\n");
    let token = state(dir)["drift"]["token"].as_str().unwrap().to_string();
    append(dir, CONFIG, "\n# later drift\n");

    let error = run_err(
        dir,
        &["adopt", "--expected-state", &token, "--json"],
        "TELOS_CHANGE_STATE_INVALID",
    );

    assert_eq!(
        error["message"],
        json!("project drift no longer matches the expected state token")
    );
    assert!(prepared_change_is_empty(dir));
}

#[test]
fn revert_refuses_a_drift_token_whose_scope_changed_during_review() {
    let tmp = with_fixture();
    let dir = tmp.path();
    append(dir, INVOICE, "\n");
    let token = state(dir)["drift"]["token"].as_str().unwrap().to_string();
    append(dir, CONFIG, "\n# later drift\n");
    let invoice_before = read(dir, INVOICE);
    let config_before = read(dir, CONFIG);

    let error = run_err(
        dir,
        &["revert", "--expected-state", &token, "--json"],
        "TELOS_CHANGE_STATE_INVALID",
    );

    assert_eq!(
        error["message"],
        json!("project drift no longer matches the expected state token")
    );
    assert_eq!(read(dir, INVOICE), invoice_before);
    assert_eq!(read(dir, CONFIG), config_before);
}

#[test]
fn drift_actions_refuse_a_token_when_bytes_change_inside_the_same_scope() {
    for action in ["adopt", "revert"] {
        let tmp = with_fixture();
        let dir = tmp.path();
        append(dir, INVOICE, "\n// first reviewed drift\n");
        let token = state(dir)["drift"]["token"].as_str().unwrap().to_string();
        append(dir, INVOICE, "// later unreviewed bytes\n");
        let before = read(dir, INVOICE);

        let error = run_err(
            dir,
            &[action, "--expected-state", &token, "--json"],
            "TELOS_CHANGE_STATE_INVALID",
        );

        assert_eq!(
            error["message"],
            json!("project drift no longer matches the expected state token")
        );
        assert_eq!(read(dir, INVOICE), before);
        assert!(prepared_change_is_empty(dir));
    }
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
        prepared_change_is_empty(dir),
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
    stage(
        dir,
        &[
            "edit",
            "constraint",
            "CON-0003",
            "--change",
            "CHG-00000000-0000-0000-0000-000000000001",
            "--json",
        ],
        &json!({ "title": "Hexagonal boundaries, tightened" }).to_string(),
    );

    // Only now does the project drift, on a path CHG-00000000-0000-0000-0000-000000000001 does not claim.
    append(dir, INVOICE, "\n");
    assert_eq!(state(dir)["state"], json!("drifted"));

    let result = run_ok(
        dir,
        &[
            "adopt",
            "--into",
            "CHG-00000000-0000-0000-0000-000000000001",
            "--json",
        ],
    )["result"]
        .clone();

    assert_eq!(
        result,
        json!({ "change": "CHG-00000000-0000-0000-0000-000000000001", "ops": 1, "paths": [INVOICE] })
    );
    assert_eq!(
        diff_ops(dir, "CHG-00000000-0000-0000-0000-000000000001"),
        vec![
            json!({ "op": "edit", "entity": "constraint", "key": "CON-0003" }),
            json!({ "op": "edit", "entity": "notion", "key": "billing/Invoice" }),
        ],
        "the adopted op is appended after the ops already staged"
    );

    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert!(read(dir, CON_0003).contains("Hexagonal boundaries, tightened"));
}

// --- the carry-over: nobody else's reconcile seals an adopted drift ---------

/// `Invoice`'s sealed `def` line, and the one a hand edit puts in its place.
/// Chosen so the drift *survives* canonicalization -- a stray newline would
/// be emitted away, and the final seal would land back on the sealed OID,
/// which would make the last assertion of the test below vacuous.
const SEALED_DEF: &str = "A bill issued to a Customer for delivered work.";
const HAND_EDITED_DEF: &str = "A bill issued to a Customer, edited out of protocol.";

/// A valid unrelated notion used to test recovery ownership.
fn refund_payload() -> String {
    json!({
        "owner": "billing",
        "name": "Refund", "kind": "event",
        "def": "A refund was issued against an invoice.",
        "attrs": [ {"name": "amount", "type": "money"} ]
    })
    .to_string()
}

/// An unrelated draft may be prepared, but cannot inherit execution authority
/// while an adopted out-of-protocol edit still needs recovery.
#[test]
fn unrelated_execution_waits_for_the_adopted_edit_to_be_reconciled() {
    let tmp = with_fixture();
    let dir = tmp.path();
    commit(dir);
    let sealed_oid = hash_object(dir, INVOICE);

    // --- the out-of-protocol edit, adopted and left drafted ---------------
    write(
        dir,
        INVOICE,
        &read(dir, INVOICE).replace(SEALED_DEF, HAND_EDITED_DEF),
    );
    let drifted_oid = hash_object(dir, INVOICE);
    assert_ne!(
        drifted_oid, sealed_oid,
        "the hand edit did not change the file"
    );
    assert_eq!(state(dir)["state"], json!("drifted"));

    run_ok(dir, &["adopt", "--json"]);
    assert_eq!(state(dir)["state"], json!("changing"));

    // --- unrelated execution waits for the approved recovery -------------
    run_ok(dir, &["change", "open", "record refunds", "--json"]);
    stage(
        dir,
        &[
            "add",
            "notion",
            "--change",
            "CHG-00000000-0000-0000-0000-000000000002",
            "--json",
        ],
        &refund_payload(),
    );
    let lock_before = read(dir, LOCK);
    run_err(
        dir,
        &[
            "change",
            "approve",
            "CHG-00000000-0000-0000-0000-000000000002",
            "--json",
        ],
        "TELOS_PLAN_NOT_APPROVED",
    );
    assert_eq!(read(dir, LOCK), lock_before);
    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");
    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000002");
    assert_eq!(state(dir)["state"], "coherent");
}

/// Untracked adopted content also requires its own approved recovery before
/// an unrelated task can execute and seal the repository.
#[test]
fn unrelated_execution_waits_for_the_adopted_new_file_to_be_reconciled() {
    let tmp = with_fixture();
    let dir = tmp.path();

    write(dir, ROGUE, ROGUE_TEL);
    run_ok(dir, &["adopt", "--json"]);

    run_ok(dir, &["change", "open", "record refunds", "--json"]);
    stage(
        dir,
        &[
            "add",
            "notion",
            "--change",
            "CHG-00000000-0000-0000-0000-000000000002",
            "--json",
        ],
        &refund_payload(),
    );
    let lock_before = read(dir, LOCK);
    run_err(
        dir,
        &[
            "change",
            "approve",
            "CHG-00000000-0000-0000-0000-000000000002",
            "--json",
        ],
        "TELOS_PLAN_NOT_APPROVED",
    );
    assert_eq!(read(dir, LOCK), lock_before);
    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");
    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000002");
    assert_eq!(state(dir)["state"], "coherent");
}

/// A separately approved full recovery re-proves the complete observed tree.
/// Prepared adoption drafts remain visible after that attributed recovery.
#[test]
fn a_full_reseal_seals_disk_truth_even_under_an_open_adopt_change() {
    let tmp = with_fixture();
    let dir = tmp.path();

    write(
        dir,
        INVOICE,
        &read(dir, INVOICE).replace(SEALED_DEF, HAND_EDITED_DEF),
    );
    let drifted_oid = hash_object(dir, INVOICE);
    run_ok(dir, &["adopt", "--json"]);

    run_ok(dir, &["change", "reconcile", "--full", "--json"]);

    assert_eq!(lock_oid(dir, INVOICE), Some(drifted_oid));
    // The change is still open and still claims the path (full reconciliation leaves open
    // drafts alone), so the project is `changing` -- but its op is now a
    // no-op against a seal that already holds those bytes.
    assert_eq!(state(dir)["state"], json!("changing"));
}

// --- revert -----------------------------------------------------------------

/// The golden of the result schema: what was sealed comes back, what was never sealed
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
    assert_eq!(state(dir)["state"], json!("changing"));
    let (_, task) = telos_core::plans::store::active(dir).unwrap().unwrap();
    run_ok(
        dir,
        &[
            "change",
            "reconcile",
            task.change.as_deref().unwrap(),
            "--json",
        ],
    );
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
    assert_eq!(state(dir)["state"], json!("changing"));
    let (_, task) = telos_core::plans::store::active(dir).unwrap().unwrap();
    run_ok(
        dir,
        &[
            "change",
            "reconcile",
            task.change.as_deref().unwrap(),
            "--json",
        ],
    );
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, CON_0003), sealed_constraint);
    assert_eq!(read(dir, CODE), sealed_code);
}

/// The seal writes the objects it names (`git hash-object -w`), so `revert`
/// does not depend on a commit: a project `init` sealed and never committed
/// still restores byte for byte.
#[test]
fn revert_restores_an_init_sealed_file_on_a_project_never_committed() {
    let tmp = with_fixture();
    let dir = tmp.path();
    let sealed = read(dir, INVOICE);

    append(dir, INVOICE, "\n");
    assert_eq!(state(dir)["state"], json!("drifted"));

    let result = run_ok(dir, &["revert", "--json"])["result"].clone();

    assert_eq!(result, json!({ "restored": [INVOICE], "deleted": [] }));
    assert_eq!(state(dir)["state"], json!("changing"));
    let (_, task) = telos_core::plans::store::active(dir).unwrap().unwrap();
    run_ok(
        dir,
        &[
            "change",
            "reconcile",
            task.change.as_deref().unwrap(),
            "--json",
        ],
    );
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, INVOICE), sealed);
}

/// The scenario that motivated this: a change reconciled but not yet
/// committed, then drifted. The reconcile is the seal that named the new
/// OIDs, so it is the reconcile that must have written them -- for the spec
/// it rewrote and for the code it accepted alike.
#[test]
fn revert_restores_reconciled_spec_and_code_on_a_project_never_committed() {
    let tmp = with_fixture();
    let dir = tmp.path();

    append(dir, INVOICE, "\n");
    append(dir, CODE, "// adopted\n");
    run_ok(dir, &["adopt", "--json"]);
    approve_and_reconcile(dir, "CHG-00000000-0000-0000-0000-000000000001");
    assert_eq!(state(dir)["state"], json!("coherent"));
    let sealed_invoice = read(dir, INVOICE);
    let sealed_code = read(dir, CODE);

    append(dir, INVOICE, "\n");
    append(dir, CODE, "// drifted again\n");
    assert_eq!(state(dir)["state"], json!("drifted"));

    let result = run_ok(dir, &["revert", "--json"])["result"].clone();

    assert_eq!(
        result,
        json!({ "restored": [CODE, INVOICE], "deleted": [] })
    );
    assert_eq!(state(dir)["state"], json!("changing"));
    let (_, task) = telos_core::plans::store::active(dir).unwrap().unwrap();
    run_ok(
        dir,
        &[
            "change",
            "reconcile",
            task.change.as_deref().unwrap(),
            "--json",
        ],
    );
    assert_eq!(state(dir)["state"], json!("coherent"));
    assert_eq!(read(dir, INVOICE), sealed_invoice);
    assert_eq!(read(dir, CODE), sealed_code);
}

/// The objects a seal writes stay unreachable until a commit references
/// them, and git is free to prune unreachable objects (by default only
/// after two weeks; `--expire=now` here). Once it has, the blob is gone and
/// `revert` gets the one refusal that says exactly that.
#[test]
fn revert_after_git_pruned_the_sealed_blob_is_refused() {
    let tmp = with_fixture();
    let dir = tmp.path();
    git(dir, &["prune", "--expire=now"]);

    append(dir, INVOICE, "\n");

    let error = run_err(dir, &["revert", "--json"], "TELOS_GIT_ERROR");

    assert_eq!(error["hint"], json!(SEALED_HINT));
}

fn prepared_change_is_empty(root: &Path) -> bool {
    let ws = telos_core::workspace::Workspace::discover(root).unwrap();
    telos_core::changes::read_change(&ws, telos_core::ids::ChangeId(1))
        .is_ok_and(|c| c.ops.is_empty())
}
