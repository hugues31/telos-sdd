//! End-to-end coverage for the bounded `telos context` work pack.

mod common;

use std::fs;

use serde_json::json;

use common::{telos, with_fixture, with_fixture_mut};

fn json_stdout(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not valid JSON ({error}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn context_result(dir: &std::path::Path, target: &str) -> serde_json::Value {
    let out = telos(dir, &["context", target, "--json"]).output().unwrap();
    assert!(out.status.success(), "context {target} failed: {out:?}");
    json_stdout(&out)["result"].clone()
}

fn bounded_fixture() -> tempfile::TempDir {
    with_fixture_mut(|root| {
        fs::write(
            root.join("telos/constraints/CON-0004.tel"),
            r#"constraint CON-0004 quality "Payment feedback is prompt" {
  rule  "Payment feedback is prompt."
  scope INT-0042
}
"#,
        )
        .unwrap();
        fs::write(
            root.join("telos/intents/INT-0099.tel"),
            r#"intent INT-0099 "An unrelated draft" {
  status draft
  telos  "Keep unrelated work out of this pack."
  statement event-driven {
    when   PaymentReceived on Invoice
    system shall set Invoice.state = cancelled
  }
}
"#,
        )
        .unwrap();
        fs::write(
            root.join("telos/constraints/CON-0005.tel"),
            r#"constraint CON-0005 quality "Draft-only rule" {
  rule  "Draft-only rule."
  scope INT-0099
}
"#,
        )
        .unwrap();
    })
}

fn expected_int_0042_pack() -> serde_json::Value {
    json!({
        "id": "INT-0042",
        "change": null,
        "canonical": "intent INT-0042 \"Invoice payment marks it settled\" {\n  status active\n  telos  \"Customers must see immediately that their debt is cleared.\"\n  statement event-driven {\n    when   PaymentReceived on Invoice\n    system shall set Invoice.state = settled\n  }\n  requires INT-0017\n\n  scenario SCN-0107 \"full payment settles the invoice\" {\n    given Invoice { state: open, balance: \"120.00 EUR\" }\n    when  PaymentReceived { amount: \"120.00 EUR\" }\n    then  Invoice.state == settled\n  }\n}\n",
        "scenarios": [{
            "id": "SCN-0107",
            "title": "full payment settles the invoice",
            "proved": true,
        }],
        "notions": [
            {
                "name": "Invoice",
                "canonical": "notion Invoice entity {\n  def  \"A bill issued to a Customer for delivered work.\"\n  attr state   enum(open, settled, cancelled)\n  attr balance money\n  rel  issued-to -> Customer\n}\n",
            },
            {
                "name": "PaymentReceived",
                "canonical": "notion PaymentReceived event {\n  def  \"A payment arrived for an invoice.\"\n  attr amount money\n}\n",
            },
        ],
        "constraints": [
            {
                "id": "CON-0003",
                "scope": "global",
                "canonical": "constraint CON-0003 architecture \"Hexagonal boundaries\" {\n  rule  \"Domain code must not import adapter modules.\"\n  scope global\n  check \"git --version\"\n}\n",
            },
            {
                "id": "CON-0004",
                "scope": "scoped",
                "canonical": "constraint CON-0004 quality \"Payment feedback is prompt\" {\n  rule  \"Payment feedback is prompt.\"\n  scope INT-0042\n}\n",
            },
        ],
        "bindings": {
            "implements": ["src/billing/invoice.rs"],
            "proves": [{
                "scenario": "SCN-0107",
                "test": "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
            }],
        },
        "neighbors": [{
            "id": "INT-0017",
            "title": "Issuing an invoice opens it",
            "rel": "requires",
            "direction": "out",
        }],
    })
}

fn open_change(dir: &std::path::Path) {
    let out = telos(dir, &["change", "open", "Context work", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "opening a change failed: {out:?}");
}

fn stage_edit_int_0042(dir: &std::path::Path, payload: &str) {
    let out = telos(
        dir,
        &[
            "edit", "intent", "INT-0042", "--change", "CHG-0001", "--json",
        ],
    )
    .write_stdin(payload)
    .output()
    .unwrap();
    assert!(out.status.success(), "staging the edit failed: {out:?}");
}

fn approve(dir: &std::path::Path) {
    let out = telos(dir, &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "approving the change failed: {out:?}");
}

fn bind(dir: &std::path::Path, path: &str) {
    let out = telos(dir, &["bind", path, "INT-0042", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "binding {path} failed: {out:?}");
}

fn stage_new_intent(dir: &std::path::Path) -> String {
    let out = telos(dir, &["add", "intent", "--change", "CHG-0001", "--json"])
        .write_stdin(
            json!({
                "title": "Invoices can be cancelled",
                "status": "active",
                "telos": "Customers need to void invoices raised in error.",
                "statement": {
                    "template": "event-driven",
                    "when": "PaymentReceived",
                    "on": "Invoice",
                    "action": "set Invoice.state = cancelled",
                },
                "refines": [],
                "requires": [],
                "excludes": [],
                "scenarios": [{
                    "title": "a payment cancels a disputed invoice",
                    "given": [{
                        "notion": "Invoice",
                        "fields": {"state": "open", "balance": "50.00 EUR"},
                    }],
                    "when": {"notion": "PaymentReceived", "fields": {"amount": "50.00 EUR"}},
                    "then": ["Invoice.state == cancelled"],
                }],
            })
            .to_string(),
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "staging an intent failed: {out:?}");
    json_stdout(&out)["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn fixture_with_green_runner() -> tempfile::TempDir {
    with_fixture_mut(|root| {
        fs::write(root.join(".context-green"), "green\n").unwrap();
        let config = root.join("telos/telos.toml");
        let source = fs::read_to_string(&config).unwrap();
        fs::write(
            config,
            source.replace("cmd = \"\"", "cmd = \"git hash-object .context-green\""),
        )
        .unwrap();
    })
}

fn stage_new_scenario(dir: &std::path::Path) -> String {
    let out = telos(
        dir,
        &[
            "edit", "intent", "INT-0042", "--change", "CHG-0001", "--json",
        ],
    )
    .write_stdin(
        json!({
            "scenarios": [
                {
                    "id": "SCN-0107",
                    "title": "full payment settles the invoice",
                    "given": [{
                        "notion": "Invoice",
                        "fields": {"state": "open", "balance": "120.00 EUR"},
                    }],
                    "when": {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
                    "then": ["Invoice.state == settled"],
                },
                {
                    "title": "a disputed payment cancels the invoice",
                    "given": [{
                        "notion": "Invoice",
                        "fields": {"state": "open", "balance": "50.00 EUR"},
                    }],
                    "when": {"notion": "PaymentReceived", "fields": {"amount": "50.00 EUR"}},
                    "then": ["Invoice.state == cancelled"],
                },
            ],
        })
        .to_string(),
    )
    .output()
    .unwrap();
    assert!(out.status.success(), "staging a scenario failed: {out:?}");
    json_stdout(&out)["result"]["scenario_ids"][0]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn rejects_non_intent_and_non_scenario_targets() {
    let tmp = with_fixture();

    for target in ["Invoice", "CON-0003", "CHG-0001"] {
        let out = telos(tmp.path(), &["context", target, "--json"])
            .output()
            .unwrap();

        assert!(
            !out.status.success(),
            "context {target} unexpectedly succeeded"
        );
        assert_eq!(
            json_stdout(&out)["error"],
            json!({
                "code": "TELOS_REFERENCE_UNKNOWN",
                "message": "`context` applies to intents and scenarios",
                "hint": null,
            }),
            "unexpected error for {target}",
        );
    }
}

#[test]
fn context_for_an_intent_is_the_exact_bounded_pack() {
    let tmp = bounded_fixture();
    let pack = context_result(tmp.path(), "INT-0042");

    assert_eq!(pack, expected_int_0042_pack());
    assert_eq!(pack["notions"][0]["name"], json!("Invoice"));
    assert_eq!(pack["notions"][1]["name"], json!("PaymentReceived"));
    assert_eq!(pack["constraints"].as_array().unwrap().len(), 2);
    assert_eq!(pack["neighbors"].as_array().unwrap().len(), 1);
}

#[test]
fn context_for_a_scenario_resolves_to_its_owning_intent_pack() {
    let tmp = bounded_fixture();

    assert_eq!(
        context_result(tmp.path(), "SCN-0107"),
        expected_int_0042_pack()
    );
}

#[test]
fn context_reads_an_edited_intent_from_its_owning_post_overlay_model() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_edit_int_0042(
        tmp.path(),
        r#"{"telos":"The edited rationale belongs to the staged model."}"#,
    );

    let pack = context_result(tmp.path(), "INT-0042");

    assert_eq!(pack["change"], json!("CHG-0001"));
    assert!(
        pack["canonical"]
            .as_str()
            .unwrap()
            .contains("The edited rationale belongs to the staged model."),
        "context did not read the post-overlay intent: {pack}"
    );
}

#[test]
fn context_reads_an_added_intent_from_its_owning_post_overlay_model() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let id = stage_new_intent(tmp.path());

    let pack = context_result(tmp.path(), &id);

    assert_eq!(pack["id"], json!(id));
    assert_eq!(pack["change"], json!("CHG-0001"));
    assert_eq!(pack["scenarios"][0]["proved"], json!(false));
    assert!(
        pack["canonical"]
            .as_str()
            .unwrap()
            .contains("Invoices can be cancelled"),
        "context did not read the staged intent: {pack}"
    );
}

#[test]
fn context_folds_journalled_bindings_and_sorts_them_by_path() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_edit_int_0042(tmp.path(), "{}");
    approve(tmp.path());

    for path in ["src/billing/z_context.rs", "src/billing/a_context.rs"] {
        fs::write(tmp.path().join(path), "// context implementation\n").unwrap();
        bind(tmp.path(), path);
    }

    assert_eq!(
        context_result(tmp.path(), "INT-0042")["bindings"]["implements"],
        json!([
            "src/billing/a_context.rs",
            "src/billing/invoice.rs",
            "src/billing/z_context.rs",
        ])
    );
}

#[test]
fn context_folds_a_green_test_witness_into_proves_and_scenario_state() {
    let tmp = fixture_with_green_runner();
    open_change(tmp.path());
    let scenario = stage_new_scenario(tmp.path());
    approve(tmp.path());
    fs::write(
        tmp.path().join("tests/billing.rs"),
        "fn scn_0108_context_witness() {}\n",
    )
    .unwrap();

    let out = telos(tmp.path(), &["test", &scenario, "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "recording a green witness failed: {out:?}"
    );

    let pack = context_result(tmp.path(), "INT-0042");
    assert_eq!(pack["change"], json!("CHG-0001"));
    assert_eq!(
        pack["scenarios"][1],
        json!({
            "id": scenario,
            "title": "a disputed payment cancels the invoice",
            "proved": true,
        })
    );
    assert_eq!(
        pack["bindings"]["proves"][1],
        json!({
            "scenario": scenario,
            "test": "tests/billing.rs::scn_0108_context_witness",
        })
    );
}
