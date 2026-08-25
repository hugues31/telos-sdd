//! Gherkin rendering: one intent becomes one Cucumber `.feature` file.
//!
//! Pure, like [`crate::emit`] and [`crate::semantic`]: a function of the
//! model alone, with no filesystem and no `Workspace`. That is what lets a
//! caller render the same model to a temp directory for a test run and to
//! `telos/features/` for a seal and get byte-identical output.
//!
//! Two rules carry most of the design.
//!
//! **A phrase is never capitalized.** Every step reads `Given the ...`,
//! `When the ...`, `Then the ...`, so a notion's phrase is never
//! sentence-initial. `SLA` stays `SLA`.
//!
//! **A phrase carries no article; this module prepends `the `.** The
//! definite article is invariant, so the renderer never has to choose
//! between `a` and `an` -- a choice that is not mechanical for the acronyms
//! a domain model is full of (`a user`, `an SLA`, `an hour`, `a European`).
//! [`crate::semantic`] rejects a phrase that starts with an article, so the
//! two can never double up.

use std::collections::BTreeMap;

use crate::ids::{IntentId, NotionName, NotionRef, Owner, RepoPath};
use crate::model::{
    AttrRef, CmpOp, Expr, InstanceStep, Intent, Literal, Notion, Operand, Scenario, TelosModel,
};

/// `telos/features/<context>/<capability>/<INT-id>.feature`, or
/// `telos/features/<context>/<INT-id>.feature` for a context-owned intent.
pub fn feature_path(owner: &Owner, intent: IntentId) -> RepoPath {
    match &owner.capability {
        Some(capability) => RepoPath::new(format!(
            "telos/features/{}/{}/{intent}.feature",
            owner.context, capability
        )),
        None => RepoPath::new(format!("telos/features/{}/{intent}.feature", owner.context)),
    }
}

/// Renders every intent in `model` as a `.feature` file, keyed by the path it
/// belongs at.
///
/// An intent with no scenarios still renders: a `Feature` with a title and a
/// narrative and no `Scenario` blocks is valid Gherkin, and omitting the file
/// would make the set of rendered paths depend on scenario counts, which a
/// later phase seals.
pub fn render_features(model: &TelosModel) -> BTreeMap<RepoPath, String> {
    let mut out = BTreeMap::new();
    for (id, intent) in &model.intents {
        let Some(owner) = model.intent_owners.get(id) else {
            continue;
        };
        out.insert(
            feature_path(owner, *id),
            render_feature(model, owner, intent),
        );
    }
    out
}

fn render_feature(model: &TelosModel, owner: &Owner, intent: &Intent) -> String {
    let mut out = String::new();
    out.push_str(&format!("@{}\n", intent.id));
    out.push_str(&format!("Feature: {}\n", intent.title));
    out.push_str(&format!("  {}\n", intent.telos));

    for scenario in &intent.scenarios {
        out.push('\n');
        out.push_str(&render_scenario(model, owner, scenario));
    }
    out
}

fn render_scenario(model: &TelosModel, owner: &Owner, scenario: &Scenario) -> String {
    let mut out = String::new();
    out.push_str(&format!("  @{}\n", scenario.id));
    out.push_str(&format!("  Scenario: {}\n", scenario.title));

    let phrase = |name: &NotionName| notion_phrase(model, owner, name);

    for (index, step) in scenario.given.iter().enumerate() {
        let keyword = if index == 0 { "Given" } else { "And" };
        out.push_str(&format!("    {keyword} {}\n", instance(&phrase, step)));
    }
    out.push_str(&format!("    When {}\n", instance(&phrase, &scenario.when)));

    let mut clauses = Vec::new();
    for expr in &scenario.then {
        flatten_and(expr, &mut clauses);
    }
    for (index, clause) in clauses.iter().enumerate() {
        let keyword = if index == 0 { "Then" } else { "And" };
        out.push_str(&format!("    {keyword} {}\n", render_expr(&phrase, clause)));
    }
    out
}

/// `the invoice with state open and balance 120.00 EUR`, or just
/// `the invoice is issued` when the step carries no fields.
fn instance(phrase: &impl Fn(&NotionName) -> String, step: &InstanceStep) -> String {
    let head = format!("the {}", phrase(&step.notion.node));
    if step.fields.is_empty() {
        return head;
    }
    let fields: Vec<String> = step
        .fields
        .iter()
        .map(|(name, value)| format!("{} {}", name.node, literal(value)))
        .collect();
    format!("{head} with {}", join_and(&fields))
}

/// `And` in a `then` clause becomes a separate `And` step; `Or` stays inside
/// one step, because Gherkin has no disjunction between steps.
fn flatten_and<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    match expr {
        Expr::And(lhs, rhs) => {
            flatten_and(lhs, out);
            flatten_and(rhs, out);
        }
        other => out.push(other),
    }
}

fn render_expr(phrase: &impl Fn(&NotionName) -> String, expr: &Expr) -> String {
    match expr {
        Expr::Cmp { op, lhs, rhs } => format!(
            "{} {} {}",
            operand(phrase, lhs),
            comparison(*op),
            operand(phrase, rhs)
        ),
        Expr::In { lhs, set } => {
            let values: Vec<String> = set.iter().map(literal).collect();
            format!("{} is one of {}", operand(phrase, lhs), join_and(&values))
        }
        Expr::And(lhs, rhs) => format!(
            "{} and {}",
            render_expr(phrase, lhs),
            render_expr(phrase, rhs)
        ),
        Expr::Or(lhs, rhs) => format!(
            "{} or {}",
            render_expr(phrase, lhs),
            render_expr(phrase, rhs)
        ),
        Expr::Not(inner) => format!("it is not the case that {}", render_expr(phrase, inner)),
    }
}

fn operand(phrase: &impl Fn(&NotionName) -> String, operand: &Operand) -> String {
    match operand {
        Operand::Ref(AttrRef { notion, attr }) => {
            format!("the {} {}", phrase(&notion.node), attr.node)
        }
        Operand::Lit(value) => literal(value),
    }
}

fn comparison(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "is",
        CmpOp::Ne => "is not",
        CmpOp::Lt => "is less than",
        CmpOp::Le => "is at most",
        CmpOp::Gt => "is greater than",
        CmpOp::Ge => "is at least",
    }
}

/// Prose, not `.tel` syntax: a string literal loses its quotes, so
/// `"120.00 EUR"` reads as `120.00 EUR` inside a sentence. Decimal, date and
/// datetime re-emit their lexeme for the same reason
/// [`crate::emit::emit_literal`] does -- an amount never goes through a float.
fn literal(value: &Literal) -> String {
    match value {
        Literal::Str(s) => s.clone(),
        Literal::Int(n) => n.to_string(),
        Literal::Decimal(lexeme) | Literal::Date(lexeme) | Literal::Datetime(lexeme) => {
            lexeme.clone()
        }
        Literal::Bool(b) => b.to_string(),
        Literal::Symbol(s) => s.node.clone(),
    }
}

/// `a`, `a and b`, `a, b and c` -- the form a sentence wants.
fn join_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The surface form of a notion, resolved in the owning intent's context.
///
/// `build_model` has already resolved every notion a scenario names, so a
/// miss here means the model was not validated -- the same contract
/// [`crate::rebuild::plan`] relies on.
fn notion_phrase(model: &TelosModel, owner: &Owner, name: &NotionName) -> String {
    let reference = NotionRef::new(owner.context.clone(), name.clone());
    let notion: &Notion = model
        .domain_notions
        .get(&reference)
        .or_else(|| model.notions.get(name))
        .expect("a validated model resolves every notion a scenario names");
    notion.phrase.clone()
}
