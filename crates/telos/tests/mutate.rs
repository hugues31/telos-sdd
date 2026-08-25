//! End-to-end tests for `telos add|edit|remove`: the three staging verbs,
//! their frozen result shapes, the exact bytes they leave in a
//! change file, and the four gates that stand between a payload and those
//! bytes -- drift, file claims, referential deletion safety, and the full
//! semantic validation of the overlay.
//!
//! The invariant every failure case here asserts is the same one: a refused
//! mutation writes *nothing*. The change file is byte-identical to what it
//! was, and `counters.toml` has not moved -- an id burnt by a payload that
//! turned out invalid never reaches the disk.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use common::{
    canonical_payload, repo, telos, with_empty_billing_domain, with_fixture, with_fixture_mut,
};

// --- plumbing --------------------------------------------------------------

const MOTIVATION: &str = "Invoices can be settled";
const CHG_0001: &str = "telos/changes/CHG-0001.tel";
const COUNTERS: &str = "telos/changes/counters.toml";

/// The exact `TELOS_DRIFT_DETECTED` hint, frozen by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// A fresh git repository with an initialized, sealed and empty `telos/`.
fn fresh() -> tempfile::TempDir {
    with_empty_billing_domain()
}

fn open_change(dir: &Path) {
    telos(dir, &["change", "open", MOTIVATION])
        .assert()
        .success();
}

#[test]
fn stages_context_capability_and_owned_vocabulary_in_one_change() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    open_change(tmp.path());

    stage_ok(
        tmp.path(),
        &["add", "context", "--change", "CHG-0001", "--json"],
        &json!({
            "id": "billing", "kind": "core", "title": "Billing",
            "def": "Owns invoice rules."
        })
        .to_string(),
    );
    stage_ok(
        tmp.path(),
        &["add", "capability", "--change", "CHG-0001", "--json"],
        &json!({
            "owner": "billing", "id": "invoicing", "title": "Invoicing",
            "def": "Issues invoices."
        })
        .to_string(),
    );
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &json!({
            "owner": "billing", "name": "Invoice", "kind": "entity",
            "def": "A bill."
        })
        .to_string(),
    );

    let change = read(tmp.path(), CHG_0001);
    assert!(change.contains("op add context billing core \"Billing\""));
    assert!(change.contains("op add capability billing/invoicing \"Invoicing\""));
    assert!(change.contains("op add notion billing/Invoice entity"));
}

#[test]
fn move_claims_both_paths_and_makes_an_existing_approval_stale() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_ok(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "Invoices begin open and unpaid."}).to_string(),
    );
    telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .assert()
        .success();

    let out = telos(
        tmp.path(),
        &[
            "move",
            "INT-0017",
            "--to",
            "billing/settlement",
            "--change",
            "CHG-0001",
            "--json",
        ],
    )
    .output()
    .unwrap();
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(
        envelope["result"]["claims"],
        json!([
            "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel",
            "telos/contexts/billing/capabilities/settlement/intents/INT-0017.tel"
        ])
    );

    let diff = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_stdout(&diff)["result"]["stale"], json!(true));
}

/// Runs one staging command with `payload` on stdin and returns its
/// envelope, asserting only that the process ran.
fn stage(dir: &Path, args: &[&str], payload: &str) -> Value {
    let mut cmd = telos(dir, args);
    let out = cmd
        .write_stdin(canonical_payload(args, payload))
        .output()
        .unwrap();
    json_stdout(&out)
}

fn stage_ok(dir: &Path, args: &[&str], payload: &str) -> Value {
    let envelope = stage(dir, args, payload);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected success, got {envelope}"
    );
    envelope["result"].clone()
}

fn stage_err(dir: &Path, args: &[&str], payload: &str) -> Value {
    let envelope = stage(dir, args, payload);
    assert_eq!(
        envelope["ok"],
        json!(false),
        "expected a failure, got {envelope}"
    );
    envelope["error"].clone()
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

// --- the payloads -------------------------------------------------

fn customer_payload() -> String {
    json!({"owner": "billing", "name": "Customer", "kind": "actor", "def": "A party that receives invoices."})
        .to_string()
}

/// The documented `add notion` example, verbatim.
fn invoice_payload() -> String {
    json!({
        "owner": "billing", "name": "Invoice", "kind": "entity",
        "def": "A bill issued to a Customer for delivered work.",
        "attrs": [ {"name": "state", "type": "enum", "values": ["open", "settled"]},
                   {"name": "balance", "type": "money"},
                   {"name": "customer", "type": "ref", "target": "Customer"} ],
        "rels":  [ {"name": "issued-to", "target": "Customer"} ]
    })
    .to_string()
}

/// The same `Invoice`, without the `Customer` reference: what a change that
/// stages an intent about invoices needs, and nothing more.
fn plain_invoice_payload() -> String {
    json!({
        "owner": "billing", "name": "Invoice", "kind": "entity",
        "def": "A bill issued to a Customer for delivered work.",
        "attrs": [ {"name": "state", "type": "enum", "values": ["open", "settled"]},
                   {"name": "balance", "type": "money"} ]
    })
    .to_string()
}

fn payment_received_payload() -> String {
    json!({
        "owner": "billing/settlement", "name": "PaymentReceived", "kind": "event",
        "def": "A payment arrived for an invoice.",
        "attrs": [ {"name": "amount", "type": "money"} ]
    })
    .to_string()
}

/// The documented `add intent` example, with `requires` emptied: `INT-0017`
/// exists in the corpus, not in a freshly initialized project.
fn settle_intent_payload(on: &str) -> String {
    json!({
        "owner": "billing/settlement", "title": "Invoices can be settled", "status": "active",
        "telos": "Customers must see immediately that their debt is cleared.",
        "statement": { "template": "event-driven", "when": "PaymentReceived",
                       "on": on, "action": "set Invoice.state = settled" },
        "refines": [], "requires": [], "excludes": [],
        "scenarios": [
          { "title": "a full payment settles the invoice",
            "given": [ {"notion": "Invoice", "fields": {"state": "open", "balance": "120.00 EUR"}} ],
            "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
            "then":  ["Invoice.state == settled"] } ]
    })
    .to_string()
}

/// A project with one open change holding `Invoice` and `PaymentReceived`:
/// the base every `add intent` test below stages its intent on.
fn project_with_two_notions() -> tempfile::TempDir {
    let tmp = fresh();
    open_change(tmp.path());
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &plain_invoice_payload(),
    );
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &payment_received_payload(),
    );
    tmp
}

// --- add notion -------------------------------------------------------------

#[test]
fn add_notion_answers_with_the_public_result_result() {
    let tmp = fresh();
    open_change(tmp.path());

    let mut cmd = telos(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
    );
    let out = cmd.write_stdin(customer_payload()).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "add",
            "result": {
                "change": "CHG-0001",
                "entity": "notion",
                // A notion's natural key is its name, so that is
                // what `id` carries for one.
                "id": "billing/Customer",
                "scenario_ids": [],
                "claims": ["telos/contexts/billing/notions/Customer.tel"]
            },
            "error": null,
            "next_actions": ["telos change diff CHG-0001"]
        })
    );
}

/// The first staged op takes the change from `open` to `drafted`, and
/// the op block is the entity's whole canonical form -- `emit_notion`'s
/// output, shifted one level right.
#[test]
fn add_notion_writes_the_canonical_change_file_byte_for_byte() {
    let tmp = fresh();
    open_change(tmp.path());

    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &customer_payload(),
    );

    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op add notion billing/Customer actor {\n    \
             def  \"A party that receives invoices.\"\n  \
           }\n\
         }\n"
    );
}

/// Two ops, in staged order, each its own block -- and the second one is
/// The full `add notion` payload, `ref` attribute and `rel` included,
/// which only validates because the first op staged `Customer`.
#[test]
fn a_second_op_appends_a_block_and_resolves_against_the_first() {
    let tmp = fresh();
    open_change(tmp.path());
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &customer_payload(),
    );

    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &invoice_payload(),
    );

    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op add notion billing/Customer actor {\n    \
             def  \"A party that receives invoices.\"\n  \
           }\n\
         \n  \
           op add notion billing/Invoice entity {\n    \
             def  \"A bill issued to a Customer for delivered work.\"\n    \
             attr state    enum(open, settled)\n    \
             attr balance  money\n    \
             attr customer ref(Customer)\n    \
             rel  issued-to -> Customer\n  \
           }\n\
         }\n"
    );
}

/// Staging `Invoice` alone would leave `ref(Customer)` dangling: the whole
/// overlay is validated, so the op is refused and nothing is written.
#[test]
fn add_notion_is_validated_against_the_whole_overlay() {
    let tmp = fresh();
    open_change(tmp.path());
    let before = read(tmp.path(), CHG_0001);

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &invoice_payload(),
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(read(tmp.path(), CHG_0001), before);
}

#[test]
fn add_notion_refuses_a_name_the_base_already_holds() {
    let tmp = with_fixture();
    open_change(tmp.path());

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &plain_invoice_payload(),
    );

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    assert_eq!(
        error["message"],
        json!("notion `billing/Invoice` already exists")
    );
}

// --- add intent -------------------------------------------------------------

/// The ids come from the allocator, not from the payload: a fresh
/// project starts at `INT-0001` / `SCN-0001`.
#[test]
fn add_intent_allocates_the_intent_and_its_scenario_ids() {
    let tmp = project_with_two_notions();

    let result = stage_ok(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
        &settle_intent_payload("Invoice"),
    );

    assert_eq!(
        result,
        json!({
            "change": "CHG-0001",
            "entity": "intent",
            "id": "INT-0001",
            "scenario_ids": ["SCN-0001"],
            "claims": [
                "telos/contexts/billing/capabilities/settlement/intents/INT-0001.tel",
                "telos/contexts/billing/capabilities/settlement/notions/PaymentReceived.tel",
                "telos/contexts/billing/notions/Invoice.tel"
            ]
        })
    );
}

#[test]
fn add_intent_persists_the_counters_it_advanced() {
    let tmp = project_with_two_notions();
    assert_eq!(
        read(tmp.path(), COUNTERS),
        "intent = 0\nscenario = 0\nconstraint = 0\nchange = 1\n",
        "staging notions allocates nothing: notions are named, not numbered"
    );

    stage_ok(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
        &settle_intent_payload("Invoice"),
    );

    assert_eq!(
        read(tmp.path(), COUNTERS),
        "intent = 1\nscenario = 1\nconstraint = 0\nchange = 1\n"
    );
}

/// The failure that must cost nothing: an unresolvable reference is caught
/// by the overlay's semantic pass, after ids were already handed out. Both
/// the change file and `counters.toml` must be exactly what they were.
#[test]
fn an_unknown_reference_writes_neither_the_change_nor_the_counters() {
    let tmp = project_with_two_notions();
    let change_before = read(tmp.path(), CHG_0001);
    let counters_before = read(tmp.path(), COUNTERS);

    let error = stage_err(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
        &settle_intent_payload("Invoce"),
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("unknown notion `Invoce`; closest is `Invoice`"),
        "{message}"
    );
    assert_eq!(read(tmp.path(), CHG_0001), change_before);
    assert_eq!(read(tmp.path(), COUNTERS), counters_before);
}

/// End-to-end lexeme validation: a JSON payload is
/// the one way a malformed `date` can reach the model, and the semantic
/// pass is where it is caught.
#[test]
fn a_date_field_that_is_not_a_date_lexeme_is_refused() {
    let tmp = fresh();
    open_change(tmp.path());
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &json!({"owner": "billing", "name": "Booking", "kind": "entity", "def": "A reserved slot.",
                "attrs": [{"name": "due", "type": "date"}]})
        .to_string(),
    );
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &json!({"owner": "billing/settlement", "name": "BookingMade", "kind": "event", "def": "A booking was made."}).to_string(),
    );

    let error = stage_err(
        tmp.path(),
        &["add", "intent", "--change", "CHG-0001", "--json"],
        &json!({
            "owner": "billing/settlement", "title": "A booking has a due date", "status": "active",
            "telos": "A slot nobody can date is a slot nobody can keep.",
            "statement": {"template": "event-driven", "when": "BookingMade",
                          "on": "Booking", "action": "record the due date"},
            "scenarios": [
                {"title": "a booking is due",
                 "given": [{"notion": "Booking", "fields": {"due": "19-08-2026"}}],
                 "when": {"notion": "BookingMade", "fields": {}},
                 "then": ["Booking.due == 2026-08-19"]}]
        })
        .to_string(),
    );

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(
            "attribute `Booking.due` has type `date`, \
             but `19-08-2026` is not a date of the form `2026-08-19`"
        ),
        "{message}"
    );
}

// --- add constraint ---------------------------------------------------------

/// The documented `add constraint` example: the id is allocated past the
/// corpus' `CON-0003`, and the op block is the constraint's canonical form.
#[test]
fn add_constraint_allocates_its_id_past_the_corpus_floor() {
    let tmp = with_fixture();
    open_change(tmp.path());

    let result = stage_ok(
        tmp.path(),
        &["add", "constraint", "--change", "CHG-0001", "--json"],
        &json!({
            "owner": "billing", "kind": "architecture", "title": "Hexagonal boundaries",
            "rule": {"text": "Domain code must not import adapter modules."},
            "scope": "global", "check": "scripts/check-imports.sh --layer domain"
        })
        .to_string(),
    );

    assert_eq!(
        result,
        json!({
            "change": "CHG-0001",
            "entity": "constraint",
            "id": "CON-0004",
            "scenario_ids": [],
            "claims": ["telos/contexts/billing/constraints/CON-0004.tel"]
        })
    );
    assert_eq!(
        read(tmp.path(), COUNTERS),
        "intent = 42\nscenario = 107\nconstraint = 4\nchange = 1\n"
    );
    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op add constraint CON-0004 in context billing architecture \"Hexagonal boundaries\" {\n    \
             rule  \"Domain code must not import adapter modules.\"\n    \
             check \"scripts/check-imports.sh --layer domain\"\n  \
           }\n\
         }\n"
    );
}

// --- edit -------------------------------------------------------------------

/// An `edit` op carries the complete post-state, not the patch: the payload
/// names one field, the op block is the whole intent.
#[test]
fn edit_intent_stages_the_full_post_state() {
    let tmp = with_fixture();
    open_change(tmp.path());

    let result = stage_ok(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "An invoice must start its life open and unpaid -- reworded."})
            .to_string(),
    );

    assert_eq!(
        result,
        json!({
            "change": "CHG-0001",
            "entity": "intent",
            "id": "INT-0017",
            "scenario_ids": [],
            "claims": ["telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"]
        })
    );
    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op edit intent INT-0017 in billing/invoicing \"Issuing an invoice opens it\" {\n    \
             status active\n    \
             telos  \"An invoice must start its life open and unpaid -- reworded.\"\n    \
             statement event-driven {\n      \
               when   InvoiceIssued on Invoice\n      \
               system shall set Invoice.state = open\n    \
             }\n\
         \n    \
             scenario SCN-0091 \"a newly issued invoice is open\" {\n      \
               given Customer { name: \"ACME\" }\n      \
               when  InvoiceIssued {}\n      \
               then  Invoice.state == open\n    \
             }\n  \
           }\n\
         }\n"
    );
}

#[test]
fn edit_notion_stages_the_full_post_state() {
    let tmp = with_fixture();
    open_change(tmp.path());

    stage_ok(
        tmp.path(),
        &[
            "edit",
            "notion",
            "NOT:billing/Customer",
            "--change",
            "CHG-0001",
            "--json",
        ],
        &json!({"def": "Reworded."}).to_string(),
    );

    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op edit notion billing/Customer entity {\n    \
             def  \"Reworded.\"\n    \
             attr name string\n  \
           }\n\
         }\n"
    );
}

/// A notion's identity is a payload field, so an `edit` could pretend to
/// rename one -- which would claim the new name's file and quietly leave the
/// old one behind. Refused, with the two ops that really express it.
#[test]
fn edit_notion_refuses_to_rename() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let before = read(tmp.path(), CHG_0001);

    let error = stage_err(
        tmp.path(),
        &[
            "edit",
            "notion",
            "NOT:billing/Invoice",
            "--change",
            "CHG-0001",
            "--json",
        ],
        &json!({"name": "Bill"}).to_string(),
    );

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    assert_eq!(
        error["message"],
        json!("cannot rename notion `billing/Invoice` to `Bill`")
    );
    assert_eq!(
        error["hint"],
        json!("stage `remove notion billing/Invoice` and an `add` of the new one instead")
    );
    assert_eq!(read(tmp.path(), CHG_0001), before);
}

#[test]
fn edit_of_an_entity_the_base_does_not_hold_is_a_reference_error() {
    let tmp = with_fixture();
    open_change(tmp.path());

    let error = stage_err(
        tmp.path(),
        &[
            "edit", "intent", "INT-9999", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "..."}).to_string(),
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(error["message"], json!("unknown intent `INT-9999`"));
}

// --- remove -----------------------------------------------------------------

#[test]
fn remove_constraint_stages_a_single_line() {
    let tmp = with_fixture();
    open_change(tmp.path());

    let result = stage_ok(
        tmp.path(),
        &[
            "remove",
            "constraint",
            "CON-0003",
            "--change",
            "CHG-0001",
            "--json",
        ],
        "",
    );

    assert_eq!(
        result,
        json!({"change": "CHG-0001", "entity": "constraint", "id": "CON-0003"})
    );
    assert_eq!(
        read(tmp.path(), CHG_0001),
        "change CHG-0001 \"Invoices can be settled\" {\n  \
           status drafted\n\
         \n  \
           op remove constraint CON-0003 from billing\n\
         }\n"
    );
}

/// Referential deletion safety through the CLI: the referrer is named, and nothing is
/// written.
#[test]
fn remove_of_a_still_referenced_intent_names_the_referrer() {
    let tmp = with_fixture_mut(|root| {
        let path = root.join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
        let source = fs::read_to_string(&path).unwrap();
        fs::write(path, source.replace("status active", "status draft")).unwrap();
    });
    open_change(tmp.path());
    let before = read(tmp.path(), CHG_0001);

    let error = stage_err(
        tmp.path(),
        &[
            "remove", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        "",
    );

    assert_eq!(error["code"], json!("TELOS_INTEGRITY_VIOLATION"));
    assert_eq!(
        error["message"],
        json!("cannot remove intent INT-0017: INT-0042 requires it")
    );
    assert_eq!(read(tmp.path(), CHG_0001), before);
}

// --- file claims -------------------------------------------------------------

#[test]
fn a_second_change_cannot_stage_a_file_the_first_one_claims() {
    let tmp = with_fixture();
    open_change(tmp.path());
    stage_ok(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "Reworded once."}).to_string(),
    );
    telos(tmp.path(), &["change", "open", "a second change"])
        .assert()
        .success();
    let before = read(tmp.path(), "telos/changes/CHG-0002.tel");

    let error = stage_err(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0002", "--json",
        ],
        &json!({"telos": "Reworded twice."}).to_string(),
    );

    assert_eq!(error["code"], json!("TELOS_FILE_CLAIMED"));
    assert_eq!(
        error["message"],
        json!(
            "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel is already claimed by CHG-0001"
        )
    );
    assert_eq!(
        error["hint"],
        json!("reconcile or abandon CHG-0001 first, or work within it")
    );
    assert_eq!(read(tmp.path(), "telos/changes/CHG-0002.tel"), before);
}

/// The same change may stage the same file again -- a claim keeps *other*
/// changes out, it is not a lock against its owner.
#[test]
fn a_change_may_stage_the_same_file_twice() {
    let tmp = with_fixture();
    open_change(tmp.path());

    for telos_text in ["Reworded once.", "Reworded twice."] {
        stage_ok(
            tmp.path(),
            &[
                "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
            ],
            &json!({ "telos": telos_text }).to_string(),
        );
    }

    let text = read(tmp.path(), CHG_0001);
    assert_eq!(text.matches("op edit intent INT-0017").count(), 2);
    assert!(text.contains("Reworded twice."));
}

// --- drift gate --------------------------------------------------------------

#[test]
fn add_on_a_drifted_project_is_refused() {
    let tmp = with_fixture();
    open_change(tmp.path());
    let path = tmp
        .path()
        .join("telos/contexts/billing/notions/Invoice.tel");
    let mut content = fs::read_to_string(&path).unwrap();
    content.push('\n');
    fs::write(&path, content).unwrap();
    let before = read(tmp.path(), CHG_0001);

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &customer_payload(),
    );

    assert_eq!(error["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(error["hint"], json!(DRIFT_HINT));
    assert_eq!(read(tmp.path(), CHG_0001), before);
}

// --- the payload itself -----------------------------------------------------

#[test]
fn add_without_a_payload_on_stdin_is_a_parse_error() {
    let tmp = fresh();
    open_change(tmp.path());

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        "",
    );

    assert_eq!(error["code"], json!("TELOS_PARSE_ERROR"));
    assert_eq!(
        error["message"],
        json!("payload: expected a JSON object on stdin")
    );
}

#[test]
fn add_with_a_payload_that_is_not_json_is_a_parse_error() {
    let tmp = fresh();
    open_change(tmp.path());

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        "not json at all",
    );

    assert_eq!(error["code"], json!("TELOS_PARSE_ERROR"));
    assert_eq!(
        error["message"],
        json!("payload: expected a JSON object on stdin")
    );
}

#[test]
fn staging_into_a_change_the_store_does_not_hold_is_refused() {
    let tmp = fresh();
    open_change(tmp.path());

    let error = stage_err(
        tmp.path(),
        &["add", "notion", "--change", "CHG-9999", "--json"],
        &customer_payload(),
    );

    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(error["message"], json!("unknown change `CHG-9999`"));
}

// --- human mode -------------------------------------------------------------

#[test]
fn human_mode_names_the_change_the_verb_and_the_target() {
    let tmp = fresh();
    open_change(tmp.path());

    let mut cmd = telos(tmp.path(), &["add", "notion", "--change", "CHG-0001"]);
    let out = cmd.write_stdin(customer_payload()).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "CHG-0001: add notion billing/Customer\n"
    );
}
