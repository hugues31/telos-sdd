//! Changes: the staged, reviewable unit of spec mutation (D1, Annex C).
//!
//! A change is *not* part of [`super::TelFile`]. It is its own kind of file,
//! living under `telos/changes/` -- a directory deliberately excluded from
//! [`crate::workspace::Workspace::spec_files`], so a change never enters the
//! seal and never contributes to the spec model. It is a transaction record:
//! an ordered list of [`StagedOp`]s plus enough state to decide whether that
//! list may still be applied.
//!
//! Three properties carry the whole design:
//!
//! - **Order is data** (Annex C). `ops` is a `Vec`, never sorted -- the ops
//!   replay sequentially against the sealed base, so `add` then `edit` of
//!   the same entity is a different transaction from `edit` then `add`.
//! - **The digest is over the canonical ops, not over the file** (D3).
//!   [`Change::ops_digest`] hashes `emit_op` output, so the blank lines the
//!   change file puts *between* ops, and the motivation line above them,
//!   cannot move it. What can move it is any edit to an op's content or to
//!   the ops' order -- which is exactly what an approval is an approval of.
//! - **Claims are paths** (D5). [`Change::claims`] is the set of files this
//!   change owns while it is open; a second change may not stage them.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::emit::emit_op;
use crate::git::Oid;
use crate::ids::{ChangeId, ConstraintId, IntentId, NotionName, RepoPath};

use super::constraint::Constraint;
use super::intent::Intent;
use super::notion::Notion;

/// Where a change sits in the lifecycle of D16.
///
/// `open` (no op yet) advances to `drafted` automatically on the first
/// stage, to `approved` by `telos change approve`, and to `implementing`
/// in M3. Both terminal outcomes -- reconciled and abandoned -- delete the
/// file rather than storing a status, so `Abandoned` is only ever observed
/// in flight; the audit trail is git, and D4's counters guarantee the id is
/// never handed out again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Open,
    Drafted,
    Approved,
    Implementing,
    Abandoned,
}

impl ChangeStatus {
    /// The keyword this status is written as in a change file, and reported
    /// as in CLI JSON (Annex E).
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeStatus::Open => "open",
            ChangeStatus::Drafted => "drafted",
            ChangeStatus::Approved => "approved",
            ChangeStatus::Implementing => "implementing",
            ChangeStatus::Abandoned => "abandoned",
        }
    }
}

/// One staged operation: the post-state of one spec file, or the acceptance
/// of one non-entity file's current bytes.
///
/// `Edit` carries the *complete* post-state rather than a patch (Annex C):
/// replaying a change never needs the base to reconstruct what an entity
/// became, only to check that what it was is still what it was.
///
/// `Accept` is the escape hatch for files the engine does not model --
/// `telos/telos.toml`, and code files pulled in by `adopt` (D7). It seals a
/// specific blob OID: reconciling it means "the current bytes at this path
/// are the intended bytes".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedOp {
    AddNotion(Notion),
    EditNotion(Notion),
    RemoveNotion(NotionName),
    AddIntent(Intent),
    EditIntent(Intent),
    RemoveIntent(IntentId),
    AddConstraint(Constraint),
    EditConstraint(Constraint),
    RemoveConstraint(ConstraintId),
    Accept { path: RepoPath, oid: Oid },
}

impl StagedOp {
    /// The one file this op writes, deletes or seals -- its claim (D5).
    ///
    /// Every entity op derives its path from the entity's identity, not
    /// from where the file happens to be: an entity's location in the spec
    /// tree is a function of its kind and its id, so two ops on the same
    /// entity always collide on the same claim.
    pub fn target_path(&self) -> RepoPath {
        match self {
            StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => notion_path(&n.name),
            StagedOp::RemoveNotion(name) => notion_path(name),
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => intent_path(i.id),
            StagedOp::RemoveIntent(id) => intent_path(*id),
            StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => constraint_path(c.id),
            StagedOp::RemoveConstraint(id) => constraint_path(*id),
            StagedOp::Accept { path, .. } => path.clone(),
        }
    }

    /// The op's verb, as written and as reported (Annex E `change diff`).
    pub fn verb(&self) -> &'static str {
        match self {
            StagedOp::AddNotion(_) | StagedOp::AddIntent(_) | StagedOp::AddConstraint(_) => "add",
            StagedOp::EditNotion(_) | StagedOp::EditIntent(_) | StagedOp::EditConstraint(_) => {
                "edit"
            }
            StagedOp::RemoveNotion(_)
            | StagedOp::RemoveIntent(_)
            | StagedOp::RemoveConstraint(_) => "remove",
            StagedOp::Accept { .. } => "accept",
        }
    }

    /// What kind of thing the op acts on. `Accept` reports `"file"`: it is
    /// the one op whose target is a path rather than a modelled entity.
    pub fn entity(&self) -> &'static str {
        match self {
            StagedOp::AddNotion(_) | StagedOp::EditNotion(_) | StagedOp::RemoveNotion(_) => {
                "notion"
            }
            StagedOp::AddIntent(_) | StagedOp::EditIntent(_) | StagedOp::RemoveIntent(_) => {
                "intent"
            }
            StagedOp::AddConstraint(_)
            | StagedOp::EditConstraint(_)
            | StagedOp::RemoveConstraint(_) => "constraint",
            StagedOp::Accept { .. } => "file",
        }
    }

    /// The target's identity as a string: a notion name, an entity id, or --
    /// for `Accept`, which has no id -- the path itself.
    pub fn key(&self) -> String {
        match self {
            StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => n.name.to_string(),
            StagedOp::RemoveNotion(name) => name.to_string(),
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => i.id.to_string(),
            StagedOp::RemoveIntent(id) => id.to_string(),
            StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => c.id.to_string(),
            StagedOp::RemoveConstraint(id) => id.to_string(),
            StagedOp::Accept { path, .. } => path.to_string(),
        }
    }
}

fn notion_path(name: &NotionName) -> RepoPath {
    RepoPath::new(format!("telos/notions/{name}.tel"))
}

fn intent_path(id: IntentId) -> RepoPath {
    RepoPath::new(format!("telos/intents/{id}.tel"))
}

fn constraint_path(id: ConstraintId) -> RepoPath {
    RepoPath::new(format!("telos/constraints/{id}.tel"))
}

/// One staged transaction over the spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: ChangeId,
    pub motivation: String,
    pub status: ChangeStatus,
    /// The digest frozen by `approve`. `Some` exactly when the status is
    /// `Approved` (or, in M3, `Implementing`); comparing it against a fresh
    /// [`Change::ops_digest`] is what detects a delta edited after review.
    pub approved_digest: Option<String>,
    /// Staged order, never sorted: the order is part of the transaction.
    pub ops: Vec<StagedOp>,
}

impl Change {
    /// `"sha256:" + hex(SHA-256(emit_op(op) for every op, in staged order))`
    /// (D3).
    ///
    /// The unit of input is one op's canonical bytes, concatenated with no
    /// separator -- each `emit_op` output already ends in exactly one `\n`,
    /// so ops cannot run together and no delimiter needs inventing. The
    /// blank lines `emit_change` writes *between* ops are not part of any
    /// op, so reformatting the file's separators cannot move the digest,
    /// while editing an op's content or swapping two ops always does.
    ///
    /// The canonical form is therefore load-bearing here: were it ever to
    /// change, every prior approval would be invalidated mechanically. That
    /// is the intended behaviour, not a side effect.
    pub fn ops_digest(&self) -> String {
        let mut hasher = Sha256::new();
        for op in &self.ops {
            hasher.update(emit_op(op).as_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Whether the delta moved since it was approved.
    ///
    /// An unapproved change is never stale -- there is nothing to be stale
    /// *against*. Reconcile refuses a stale change with
    /// `TELOS_APPROVAL_STALE`.
    pub fn is_stale(&self) -> bool {
        match &self.approved_digest {
            Some(approved) => *approved != self.ops_digest(),
            None => false,
        }
    }

    /// The set of files this change owns while it lives (D5).
    ///
    /// A `BTreeSet` both deduplicates (two ops on the same entity claim it
    /// once) and orders, so the claim list a CLI prints or a conflict
    /// message names is stable regardless of staging order.
    pub fn claims(&self) -> BTreeSet<RepoPath> {
        self.ops.iter().map(StagedOp::target_path).collect()
    }

    /// What remains to be done, as the frozen strings of Annex E.
    ///
    /// These are contract, not prose: `status --json` reports them verbatim
    /// and T13 freezes them in `contracts.md`. `Abandoned` has none -- an
    /// abandoned change is deleted, so nothing is owed.
    pub fn obligations(&self) -> Vec<String> {
        let steps: &[&str] = match self.status {
            ChangeStatus::Open => &["stage the delta", "approve", "reconcile"],
            ChangeStatus::Drafted => &["approve", "reconcile"],
            ChangeStatus::Approved | ChangeStatus::Implementing => &["reconcile"],
            ChangeStatus::Abandoned => &[],
        };
        steps.iter().map(|s| (*s).to_string()).collect()
    }
}

/// The Annex C canonical example, as a model *and* as its canonical bytes.
///
/// It lives outside `mod tests` so that the rest of the crate's tests can
/// reach it: the byte-exact golden is what `emit.rs` must produce and what
/// `syntax/parser.rs` must read back, the `claims`/digest assertions belong
/// here, and all three must be talking about the same change or none of
/// them proves anything about the others. Hence one copy of each, here.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::ids::{FieldName, ScenarioId};
    use crate::model::{
        Action, Attr, AttrRef, AttrType, CmpOp, ConstraintKind, Expr, InstanceStep, IntentStatus,
        Literal, NotionKind, Operand, Rule, Scenario, Scope, Statement,
    };
    use crate::span::{Sp, Span};

    /// Wraps a node in the zero span: nothing here was parsed from a file,
    /// and spans take no part in emission or equality of the model.
    pub(crate) fn sp<T>(node: T) -> Sp<T> {
        Sp {
            node,
            span: Span::default(),
        }
    }

    pub(crate) fn notion_name(s: &str) -> NotionName {
        NotionName::new(s).unwrap()
    }

    pub(crate) fn field(s: &str) -> FieldName {
        FieldName::new(s).unwrap()
    }

    fn attr_ref(notion: &str, attr: &str) -> AttrRef {
        AttrRef {
            notion: sp(notion_name(notion)),
            attr: sp(field(attr)),
        }
    }

    /// The notion of the example's `op add`: a trimmed-down `Invoice`.
    pub(crate) fn invoice() -> Notion {
        Notion {
            name: notion_name("Invoice"),
            kind: NotionKind::Entity,
            def: "A bill issued to a Customer for delivered work.".to_string(),
            attrs: vec![Attr {
                name: field("state"),
                ty: AttrType::Enum(vec!["open".to_string(), "settled".to_string()]),
            }],
            rels: vec![],
        }
    }

    /// The intent of the example's `op edit`: the corpus' INT-0017 with its
    /// `telos` reworded.
    pub(crate) fn int_0017() -> Intent {
        Intent {
            id: IntentId(17),
            title: "Issuing an invoice opens it".to_string(),
            status: IntentStatus::Active,
            telos: "An invoice must start its life open and unpaid -- reworded.".to_string(),
            statement: Statement::EventDriven {
                event: sp(notion_name("InvoiceIssued")),
                on: Some(sp(notion_name("Invoice"))),
                action: Action::Set {
                    target: attr_ref("Invoice", "state"),
                    value: Literal::Symbol(sp("open".to_string())),
                },
            },
            refines: vec![],
            requires: vec![],
            excludes: vec![],
            scenarios: vec![Scenario {
                id: ScenarioId(91),
                title: "a newly issued invoice is open".to_string(),
                given: vec![InstanceStep {
                    notion: sp(notion_name("Customer")),
                    fields: vec![(sp(field("name")), Literal::Str("ACME".to_string()))],
                }],
                when: InstanceStep {
                    notion: sp(notion_name("InvoiceIssued")),
                    fields: vec![],
                },
                then: vec![Expr::Cmp {
                    op: CmpOp::Eq,
                    lhs: Operand::Ref(attr_ref("Invoice", "state")),
                    rhs: Operand::Lit(Literal::Symbol(sp("open".to_string()))),
                }],
            }],
        }
    }

    pub(crate) fn con_0003() -> Constraint {
        Constraint {
            id: ConstraintId(3),
            kind: ConstraintKind::Architecture,
            title: "Hexagonal boundaries".to_string(),
            rule: Rule::Text("No adapter imports a domain module.".to_string()),
            scope: Scope::Global,
            check: None,
        }
    }

    /// The four ops of the Annex C example, in staged order.
    pub(crate) fn annex_c_ops() -> Vec<StagedOp> {
        vec![
            StagedOp::AddNotion(invoice()),
            StagedOp::EditIntent(int_0017()),
            StagedOp::RemoveConstraint(ConstraintId(3)),
            StagedOp::Accept {
                path: RepoPath::new("telos/telos.toml"),
                oid: Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
            },
        ]
    }

    /// The Annex C example. Its `digest` is the annex' placeholder, not a
    /// real `ops_digest()` -- the golden pins the *layout* of the digest
    /// line, and a self-referential digest would make the golden move every
    /// time the fixture does.
    pub(crate) fn annex_c_change() -> Change {
        Change {
            id: ChangeId(7),
            motivation: "Invoices can be settled".to_string(),
            status: ChangeStatus::Approved,
            approved_digest: Some(
                "sha256:9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0"
                    .to_string(),
            ),
            ops: annex_c_ops(),
        }
    }

    /// The canonical example of Annex C, byte for byte: what
    /// `emit_change(&annex_c_change())` writes, and what
    /// `parse_change_file` reads back into `annex_c_change()`.
    ///
    /// It differs from the example as printed in the annex in exactly two
    /// tokens, both of which the annex' sketch dropped and neither of which
    /// the emitter may: the notion's kind (`entity`) and the intent's
    /// title. `entity-decl` is the M1 `notion-file` / `intent-file` grammar
    /// nested verbatim (Annex C), and both tokens are mandatory there -- an
    /// `op add notion Invoice {` carries no kind, so replaying it could not
    /// write a valid notion file, and D1's byte-level round-trip could not
    /// hold. The values used here are the corpus' own:
    /// `crates/telos-core/tests/corpus/billing`, which is visibly where the
    /// annex drew the example from.
    pub(crate) const ANNEX_C_EXAMPLE: &str = r#"change CHG-0007 "Invoices can be settled" {
  status approved
  digest "sha256:9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0"

  op add notion Invoice entity {
    def  "A bill issued to a Customer for delivered work."
    attr state enum(open, settled)
  }

  op edit intent INT-0017 "Issuing an invoice opens it" {
    status active
    telos  "An invoice must start its life open and unpaid -- reworded."
    statement event-driven {
      when   InvoiceIssued on Invoice
      system shall set Invoice.state = open
    }

    scenario SCN-0091 "a newly issued invoice is open" {
      given Customer { name: "ACME" }
      when  InvoiceIssued {}
      then  Invoice.state == open
    }
  }

  op remove constraint CON-0003

  op accept "telos/telos.toml" "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
}
"#;

    /// A change with no op: the state `telos change open` leaves behind.
    pub(crate) fn empty_change() -> Change {
        Change {
            id: ChangeId(1),
            motivation: "x".to_string(),
            status: ChangeStatus::Open,
            approved_digest: None,
            ops: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    // --- ChangeStatus ----------------------------------------------------

    #[test]
    fn as_str_names_every_status() {
        assert_eq!(ChangeStatus::Open.as_str(), "open");
        assert_eq!(ChangeStatus::Drafted.as_str(), "drafted");
        assert_eq!(ChangeStatus::Approved.as_str(), "approved");
        assert_eq!(ChangeStatus::Implementing.as_str(), "implementing");
        assert_eq!(ChangeStatus::Abandoned.as_str(), "abandoned");
    }

    // --- StagedOp descriptors --------------------------------------------

    fn accept_op() -> StagedOp {
        StagedOp::Accept {
            path: RepoPath::new("telos/telos.toml"),
            oid: Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
        }
    }

    /// Every variant, with the `(target_path, verb, entity, key)` tuple it
    /// must report. One table so a new variant cannot be added without a row.
    fn every_variant() -> Vec<(
        StagedOp,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    )> {
        vec![
            (
                StagedOp::AddNotion(invoice()),
                "telos/notions/Invoice.tel",
                "add",
                "notion",
                "Invoice",
            ),
            (
                StagedOp::EditNotion(invoice()),
                "telos/notions/Invoice.tel",
                "edit",
                "notion",
                "Invoice",
            ),
            (
                StagedOp::RemoveNotion(notion_name("Invoice")),
                "telos/notions/Invoice.tel",
                "remove",
                "notion",
                "Invoice",
            ),
            (
                StagedOp::AddIntent(int_0017()),
                "telos/intents/INT-0017.tel",
                "add",
                "intent",
                "INT-0017",
            ),
            (
                StagedOp::EditIntent(int_0017()),
                "telos/intents/INT-0017.tel",
                "edit",
                "intent",
                "INT-0017",
            ),
            (
                StagedOp::RemoveIntent(IntentId(42)),
                "telos/intents/INT-0042.tel",
                "remove",
                "intent",
                "INT-0042",
            ),
            (
                StagedOp::AddConstraint(con_0003()),
                "telos/constraints/CON-0003.tel",
                "add",
                "constraint",
                "CON-0003",
            ),
            (
                StagedOp::EditConstraint(con_0003()),
                "telos/constraints/CON-0003.tel",
                "edit",
                "constraint",
                "CON-0003",
            ),
            (
                StagedOp::RemoveConstraint(ConstraintId(3)),
                "telos/constraints/CON-0003.tel",
                "remove",
                "constraint",
                "CON-0003",
            ),
            (
                accept_op(),
                "telos/telos.toml",
                "accept",
                "file",
                "telos/telos.toml",
            ),
        ]
    }

    #[test]
    fn every_variant_reports_its_target_path_verb_entity_and_key() {
        for (op, path, verb, entity, key) in every_variant() {
            assert_eq!(op.target_path(), RepoPath::new(path), "target_path {op:?}");
            assert_eq!(op.verb(), verb, "verb {op:?}");
            assert_eq!(op.entity(), entity, "entity {op:?}");
            assert_eq!(op.key(), key, "key {op:?}");
        }
    }

    #[test]
    fn target_path_follows_the_entity_identity_not_the_op_verb() {
        // add / edit / remove of the same notion all claim one file, which
        // is what makes two ops on one entity collide (D5).
        let add = StagedOp::AddNotion(invoice()).target_path();
        let edit = StagedOp::EditNotion(invoice()).target_path();
        let remove = StagedOp::RemoveNotion(notion_name("Invoice")).target_path();
        assert_eq!(add, edit);
        assert_eq!(edit, remove);
    }

    // --- ops_digest ------------------------------------------------------

    fn is_sha256_hex(digest: &str) -> bool {
        match digest.strip_prefix("sha256:") {
            Some(hex) => {
                hex.len() == 64
                    && hex
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            }
            None => false,
        }
    }

    #[test]
    fn ops_digest_is_a_lowercase_sha256_hex_string() {
        let digest = annex_c_change().ops_digest();
        assert!(
            is_sha256_hex(&digest),
            "not `sha256:<64 lowercase hex>`: {digest}"
        );
    }

    #[test]
    fn ops_digest_of_no_ops_is_still_well_formed() {
        // The SHA-256 of the empty input -- a change with no op has a
        // digest, it is just the empty one.
        let digest = empty_change().ops_digest();
        assert!(
            is_sha256_hex(&digest),
            "not `sha256:<64 lowercase hex>`: {digest}"
        );
    }

    /// Pins D3's algorithm to a value, not just to a shape.
    ///
    /// Every assertion above would still pass if the digest hashed, say,
    /// the ops separated by a delimiter, or the whole change file. This one
    /// would not. It is deliberately brittle: the canonical form is what
    /// approvals are taken against, so a change to it must break a test
    /// here rather than silently revalidate deltas nobody re-reviewed.
    #[test]
    fn ops_digest_is_pinned_to_the_bytes_of_the_canonical_ops() {
        assert_eq!(
            annex_c_change().ops_digest(),
            "sha256:3c7b089eed526d2dead2aadd21095e1b099be1009ac99379db4795a70be2945d"
        );
        // No op means no byte hashed: the SHA-256 of the empty input.
        assert_eq!(
            empty_change().ops_digest(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn ops_digest_is_stable_across_recomputation() {
        let change = annex_c_change();
        assert_eq!(change.ops_digest(), change.ops_digest());
        // And across two independently built, equal changes.
        assert_eq!(annex_c_change().ops_digest(), change.ops_digest());
    }

    #[test]
    fn ops_digest_changes_when_an_op_content_changes() {
        let before = annex_c_change().ops_digest();

        let mut edited = annex_c_change();
        let StagedOp::AddNotion(notion) = &mut edited.ops[0] else {
            panic!("the first op of the example is `add notion`");
        };
        notion.def = "A bill.".to_string();

        assert_ne!(before, edited.ops_digest());
    }

    #[test]
    fn ops_digest_changes_when_two_ops_are_reordered() {
        let before = annex_c_change().ops_digest();

        let mut reordered = annex_c_change();
        reordered.ops.swap(0, 1);

        assert_ne!(before, reordered.ops_digest());
    }

    #[test]
    fn ops_digest_ignores_the_blank_lines_the_change_file_puts_between_ops() {
        // The digest is over `emit_op` output, never over the file: the
        // separators `emit_change` writes are outside every op's bytes.
        let change = annex_c_change();
        let concatenated: String = change.ops.iter().map(crate::emit::emit_op).collect();
        assert!(
            !concatenated.contains("\n\nop "),
            "op bytes must not carry the file's separating blank lines"
        );

        let mut hasher = Sha256::new();
        hasher.update(concatenated.as_bytes());
        assert_eq!(
            change.ops_digest(),
            format!("sha256:{:x}", hasher.finalize())
        );
    }

    #[test]
    fn ops_digest_does_not_depend_on_the_motivation_status_or_approved_digest() {
        let before = annex_c_change().ops_digest();

        let mut other = annex_c_change();
        other.motivation = "something else entirely".to_string();
        other.status = ChangeStatus::Drafted;
        other.approved_digest = None;

        assert_eq!(before, other.ops_digest());
    }

    // --- is_stale --------------------------------------------------------

    #[test]
    fn an_unapproved_change_is_never_stale() {
        assert!(!empty_change().is_stale());

        let mut drafted = annex_c_change();
        drafted.status = ChangeStatus::Drafted;
        drafted.approved_digest = None;
        assert!(!drafted.is_stale());
    }

    #[test]
    fn an_approved_change_whose_digest_matches_is_not_stale() {
        let mut change = annex_c_change();
        change.approved_digest = Some(change.ops_digest());
        assert!(!change.is_stale());
    }

    #[test]
    fn an_approved_change_goes_stale_when_an_op_is_edited() {
        let mut change = annex_c_change();
        change.approved_digest = Some(change.ops_digest());
        change.ops.push(StagedOp::RemoveIntent(IntentId(42)));
        assert!(change.is_stale());
    }

    // --- claims ----------------------------------------------------------

    #[test]
    fn claims_of_the_annex_example_are_its_four_target_paths() {
        let expected: BTreeSet<RepoPath> = [
            "telos/constraints/CON-0003.tel",
            "telos/intents/INT-0017.tel",
            "telos/notions/Invoice.tel",
            "telos/telos.toml",
        ]
        .into_iter()
        .map(RepoPath::new)
        .collect();
        assert_eq!(annex_c_change().claims(), expected);
    }

    #[test]
    fn claims_deduplicate_two_ops_on_the_same_entity() {
        let change = Change {
            id: ChangeId(1),
            motivation: "twice".to_string(),
            status: ChangeStatus::Drafted,
            approved_digest: None,
            ops: vec![
                StagedOp::AddNotion(invoice()),
                StagedOp::EditNotion(invoice()),
            ],
        };
        assert_eq!(
            change.claims(),
            [RepoPath::new("telos/notions/Invoice.tel")]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn a_change_with_no_op_claims_nothing() {
        assert!(empty_change().claims().is_empty());
    }

    // --- obligations -----------------------------------------------------

    #[test]
    fn obligations_are_the_frozen_strings_of_annex_e() {
        let for_status = |status| {
            let mut change = empty_change();
            change.status = status;
            change.obligations()
        };
        assert_eq!(
            for_status(ChangeStatus::Open),
            vec!["stage the delta", "approve", "reconcile"]
        );
        assert_eq!(
            for_status(ChangeStatus::Drafted),
            vec!["approve", "reconcile"]
        );
        assert_eq!(for_status(ChangeStatus::Approved), vec!["reconcile"]);
        assert_eq!(for_status(ChangeStatus::Implementing), vec!["reconcile"]);
        assert_eq!(for_status(ChangeStatus::Abandoned), Vec::<String>::new());
    }
}
