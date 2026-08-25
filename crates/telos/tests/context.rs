//! End-to-end coverage for the bounded `telos pack` work pack.

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

#[test]
fn pack_replaces_context_without_a_legacy_alias() {
    let tmp = with_fixture();
    let pack = telos(tmp.path(), &["pack", "INT-0042", "--json"])
        .output()
        .unwrap();
    assert!(pack.status.success(), "pack failed: {pack:?}");
    let envelope = json_stdout(&pack);
    assert_eq!(envelope["command"], json!("pack"));
    assert_eq!(
        envelope["result"]["owner"],
        json!({"context": "billing", "capability": "settlement"})
    );
    assert_eq!(
        envelope["result"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["billing/Invoice", "billing/PaymentReceived"]
    );
    assert_eq!(
        envelope["result"]["constraints"][0]["scope"],
        json!("context")
    );

    let legacy = telos(tmp.path(), &["context", "INT-0042", "--json"])
        .output()
        .unwrap();
    assert_eq!(legacy.status.code(), Some(2));
}

#[test]
fn map_prints_and_stages_the_complete_context_map() {
    let tmp = with_fixture();
    let shown = telos(tmp.path(), &["map", "--json"]).output().unwrap();
    assert_eq!(json_stdout(&shown)["result"]["dependencies"], json!([]));

    open_change(tmp.path());
    let staged = telos(tmp.path(), &["map", "--change", "CHG-0001", "--json"])
        .write_stdin("context-map {\n}\n")
        .output()
        .unwrap();
    assert!(staged.status.success(), "map staging failed: {staged:?}");
    assert_eq!(
        json_stdout(&staged)["result"]["claims"],
        json!(["telos/context-map.tel"])
    );
}

#[test]
fn pack_includes_only_required_mappings_without_supplier_internals() {
    let tmp = with_fixture_mut(|root| {
        let capability = root.join("telos/contexts/terminal/capabilities/portrait");
        fs::create_dir_all(capability.join("notions")).unwrap();
        fs::create_dir_all(capability.join("intents")).unwrap();
        fs::create_dir_all(root.join("telos/contexts/terminal/notions")).unwrap();
        fs::write(
            root.join("telos/contexts/terminal/context.tel"),
            "context terminal supporting \"Terminal\" {\n  def \"Presents billing state.\"\n}\n",
        )
        .unwrap();
        fs::write(
            capability.join("capability.tel"),
            "capability terminal/portrait \"Portrait\" {\n  def \"Renders a compact view.\"\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("telos/contexts/terminal/notions/PetView.tel"),
            "notion terminal/PetView entity {\n  def \"A local projection of an invoice.\"\n  phrase \"pet view\"\n}\n",
        )
        .unwrap();
        fs::write(
            capability.join("notions/RenderRequested.tel"),
            "notion terminal/portrait/RenderRequested event {\n  def \"A render request.\"\n  phrase \"render requested\"\n}\n",
        )
        .unwrap();
        fs::write(
            capability.join("intents/INT-0099.tel"),
            r#"intent INT-0099 in terminal/portrait "Render the local view" {
  status draft
  telos  "The terminal renders its own projection."
  statement event-driven {
    when   RenderRequested on PetView
    system shall "render the local view"
  }
}
"#,
        )
        .unwrap();
        fs::write(
            root.join("telos/context-map.tel"),
            "context-map {\n  dependency terminal on billing {\n    map billing/Invoice -> terminal/PetView\n  }\n}\n",
        )
        .unwrap();
    });

    let out = telos(tmp.path(), &["pack", "INT-0099", "--json"])
        .output()
        .unwrap();
    let pack = json_stdout(&out)["result"].clone();
    assert_eq!(
        pack["mappings"],
        json!([{"from": "billing/Invoice", "to": "terminal/PetView"}])
    );
    assert_eq!(
        pack["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["terminal/PetView", "terminal/RenderRequested"]
    );
    assert!(!pack.to_string().contains("A bill issued to a Customer"));
}

fn pack_result(dir: &std::path::Path, target: &str) -> serde_json::Value {
    let out = telos(dir, &["pack", target, "--json"]).output().unwrap();
    assert!(out.status.success(), "pack {target} failed: {out:?}");
    json_stdout(&out)["result"].clone()
}

fn bounded_fixture() -> tempfile::TempDir {
    with_fixture_mut(|root| {
        fs::write(
            root.join("telos/constraints/CON-0004.tel"),
            r#"constraint CON-0004 in project quality "Payment feedback is prompt" {
  rule  "Payment feedback is prompt."
}
"#,
        )
        .unwrap();
        fs::write(
            root.join("telos/contexts/billing/capabilities/settlement/intents/INT-0099.tel"),
            r#"intent INT-0099 in billing/settlement "An unrelated draft" {
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
    })
}

fn expected_int_0042_pack() -> serde_json::Value {
    json!({
        "id": "INT-0042",
        "owner": {"context": "billing", "capability": "settlement"},
        "change": null,
        "canonical": "intent INT-0042 in billing/settlement \"Invoice payment marks it settled\" {\n  status active\n  telos  \"Customers must see immediately that their debt is cleared.\"\n  statement event-driven {\n    when   PaymentReceived on Invoice\n    system shall set Invoice.state = settled\n  }\n  requires INT-0017\n\n  scenario SCN-0107 \"full payment settles the invoice\" {\n    given Invoice { state: open, balance: \"120.00 EUR\" }\n    when  PaymentReceived { amount: \"120.00 EUR\" }\n    then  Invoice.state == settled\n  }\n}\n",
        "scenarios": [{
            "id": "SCN-0107",
            "title": "full payment settles the invoice",
            "proved": true,
        }],
        "notions": [
            {
                "name": "billing/Invoice",
                "canonical": "notion billing/Invoice entity {\n  def    \"A bill issued to a Customer for delivered work.\"\n  phrase \"invoice\"\n  attr   state   enum(open, settled, cancelled)\n  attr   balance money\n  rel    issued-to -> Customer\n}\n",
            },
            {
                "name": "billing/PaymentReceived",
                "canonical": "notion billing/settlement/PaymentReceived event {\n  def    \"A payment arrived for an invoice.\"\n  phrase \"payment is received\"\n  attr   amount money\n}\n",
            },
        ],
        "constraints": [
            {
                "id": "CON-0003",
                "scope": "context",
                "canonical": "constraint CON-0003 in context billing architecture \"Hexagonal boundaries\" {\n  rule  \"Domain code must not import adapter modules.\"\n  check \"git --version\"\n}\n",
            },
            {
                "id": "CON-0004",
                "scope": "project",
                "canonical": "constraint CON-0004 in project quality \"Payment feedback is prompt\" {\n  rule  \"Payment feedback is prompt.\"\n}\n",
            },
        ],
        "bindings": {
            "implements": ["src/billing/invoice.rs"],
            "proves": [{
                "scenario": "SCN-0107",
                "test": "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
            }],
        },
        "mappings": [],
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
                "owner": "billing/settlement",
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

fn open_change_with_id(dir: &std::path::Path, motivation: &str) -> String {
    let out = telos(dir, &["change", "open", motivation, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "opening a change failed: {out:?}");
    json_stdout(&out)["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn seed_draft_intent(dir: &std::path::Path) -> String {
    let change = open_change_with_id(dir, "Seed a removable draft");
    let out = telos(dir, &["add", "intent", "--change", &change, "--json"])
        .write_stdin(
            json!({
                "owner": "billing/settlement",
                "title": "A removable draft",
                "status": "draft",
                "telos": "Exercise removal from a final overlay.",
                "statement": {"template": "ubiquitous", "action": "record the draft"},
                "refines": [],
                "requires": [],
                "excludes": [],
                "scenarios": [],
            })
            .to_string(),
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "seeding the draft failed: {out:?}");
    let id = json_stdout(&out)["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = telos(dir, &["change", "approve", &change, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "approving the seed failed: {out:?}");
    let out = telos(dir, &["change", "reconcile", &change, "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "reconciling the seed failed: {out:?}");
    id
}

fn assert_unknown_pack(dir: &std::path::Path, target: &str, kind: &str, hint: &str) {
    let out = telos(dir, &["pack", target, "--json"]).output().unwrap();
    assert!(
        !out.status.success(),
        "pack {target} unexpectedly succeeded: {out:?}"
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["command"], json!("pack"));
    assert_eq!(envelope["result"], serde_json::Value::Null);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!(format!("unknown {kind} `{target}`"))
    );
    assert_eq!(envelope["error"]["hint"], json!(hint));
}

#[test]
fn rejects_non_intent_and_non_scenario_targets() {
    let tmp = with_fixture();

    for target in ["NOT:billing/Invoice", "CON-0003", "CHG-0001"] {
        let out = telos(tmp.path(), &["pack", target, "--json"])
            .output()
            .unwrap();

        assert!(
            !out.status.success(),
            "pack {target} unexpectedly succeeded"
        );
        assert_eq!(
            json_stdout(&out)["error"],
            json!({
                "code": "TELOS_REFERENCE_UNKNOWN",
                "message": "`pack` applies to intents and scenarios",
                "hint": null,
            }),
            "unexpected error for {target}",
        );
    }
}

#[test]
fn pack_for_an_intent_is_the_exact_bounded_pack() {
    let tmp = bounded_fixture();
    let pack = pack_result(tmp.path(), "INT-0042");

    assert_eq!(pack, expected_int_0042_pack());
    assert_eq!(pack["notions"][0]["name"], json!("billing/Invoice"));
    assert_eq!(pack["notions"][1]["name"], json!("billing/PaymentReceived"));
    assert_eq!(pack["constraints"].as_array().unwrap().len(), 2);
    assert_eq!(pack["neighbors"].as_array().unwrap().len(), 1);
}

#[test]
fn pack_for_a_scenario_resolves_to_its_owning_intent_pack() {
    let tmp = bounded_fixture();

    assert_eq!(
        pack_result(tmp.path(), "SCN-0107"),
        expected_int_0042_pack()
    );
}

#[test]
fn pack_reads_an_edited_intent_from_its_owning_post_overlay_model() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_edit_int_0042(
        tmp.path(),
        r#"{"telos":"The edited rationale belongs to the staged model."}"#,
    );

    let pack = pack_result(tmp.path(), "INT-0042");

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
fn pack_reads_an_added_intent_from_its_owning_post_overlay_model() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let id = stage_new_intent(tmp.path());

    let pack = pack_result(tmp.path(), &id);

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
fn pack_rejects_an_intent_removed_after_an_edit_in_its_owning_change() {
    let tmp = with_fixture();
    let intent = seed_draft_intent(tmp.path());
    let change = open_change_with_id(tmp.path(), "Edit then remove the draft");

    let out = telos(
        tmp.path(),
        &["edit", "intent", &intent, "--change", &change, "--json"],
    )
    .write_stdin(r#"{"telos":"This edit is superseded by removal."}"#)
    .output()
    .unwrap();
    assert!(out.status.success(), "editing the draft failed: {out:?}");
    let out = telos(
        tmp.path(),
        &["remove", "intent", &intent, "--change", &change, "--json"],
    )
    .output()
    .unwrap();
    assert!(out.status.success(), "removing the draft failed: {out:?}");

    assert_unknown_pack(tmp.path(), &intent, "intent", "closest is INT-0042");
}

#[test]
fn pack_rejects_a_scenario_removed_from_an_edited_intent() {
    let tmp = with_fixture_mut(|root| {
        let path = root.join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
        let source = fs::read_to_string(&path).unwrap();
        fs::write(path, source.replace("status active", "status draft")).unwrap();
    });
    open_change(tmp.path());
    let out = telos(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
    )
    .write_stdin(r#"{"status":"draft","scenarios":[]}"#)
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "removing the scenario failed: {out:?}"
    );

    assert_unknown_pack(tmp.path(), "SCN-0091", "scenario", "closest is SCN-0107");
}

#[test]
fn pack_folds_journalled_bindings_and_sorts_them_by_path() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_edit_int_0042(tmp.path(), "{}");
    approve(tmp.path());

    for path in ["src/billing/z_context.rs", "src/billing/a_context.rs"] {
        fs::write(tmp.path().join(path), "// context implementation\n").unwrap();
        bind(tmp.path(), path);
    }

    assert_eq!(
        pack_result(tmp.path(), "INT-0042")["bindings"]["implements"],
        json!([
            "src/billing/a_context.rs",
            "src/billing/invoice.rs",
            "src/billing/z_context.rs",
        ])
    );
}

#[test]
fn pack_folds_a_green_test_witness_into_proves_and_scenario_state() {
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

    let pack = pack_result(tmp.path(), "INT-0042");
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
