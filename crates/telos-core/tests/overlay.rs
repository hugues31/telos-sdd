//! The overlay: the sealed base plus a change's staged ops, and the rules
//! that govern applying them (referential deletion safety in particular).
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

use telos_core::config::{AgentsCfg, Config, GherkinCfg, Globs, Policy, TddPolicy, TestCfg};
use telos_core::counters::{Alloc, Counters, floors};
use telos_core::error::{Diagnostic, ErrorCode};
use telos_core::ids::{
    CapabilityId, CapabilityRef, ConstraintId, ContextId, IntentId, NotionName, Owner, RepoPath,
};
use telos_core::model::{Intent, Notion, StagedOp, TelFile};
use telos_core::overlay::{
    apply_config_ops, apply_ops, apply_ops_idempotent, notions_of, op_before_after, parse_base,
    validate_ops_idempotent,
};
use telos_core::payload::{intent_from_json, notion_from_json, patch_intent};
use telos_core::workspace::Workspace;

// --- fixture plumbing ------------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

#[test]
fn apply_config_ops_uses_the_last_typed_edit_and_normalizes_sets() {
    let base = Config::default();
    let first = Config {
        code: Globs {
            globs: vec!["b/**/*.rs".into(), "a/**/*.rs".into()],
        },
        tests: Globs::default(),
        test: TestCfg {
            cmd: "first".into(),
        },
        policy: Policy {
            tdd: TddPolicy::Advisory,
        },
        gherkin: GherkinCfg::default(),
        agents: AgentsCfg::default(),
    };
    let second = Config {
        code: Globs {
            globs: vec!["src/**/*.rs".into(), "src/**/*.rs".into()],
        },
        tests: Globs {
            globs: vec!["tests/**/*.rs".into()],
        },
        test: TestCfg {
            cmd: "second".into(),
        },
        policy: Policy {
            tdd: TddPolicy::Strict,
        },
        gherkin: GherkinCfg::default(),
        agents: AgentsCfg::default(),
    };

    let effective = apply_config_ops(
        &base,
        &[StagedOp::EditConfig(first), StagedOp::EditConfig(second)],
    );

    assert_eq!(effective.code.globs, ["src/**/*.rs"]);
    assert_eq!(effective.tests.globs, ["tests/**/*.rs"]);
    assert_eq!(effective.test.cmd, "second");
    assert_eq!(effective.policy.tdd, TddPolicy::Strict);
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
            TelFile::OwnedIntent { intent, .. } | TelFile::Intent(intent) if intent.id == id => {
                Some(intent.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the corpus holds {id}"))
}

fn notion(name: &str) -> NotionName {
    NotionName::new(name).unwrap()
}

fn billing_owner() -> Owner {
    Owner::context(ContextId::new("billing").unwrap())
}

fn invoicing_owner() -> Owner {
    Owner::capability(CapabilityRef::new(
        ContextId::new("billing").unwrap(),
        CapabilityId::new("invoicing").unwrap(),
    ))
}

fn settlement_owner() -> Owner {
    Owner::capability(CapabilityRef::new(
        ContextId::new("billing").unwrap(),
        CapabilityId::new("settlement").unwrap(),
    ))
}

/// The `Refund` event notion, absent from the corpus: what an `add notion`
/// op stages.
fn refund_notion() -> Notion {
    notion_from_json(&json!({
        "name": "Refund", "kind": "event", "def": "Money went back to a Customer.",
        "phrase": "refund is issued",
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
            "telos/context-map.tel",
            "telos/contexts/billing/bindings.tel",
            "telos/contexts/billing/capabilities/invoicing/capability.tel",
            "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel",
            "telos/contexts/billing/capabilities/invoicing/notions/InvoiceIssued.tel",
            "telos/contexts/billing/capabilities/settlement/capability.tel",
            "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel",
            "telos/contexts/billing/capabilities/settlement/notions/PaymentReceived.tel",
            "telos/contexts/billing/constraints/CON-0003.tel",
            "telos/contexts/billing/context.tel",
            "telos/contexts/billing/notions/Customer.tel",
            "telos/contexts/billing/notions/Invoice.tel",
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
    let invoice = match find(&base, "telos/contexts/billing/notions/Invoice.tel").unwrap() {
        TelFile::OwnedNotion { notion, .. } => notion,
        other => panic!("expected a notion, got {other:?}"),
    };

    let err = apply_ops(
        base,
        &[StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: invoice,
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(err.message, "notion `billing/Invoice` already exists");
}

#[test]
fn apply_ops_rejects_adding_an_intent_that_already_exists() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let existing = intent_of(&base, IntentId(17));

    let err = apply_ops(
        base,
        &[StagedOp::AddOwnedIntent {
            owner: invoicing_owner(),
            intent: existing,
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(err.message, "intent INT-0017 already exists");
}

#[test]
fn apply_ops_adds_an_absent_notion_at_its_own_path() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let overlay = apply_ops(
        base,
        &[StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        }],
    )
    .unwrap();

    assert_eq!(
        find(&overlay, "telos/contexts/billing/notions/Refund.tel"),
        Some(TelFile::OwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        })
    );
}

// --- apply_ops: edit / remove of something absent ---------------------------

#[test]
fn apply_ops_rejects_editing_a_notion_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut ghost = refund_notion();
    ghost.name = notion("Invoce");

    let err = apply_ops(
        base,
        &[StagedOp::EditOwnedNotion {
            owner: billing_owner(),
            notion: ghost,
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        err.message,
        "unknown notion `billing/Invoce`; closest is `billing/Invoice`"
    );
}

#[test]
fn apply_ops_rejects_removing_an_intent_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(
        base,
        &[StagedOp::RemoveOwnedIntent {
            owner: settlement_owner(),
            id: IntentId(9999),
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown intent `INT-9999`");
}

#[test]
fn apply_ops_rejects_removing_a_constraint_the_base_does_not_hold() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(
        base,
        &[StagedOp::RemoveOwnedConstraint {
            owner: Some(billing_owner()),
            id: ConstraintId(9),
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(
        err.message,
        "unknown constraint `CON-0009`; closest is `CON-0003`"
    );
}

// --- apply_ops: the referential-deletion check, removing something still referenced -----------------

/// A referenced entity cannot be removed; the error names its referrer:
/// the referrer is named, so the caller knows what to fix first.
#[test]
fn apply_ops_refuses_to_remove_an_intent_another_intent_requires() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let err = apply_ops(
        base,
        &[StagedOp::RemoveOwnedIntent {
            owner: invoicing_owner(),
            id: IntentId(17),
        }],
    )
    .unwrap_err();

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

    let err = apply_ops(
        base,
        &[StagedOp::RemoveOwnedNotion {
            owner: billing_owner(),
            name: notion("Invoice"),
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        err.message,
        "cannot remove notion `billing/Invoice`: INT-0017 uses it"
    );
}

#[test]
fn apply_ops_refuses_to_remove_an_intent_a_binding_implements() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    // INT-0042 is required by nobody, but `telos/contexts/billing/bindings.tel` implements it.
    let err = apply_ops(
        base,
        &[StagedOp::RemoveOwnedIntent {
            owner: settlement_owner(),
            id: IntentId(42),
        }],
    )
    .unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        err.message,
        "cannot remove intent INT-0042: `src/billing/invoice.rs` implements it"
    );
}

/// The referrers are computed from the state *after* the removal, so a
/// change that removes the referrer first and the referred-to entity second
/// is allowed -- order is data.
#[test]
fn apply_ops_allows_removing_an_intent_once_its_referrers_are_gone() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut bindings = base.clone();
    // `bindings.tel` is not an entity op's target, so drop the two bindings
    // by hand: this test is about the intent-to-intent reference.
    bindings.retain(|(path, _)| path.as_str() != "telos/contexts/billing/bindings.tel");

    let overlay = apply_ops(
        bindings,
        &[
            StagedOp::RemoveOwnedIntent {
                owner: settlement_owner(),
                id: IntentId(42),
            },
            StagedOp::RemoveOwnedIntent {
                owner: invoicing_owner(),
                id: IntentId(17),
            },
        ],
    )
    .unwrap();

    assert_eq!(
        find(
            &overlay,
            "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"
        ),
        None
    );
    assert_eq!(
        find(
            &overlay,
            "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel"
        ),
        None
    );
}

/// Nothing in the model points at a constraint, so removing one is never
/// blocked by the referential-deletion check.
#[test]
fn apply_ops_removes_a_constraint_nothing_can_reference() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);

    let overlay = apply_ops(
        base,
        &[StagedOp::RemoveOwnedConstraint {
            owner: Some(billing_owner()),
            id: ConstraintId(3),
        }],
    )
    .unwrap();

    assert_eq!(
        find(&overlay, "telos/contexts/billing/constraints/CON-0003.tel"),
        None
    );
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
            StagedOp::AddOwnedNotion {
                owner: billing_owner(),
                notion: refund_notion(),
            },
            StagedOp::EditOwnedNotion {
                owner: billing_owner(),
                notion: edited.clone(),
            },
        ],
    )
    .unwrap();

    assert_eq!(
        find(&overlay, "telos/contexts/billing/notions/Refund.tel"),
        Some(TelFile::OwnedNotion {
            owner: billing_owner(),
            notion: edited,
        })
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

// --- apply_ops_idempotent ---------------------------------------------------
//
// The second application: each op puts its post-state at its target
// path and refuses nothing, so it can be replayed over a tree that already
// shows it -- which is exactly what `adopt` produces and what `reconcile`
// then has to validate. These tests pin the two halves of that: that it
// agrees with `apply_ops` wherever `apply_ops` applies at all, and that it
// is a no-op where `apply_ops` would refuse.

/// The delta `adopt` produces for a hand-created file and a hand-deleted
/// one, in one plan.
fn adopted_delta() -> [StagedOp; 2] {
    [
        StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        },
        StagedOp::RemoveOwnedConstraint {
            owner: Some(billing_owner()),
            id: ConstraintId(3),
        },
    ]
}

/// Where both apply, both produce the same spec: the lenient application is
/// not a *different* overlay, only a more tolerant way of reaching the same
/// one.
#[test]
fn apply_ops_idempotent_agrees_with_apply_ops_wherever_both_apply() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = adopted_delta();

    let strict = apply_ops(base.clone(), &ops).unwrap();

    assert_eq!(apply_ops_idempotent(base, &ops), strict);
}

/// The property the whole split rests on: replaying a delta over a tree that
/// already shows it changes nothing. `adopt` stages ops describing the disk,
/// and `reconcile` re-applies them to that same disk -- so if this were not a
/// no-op, an adopted change could not be reconciled at all.
#[test]
fn apply_ops_idempotent_is_a_no_op_on_an_already_applied_delta() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = adopted_delta();

    let once = apply_ops_idempotent(base, &ops);
    let twice = apply_ops_idempotent(once.clone(), &ops);

    assert_eq!(once, twice);
    // And `once` really is the post-state, not the base: the added notion is
    // there, the removed constraint is gone.
    assert_eq!(
        find(&once, "telos/contexts/billing/notions/Refund.tel"),
        Some(TelFile::OwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        })
    );
    assert_eq!(
        find(&once, "telos/contexts/billing/constraints/CON-0003.tel"),
        None
    );

    // The contrast that justifies the second function at all: the strict
    // application refuses the very same replay.
    let refused = apply_ops(once, &ops).unwrap_err();
    assert_eq!(refused.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(refused.message, "notion `billing/Refund` already exists");
}

// --- validate_ops_idempotent ------------------------------------------------

/// The point of the overlay: an intent may name a notion that exists only
/// because an earlier op of the same change added it.
#[test]
fn validate_ops_idempotent_accepts_a_notion_and_an_intent_that_uses_it() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(
        base,
        &[StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        }],
    )
    .unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    let model = validate_ops_idempotent(
        &ws,
        &[
            StagedOp::AddOwnedNotion {
                owner: billing_owner(),
                notion: refund_notion(),
            },
            StagedOp::AddOwnedIntent {
                owner: settlement_owner(),
                intent,
            },
        ],
    )
    .unwrap_or_else(|diags| panic!("expected the overlay to validate, got {diags:?}"));

    assert!(model.notions.contains_key(&notion("Refund")));
    assert!(model.intents.contains_key(&IntentId(43)));
}

/// What the validation judges is the spec the *whole* delta describes, not
/// each intermediate state: the semantic pass runs once, on the overlay the
/// last op leaves behind. So these two ops validate in either order.
///
/// Staging them in the wrong order is still impossible, but that is a
/// different gate and a different layer: `telos add intent` resolves its
/// scenario fields against the notions the change holds *so far*
/// (`notions_of` over the ops staged before it), so the intent could not
/// even be built before its notion was staged.
#[test]
fn validate_ops_idempotent_judges_the_final_overlay_not_each_intermediate_state() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(
        base,
        &[StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        }],
    )
    .unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    let model = validate_ops_idempotent(
        &ws,
        &[
            StagedOp::AddOwnedIntent {
                owner: settlement_owner(),
                intent,
            },
            StagedOp::AddOwnedNotion {
                owner: billing_owner(),
                notion: refund_notion(),
            },
        ],
    )
    .unwrap_or_else(|diags| panic!("expected the overlay to validate, got {diags:?}"));

    assert!(model.notions.contains_key(&notion("Refund")));
}

/// A reference nothing in the overlay resolves is the semantic pass'
/// diagnostic, reported against the file the staged entity would be
/// written to.
#[test]
fn validate_ops_idempotent_reports_a_dangling_reference_in_a_staged_op() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let mut alloc = corpus_alloc(&ws);
    let staged = apply_ops(
        base,
        &[StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        }],
    )
    .unwrap();
    let intent = refund_intent(&notions_of(&staged), &mut alloc);

    // The notion the intent needs is never staged.
    let diagnostics = validate_ops_idempotent(
        &ws,
        &[StagedOp::AddOwnedIntent {
            owner: settlement_owner(),
            intent,
        }],
    )
    .unwrap_err();

    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::TelosReferenceUnknown
                && d.message.contains("unknown notion `Refund`")),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(RepoPath::as_str),
        Some("telos/contexts/billing/capabilities/settlement/intents/INT-0043.tel")
    );
}

/// **The safety property of the whole idempotent path**, pinned rather than
/// argued: `apply_ops_idempotent` cannot enforce the referential-deletion check (it refuses
/// nothing), so a removal that leaves a referrer dangling has to be caught by
/// the semantic pass instead. It is -- with a different, file-located
/// message, and the same refusal.
///
/// `INT-0017` is required by `INT-0042`, which is exactly the case
/// `apply_ops` reports as `` cannot remove intent INT-0017: INT-0042
/// requires it ``. Both are asserted here, side by side, so that a change to
/// either path has to face the comparison.
#[test]
fn validate_ops_idempotent_catches_a_rule_2_violation_as_an_unresolvable_reference() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::RemoveOwnedIntent {
        owner: invoicing_owner(),
        id: IntentId(17),
    }];

    // The staging path: refused, naming the referrer.
    let strict = apply_ops(base, &ops).unwrap_err();
    assert_eq!(strict.code, ErrorCode::TelosIntegrityViolation);
    assert_eq!(
        strict.message,
        "cannot remove intent INT-0017: INT-0042 requires it"
    );

    // The whole-change path: refused too, by the reference INT-0042 is left
    // holding.
    let diagnostics = validate_ops_idempotent(&ws, &ops).unwrap_err();
    assert_eq!(codes(&diagnostics), [ErrorCode::TelosReferenceUnknown]);
    assert!(
        diagnostics[0].message.contains("INT-0017"),
        "the diagnostic must name the reference that no longer resolves: {:?}",
        diagnostics[0]
    );
    assert_eq!(
        diagnostics[0].file.as_ref().map(RepoPath::as_str),
        Some("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel"),
        "and locate it in the file that still points at it"
    );
}

#[test]
fn validate_ops_idempotent_with_no_op_is_the_sealed_model() {
    let (_tmp, ws) = corpus();

    let model = validate_ops_idempotent(&ws, &[]).unwrap();

    assert_eq!(model.intents.len(), 2);
    assert_eq!(model.notions.len(), 4);
}

// --- op_before_after --------------------------------------------------------

#[test]
fn op_before_after_of_an_add_has_no_before() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::AddOwnedNotion {
        owner: billing_owner(),
        notion: refund_notion(),
    }];

    let (before, after) = op_before_after(&base, &ops, 0);

    assert_eq!(before, None);
    assert_eq!(
        after,
        Some(telos_core::emit::emit_owned_notion(
            &billing_owner(),
            &refund_notion(),
        ))
    );
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
    let ops = [StagedOp::EditOwnedIntent {
        owner: invoicing_owner(),
        intent: patched.clone(),
    }];

    let (before, after) = op_before_after(&base, &ops, 0);

    assert_eq!(
        before,
        Some(
            fs::read_to_string(
                ws.repo_root
                    .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel")
            )
            .unwrap()
        )
    );
    assert_eq!(
        after,
        Some(telos_core::emit::emit_owned_intent(
            &invoicing_owner(),
            &patched,
        ))
    );
    assert!(after.unwrap().contains("Reworded."));
}

#[test]
fn op_before_after_of_a_remove_has_no_after() {
    let (_tmp, ws) = corpus();
    let base = base_of(&ws);
    let ops = [StagedOp::RemoveOwnedConstraint {
        owner: Some(billing_owner()),
        id: ConstraintId(3),
    }];

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
        StagedOp::AddOwnedNotion {
            owner: billing_owner(),
            notion: refund_notion(),
        },
        StagedOp::EditOwnedNotion {
            owner: billing_owner(),
            notion: edited.clone(),
        },
    ];

    let (before, after) = op_before_after(&base, &ops, 1);

    assert_eq!(
        before,
        Some(telos_core::emit::emit_owned_notion(
            &billing_owner(),
            &refund_notion(),
        ))
    );
    assert_eq!(
        after,
        Some(telos_core::emit::emit_owned_notion(
            &billing_owner(),
            &edited,
        ))
    );
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
