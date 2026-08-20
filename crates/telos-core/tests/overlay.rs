//! The overlay: the sealed base plus a change's staged ops, and the rules
//! that govern applying them (rule 2 of spec §3.3 in particular).
//!
//! Everything here runs against a copy of the `billing` corpus in a
//! tempdir. The overlay never writes: these tests read a real spec tree,
//! apply ops in memory, and check what comes back -- which is exactly what
//! `telos add|edit|remove` does before it decides whether a staged op is
//! allowed to reach a change file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;

use telos_core::counters::{Alloc, Counters, floors};
use telos_core::error::{Diagnostic, ErrorCode};
use telos_core::ids::{ConstraintId, IntentId, NotionName, RepoPath};
use telos_core::model::{Intent, Notion, StagedOp, TelFile};
use telos_core::overlay::{apply_ops, notions_of, op_before_after, parse_base, validate_ops};
use telos_core::payload::{intent_from_json, notion_from_json, patch_intent};
use telos_core::workspace::Workspace;

// --- fixture plumbing ------------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {}: {e}", dst.display()));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display())) {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("copy {}: {e}", entry.path().display()));
        }
    }
}

/// A copy of the `billing` corpus, and the workspace discovered on it. The
/// `TempDir` is returned alongside so the caller keeps it alive.
fn corpus() -> (TempDir, Workspace) {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&corpus_root(), tmp.path());
    let ws = Workspace::discover(tmp.path()).expect("the corpus is an initialized workspace");
    (tmp, ws)
}

fn base_of(ws: &Workspace) -> Vec<(RepoPath, TelFile)> {
    parse_base(ws).unwrap_or_else(|diags| panic!("the corpus parses cleanly, got {diags:?}"))
}

/// The corpus' allocator: no persisted counters, floors scanned off the
/// sealed model (INT-0042, SCN-0107, CON-0003).
fn corpus_alloc(ws: &Workspace) -> Alloc {
    let model = ws.load_model().expect("the corpus loads cleanly");
    Alloc::new(Counters::default(), floors(&model, &[], None))
}

fn intent_of(base: &[(RepoPath, TelFile)], id: IntentId) -> Intent {
    base.iter()
        .find_map(|(_, file)| match file {
            TelFile::Intent(intent) if intent.id == id => Some(intent.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the corpus holds {id}"))
}

fn notion(name: &str) -> NotionName {
    NotionName::new(name).unwrap()
}

/// The `Refund` event notion, absent from the corpus: what an `add notion`
/// op stages.
fn refund_notion() -> Notion {
    notion_from_json(&json!({
        "name": "Refund", "kind": "event", "def": "Money went back to a Customer.",
        "attrs": [{"name": "amount", "type": "money"}]
    }))
    .unwrap()
}

/// An intent whose statement and scenario both name `Refund` -- valid only
/// against an overlay in which the notion above was already staged.
fn refund_intent(notions: &BTreeMap<NotionName, Notion>, alloc: &mut Alloc) -> Intent {
    intent_from_json(
        &json!({
            "title": "A refund reopens the invoice", "status": "active",
            "telos": "A refunded invoice is not a settled one.",
            "statement": {"template": "event-driven", "when": "Refund", "on": "Invoice",
                          "action": "set Invoice.state = open"},
            "scenarios": [
                {"title": "a refund reopens a settled invoice",
                 "given": [{"notion": "Invoice", "fields": {"state": "settled"}}],
                 "when": {"notion": "Refund", "fields": {"amount": "120.00 EUR"}},
                 "then": ["Invoice.state == open"]}
            ]
        }),
        notions,
        alloc,
    )
    .unwrap()
    .0
}

fn find(base: &[(RepoPath, TelFile)], path: &str) -> Option<TelFile> {
    base.iter()
        .find(|(p, _)| p.as_str() == path)
        .map(|(_, file)| file.clone())
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<ErrorCode> {
    diagnostics.iter().map(|d| d.code).collect()
}

// --- parse_base ------------------------------------------------------------

#[test]
fn parse_base_parses_every_spec_file_and_no_configuration() {
    let (_tmp, ws) = corpus();

    let base = base_of(&ws);

    let paths: Vec<&str> = base.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "telos/bindings.tel",
            "telos/constraints/CON-0003.tel",
            "telos/intents/INT-0017.tel",
            "telos/intents/INT-0042.tel",
            "telos/notions/Customer.tel",
            "telos/notions/Invoice.tel",
            "telos/notions/InvoiceIssued.tel",
            "telos/notions/PaymentReceived.tel",
        ],
        "`telos.toml` is configuration, not a `.tel` source"
    );
}

#[test]
fn notions_of_collects_the_bases_notions_by_name() {
    let (_tmp, ws) = corpus();

    let notions = notions_of(&base_of(&ws));

    let names: Vec<&str> = notions.keys().map(NotionName::as_str).collect();
    assert_eq!(
        names,
        ["Customer", "Invoice", "InvoiceIssued", "PaymentReceived"]
    );
}

// --- apply_ops: add --------------------------------------------------------

#[test]
fn apply_ops_rejects_adding_a_notion_that_already_exists() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let invoice = match find(&base, "telos/notions/Invoice.tel").unwrap() {
        TelFile::Notion(n) => n,
        other => panic!("expected a notion, got {other:?}"),
    };

    let err = apply_ops(base, &[StagedOp::AddNotion(invoice)]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(err.message, "notion `Invoice` already exists");
}

#[test]
fn apply_ops_rejects_adding_an_intent_that_already_exists() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let existing = intent_of(&base, IntentId(17));

    let err = apply_ops(base, &[StagedOp::AddIntent(existing)]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(err.message, "intent INT-0017 already exists");
}

#[test]
fn apply_ops_adds_an_absent_notion_at_its_own_path() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let overlay = apply_ops(base, &[StagedOp::AddNotion(refund_notion())]).unwrap();

    assert_eq!(
        find(&overlay, "telos/notions/Refund.tel"),
        Some(TelFile::Notion(refund_notion()))
    );
}

// --- apply_ops: edit / remove of something absent ---------------------------

#[test]
fn apply_ops_rejects_editing_a_notion_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut ghost = refund_notion();
    ghost.name = notion("Invoce");

    let err = apply_ops(base, &[StagedOp::EditNotion(ghost)]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown notion `Invoce`; closest is `Invoice`");
}

#[test]
fn apply_ops_rejects_removing_an_intent_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(base, &[StagedOp::RemoveIntent(IntentId(9999))]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown intent `INT-9999`");
}

#[test]
fn apply_ops_rejects_removing_a_constraint_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(base, &[StagedOp::RemoveConstraint(ConstraintId(9))]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        err.message,
        "unknown constraint `CON-0009`; closest is `CON-0003`"
    );
}

// --- apply_ops: rule 2, removing something still referenced -----------------

/// The rule as spec §3.3 states it, and as the brief freezes its wording:
/// the referrer is named, so the caller knows what to fix first.
#[test]
fn apply_ops_refuses_to_remove_an_intent_another_intent_requires() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(base, &[StagedOp::RemoveIntent(IntentId(17))]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        err.message,
        "cannot remove intent INT-0017: INT-0042 requires it"
    );
}

#[test]
fn apply_ops_refuses_to_remove_a_notion_an_intent_uses() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(base, &[StagedOp::RemoveNotion(notion("Invoice"))]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        err.message,
        "cannot remove notion `Invoice`: INT-0017 uses it"
    );
}

#[test]
fn apply_ops_refuses_to_remove_an_intent_a_binding_implements() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    // INT-0042 is required by nobody, but `telos/bindings.tel` implements it.
    let err = apply_ops(base, &[StagedOp::RemoveIntent(IntentId(42))]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        err.message,
        "cannot remove intent INT-0042: `src/billing/invoice.rs` implements it"
    );
}

/// The referrers are computed from the state *after* the removal, so a
/// change that removes the referrer first and the referred-to entity second
/// is allowed -- order is data (D1).
#[test]
fn apply_ops_allows_removing_an_intent_once_its_referrers_are_gone() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut bindings = base.clone();
    // `bindings.tel` is not an entity op's target, so drop the two bindings
    // by hand: this test is about the intent-to-intent reference.
    bindings.retain(|(path, _)| path.as_str() != "telos/bindings.tel");

    let overlay = apply_ops(
        bindings,
        &[
            StagedOp::RemoveIntent(IntentId(42)),
            StagedOp::RemoveIntent(IntentId(17)),
        ],
    )
    .unwrap();

    assert_eq!(find(&overlay, "telos/intents/INT-0017.tel"), None);
    assert_eq!(find(&overlay, "telos/intents/INT-0042.tel"), None);
}

/// Nothing in the model points at a constraint, so removing one is never
/// blocked by rule 2.
#[test]
fn apply_ops_removes_a_constraint_nothing_can_reference() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let overlay = apply_ops(base, &[StagedOp::RemoveConstraint(ConstraintId(3))]).unwrap();

    assert_eq!(find(&overlay, "telos/constraints/CON-0003.tel"), None);
}

// --- apply_ops: order -------------------------------------------------------

#[test]
fn apply_ops_replays_its_ops_in_staged_order() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut edited = refund_notion();
    edited.def = "Reworded.".to_string();

    let overlay = apply_ops(
        base,
        &[
            StagedOp::AddNotion(refund_notion()),
            StagedOp::EditNotion(edited.clone()),
        ],
    )
    .unwrap();

    assert_eq!(
        find(&overlay, "telos/notions/Refund.tel"),
        Some(TelFile::Notion(edited))
    );
}

/// An `accept` op names a path the overlay does not model (`telos.toml`, a
/// code file): it seals bytes at reconcile time and is inert here.
#[test]
fn apply_ops_leaves_an_accept_op_alone() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let overlay = apply_ops(
        base.clone(),
        &[StagedOp::Accept {
            path: RepoPath::new("telos/telos.toml"),
            oid: telos_core::git::Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
        }],
    )
    .unwrap();

    assert_eq!(overlay, base);
}

// --- validate_ops -----------------------------------------------------------

/// The point of the overlay: an intent may name a notion that exists only
/// because an earlier op of the same change added it.
#[test]
fn validate_ops_accepts_a_notion_and_an_intent_that_uses_it() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(base, &[StagedOp::AddNotion(refund_notion())]).unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    let model = validate_ops(
        &ws,
        &[
            StagedOp::AddNotion(refund_notion()),
            StagedOp::AddIntent(intent),
        ],
    )
    .unwrap_or_else(|diags| panic!("expected the overlay to validate, got {diags:?}"));

    assert!(model.notions.contains_key(&notion("Refund")));
    assert!(model.intents.contains_key(&IntentId(43)));
}

/// What `validate_ops` judges is the spec the *whole* delta describes, not
/// each intermediate state: the semantic pass runs once, on the overlay the
/// last op leaves behind. So these two ops validate in either order.
///
/// Staging them in the wrong order is still impossible, but that is a
/// different gate and a different layer: `telos add intent` resolves its
/// scenario fields against the notions the change holds *so far*
/// (`notions_of` over the ops staged before it), so the intent could not
/// even be built before its notion was staged.
#[test]
fn validate_ops_judges_the_final_overlay_not_each_intermediate_state() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(base, &[StagedOp::AddNotion(refund_notion())]).unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    let model = validate_ops(
        &ws,
        &[
            StagedOp::AddIntent(intent),
            StagedOp::AddNotion(refund_notion()),
        ],
    )
    .unwrap_or_else(|diags| panic!("expected the overlay to validate, got {diags:?}"));

    assert!(model.notions.contains_key(&notion("Refund")));
}

/// A reference nothing in the overlay resolves is the semantic pass'
/// diagnostic, reported against the file the staged entity would be
/// written to.
#[test]
fn validate_ops_reports_a_dangling_reference_in_a_staged_op() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(base, &[StagedOp::AddNotion(refund_notion())]).unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    // The notion the intent needs is never staged.
    let diagnostics = validate_ops(&ws, &[StagedOp::AddIntent(intent)]).unwrap_err();

    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::TelosReferenceUnknown
                && d.message.contains("unknown notion `Refund`")),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(RepoPath::as_str),
        Some("telos/intents/INT-0043.tel")
    );
}

#[test]
fn validate_ops_surfaces_an_apply_failure_as_a_diagnostic() {
    let (_tmp, ws) = corpus();

    let diagnostics = validate_ops(&ws, &[StagedOp::RemoveIntent(IntentId(17))]).unwrap_err();

    assert_eq!(codes(&diagnostics), [ErrorCode::TelosIntegrityViolation]);
    assert_eq!(
        diagnostics[0].message,
        "cannot remove intent INT-0017: INT-0042 requires it"
    );
}

#[test]
fn validate_ops_with_no_op_is_the_sealed_model() {
    let (_tmp, ws) = corpus();

    let model = validate_ops(&ws, &[]).unwrap();

    assert_eq!(model.intents.len(), 2);
    assert_eq!(model.notions.len(), 4);
}

// --- op_before_after --------------------------------------------------------

#[test]
fn op_before_after_of_an_add_has_no_before() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::AddNotion(refund_notion())];

    let (before, after) = op_before_after(&base, &ops, 0);

    assert_eq!(before, None);
    assert_eq!(after, Some(telos_core::emit::emit_notion(&refund_notion())));
}

#[test]
fn op_before_after_of_an_edit_reports_both_canonical_states() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let patched = patch_intent(
        &intent_of(&base, IntentId(17)),
        &json!({"telos": "Reworded."}),
        &notions_of(&base),
        &mut alloc,
    )
    .unwrap()
    .0;
    let ops = [StagedOp::EditIntent(patched.clone())];

    let (before, after) = op_before_after(&base, &ops, 0);

    assert_eq!(
        before,
        Some(fs::read_to_string(ws.repo_root.join("telos/intents/INT-0017.tel")).unwrap())
    );
    assert_eq!(after, Some(telos_core::emit::emit_intent(&patched)));
    assert!(after.unwrap().contains("Reworded."));
}

#[test]
fn op_before_after_of_a_remove_has_no_after() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::RemoveConstraint(ConstraintId(3))];

    let (before, after) = op_before_after(&base, &ops, 0);

    assert!(before.unwrap().starts_with("constraint CON-0003"));
    assert_eq!(after, None);
}

/// The ops before the target one are replayed first, so an `edit` of
/// something an earlier op added reports that op's output as its `before`.
#[test]
fn op_before_after_replays_the_ops_that_precede_the_target() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut edited = refund_notion();
    edited.def = "Reworded.".to_string();
    let ops = [
        StagedOp::AddNotion(refund_notion()),
        StagedOp::EditNotion(edited.clone()),
    ];

    let (before, after) = op_before_after(&base, &ops, 1);

    assert_eq!(
        before,
        Some(telos_core::emit::emit_notion(&refund_notion()))
    );
    assert_eq!(after, Some(telos_core::emit::emit_notion(&edited)));
}

/// An `accept` op targets a path the overlay holds no entity for: nothing
/// to render on either side.
#[test]
fn op_before_after_of_an_accept_is_empty_on_both_sides() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::Accept {
        path: RepoPath::new("telos/telos.toml"),
        oid: telos_core::git::Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
    }];

    assert_eq!(op_before_after(&base, &ops, 0), (None, None));
}

#[test]
fn op_before_after_of_an_index_past_the_ops_is_empty() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    assert_eq!(op_before_after(&base, &[], 0), (None, None));
}
