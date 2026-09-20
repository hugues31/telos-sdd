//! The native workflow uses the real CLI without fixture approval adapters.
mod common;

use common::{raw_telos, repo};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn call(root: &Path, args: &[&str], payload: Option<Value>) -> Value {
    let mut command = raw_telos(root, args);
    command.arg("--json");
    if let Some(payload) = payload {
        command.write_stdin(payload.to_string());
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)))
}
fn ok(root: &Path, args: &[&str], payload: Option<Value>) -> Value {
    let value = call(root, args, payload);
    assert_eq!(value["ok"], true, "{value}");
    value["result"].clone()
}
fn definition() -> Value {
    json!({"title":"Document setup","request":"Explain installation","goal":"Readers can install the project",
        "success_criteria":["Instructions are accurate"],"brief":{"summary":"Document existing behavior","brainstormed":true},
        "scope":["README.md"],"tasks":[{"id":"TSK-001","title":"Write installation instructions","kind":"docs",
            "allowed_paths":["README.md"],"acceptance":["Commands reviewed"],"validation":[{"kind":"review","name":"content"}]}],
        "validation":[{"kind":"review","name":"final"}]})
}
fn draft(root: &Path, definition: Value) -> String {
    let id = ok(root, &["plan", "open", "Documentation"], None)["plan"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(root, &["plan", "edit", &id], Some(definition));
    id
}
fn approve(root: &Path, id: &str) {
    let digest = ok(root, &["plan", "diff", id], None)["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        root,
        &["plan", "approve", id, "--expected-digest", &digest],
        None,
    );
}
fn start(root: &Path, id: &str) -> String {
    ok(root, &["plan", "task", "start", id, "TSK-001"], None)["result"]["change"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn review(root: &Path, id: &str, task: bool) {
    let args = if task {
        vec![
            "plan",
            "verify",
            id,
            "--task",
            "TSK-001",
            "content",
            "--review",
            "Reviewed every acceptance criterion",
        ]
    } else {
        vec![
            "plan",
            "verify",
            id,
            "final",
            "--review",
            "All success criteria verified",
        ]
    };
    assert_eq!(ok(root, &args, None)["result"]["passed"], true);
}

#[test]
fn documentation_is_planned_reconciled_validated_and_visible_in_static_export() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let id = draft(root.path(), definition());
    approve(root.path(), &id);
    let change = start(root.path(), &id);
    fs::write(root.path().join("README.md"), "Install the project.\n").unwrap();
    ok(
        root.path(),
        &[
            "plan",
            "checkpoint",
            &id,
            "--summary",
            "Instructions written",
            "--next-action",
            "Review the commands",
        ],
        None,
    );
    let resume = ok(root.path(), &["plan", "resume", &id], None);
    assert_eq!(resume["plan"]["progress"]["percent"], 0);
    assert_eq!(
        resume["checkpoint"]["data"]["next_action"],
        "Review the commands"
    );
    assert_eq!(resume["changes_since_checkpoint"], json!([]));
    ok(
        root.path(),
        &["change", "reconcile", &change, "--request-id", "seal-once"],
        None,
    );
    // The response can be replayed after the open change has disappeared.
    ok(
        root.path(),
        &["change", "reconcile", &change, "--request-id", "seal-once"],
        None,
    );
    let history = ok(root.path(), &["history", "README.md"], None);
    assert_eq!(history["history"][0]["plan"], id);
    review(root.path(), &id, true);
    ok(
        root.path(),
        &["plan", "task", "finish", &id, "TSK-001"],
        None,
    );
    let status = ok(root.path(), &["status"], None);
    assert_eq!(status["state"], "coherent");
    let plan = status["plans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap();
    assert_eq!(plan["progress"]["percent"], 100);
    assert_ne!(plan["state"], "completed");
    assert_eq!(
        call(root.path(), &["plan", "complete", &id], None)["error"]["code"],
        "TELOS_PLAN_VALIDATION_FAILED"
    );
    review(root.path(), &id, false);
    ok(root.path(), &["plan", "complete", &id], None);
    ok(root.path(), &["check", "--sealed", "--planned"], None);
    let export_root = tempfile::tempdir().unwrap();
    let destination = export_root.path().join("site");
    ok(
        root.path(),
        &["view", "--export", destination.to_str().unwrap()],
        None,
    );
    let data = fs::read_to_string(destination.join("data.js")).unwrap();
    assert!(data.contains(&id) && data.contains(&change) && data.contains("completed"));
}

#[test]
fn all_paths_are_governed_and_nothing_runs_before_approval() {
    let root = repo();
    ok(root.path(), &["init"], None);
    assert_eq!(
        call(root.path(), &["change", "open", "Bypass"], None)["error"]["code"],
        "TELOS_PLAN_REQUIRED"
    );
    assert_eq!(
        call(root.path(), &["change", "reconcile", "--full"], None)["error"]["code"],
        "TELOS_PLAN_REQUIRED"
    );
    let id = draft(root.path(), definition());
    assert_eq!(
        call(
            root.path(),
            &["plan", "task", "start", &id, "TSK-001"],
            None
        )["error"]["code"],
        "TELOS_PLAN_NOT_APPROVED"
    );
    approve(root.path(), &id);
    let change = start(root.path(), &id);
    fs::write(root.path().join("Cargo.lock"), "unplanned dependency\n").unwrap();
    assert_eq!(
        call(root.path(), &["check", "--planned"], None)["error"]["code"],
        "TELOS_UNPLANNED_CHANGE"
    );
    assert_eq!(
        call(root.path(), &["change", "reconcile", &change], None)["error"]["code"],
        "TELOS_UNPLANNED_CHANGE"
    );
    let resume = ok(root.path(), &["plan", "resume", &id], None);
    assert_eq!(resume["unplanned"][0]["path"], "Cargo.lock");
    assert!(
        root.path()
            .join(format!("telos/changes/{change}.tel"))
            .exists()
    );
}

#[test]
fn request_replay_precedes_cas_and_different_input_is_refused() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let first = ok(
        root.path(),
        &["plan", "open", "Once", "--request-id", "once"],
        None,
    );
    let second = ok(
        root.path(),
        &["plan", "open", "Once", "--request-id", "once"],
        None,
    );
    assert_eq!(first, second);
    assert_eq!(
        call(
            root.path(),
            &["plan", "open", "Different", "--request-id", "once"],
            None
        )["error"]["code"],
        "TELOS_REQUEST_ID_CONFLICT"
    );
    let id = first["plan"].as_str().unwrap();
    let args = [
        "plan",
        "edit",
        id,
        "--request-id",
        "edit-once",
        "--expected-version",
        "1",
    ];
    let edited = ok(root.path(), &args, Some(definition()));
    assert_eq!(edited, ok(root.path(), &args, Some(definition())));
    assert_eq!(
        call(
            root.path(),
            &["plan", "edit", id, "--expected-version", "1"],
            Some(definition())
        )["error"]["code"],
        "TELOS_PLAN_VERSION_STALE"
    );
}

#[test]
fn pause_revise_and_approve_scope_without_repeating_completed_decisions() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let id = draft(root.path(), definition());
    approve(root.path(), &id);
    start(root.path(), &id);
    fs::write(root.path().join("README.md"), "work in progress").unwrap();
    ok(
        root.path(),
        &[
            "plan",
            "pause",
            &id,
            "--reason",
            "Include contributor instructions",
        ],
        None,
    );
    let mut revised = definition();
    revised["scope"] = json!(["README.md", "CONTRIBUTING.md"]);
    revised["tasks"][0]["allowed_paths"] = revised["scope"].clone();
    ok(root.path(), &["plan", "edit", &id], Some(revised));
    assert_eq!(
        ok(root.path(), &["plan", "resume", &id], None)["plan"]["approved"],
        false
    );
    approve(root.path(), &id);
    assert_eq!(
        ok(root.path(), &["plan", "resume", &id], None)["plan"]["approved"],
        true
    );
}

#[test]
fn blocking_questions_and_dependency_cycles_cannot_be_approved() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let mut value = definition();
    value["brief"]["questions"] =
        json!([{"id":"Q-1","text":"Which package manager?","blocking":true,"answer":null}]);
    let id = draft(root.path(), value.clone());
    let digest = ok(root.path(), &["plan", "diff", &id], None)["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        call(
            root.path(),
            &["plan", "approve", &id, "--expected-digest", &digest],
            None
        )["ok"],
        false
    );
    value["brief"]["questions"][0]["answer"] = json!("Use the existing manager");
    value["tasks"][0]["depends_on"] = json!(["TSK-001"]);
    assert_eq!(
        call(root.path(), &["plan", "edit", &id], Some(value))["error"]["code"],
        "TELOS_CYCLE_DETECTED"
    );
}

#[test]
fn ci_rejects_rewritten_historical_records_and_unattributed_deletions() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let git = |args: &[&str]| telos_core::inventory::git_output(root.path(), args).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "Bootstrap"]);
    let base = telos_core::inventory::head(root.path()).unwrap().unwrap();
    let id = draft(root.path(), definition());
    approve(root.path(), &id);
    let change = start(root.path(), &id);
    fs::write(root.path().join("README.md"), "Instructions").unwrap();
    ok(root.path(), &["change", "reconcile", &change], None);
    ok(root.path(), &["check", "--planned", "--base", &base], None);
    fs::remove_file(root.path().join("README.md")).unwrap();
    assert_eq!(
        call(root.path(), &["check", "--planned", "--base", &base], None)["error"]["code"],
        "TELOS_UNPLANNED_CHANGE"
    );
    fs::write(root.path().join("README.md"), "Instructions").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "Document"]);
    let base = telos_core::inventory::head(root.path()).unwrap().unwrap();
    let receipt = root.path().join(format!("telos/history/{change}.tel"));
    let bytes = fs::read_to_string(&receipt).unwrap();
    fs::write(
        &receipt,
        bytes.replace("Write installation instructions", "Altered history"),
    )
    .unwrap();
    // Even a syntactically valid receipt cannot rewrite the trusted past.
    assert_eq!(
        call(root.path(), &["check", "--planned", "--base", &base], None)["ok"],
        false
    );
}

#[test]
fn validation_cannot_certify_a_runner_that_modifies_its_inputs() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let mut value = definition();
    value["validation"] = json!([{"kind":"command","name":"mutating","argv":["git","config","--file","README.md","test.changed","true"]}]);
    let id = draft(root.path(), value);
    approve(root.path(), &id);
    let change = start(root.path(), &id);
    fs::write(root.path().join("README.md"), "").unwrap();
    ok(root.path(), &["change", "reconcile", &change], None);
    review(root.path(), &id, true);
    ok(
        root.path(),
        &["plan", "task", "finish", &id, "TSK-001"],
        None,
    );
    let result = ok(root.path(), &["plan", "verify", &id, "mutating"], None);
    assert_eq!(result["result"]["passed"], false);
    assert_eq!(result["result"]["unchanged"], false);
    assert_eq!(
        call(root.path(), &["plan", "complete", &id], None)["ok"],
        false
    );
}

#[test]
fn runner_child() {
    let Ok(gate) = std::env::var("TELOS_TEST_RUNNER_GATE") else {
        return;
    };
    fs::write(&gate, "started").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    fs::write(&gate, "finished").unwrap();
}

#[test]
fn interrupted_runner_is_unknown_and_a_clone_resumes_without_runtime_or_chat() {
    let root = repo();
    ok(root.path(), &["init"], None);
    let mut value = definition();
    value["tasks"][0]["validation"] = json!([{"kind":"command","name":"slow","argv":[std::env::current_exe().unwrap().to_str().unwrap(),"--exact","runner_child","--nocapture"]}]);
    let id = draft(root.path(), value);
    approve(root.path(), &id);
    start(root.path(), &id);
    let gate = root.path().join("telos/.runtime/runner-gate");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_telos"))
        .args([
            "plan",
            "verify",
            &id,
            "--task",
            "TSK-001",
            "slow",
            "--request-id",
            "interrupted",
            "--json",
        ])
        .env("TELOS_TEST_RUNNER_GATE", &gate)
        .current_dir(root.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..300 {
        if gate.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(gate.exists());
    child.kill().unwrap();
    child.wait().unwrap();
    for _ in 0..300 {
        if fs::read_to_string(&gate).is_ok_and(|s| s == "finished") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let resume = ok(root.path(), &["plan", "resume", &id], None);
    assert!(!resume["unknown_validation"].is_null());
    assert_eq!(
        call(
            root.path(),
            &[
                "plan",
                "verify",
                &id,
                "--task",
                "TSK-001",
                "slow",
                "--request-id",
                "interrupted"
            ],
            None
        )["ok"],
        false
    );
    telos_core::inventory::git_output(root.path(), &["add", "."]).unwrap();
    telos_core::inventory::git_output(root.path(), &["commit", "-qm", "Persist interrupted work"])
        .unwrap();
    let clone = tempfile::tempdir().unwrap();
    telos_core::inventory::git_output(
        clone.path(),
        &[
            "clone",
            "--quiet",
            root.path().to_str().unwrap(),
            "checkout",
        ],
    )
    .unwrap();
    let checkout = clone.path().join("checkout");
    assert!(!checkout.join("telos/.runtime/runner-gate").exists());
    let resumed = ok(&checkout, &["plan", "resume", &id], None);
    assert_eq!(resumed["plan"]["current_task"], "TSK-001");
    assert!(!resumed["unknown_validation"].is_null());
}

fn finish_plan(root: &Path, id: &str) {
    review(root, id, true);
    ok(root, &["plan", "task", "finish", id, "TSK-001"], None);
    review(root, id, false);
    ok(root, &["plan", "complete", id], None);
}

#[test]
fn integration_retains_both_branch_histories_and_attributes_resolved_files() {
    let root = repo();
    let root = root.path();
    ok(root, &["init"], None);
    let git = |args: &[&str]| telos_core::inventory::git_output(root, args).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "Observed baseline"]);
    git(&["branch", "side"]);
    let first = draft(root, definition());
    approve(root, &first);
    let a = start(root, &first);
    fs::write(root.join("README.md"), "Primary instructions\n").unwrap();
    ok(root, &["change", "reconcile", &a], None);
    finish_plan(root, &first);
    git(&["add", "."]);
    git(&["commit", "-qm", "Primary work"]);
    let primary = telos_core::inventory::head(root).unwrap().unwrap();
    git(&["checkout", "-q", "side"]);
    let mut side = definition();
    side["scope"] = json!(["CONTRIBUTING.md"]);
    side["tasks"][0]["allowed_paths"] = side["scope"].clone();
    let second = draft(root, side);
    approve(root, &second);
    let b = start(root, &second);
    assert_ne!(a, b);
    fs::write(root.join("CONTRIBUTING.md"), "Contribute a reviewed plan\n").unwrap();
    ok(root, &["change", "reconcile", &b], None);
    finish_plan(root, &second);
    git(&["add", "."]);
    git(&["commit", "-qm", "Side work"]);
    let source = telos_core::inventory::head(root).unwrap().unwrap();
    git(&["checkout", "-q", &primary]);
    let mut definition = definition();
    definition["scope"] = json!(["CONTRIBUTING.md"]);
    definition["tasks"][0]["allowed_paths"] = definition["scope"].clone();
    definition["tasks"][0]["kind"] = json!("integration");
    let integration = draft(root, definition);
    approve(root, &integration);
    let merged = start(root, &integration);
    // Resolve code against the primary work records, then import the source
    // journal explicitly. This also works after resolving a Git merge conflict.
    git(&["checkout", &source, "--", "CONTRIBUTING.md"]);
    ok(
        root,
        &["plan", "integrate", &integration, "--source", &source],
        None,
    );
    assert_eq!(call(root, &["check", "--planned"], None)["ok"], false);
    ok(root, &["change", "reconcile", &merged], None);
    finish_plan(root, &integration);
    ok(
        root,
        &["check", "--sealed", "--planned", "--base", &primary],
        None,
    );
    let history = ok(root, &["history"], None);
    let receipts = history["history"].as_array().unwrap();
    assert_eq!(receipts.len(), 3);
    assert!(receipts.iter().any(|r| r["plan"] == first));
    assert!(receipts.iter().any(|r| r["plan"] == second));
    assert_eq!(
        receipts.last().unwrap()["parents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn provenance_tracks_semantic_entities_and_new_bindings_to_existing_code() {
    let root = common::with_fixture();
    let root = root.path();
    let mut value = definition();
    value["scope"] = json!(["telos/contexts/**"]);
    value["tasks"][0]["allowed_paths"] = value["scope"].clone();
    value["tasks"][0]["kind"] = json!("recovery");
    let first = draft(root, value.clone());
    approve(root, &first);
    start(root, &first);
    let path = root.join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
    let mut model = telos_core::workspace::Workspace::discover(root)
        .unwrap()
        .load_model()
        .unwrap();
    let intent = model
        .intents
        .get_mut(&telos_core::ids::IntentId(17))
        .unwrap();
    intent.title = "Reworded invoice issuance".into();
    let file = telos_core::model::TelFile::OwnedIntent {
        owner: model.intent_owners[&telos_core::ids::IntentId(17)].clone(),
        intent: intent.clone(),
    };
    fs::write(&path, telos_core::emit::emit_file(&file)).unwrap();
    ok(root, &["change", "reconcile", "--full"], None);
    finish_plan(root, &first);
    let history = ok(root, &["history", "INT-0017"], None);
    assert_eq!(history["history"].as_array().unwrap().len(), 1);
    assert!(
        history["history"][0]["entities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["selector"] != "SCN-0091")
    );
    let second = draft(root, value);
    approve(root, &second);
    start(root, &second);
    let path = root.join("telos/contexts/billing/bindings.tel");
    let mut bindings = fs::read_to_string(&path).unwrap();
    bindings.push_str("implements \"src/billing/invoice.rs\" -> INT-0017\n");
    fs::write(&path, bindings).unwrap();
    ok(root, &["change", "reconcile", "--full"], None);
    finish_plan(root, &second);
    let history = ok(root, &["history", "INT-0017"], None);
    assert_eq!(history["history"].as_array().unwrap().len(), 2);
    assert!(
        history["history"][1]["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["selector"] == "INT-0017" && e["kind"] == "implementation_changed")
    );
}

#[test]
fn staged_mutations_replay_their_exact_payload_before_version_checks() {
    let root = common::with_empty_billing_domain();
    let root = root.path();
    let mut definition = definition();
    definition["scope"] = json!(["telos/contexts/**"]);
    definition["tasks"][0]["allowed_paths"] = definition["scope"].clone();
    let id = draft(root, definition);
    let change = ok(root, &["plan", "task", "prepare", &id, "TSK-001"], None)["result"]["change"]
        .as_str()
        .unwrap()
        .to_owned();
    let args = [
        "add",
        "notion",
        "--change",
        &change,
        "--request-id",
        "stage-once",
    ];
    let payload = json!({"owner":"billing","name":"Customer","kind":"actor","def":"The party receiving an invoice"});
    let first = ok(root, &args, Some(payload.clone()));
    assert_eq!(ok(root, &args, Some(payload.clone())), first);
    let mut altered = payload;
    altered["def"] = json!("Different input");
    assert_eq!(
        call(root, &args, Some(altered))["error"]["code"],
        "TELOS_REQUEST_ID_CONFLICT"
    );
    assert_eq!(
        ok(root, &["change", "diff", &change], None)["ops"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn reconciled_task_can_be_reworked_without_erasing_its_first_receipt() {
    let root = repo();
    let root = root.path();
    ok(root, &["init"], None);
    let id = draft(root, definition());
    approve(root, &id);
    let first = start(root, &id);
    fs::write(root.join("README.md"), "First draft").unwrap();
    ok(root, &["change", "reconcile", &first], None);
    let second = start(root, &id);
    assert_ne!(first, second);
    fs::write(root.join("README.md"), "Reviewed instructions").unwrap();
    ok(root, &["change", "reconcile", &second], None);
    finish_plan(root, &id);
    let history = ok(root, &["history", "README.md"], None);
    assert_eq!(history["history"].as_array().unwrap().len(), 2);
    assert_eq!(history["history"][0]["id"], first);
    assert_eq!(history["history"][1]["id"], second);
}

#[test]
fn runtime_files_cannot_bypass_governance_by_becoming_tracked() {
    let root = repo();
    ok(root.path(), &["init"], None);
    fs::write(
        root.path().join("telos/.runtime/hidden-code.rs"),
        "fn hidden() {}\n",
    )
    .unwrap();
    common::git(
        root.path(),
        &["add", "--force", "telos/.runtime/hidden-code.rs"],
    );
    let error = call(root.path(), &["check", "--planned"], None);
    assert_eq!(error["error"]["code"], "TELOS_UNPLANNED_CHANGE");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("must not be tracked")
    );
    common::git(
        root.path(),
        &["commit", "-m", "Attempt to track runtime content"],
    );
    assert!(telos_core::inventory::at_commit(root.path(), "HEAD").is_err());
}
