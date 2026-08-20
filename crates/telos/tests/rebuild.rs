//! End-to-end contracts for deterministic reconstruction planning and progress.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use common::{telos, unsealed_fixture, with_fixture, with_fixture_mut};

const FULL_PAYMENT_TEST: &str = "tests/billing.rs::scn_0107_full_payment_settles_the_invoice";
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

fn run(dir: &Path, args: &[&str]) -> (bool, Value) {
    let out = telos(dir, args).output().unwrap();
    let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    });
    (out.status.success(), value)
}

fn success(dir: &Path, args: &[&str]) -> Value {
    let (ok, envelope) = run(dir, args);
    assert!(ok, "command failed: {envelope:#}");
    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["command"], json!("rebuild"));
    envelope
}

fn write_runner_config(root: &Path, command: &str) {
    let path = root.join("telos/telos.toml");
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("cmd = \"\""));
    let command = command.replace('\\', "\\\\").replace('"', "\\\"");
    fs::write(
        path,
        source.replace("cmd = \"\"", &format!("cmd = \"{command}\"")),
    )
    .unwrap();
}

fn context(dir: &Path, id: &str) -> Value {
    let (ok, envelope) = run(dir, &["context", id, "--json"]);
    assert!(ok, "context failed: {envelope:#}");
    envelope["result"].clone()
}

fn assert_plan_contexts_equal_public_context(dir: &Path, plan: &Value) {
    for step in plan["result"]["steps"].as_array().unwrap() {
        let intent = step["intent"].as_str().unwrap();
        assert_eq!(
            step["context"],
            context(dir, intent),
            "plan context for {intent} diverged from public context"
        );
    }
}

#[test]
fn billing_plan_has_exact_metadata_and_the_full_frozen_context_results() {
    let tmp = with_fixture();

    let envelope = success(tmp.path(), &["rebuild", "plan", "--json"]);
    let steps = envelope["result"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["n"], json!(1));
    assert_eq!(steps[0]["intent"], json!("INT-0017"));
    assert_eq!(steps[0]["requires"], json!([]));
    assert_eq!(steps[1]["n"], json!(2));
    assert_eq!(steps[1]["intent"], json!("INT-0042"));
    assert_eq!(steps[1]["requires"], json!(["INT-0017"]));
    assert_eq!(steps[0]["context"], context(tmp.path(), "INT-0017"));
    assert_eq!(steps[1]["context"], context(tmp.path(), "INT-0042"));

    assert_eq!(steps[0]["context"]["id"], json!("INT-0017"));
    assert_eq!(steps[0]["context"]["change"], Value::Null);
    assert_eq!(steps[0]["context"]["scenarios"][0]["id"], json!("SCN-0091"));
    assert_eq!(
        steps[0]["context"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Customer", "Invoice", "InvoiceIssued"]
    );
    assert_eq!(
        steps[0]["context"]["constraints"][0]["id"],
        json!("CON-0003")
    );
    assert_eq!(
        steps[0]["context"]["bindings"],
        json!({"implements": [], "proves": []})
    );
    assert_eq!(steps[0]["context"]["neighbors"][0]["id"], json!("INT-0042"));

    assert_eq!(steps[1]["context"]["id"], json!("INT-0042"));
    assert_eq!(steps[1]["context"]["change"], Value::Null);
    assert_eq!(steps[1]["context"]["scenarios"][0]["id"], json!("SCN-0107"));
    assert_eq!(
        steps[1]["context"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Invoice", "PaymentReceived"]
    );
    assert_eq!(
        steps[1]["context"]["constraints"][0]["id"],
        json!("CON-0003")
    );
    assert_eq!(
        steps[1]["context"]["bindings"],
        json!({
            "implements": ["src/billing/invoice.rs"],
            "proves": [{"scenario": "SCN-0107", "test": FULL_PAYMENT_TEST}],
        })
    );
    assert_eq!(steps[1]["context"]["neighbors"][0]["id"], json!("INT-0017"));

    let out = telos(tmp.path(), &["rebuild", "plan"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "1. INT-0017\n2. INT-0042\n"
    );
}

fn progress_fixture() -> tempfile::TempDir {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green-{filter}");
    fs::write(
        tmp.path().join("tests/billing.rs"),
        "fn scn_0091() {}\nfn scn_0107_full_payment_settles_the_invoice() {}\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        format!(
            "proves     \"tests/billing.rs::scn_0091\" -> SCN-0091\n\
             proves     \"{FULL_PAYMENT_TEST}\" -> SCN-0107\n"
        ),
    )
    .unwrap();
    fs::write(tmp.path().join(".green-scn_0091"), "green\n").unwrap();
    tmp
}

#[test]
fn status_runs_each_proof_and_reports_exact_rows_and_totals() {
    let tmp = progress_fixture();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);
    assert_eq!(
        envelope["result"],
        json!({
            "scenarios_green": 1,
            "scenarios_total": 2,
            "scenarios": [
                {"id":"SCN-0091","green":true,"tests":[{
                    "test":"tests/billing.rs::scn_0091",
                    "green":true,
                    "command":"git hash-object .green-scn_0091"
                }]},
                {"id":"SCN-0107","green":false,"tests":[{
                    "test":FULL_PAYMENT_TEST,
                    "green":false,
                    "command":"git hash-object .green-scn_0107_full_payment_settles_the_invoice"
                }]}
            ]
        })
    );

    fs::write(
        tmp.path()
            .join(".green-scn_0107_full_payment_settles_the_invoice"),
        "green\n",
    )
    .unwrap();
    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);
    assert_eq!(envelope["result"]["scenarios_green"], json!(2));
    assert_eq!(envelope["result"]["scenarios"][1]["green"], json!(true));

    let out = telos(tmp.path(), &["rebuild", "status"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "scenarios: 2/2 green\n"
    );
}

#[test]
fn multiple_proofs_are_sorted_deduplicated_and_conjunctive() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green-{filter}");
    fs::write(tmp.path().join("tests/proof_a.rs"), "fn scn_0091_a() {}\n").unwrap();
    fs::write(tmp.path().join("tests/proof_b.rs"), "fn scn_0091_b() {}\n").unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        "proves \"tests/proof_b.rs::scn_0091_b\" -> SCN-0091\n\
         proves \"tests/proof_a.rs::scn_0091_a\" -> SCN-0091\n\
         proves \"tests/proof_a.rs::scn_0091_a\" -> SCN-0091\n",
    )
    .unwrap();
    fs::write(tmp.path().join(".green-scn_0091_a"), "green\n").unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);
    assert_eq!(
        envelope["result"]["scenarios"][0],
        json!({
            "id": "SCN-0091",
            "green": false,
            "tests": [
                {"test":"tests/proof_a.rs::scn_0091_a","green":true,
                 "command":"git hash-object .green-scn_0091_a"},
                {"test":"tests/proof_b.rs::scn_0091_b","green":false,
                 "command":"git hash-object .green-scn_0091_b"}
            ]
        })
    );
}

#[test]
fn a_stale_bound_name_is_red_even_when_the_runner_would_exit_zero() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green");
    fs::write(tmp.path().join(".green"), "green\n").unwrap();
    fs::write(
        tmp.path().join("tests/billing.rs"),
        "fn scn_0091_actual_name() {}\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        "proves \"tests/billing.rs::scn_0091_stale_name\" -> SCN-0091\n",
    )
    .unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);

    assert_eq!(
        envelope["result"]["scenarios"][0],
        json!({
            "id":"SCN-0091", "green":false,
            "tests":[{
                "test":"tests/billing.rs::scn_0091_stale_name",
                "green":false,
                "command":"git hash-object .green"
            }]
        })
    );
}

#[test]
fn proof_targets_sort_structurally_by_path_then_optional_name() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green");
    fs::write(tmp.path().join(".green"), "green\n").unwrap();
    fs::write(tmp.path().join("tests/a"), "fn scn_0091_z() {}\n").unwrap();
    fs::write(tmp.path().join("tests/a-"), "whole-file proof\n").unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        "proves \"tests/a-\" -> SCN-0091\n\
         proves \"tests/a::scn_0091_z\" -> SCN-0091\n",
    )
    .unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);
    let tests = envelope["result"]["scenarios"][0]["tests"]
        .as_array()
        .unwrap();

    assert_eq!(tests[0]["test"], json!("tests/a::scn_0091_z"));
    assert_eq!(tests[1]["test"], json!("tests/a-"));
}

#[test]
fn leading_runner_whitespace_is_preserved_in_the_displayed_command() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "  git hash-object .green-{filter}  ");
    fs::write(tmp.path().join(".green-scn_0091"), "green\n").unwrap();
    fs::write(tmp.path().join("tests/billing.rs"), "fn scn_0091() {}\n").unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        "proves \"tests/billing.rs::scn_0091\" -> SCN-0091\n",
    )
    .unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);

    assert_eq!(
        envelope["result"]["scenarios"][0]["tests"][0]["command"],
        json!("  git hash-object .green-scn_0091")
    );
}

#[test]
fn a_legitimate_path_with_spaces_and_shell_metacharacters_is_one_safe_filter_argument() {
    let tmp = unsealed_fixture();
    let (test_path, runner, displayed) = if cfg!(windows) {
        (
            "tests/proof&mkdir injected",
            "dir \"{filter}\"",
            "dir \"tests/proof&mkdir injected\"",
        )
    } else {
        (
            "tests/proof;mkdir injected",
            "test -f \"{filter}\"",
            "test -f \"tests/proof;mkdir injected\"",
        )
    };
    write_runner_config(tmp.path(), runner);
    fs::write(tmp.path().join(test_path), "whole-file proof\n").unwrap();
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        format!("proves \"{test_path}\" -> SCN-0091\n"),
    )
    .unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);

    assert_eq!(envelope["result"]["scenarios"][0]["green"], json!(true));
    assert_eq!(
        envelope["result"]["scenarios"][0]["tests"][0]["command"],
        json!(displayed)
    );
    assert!(
        !tmp.path().join("injected").exists(),
        "the filter was interpreted as shell syntax"
    );
}

#[test]
fn unsafe_or_escaping_proof_paths_are_red_and_never_run() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "mkdir invalid-proof-ran");
    let mut bindings = "proves \"../outside.rs\" -> SCN-0091\n\
                        proves \"/tmp/outside.rs\" -> SCN-0091\n\
                        proves \"telos/telos.toml\" -> SCN-0091\n"
        .to_string();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("../telos/telos.toml", tmp.path().join("tests/telos-link"))
            .unwrap();
        bindings.push_str("proves \"tests/telos-link\" -> SCN-0091\n");
    }
    fs::write(tmp.path().join("telos/bindings.tel"), bindings).unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);

    assert_eq!(envelope["result"]["scenarios"][0]["green"], json!(false));
    assert!(
        envelope["result"]["scenarios"][0]["tests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|test| test["green"] == json!(false))
    );
    assert!(!tmp.path().join("invalid-proof-ran").exists());
}

#[test]
fn absent_proofs_and_missing_test_files_are_red_rows_not_command_errors() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green-{filter}");
    fs::write(
        tmp.path().join("telos/bindings.tel"),
        "proves \"tests/missing.rs::scn_0107\" -> SCN-0107\n",
    )
    .unwrap();

    let envelope = success(tmp.path(), &["rebuild", "status", "--json"]);
    assert_eq!(envelope["result"]["scenarios_green"], json!(0));
    assert_eq!(
        envelope["result"]["scenarios"][0],
        json!({"id":"SCN-0091","green":false,"tests":[]})
    );
    assert_eq!(
        envelope["result"]["scenarios"][1],
        json!({"id":"SCN-0107","green":false,"tests":[{
            "test":"tests/missing.rs::scn_0107","green":false,
            "command":"git hash-object .green-scn_0107"
        }]})
    );
}

#[test]
fn missing_or_blank_test_command_is_test_not_found() {
    for configured in [None, Some("   ")] {
        let tmp = unsealed_fixture();
        if let Some(command) = configured {
            write_runner_config(tmp.path(), command);
        }
        let (ok, envelope) = run(tmp.path(), &["rebuild", "status", "--json"]);
        assert!(!ok);
        assert_eq!(envelope["command"], json!("rebuild"));
        assert_eq!(envelope["error"]["code"], json!("TELOS_TEST_NOT_FOUND"));
        assert_eq!(
            envelope["error"]["message"],
            json!("no `[test] cmd` is configured in telos/telos.toml")
        );
    }
}

fn stage_draft(dir: &Path, motivation: &str) -> (String, String) {
    let (ok, opened) = run(dir, &["change", "open", motivation, "--json"]);
    assert!(ok, "open failed: {opened:#}");
    let change = opened["result"]["id"].as_str().unwrap().to_string();
    let out = telos(dir, &["add", "intent", "--change", &change, "--json"])
        .write_stdin(
            json!({
                "title": format!("Draft for {motivation}"),
                "status": "draft",
                "telos": format!("Purpose for {motivation}"),
                "statement": {"template":"ubiquitous","action":"record it"},
                "scenarios": []
            })
            .to_string(),
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "add failed: {out:?}");
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    (
        change,
        envelope["result"]["id"].as_str().unwrap().to_string(),
    )
}

#[test]
fn changing_plan_and_status_include_an_added_intent_and_its_green_journal_proof() {
    let tmp = with_fixture_mut(|root| {
        fs::write(root.join(".green"), "green\n").unwrap();
        write_runner_config(root, "git hash-object .green");
    });
    let (ok, opened) = run(
        tmp.path(),
        &["change", "open", "Reconstruct refunds", "--json"],
    );
    assert!(ok, "open failed: {opened:#}");
    let out = telos(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
    )
    .write_stdin(
        json!({
            "title": "Refunds can reopen invoices",
            "status": "active",
            "telos": "Refunded invoices need renewed settlement.",
            "statement": {
                "template": "event-driven", "when": "PaymentReceived", "on": "Invoice",
                "action": "set Invoice.state = open"
            },
            "requires": ["INT-0042"],
            "scenarios": [{
                "title": "a refund reopens the invoice",
                "given": [{"notion":"Invoice","fields":{"state":"settled","balance":"120.00 EUR"}}],
                "when": {"notion":"PaymentReceived","fields":{"amount":"120.00 EUR"}},
                "then": ["Invoice.state == open"]
            }]
        })
        .to_string(),
    )
    .output()
    .unwrap();
    assert!(out.status.success(), "add failed: {out:?}");
    let added: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(added["result"]["id"], json!("INT-0043"));
    assert_eq!(added["result"]["scenario_ids"], json!(["SCN-0108"]));

    let (ok, approved) = run(tmp.path(), &["change", "approve", "CHG-0001", "--json"]);
    assert!(ok, "approve failed: {approved:#}");
    let test_path = tmp.path().join("tests/billing.rs");
    let mut source = fs::read_to_string(&test_path).unwrap();
    source.push_str("\nfn scn_0108_rebuild_refund() {}\n");
    fs::write(test_path, source).unwrap();
    let (ok, witnessed) = run(tmp.path(), &["test", "SCN-0108", "--json"]);
    assert!(ok, "test failed: {witnessed:#}");
    assert_eq!(witnessed["result"]["witness"], json!("green"));

    let plan = success(tmp.path(), &["rebuild", "plan", "--json"]);
    assert_plan_contexts_equal_public_context(tmp.path(), &plan);
    let added_step = &plan["result"]["steps"][2];
    assert_eq!(added_step["intent"], json!("INT-0043"));
    assert_eq!(added_step["requires"], json!(["INT-0042"]));
    assert_eq!(added_step["context"]["change"], json!("CHG-0001"));
    assert_eq!(added_step["context"]["scenarios"][0]["proved"], json!(true));
    assert_eq!(
        added_step["context"]["bindings"]["proves"],
        json!([{
            "scenario":"SCN-0108",
            "test":"tests/billing.rs::scn_0108_rebuild_refund"
        }])
    );

    let status = success(tmp.path(), &["rebuild", "status", "--json"]);
    let row = status["result"]["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!("SCN-0108"))
        .unwrap();
    assert_eq!(
        row,
        &json!({
            "id":"SCN-0108", "green":true,
            "tests":[{
                "test":"tests/billing.rs::scn_0108_rebuild_refund",
                "green":true,
                "command":"git hash-object .green"
            }]
        })
    );
}

#[test]
fn changing_plan_folds_all_changes_in_id_order_and_marks_each_intents_owner() {
    let tmp = with_fixture();
    let (first_change, first_intent) = stage_draft(tmp.path(), "First");
    let (second_change, second_intent) = stage_draft(tmp.path(), "Second");

    let envelope = success(tmp.path(), &["rebuild", "plan", "--json"]);
    let steps = envelope["result"]["steps"].as_array().unwrap();
    assert_eq!(
        steps
            .iter()
            .map(|step| step["intent"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "INT-0017",
            "INT-0042",
            first_intent.as_str(),
            second_intent.as_str()
        ]
    );
    assert_eq!(steps[2]["context"]["change"], json!(first_change));
    assert_eq!(steps[3]["context"]["change"], json!(second_change));
    assert_plan_contexts_equal_public_context(tmp.path(), &envelope);
}

fn stage_title_edit(dir: &Path, intent: &str, title: &str, motivation: &str) -> String {
    let (ok, opened) = run(dir, &["change", "open", motivation, "--json"]);
    assert!(ok, "open failed: {opened:#}");
    let change = opened["result"]["id"].as_str().unwrap().to_string();
    let out = telos(
        dir,
        &["edit", "intent", intent, "--change", &change, "--json"],
    )
    .write_stdin(json!({"title": title}).to_string())
    .output()
    .unwrap();
    assert!(out.status.success(), "edit failed: {out:?}");
    change
}

#[test]
fn multi_change_plan_uses_each_public_owner_overlay_pack() {
    let tmp = with_fixture();
    stage_title_edit(
        tmp.path(),
        "INT-0017",
        "Issuing is reconstructed",
        "First edit",
    );
    stage_title_edit(
        tmp.path(),
        "INT-0042",
        "Payment is reconstructed",
        "Second edit",
    );

    let plan = success(tmp.path(), &["rebuild", "plan", "--json"]);

    assert_plan_contexts_equal_public_context(tmp.path(), &plan);
    assert_eq!(
        plan["result"]["steps"][0]["context"]["change"],
        json!("CHG-0001")
    );
    assert_eq!(
        plan["result"]["steps"][1]["context"]["change"],
        json!("CHG-0002")
    );
}

fn corrupt_change(dir: &Path, id: &str, marker: &str) {
    fs::write(
        dir.join(format!("telos/changes/{id}.tel")),
        format!("change {id} \"{marker}\" {{\n  this is not valid\n}}\n"),
    )
    .unwrap();
}

fn assert_parse_error_from(dir: &Path, subcommand: &str, id: &str) {
    let (ok, envelope) = run(dir, &["rebuild", subcommand, "--json"]);
    assert!(!ok, "{subcommand} silently omitted invalid {id}");
    assert_eq!(envelope["error"]["code"], json!("TELOS_PARSE_ERROR"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("telos/changes/{id}.tel")),
        "parse diagnostic did not name {id}: {message}"
    );
    assert!(
        message.contains("expected"),
        "parse diagnostic lost the useful parser detail: {message}"
    );
}

#[test]
fn invalid_only_change_is_rejected_by_plan_and_status() {
    for subcommand in ["plan", "status"] {
        let tmp = with_fixture();
        let (ok, opened) = run(tmp.path(), &["change", "open", "Broken", "--json"]);
        assert!(ok, "open failed: {opened:#}");
        corrupt_change(tmp.path(), "CHG-0001", "lowest broken");

        assert_parse_error_from(tmp.path(), subcommand, "CHG-0001");
    }
}

#[test]
fn valid_and_invalid_changes_report_the_lowest_invalid_id_for_both_subcommands() {
    for subcommand in ["plan", "status"] {
        let tmp = with_fixture();
        stage_draft(tmp.path(), "Valid first");
        let (ok, second) = run(tmp.path(), &["change", "open", "Broken second", "--json"]);
        assert!(ok, "second open failed: {second:#}");
        let (ok, third) = run(tmp.path(), &["change", "open", "Broken third", "--json"]);
        assert!(ok, "third open failed: {third:#}");
        corrupt_change(tmp.path(), "CHG-0002", "lowest broken");
        corrupt_change(tmp.path(), "CHG-0003", "later broken");

        assert_parse_error_from(tmp.path(), subcommand, "CHG-0002");
    }
}

#[test]
fn conflicting_changes_fail_with_the_first_deterministic_integrity_error() {
    let tmp = with_fixture();
    let (_first_change, first_intent) = stage_draft(tmp.path(), "First");
    let (second_change, second_intent) = stage_draft(tmp.path(), "Second");
    let path = tmp
        .path()
        .join(format!("telos/changes/{second_change}.tel"));
    let source = fs::read_to_string(&path).unwrap();
    fs::write(&path, source.replace(&second_intent, &first_intent)).unwrap();

    let (ok, envelope) = run(tmp.path(), &["rebuild", "plan", "--json"]);
    assert!(!ok);
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_INTEGRITY_VIOLATION")
    );
    assert_eq!(
        envelope["error"]["message"],
        json!(format!(
            "telos/intents/{first_intent}.tel is claimed by both CHG-0001 and CHG-0002"
        ))
    );
}

#[test]
fn sealed_source_and_spec_drift_refuse_both_rebuild_subcommands() {
    for (path, suffix) in [
        ("src/billing/invoice.rs", "\n// drift\n"),
        ("telos/intents/INT-0042.tel", "\n"),
    ] {
        for subcommand in ["plan", "status"] {
            let tmp = with_fixture();
            let target = tmp.path().join(path);
            let mut source = fs::read_to_string(&target).unwrap();
            source.push_str(suffix);
            fs::write(target, source).unwrap();

            let (ok, envelope) = run(tmp.path(), &["rebuild", subcommand, "--json"]);
            assert!(!ok, "{subcommand} accepted drift of {path}");
            assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
            assert_eq!(envelope["error"]["hint"], json!(DRIFT_HINT));
        }
    }
}

fn tree_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".git" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), out);
            } else {
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    collect(root, root, &mut out);
    out
}

#[test]
fn spec_only_plan_and_status_are_read_only() {
    let tmp = unsealed_fixture();
    write_runner_config(tmp.path(), "git hash-object .green");
    fs::write(tmp.path().join(".green"), "green\n").unwrap();
    let before = tree_bytes(tmp.path());

    success(tmp.path(), &["rebuild", "plan", "--json"]);
    success(tmp.path(), &["rebuild", "status", "--json"]);

    assert_eq!(tree_bytes(tmp.path()), before);
    assert!(!tmp.path().join("telos/telos.lock").exists());
}

#[cfg(unix)]
#[test]
fn dangling_lock_is_not_mistaken_for_a_spec_only_workspace() {
    use std::os::unix::fs::symlink;

    for subcommand in ["plan", "status"] {
        let tmp = unsealed_fixture();
        symlink("missing-lock-target", tmp.path().join("telos/telos.lock")).unwrap();

        let (ok, envelope) = run(tmp.path(), &["rebuild", subcommand, "--json"]);

        assert!(!ok, "{subcommand} admitted a dangling lock as spec-only");
        assert_eq!(envelope["error"]["code"], json!("TELOS_NOT_INITIALIZED"));
        assert_eq!(envelope["error"]["message"], json!("telos.lock is missing"));
    }
}
