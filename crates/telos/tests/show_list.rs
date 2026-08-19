//! End-to-end tests for `telos show <id|Name>` and `telos list <type>`:
//! canonical-block reuse, the relations section, id/notion resolution
//! (including the numeric-distance and edit-distance suggestion
//! algorithms), and the sorted `list` output. Every test runs the real
//! binary against the sealed `billing` corpus fixture.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use common::{telos, with_fixture};

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The `billing` corpus root -- the same fixture `with_fixture()` copies
/// into each throwaway repository, read directly here to get the exact
/// canonical bytes a golden-block test compares against.
fn corpus_file(rel: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../telos-core/tests/corpus/billing")
        .join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// --- show: human mode ------------------------------------------------------

/// `show INT-0042`'s stdout contains the intent's canonical block verbatim
/// -- byte for byte what `INT-0042.tel` already holds, since the corpus is
/// itself canonical.
#[test]
fn show_intent_human_contains_the_exact_canonical_block() {
    let tmp = with_fixture();
    let canonical = corpus_file("telos/intents/INT-0042.tel");

    let out = telos(tmp.path(), &["show", "INT-0042"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&canonical),
        "stdout did not contain the canonical block verbatim:\n{stdout}"
    );
}

// --- show: JSON mode ---------------------------------------------------

/// `show Invoice --json`: the entity is the notion struct (an `entity`
/// notion has `kind == "entity"`), and `INT-0042` -- which uses `Invoice`
/// in its statement -- shows up among the incoming `uses` edges.
#[test]
fn show_notion_json_reports_the_entity_and_its_incoming_uses_edge() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["show", "Invoice", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["entity"]["kind"], json!("entity"));
    let incoming = envelope["result"]["relations"]["in"]
        .as_array()
        .expect("relations.in is an array");
    assert!(
        incoming.contains(&json!({ "rel": "uses", "from": "INT-0042" })),
        "relations.in: {incoming:?}"
    );
}

/// `show SCN-0107 --json`: the entity is the scenario struct, and the
/// canonical block is the *owning* intent's, not the scenario's own (a
/// scenario has none).
#[test]
fn show_scenario_json_reports_the_scenario_and_the_owning_intents_canonical_block() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["show", "SCN-0107", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["entity"]["id"], json!("SCN-0107"));
    let canonical = envelope["result"]["canonical"].as_str().unwrap();
    assert!(
        canonical.contains("intent INT-0042"),
        "canonical: {canonical}"
    );
}

/// `show`'s header line for a scenario names the scenario and the intent
/// that owns it.
#[test]
fn show_scenario_human_header_names_scenario_and_owning_intent() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["show", "SCN-0107"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("scenario SCN-0107 belongs to intent INT-0042:"),
        "stdout: {stdout}"
    );
}

// --- show: unresolved references ----------------------------------------

/// An intent id absent from the spec resolves to nothing; the hint
/// suggests the nearest *existing* intent id by numeric distance
/// (`|9999-42| < |9999-17|`), not by edit distance on the rendered string.
#[test]
fn show_unknown_intent_reports_the_numerically_closest_id() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["show", "INT-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let hint = envelope["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("INT-0042"), "hint: {hint}");
}

/// An argument that is neither a typed id nor a valid (PascalCase) notion
/// name is `show`'s own diagnosis, not the notion grammar's parse error.
#[test]
fn show_an_unparseable_argument_reports_telos_reference_unknown() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["show", "foo-bar", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("cannot parse `foo-bar`"),
        "message: {message}"
    );
    assert_eq!(envelope["error"]["hint"], Value::Null);
}

// --- list: JSON mode -----------------------------------------------------

/// `list intent --json` is sorted by id, ascending.
#[test]
fn list_intent_json_is_sorted_by_id() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["list", "intent", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    let ids: Vec<&str> = envelope["result"]["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["INT-0017", "INT-0042"]);
}

/// `list scenario --json`'s items name the scenario, its title, and the
/// intent that owns it -- sorted by scenario id.
#[test]
fn list_scenario_json_reports_id_title_and_owning_intent() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["list", "scenario", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["result"]["items"],
        json!([
            {
                "id": "SCN-0091",
                "title": "a newly issued invoice is open",
                "intent": "INT-0017"
            },
            {
                "id": "SCN-0107",
                "title": "full payment settles the invoice",
                "intent": "INT-0042"
            },
        ])
    );
}

/// `list notion --json` holds all four corpus notions, sorted
/// alphabetically by name.
#[test]
fn list_notion_json_is_sorted_alphabetically() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["list", "notion", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["result"]["items"],
        json!([
            {
                "name": "Customer",
                "kind": "entity",
                "def": "A person or company that receives invoices."
            },
            {
                "name": "Invoice",
                "kind": "entity",
                "def": "A bill issued to a Customer for delivered work."
            },
            {
                "name": "InvoiceIssued",
                "kind": "event",
                "def": "An invoice was issued to a customer."
            },
            {
                "name": "PaymentReceived",
                "kind": "event",
                "def": "A payment arrived for an invoice."
            },
        ])
    );
}

// --- list: usage errors ----------------------------------------------------

/// `list widget` names no real entity type: clap rejects it before any
/// command runs, exiting 2.
#[test]
fn list_of_an_unknown_type_is_a_clap_usage_error() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["list", "widget"]).output().unwrap();

    assert_eq!(out.status.code(), Some(2), "a usage error exits 2");
}
