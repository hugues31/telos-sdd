//! The semantic pass, end to end on the Annex D corpus.
//!
//! Every case is the shared `telos/` fixture tree, parsed as the workspace
//! would parse it, with at most one line rewritten -- so what a test asserts
//! is exactly what that one edit caused. The corpus itself must build
//! clean; every deviation from it must produce one precise diagnostic, whose
//! message is pinned to the byte (agents read these messages).

use std::fs;
use std::path::{Path, PathBuf};

use telos_core::error::{Diagnostic, ErrorCode};
use telos_core::graph::{ImpactEntry, NodeRef, Relation};
use telos_core::ids::{
    ChangeId, ConstraintId, EntityRef, IntentId, NotionName, RepoPath, ScenarioId,
};
use telos_core::model::{SourceKind, TelFile, TelosModel};
use telos_core::semantic::build_model;
use telos_core::syntax::{
    parse_bindings_file, parse_constraint_file, parse_intent_file, parse_notion_file,
};

// --- fixture plumbing ----------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

/// The corpus `.tel` files, repo-relative and sorted -- the order
/// `Workspace::spec_files()` will hand them over in.
const CORPUS: [&str; 8] = [
    "telos/bindings.tel",
    "telos/constraints/CON-0003.tel",
    "telos/intents/INT-0017.tel",
    "telos/intents/INT-0042.tel",
    "telos/notions/Customer.tel",
    "telos/notions/Invoice.tel",
    "telos/notions/InvoiceIssued.tel",
    "telos/notions/PaymentReceived.tel",
];

fn read(rel: &str) -> String {
    let path = corpus_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Parses one file with the rule its location calls for (Annex C.4.2).
fn parse(rel: &str, src: &str) -> TelFile {
    fn ok<T, E>(rel: &str, result: Result<T, Vec<E>>) -> T {
        match result {
            Ok(node) => node,
            Err(diags) => panic!("{rel} must parse, got {} diagnostic(s)", diags.len()),
        }
    }
    let path = RepoPath::new(rel);
    if rel.ends_with("bindings.tel") {
        TelFile::Bindings(ok(rel, parse_bindings_file(&path, src)))
    } else if rel.contains("notions/") {
        TelFile::Notion(ok(rel, parse_notion_file(&path, src)))
    } else if rel.contains("intents/") {
        TelFile::Intent(ok(rel, parse_intent_file(&path, src)))
    } else if rel.contains("constraints/") {
        TelFile::Constraint(ok(rel, parse_constraint_file(&path, src)))
    } else {
        panic!("{rel}: no parse rule for this location");
    }
}

/// The corpus as `(path, source)` pairs, ready to be edited then parsed.
fn sources() -> Vec<(String, String)> {
    CORPUS
        .iter()
        .map(|rel| ((*rel).to_string(), read(rel)))
        .collect()
}

/// The corpus with one occurrence of `from` rewritten to `to` in `rel`.
///
/// The anchor must be present and unambiguous: a silently missed edit would
/// turn an assertion about a deviation into an assertion about the pristine
/// corpus.
fn edited(rel: &str, from: &str, to: &str) -> Vec<(String, String)> {
    let mut files = sources();
    let (_, src) = files
        .iter_mut()
        .find(|(path, _)| path == rel)
        .unwrap_or_else(|| panic!("{rel} is not a corpus file"));
    assert_eq!(
        src.matches(from).count(),
        1,
        "`{from}` must appear exactly once in {rel}"
    );
    *src = src.replace(from, to);
    files
}

/// The corpus plus one extra file.
fn plus(rel: &str, src: &str) -> Vec<(String, String)> {
    let mut files = sources();
    files.push((rel.to_string(), src.to_string()));
    files
}

fn build(files: Vec<(String, String)>) -> Result<TelosModel, Vec<Diagnostic>> {
    build_model(
        files
            .iter()
            .map(|(rel, src)| (RepoPath::new(rel.clone()), parse(rel, src)))
            .collect(),
    )
}

fn model_of(files: Vec<(String, String)>) -> TelosModel {
    build(files).unwrap_or_else(|diags| {
        panic!(
            "expected a clean model, got:\n{}",
            diags
                .iter()
                .map(|d| format!("  {}", d.message))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn diagnostics(files: Vec<(String, String)>) -> Vec<Diagnostic> {
    match build(files) {
        Ok(_) => panic!("expected diagnostics, got a clean model"),
        Err(diags) => diags,
    }
}

/// The single diagnostic a one-line deviation must produce -- asserting the
/// count too, so a check that fires twice, or a second check firing by
/// accident, is a failure.
fn only_diagnostic(files: Vec<(String, String)>) -> Diagnostic {
    let mut diags = diagnostics(files);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one diagnostic, got:\n{}",
        diags
            .iter()
            .map(|d| format!("  {}", d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    diags.remove(0)
}

fn notion(name: &str) -> NodeRef {
    NodeRef::Notion(NotionName::new(name).unwrap())
}

fn intent(n: u32) -> NodeRef {
    NodeRef::Intent(IntentId(n))
}

fn scenario(n: u32) -> NodeRef {
    NodeRef::Scenario(ScenarioId(n))
}

// --- the corpus builds ---------------------------------------------------

#[test]
fn the_corpus_builds_a_model_without_a_single_diagnostic() {
    let model = model_of(sources());
    assert_eq!(model.notions.len(), 4);
    assert_eq!(model.intents.len(), 2);
    assert_eq!(model.constraints.len(), 1);
    assert_eq!(model.bindings.len(), 2);
}

#[test]
fn every_scenario_knows_the_intent_that_owns_it() {
    let model = model_of(sources());
    assert_eq!(
        model.scenario_owner.get(&ScenarioId(91)),
        Some(&IntentId(17))
    );
    assert_eq!(
        model.scenario_owner.get(&ScenarioId(107)),
        Some(&IntentId(42))
    );
    let (owner, scn) = model.scenario(ScenarioId(107)).expect("SCN-0107 is known");
    assert_eq!(owner.id, IntentId(42));
    assert_eq!(scn.title, "full payment settles the invoice");
}

#[test]
fn sources_record_which_file_declared_which_entity() {
    let model = model_of(sources());
    let kind = |rel: &str| model.sources.get(&RepoPath::new(rel)).cloned();
    assert_eq!(
        kind("telos/notions/Invoice.tel"),
        Some(SourceKind::Notion(NotionName::new("Invoice").unwrap()))
    );
    assert_eq!(
        kind("telos/intents/INT-0042.tel"),
        Some(SourceKind::Intent(IntentId(42)))
    );
    assert_eq!(
        kind("telos/constraints/CON-0003.tel"),
        Some(SourceKind::Constraint(ConstraintId(3)))
    );
    assert_eq!(kind("telos/bindings.tel"), Some(SourceKind::Bindings));
    assert_eq!(model.sources.len(), CORPUS.len());
}

#[test]
fn resolve_turns_a_show_argument_into_a_graph_node() {
    let model = model_of(sources());
    assert_eq!(
        model.resolve(&"Invoice".parse::<EntityRef>().unwrap()),
        Some(notion("Invoice"))
    );
    assert_eq!(
        model.resolve(&"SCN-0107".parse::<EntityRef>().unwrap()),
        Some(scenario(107))
    );
    assert_eq!(
        model.resolve(&"INT-0099".parse::<EntityRef>().unwrap()),
        None
    );
    assert_eq!(model.resolve(&EntityRef::Change(ChangeId(7))), None);
}

// --- the derived graph ---------------------------------------------------

#[test]
fn an_intent_uses_the_notions_of_its_statement_and_requires_what_it_declares() {
    let model = model_of(sources());
    assert_eq!(
        model.graph.out_edges(&intent(42)),
        [
            (Relation::Requires, intent(17)),
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("PaymentReceived")),
        ]
    );
}

#[test]
fn a_notion_used_twice_by_one_intent_is_a_single_edge() {
    // INT-0017 names `Invoice` twice: `on Invoice` and `set Invoice.state`.
    let model = model_of(sources());
    assert_eq!(
        model.graph.out_edges(&intent(17)),
        [
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("InvoiceIssued")),
        ]
    );
}

#[test]
fn a_scenario_verifies_its_intent_and_uses_its_own_notions() {
    // The `uses` edges of a scenario hang off the scenario, not off the
    // intent nesting it: SCN-0091 uses Customer, INT-0017 does not.
    let model = model_of(sources());
    assert_eq!(
        model.graph.out_edges(&scenario(91)),
        [
            (Relation::Verifies, intent(17)),
            (Relation::Uses, notion("Customer")),
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("InvoiceIssued")),
        ]
    );
    assert_eq!(
        model.graph.out_edges(&scenario(107)),
        [
            (Relation::Verifies, intent(42)),
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("PaymentReceived")),
        ]
    );
}

#[test]
fn bindings_attach_code_and_tests_to_the_spec() {
    let model = model_of(sources());
    let code = NodeRef::Code(RepoPath::new("src/billing/invoice.rs"));
    let test = NodeRef::Test("tests/billing.rs::scn_0107_full_payment_settles_the_invoice".into());
    assert_eq!(
        model.graph.out_edges(&code),
        [(Relation::Implements, intent(42))]
    );
    assert_eq!(
        model.graph.out_edges(&test),
        [(Relation::Proves, scenario(107))]
    );
}

#[test]
fn a_global_constraint_constrains_no_intent_in_particular() {
    let model = model_of(sources());
    assert!(
        model
            .graph
            .out_edges(&NodeRef::Constraint(ConstraintId(3)))
            .is_empty()
    );
}

#[test]
fn a_scoped_constraint_gets_one_edge_per_intent_in_scope() {
    let model = model_of(edited(
        "telos/constraints/CON-0003.tel",
        "scope global",
        "scope INT-0042, INT-0017",
    ));
    assert_eq!(
        model.graph.out_edges(&NodeRef::Constraint(ConstraintId(3))),
        [
            (Relation::Constrains, intent(17)),
            (Relation::Constrains, intent(42)),
        ]
    );
}

#[test]
fn the_corpus_has_no_cycle_on_either_relation() {
    let model = model_of(sources());
    assert_eq!(model.graph.find_cycle(Relation::Requires), None);
    assert_eq!(model.graph.find_cycle(Relation::Refines), None);
}

#[test]
fn changing_a_notion_impacts_everything_that_reaches_it() {
    let model = model_of(sources());
    assert_eq!(
        model.graph.reverse_closure(&notion("Invoice")),
        vec![
            ImpactEntry {
                node: intent(17),
                via: Relation::Uses,
                distance: 1
            },
            ImpactEntry {
                node: intent(42),
                via: Relation::Uses,
                distance: 1
            },
            ImpactEntry {
                node: scenario(91),
                via: Relation::Uses,
                distance: 1
            },
            ImpactEntry {
                node: scenario(107),
                via: Relation::Uses,
                distance: 1
            },
            ImpactEntry {
                node: NodeRef::Code(RepoPath::new("src/billing/invoice.rs")),
                via: Relation::Implements,
                distance: 2
            },
            ImpactEntry {
                node: NodeRef::Test(
                    "tests/billing.rs::scn_0107_full_payment_settles_the_invoice".into()
                ),
                via: Relation::Proves,
                distance: 2
            },
        ]
    );
}

// --- dangling references -------------------------------------------------

#[test]
fn an_unknown_notion_is_reported_with_the_closest_known_one() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "on Invoice",
        "on Invoce",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown notion `Invoce`; closest is `Invoice`"
    );
    assert_eq!(diag.hint, None);
    assert_eq!(diag.file, Some(RepoPath::new("telos/intents/INT-0042.tel")));
}

#[test]
fn an_unknown_notion_with_no_close_match_is_reported_bare() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "on Invoice",
        "on Rogue",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(diag.message, "unknown notion `Rogue`");
}

#[test]
fn an_unknown_notion_in_a_scenario_step_is_reported() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0017.tel",
        "given Customer { name: \"ACME\" }",
        "given Custome {}",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown notion `Custome`; closest is `Customer`"
    );
}

#[test]
fn an_unknown_rel_target_is_reported() {
    let diag = only_diagnostic(edited(
        "telos/notions/Invoice.tel",
        "rel  issued-to -> Customer",
        "rel  issued-to -> Custome",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown notion `Custome`; closest is `Customer`"
    );
    assert_eq!(diag.file, Some(RepoPath::new("telos/notions/Invoice.tel")));
}

#[test]
fn an_unknown_ref_attr_target_is_reported() {
    let diag = only_diagnostic(plus(
        "telos/notions/Ledger.tel",
        concat!(
            "notion Ledger entity {\n",
            "  def  \"A book of invoices.\"\n",
            "  attr owner ref(Custome)\n",
            "}\n",
        ),
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown notion `Custome`; closest is `Customer`"
    );
    assert_eq!(diag.file, Some(RepoPath::new("telos/notions/Ledger.tel")));
}

#[test]
fn an_unknown_attribute_is_reported_with_the_closest_one() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "then  Invoice.state == settled",
        "then  Invoice.stat == settled",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown attribute `stat` on notion `Invoice`; closest is `state`"
    );
}

#[test]
fn an_unknown_instance_field_is_reported_as_an_attribute() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "when  PaymentReceived { amount: \"120.00 EUR\" }",
        "when  PaymentReceived { amont: \"120.00 EUR\" }",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown attribute `amont` on notion `PaymentReceived`; closest is `amount`"
    );
}

#[test]
fn an_unknown_enum_symbol_is_reported_with_the_closest_one() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "then  Invoice.state == settled",
        "then  Invoice.state == setled",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "`setled` is not a symbol of enum `Invoice.state`; closest is `settled`"
    );
}

#[test]
fn an_unknown_intent_in_a_relation_is_reported() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "requires INT-0017",
        "requires INT-0018",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown intent `INT-0018`; closest is `INT-0017`"
    );
}

#[test]
fn an_unknown_intent_in_a_constraint_scope_is_reported() {
    let diag = only_diagnostic(edited(
        "telos/constraints/CON-0003.tel",
        "scope global",
        "scope INT-0099",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown intent `INT-0099`; closest is `INT-0017`"
    );
    assert_eq!(
        diag.file,
        Some(RepoPath::new("telos/constraints/CON-0003.tel"))
    );
}

#[test]
fn unknown_binding_targets_are_reported() {
    let mut files = edited("telos/bindings.tel", "-> INT-0042", "-> INT-0043");
    let (_, src) = files
        .iter_mut()
        .find(|(path, _)| path == "telos/bindings.tel")
        .expect("bindings");
    *src = src.replace("-> SCN-0107", "-> SCN-0108");

    let diags = diagnostics(files);
    let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        vec![
            "unknown intent `INT-0043`; closest is `INT-0042`",
            "unknown scenario `SCN-0108`; closest is `SCN-0107`",
        ]
    );
    assert!(
        diags
            .iter()
            .all(|d| d.code == ErrorCode::TelosReferenceUnknown)
    );
    assert!(
        diags
            .iter()
            .all(|d| d.file == Some(RepoPath::new("telos/bindings.tel")))
    );
}

// --- cycles --------------------------------------------------------------

#[test]
fn a_requires_cycle_is_reported_with_its_path() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0017.tel",
        "  }\n\n  scenario SCN-0091",
        "  }\n  requires INT-0042\n\n  scenario SCN-0091",
    ));
    assert_eq!(diag.code, ErrorCode::TelosCycleDetected);
    assert!(
        diag.message.contains("INT-0017 → INT-0042 → INT-0017"),
        "cycle path missing from: {}",
        diag.message
    );
    assert_eq!(
        diag.message,
        "cycle on `requires`: INT-0017 → INT-0042 → INT-0017"
    );
}

#[test]
fn a_refines_cycle_is_reported_independently_of_requires() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "requires INT-0017",
        "refines INT-0042",
    ));
    assert_eq!(diag.code, ErrorCode::TelosCycleDetected);
    assert_eq!(diag.message, "cycle on `refines`: INT-0042 → INT-0042");
}

// --- integrity rules -----------------------------------------------------

#[test]
fn an_active_intent_without_a_scenario_is_a_violation() {
    let diag = only_diagnostic(plus(
        "telos/intents/INT-0055.tel",
        concat!(
            "intent INT-0055 \"Invoices are tracked\" {\n",
            "  status active\n",
            "  telos  \"Nothing is billed that is not tracked.\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"track every invoice\"\n",
            "  }\n",
            "}\n",
        ),
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "intent INT-0055 is active but has no scenario"
    );
    assert_eq!(diag.file, Some(RepoPath::new("telos/intents/INT-0055.tel")));
}

#[test]
fn a_draft_intent_without_a_scenario_is_allowed() {
    let model = model_of(plus(
        "telos/intents/INT-0055.tel",
        concat!(
            "intent INT-0055 \"Invoices are tracked\" {\n",
            "  status draft\n",
            "  telos  \"Nothing is billed that is not tracked.\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"track every invoice\"\n",
            "  }\n",
            "}\n",
        ),
    ));
    assert_eq!(model.intents.len(), 3);
}

#[test]
fn a_when_step_on_a_non_event_notion_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "when  PaymentReceived { amount: \"120.00 EUR\" }",
        "when  Invoice { state: open }",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "`Invoice` is used as an event but its kind is `entity`, not `event`"
    );
}

#[test]
fn an_event_driven_statement_on_a_non_event_notion_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "when   PaymentReceived on Invoice",
        "when   Customer on Invoice",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "`Customer` is used as an event but its kind is `entity`, not `event`"
    );
}

#[test]
fn a_given_step_may_name_a_notion_of_any_kind() {
    // Only `when` demands an event: `given` sets up state of any kind.
    let model = model_of(edited(
        "telos/intents/INT-0017.tel",
        "given Customer { name: \"ACME\" }",
        "given PaymentReceived { amount: \"120.00 EUR\" }",
    ));
    assert_eq!(
        model.graph.out_edges(&scenario(91)),
        [
            (Relation::Verifies, intent(17)),
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("InvoiceIssued")),
            (Relation::Uses, notion("PaymentReceived")),
        ]
    );
}

// --- duplicates ----------------------------------------------------------

#[test]
fn two_notions_with_the_same_name_are_a_violation() {
    let diag = only_diagnostic(plus(
        "telos/notions/Invoice2.tel",
        concat!(
            "notion Invoice entity {\n",
            "  def  \"A second, conflicting definition.\"\n",
            "}\n",
        ),
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "notion `Invoice` is declared twice: telos/notions/Invoice.tel and telos/notions/Invoice2.tel"
    );
    assert_eq!(diag.file, Some(RepoPath::new("telos/notions/Invoice2.tel")));
}

#[test]
fn two_intents_with_the_same_id_are_a_violation() {
    let diag = only_diagnostic(plus(
        "telos/intents/INT-0042-bis.tel",
        concat!(
            "intent INT-0042 \"A conflicting redefinition\" {\n",
            "  status draft\n",
            "  telos  \"Two files claim the same id.\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"do something else\"\n",
            "  }\n",
            "}\n",
        ),
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "intent INT-0042 is declared twice: telos/intents/INT-0042.tel and telos/intents/INT-0042-bis.tel"
    );
}

#[test]
fn two_constraints_with_the_same_id_are_a_violation() {
    let diag = only_diagnostic(plus(
        "telos/constraints/CON-0003-bis.tel",
        concat!(
            "constraint CON-0003 quality \"A conflicting redefinition\" {\n",
            "  rule  \"Two files claim the same id.\"\n",
            "  scope global\n",
            "}\n",
        ),
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "constraint CON-0003 is declared twice: telos/constraints/CON-0003.tel and telos/constraints/CON-0003-bis.tel"
    );
}

#[test]
fn one_scenario_id_in_two_intents_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0017.tel",
        "scenario SCN-0091",
        "scenario SCN-0107",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        concat!(
            "scenario SCN-0107 is declared twice: ",
            "in INT-0017 (telos/intents/INT-0017.tel) ",
            "and in INT-0042 (telos/intents/INT-0042.tel)"
        )
    );
}

// --- literal / attribute type compatibility ------------------------------

#[test]
fn an_int_where_a_string_is_expected_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0017.tel",
        "given Customer { name: \"ACME\" }",
        "given Customer { name: 42 }",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "attribute `Customer.name` has type `string`, but the value is an int"
    );
}

#[test]
fn a_malformed_money_amount_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "given Invoice { state: open, balance: \"120.00 EUR\" }",
        "given Invoice { state: open, balance: \"120 EUR\" }",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "attribute `Invoice.balance` has type `money`, but `120 EUR` is not an amount of the form `0.00 EUR`"
    );
}

#[test]
fn a_symbol_where_a_money_amount_is_expected_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "when  PaymentReceived { amount: \"120.00 EUR\" }",
        "when  PaymentReceived { amount: settled }",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "attribute `PaymentReceived.amount` has type `money`, but the value is a symbol"
    );
}

#[test]
fn a_string_where_an_enum_symbol_is_expected_is_a_violation() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "then  Invoice.state == settled",
        "then  Invoice.state == \"settled\"",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "attribute `Invoice.state` has type `enum`, but the value is a string"
    );
}

#[test]
fn the_value_of_a_set_action_is_checked_against_its_attribute() {
    let diag = only_diagnostic(edited(
        "telos/intents/INT-0042.tel",
        "system shall set Invoice.state = settled",
        "system shall set Invoice.state = 3",
    ));
    assert_eq!(diag.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        diag.message,
        "attribute `Invoice.state` has type `enum`, but the value is an int"
    );
}

#[test]
fn a_comparison_between_two_attributes_only_checks_that_both_exist() {
    // Cross-attribute type agreement is deliberately out of M1's scope: the
    // two sides must resolve, nothing more.
    let model = model_of(edited(
        "telos/intents/INT-0042.tel",
        "then  Invoice.state == settled",
        "then  Invoice.state == Invoice.balance",
    ));
    assert_eq!(
        model.graph.out_edges(&scenario(107)),
        [
            (Relation::Verifies, intent(42)),
            (Relation::Uses, notion("Invoice")),
            (Relation::Uses, notion("PaymentReceived")),
        ]
    );
}

#[test]
fn a_machine_constraint_rule_has_its_references_checked() {
    let diag = only_diagnostic(edited(
        "telos/constraints/CON-0003.tel",
        "rule  \"Domain code must not import adapter modules.\"",
        "rule  Invoice.blance >= 0",
    ));
    assert_eq!(diag.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        diag.message,
        "unknown attribute `blance` on notion `Invoice`; closest is `balance`"
    );
}

// --- collecting, not stopping at the first ------------------------------

#[test]
fn every_diagnostic_is_collected_not_just_the_first() {
    // Three independent faults, in three different checks: a build reports
    // all of them at once, in a stable order (by entity, then cycles last).
    let mut files = edited("telos/intents/INT-0042.tel", "on Invoice", "on Rogue");
    let (_, src) = files
        .iter_mut()
        .find(|(path, _)| path == "telos/intents/INT-0017.tel")
        .expect("INT-0017");
    *src = src.replace(
        "given Customer { name: \"ACME\" }",
        "given Customer { name: 42 }",
    );
    *src = src.replace(
        "  }\n\n  scenario SCN-0091",
        "  }\n  requires INT-0042\n\n  scenario SCN-0091",
    );

    let messages: Vec<String> = diagnostics(files).into_iter().map(|d| d.message).collect();
    assert_eq!(
        messages,
        vec![
            "attribute `Customer.name` has type `string`, but the value is an int",
            "unknown notion `Rogue`",
            "cycle on `requires`: INT-0017 → INT-0042 → INT-0017",
        ]
    );
}
