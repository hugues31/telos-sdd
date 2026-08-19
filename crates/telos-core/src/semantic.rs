//! The semantic pass: folds parsed `.tel` files into a `TelosModel`, checks
//! the integrity rules of spec 3.3 that hold at read time, and derives the
//! relation graph.
//!
//! Three rules of 3.3 apply here -- every reference resolves (rule 1), an
//! `active` intent is verified by at least one scenario and a `when` step
//! names an event (rule 3), and literals agree with the attribute types they
//! meet (rule 4). Rules 2 and 5 govern *writing* and reconciling a spec, so
//! they belong to M2's change flow, not to loading a model.
//!
//! Two properties are deliberate:
//!
//! - **Every diagnostic is collected.** A spec with four faults reports four
//!   findings in one pass, in a stable order (notions, then intents, then
//!   constraints, then bindings, then cycles), never "the first error".
//! - **No I/O.** Files arrive already parsed; this module never reads a
//!   path, which is what lets the same pass serve the CLI, the tests and
//!   (in M2) an in-memory spec that exists nowhere on disk.
//!
//! One consequence of the second point: a `Diagnostic` from here carries the
//! file it came from but no line or column. Positions are byte offsets
//! (`Span`) that only the source text they were cut from can turn into a
//! line and a column, and the source text is exactly what a parsed
//! `TelFile` no longer holds.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Diagnostic, ErrorCode};
use crate::graph::{Graph, NodeRef, Relation};
use crate::ids::{ConstraintId, FieldName, IntentId, NotionName, RepoPath, ScenarioId};
use crate::model::{
    Action, Attr, AttrRef, AttrType, Binding, Constraint, Expr, InstanceStep, Intent, IntentStatus,
    Literal, Notion, NotionKind, Operand, Rule, Scenario, Scope, SourceKind, Statement, TelFile,
    TelosModel,
};
use crate::suggest::closest;

/// Builds the whole-spec model out of every parsed `.tel` file, or reports
/// every reason it cannot be built.
pub fn build_model(files: Vec<(RepoPath, TelFile)>) -> Result<TelosModel, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let (mut model, origins) = collect(files, &mut diagnostics);

    let mut checker = Checker {
        model: &model,
        origins: &origins,
        file: None,
        diagnostics: Vec::new(),
    };
    checker.run();
    diagnostics.extend(checker.diagnostics);

    let graph = relation_graph(&model);
    model.graph = graph;
    check_cycles(&model, &origins, &mut diagnostics);

    if diagnostics.is_empty() {
        Ok(model)
    } else {
        Err(diagnostics)
    }
}

// --- phase 1: folding files into the model -------------------------------

/// Which file each entity was declared in -- the reverse of
/// `TelosModel::sources`, kept alongside the model so that a diagnostic can
/// name the file of the entity it is about (and both files of a duplicate).
#[derive(Debug, Default)]
struct Origins {
    notions: BTreeMap<NotionName, RepoPath>,
    intents: BTreeMap<IntentId, RepoPath>,
    constraints: BTreeMap<ConstraintId, RepoPath>,
    /// One entry per binding, in `TelosModel::bindings` order.
    bindings: Vec<RepoPath>,
}

/// Folds the parsed files into a model, reporting entities declared twice.
///
/// The first declaration wins: the later one is reported and dropped, so the
/// rest of the pass checks a model with exactly one entity per name.
fn collect(
    files: Vec<(RepoPath, TelFile)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (TelosModel, Origins) {
    let mut model = TelosModel::default();
    let mut origins = Origins::default();

    for (path, file) in files {
        match file {
            TelFile::Notion(notion) => {
                model
                    .sources
                    .insert(path.clone(), SourceKind::Notion(notion.name.clone()));
                match origins.notions.get(&notion.name) {
                    Some(first) => diagnostics.push(duplicate(
                        format!("notion `{}`", notion.name),
                        first,
                        &path,
                    )),
                    None => {
                        origins.notions.insert(notion.name.clone(), path);
                        model.notions.insert(notion.name.clone(), notion);
                    }
                }
            }
            TelFile::Intent(intent) => {
                model
                    .sources
                    .insert(path.clone(), SourceKind::Intent(intent.id));
                match origins.intents.get(&intent.id) {
                    Some(first) => {
                        diagnostics.push(duplicate(format!("intent {}", intent.id), first, &path))
                    }
                    None => {
                        origins.intents.insert(intent.id, path.clone());
                        claim_scenarios(&intent, &path, &mut model, &origins, diagnostics);
                        model.intents.insert(intent.id, intent);
                    }
                }
            }
            TelFile::Constraint(constraint) => {
                model
                    .sources
                    .insert(path.clone(), SourceKind::Constraint(constraint.id));
                match origins.constraints.get(&constraint.id) {
                    Some(first) => diagnostics.push(duplicate(
                        format!("constraint {}", constraint.id),
                        first,
                        &path,
                    )),
                    None => {
                        origins.constraints.insert(constraint.id, path);
                        model.constraints.insert(constraint.id, constraint);
                    }
                }
            }
            TelFile::Bindings(bindings) => {
                model.sources.insert(path.clone(), SourceKind::Bindings);
                for binding in bindings {
                    origins.bindings.push(path.clone());
                    model.bindings.push(binding);
                }
            }
        }
    }

    (model, origins)
}

/// Records `verifies`: an intent owns the scenarios nested in it, and a
/// scenario id belongs to exactly one intent, spec-wide.
fn claim_scenarios(
    intent: &Intent,
    path: &RepoPath,
    model: &mut TelosModel,
    origins: &Origins,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for scenario in &intent.scenarios {
        match model.scenario_owner.get(&scenario.id) {
            Some(owner) => {
                let owner_file = origins.intents.get(owner).unwrap_or(path);
                diagnostics.push(Diagnostic {
                    code: ErrorCode::TelosIntegrityViolation,
                    message: format!(
                        "scenario {} is declared twice: in {owner} ({owner_file}) and in {} ({path})",
                        scenario.id, intent.id
                    ),
                    hint: None,
                    file: Some(path.clone()),
                    line: None,
                    col: None,
                });
            }
            None => {
                model.scenario_owner.insert(scenario.id, intent.id);
            }
        }
    }
}

/// « <entity> is declared twice: <first file> and <second file> ».
fn duplicate(entity: String, first: &RepoPath, second: &RepoPath) -> Diagnostic {
    Diagnostic {
        code: ErrorCode::TelosIntegrityViolation,
        message: format!("{entity} is declared twice: {first} and {second}"),
        hint: None,
        file: Some(second.clone()),
        line: None,
        col: None,
    }
}

// --- phase 2: resolution and integrity -----------------------------------

/// Walks the model once, checking every reference and every literal against
/// the entity that gives it meaning.
struct Checker<'a> {
    model: &'a TelosModel,
    origins: &'a Origins,
    /// The file the entity being checked was declared in.
    file: Option<RepoPath>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn run(&mut self) {
        let model = self.model;
        for (name, notion) in &model.notions {
            self.file = self.origins.notions.get(name).cloned();
            self.check_notion(notion);
        }
        for (id, intent) in &model.intents {
            self.file = self.origins.intents.get(id).cloned();
            self.check_intent(intent);
        }
        for (id, constraint) in &model.constraints {
            self.file = self.origins.constraints.get(id).cloned();
            self.check_constraint(constraint);
        }
        for (binding, file) in model.bindings.iter().zip(&self.origins.bindings) {
            self.file = Some(file.clone());
            self.check_binding(binding);
        }
    }

    // --- diagnostics ---------------------------------------------------

    fn push(&mut self, code: ErrorCode, message: String) {
        self.diagnostics.push(Diagnostic {
            code,
            message,
            hint: None,
            file: self.file.clone(),
            line: None,
            col: None,
        });
    }

    fn reference(&mut self, message: String) {
        self.push(ErrorCode::TelosReferenceUnknown, message);
    }

    fn integrity(&mut self, message: String) {
        self.push(ErrorCode::TelosIntegrityViolation, message);
    }

    // --- resolving one reference ---------------------------------------

    /// The notion must exist; hands it back when it does, so the caller can
    /// go on checking what hangs off it.
    fn require_notion(&mut self, name: &NotionName) -> Option<&'a Notion> {
        let model = self.model;
        if let Some(notion) = model.notions.get(name) {
            return Some(notion);
        }
        let known: Vec<&str> = model.notions.keys().map(NotionName::as_str).collect();
        self.reference(with_suggestion(
            format!("unknown notion `{name}`"),
            name.as_str(),
            &known,
        ));
        None
    }

    /// The notion must exist *and* be an event: only events happen, and only
    /// something that happens can drive a statement or a `when` step.
    ///
    /// A known notion of the wrong kind is still returned: its attributes
    /// are the right ones to check the step's payload against.
    fn require_event(&mut self, name: &NotionName) -> Option<&'a Notion> {
        let notion = self.require_notion(name)?;
        if notion.kind != NotionKind::Event {
            self.integrity(format!(
                "`{name}` is used as an event but its kind is `{}`, not `event`",
                kind_name(notion.kind)
            ));
        }
        Some(notion)
    }

    fn require_attr_of(&mut self, notion: &'a Notion, name: &FieldName) -> Option<&'a Attr> {
        if let Some(attr) = notion.attrs.iter().find(|a| a.name == *name) {
            return Some(attr);
        }
        let known: Vec<&str> = notion.attrs.iter().map(|a| a.name.as_str()).collect();
        self.reference(with_suggestion(
            format!("unknown attribute `{name}` on notion `{}`", notion.name),
            name.as_str(),
            &known,
        ));
        None
    }

    fn require_intent(&mut self, id: IntentId) {
        if self.model.intents.contains_key(&id) {
            return;
        }
        let known: Vec<String> = self.model.intents.keys().map(IntentId::to_string).collect();
        self.unknown_id("intent", &id.to_string(), &known);
    }

    fn require_scenario(&mut self, id: ScenarioId) {
        if self.model.scenario_owner.contains_key(&id) {
            return;
        }
        let known: Vec<String> = self
            .model
            .scenario_owner
            .keys()
            .map(ScenarioId::to_string)
            .collect();
        self.unknown_id("scenario", &id.to_string(), &known);
    }

    /// Ids are compared as they are written (`INT-0042`), so the suggestion
    /// runs on their rendered form rather than on their number.
    fn unknown_id(&mut self, noun: &str, id: &str, known: &[String]) {
        let candidates: Vec<&str> = known.iter().map(String::as_str).collect();
        self.reference(with_suggestion(
            format!("unknown {noun} `{id}`"),
            id,
            &candidates,
        ));
    }

    /// One `Notion.attr` reference, and -- when it meets one -- the literal
    /// it is compared or assigned to.
    fn check_attr_ref(&mut self, r: &AttrRef, value: Option<&Literal>) {
        let Some(notion) = self.require_notion(&r.notion.node) else {
            return;
        };
        let Some(attr) = self.require_attr_of(notion, &r.attr.node) else {
            return;
        };
        if let Some(value) = value {
            self.check_value(&notion.name, attr, value);
        }
    }

    /// A literal must have the shape the attribute's type demands.
    ///
    /// The match is exact -- an `int` literal does not stand in for a
    /// `decimal` attribute -- with three types that are more than a shape:
    /// `money` is a string of a fixed form, `enum` admits only its declared
    /// symbols, and `ref(...)` has no literal form at all in M1 (nothing to
    /// check).
    fn check_value(&mut self, notion: &NotionName, attr: &Attr, value: &Literal) {
        let qualified = format!("{notion}.{}", attr.name);
        let matches = match (&attr.ty, value) {
            (AttrType::String, Literal::Str(_))
            | (AttrType::Int, Literal::Int(_))
            | (AttrType::Decimal, Literal::Decimal(_))
            | (AttrType::Bool, Literal::Bool(_))
            | (AttrType::Date, Literal::Date(_))
            | (AttrType::Datetime, Literal::Datetime(_)) => true,
            (AttrType::Money, Literal::Str(amount)) => {
                if !is_money(amount) {
                    self.integrity(format!(
                        "attribute `{qualified}` has type `money`, \
                         but `{amount}` is not an amount of the form `0.00 EUR`"
                    ));
                }
                return;
            }
            (AttrType::Enum(symbols), Literal::Symbol(symbol)) => {
                if !symbols.contains(&symbol.node) {
                    let known: Vec<&str> = symbols.iter().map(String::as_str).collect();
                    self.reference(with_suggestion(
                        format!("`{}` is not a symbol of enum `{qualified}`", symbol.node),
                        &symbol.node,
                        &known,
                    ));
                }
                return;
            }
            (AttrType::Ref(_), _) => return,
            _ => false,
        };
        if !matches {
            self.integrity(format!(
                "attribute `{qualified}` has type `{}`, but the value is {}",
                type_name(&attr.ty),
                literal_kind(value)
            ));
        }
    }

    fn check_expr(&mut self, expr: &'a Expr) {
        walk_expr(expr, &mut |r, value| self.check_attr_ref(r, value));
    }

    // --- one entity at a time -------------------------------------------

    fn check_notion(&mut self, notion: &'a Notion) {
        for rel in &notion.rels {
            let _ = self.require_notion(&rel.target.node);
        }
        for attr in &notion.attrs {
            if let AttrType::Ref(target) = &attr.ty {
                let _ = self.require_notion(target);
            }
        }
    }

    fn check_intent(&mut self, intent: &'a Intent) {
        self.check_statement(&intent.statement);
        for id in intent
            .refines
            .iter()
            .chain(&intent.requires)
            .chain(&intent.excludes)
        {
            self.require_intent(id.node);
        }
        if intent.status == IntentStatus::Active && intent.scenarios.is_empty() {
            self.integrity(format!(
                "intent {} is active but has no scenario",
                intent.id
            ));
        }
        for scenario in &intent.scenarios {
            self.check_scenario(scenario);
        }
    }

    fn check_statement(&mut self, statement: &'a Statement) {
        match statement {
            Statement::Ubiquitous { action } => self.check_action(action),
            Statement::EventDriven { event, on, action } => {
                let _ = self.require_event(&event.node);
                if let Some(on) = on {
                    let _ = self.require_notion(&on.node);
                }
                self.check_action(action);
            }
            Statement::StateDriven {
                subject,
                value,
                action,
            } => {
                self.check_attr_ref(subject, Some(value));
                self.check_action(action);
            }
            Statement::Unwanted { condition, action } => {
                self.check_expr(condition);
                self.check_action(action);
            }
            Statement::Optional { action, .. } => self.check_action(action),
        }
    }

    fn check_action(&mut self, action: &'a Action) {
        match action {
            Action::Set { target, value } => self.check_attr_ref(target, Some(value)),
            Action::Free(_) => {}
        }
    }

    fn check_scenario(&mut self, scenario: &'a Scenario) {
        for step in &scenario.given {
            self.check_step(step, false);
        }
        self.check_step(&scenario.when, true);
        for expr in &scenario.then {
            self.check_expr(expr);
        }
    }

    /// A `given` step sets up state of any kind; a `when` step is the event
    /// that happens, so its notion must be one.
    fn check_step(&mut self, step: &'a InstanceStep, must_be_an_event: bool) {
        let notion = if must_be_an_event {
            self.require_event(&step.notion.node)
        } else {
            self.require_notion(&step.notion.node)
        };
        let Some(notion) = notion else {
            return;
        };
        for (name, value) in &step.fields {
            if let Some(attr) = self.require_attr_of(notion, &name.node) {
                self.check_value(&notion.name, attr, value);
            }
        }
    }

    fn check_constraint(&mut self, constraint: &'a Constraint) {
        if let Rule::Machine(expr) = &constraint.rule {
            self.check_expr(expr);
        }
        if let Scope::Intents(ids) = &constraint.scope {
            for id in ids {
                self.require_intent(id.node);
            }
        }
    }

    fn check_binding(&mut self, binding: &'a Binding) {
        match binding {
            Binding::Implements { intent, .. } => self.require_intent(intent.node),
            Binding::Proves { scenario, .. } => self.require_scenario(scenario.node),
        }
    }
}

// --- the walker ----------------------------------------------------------

/// Visits every attribute reference of an expression, paired with the
/// literal it meets when it meets exactly one.
///
/// This is the single traversal of an `Expr` in the engine: the checker uses
/// it to resolve references and type literals, the `uses` derivation uses it
/// to collect notions. A comparison between two references yields both, each
/// without a literal -- M1 does not check that two attributes have
/// compatible types, only that both exist.
fn walk_expr(expr: &Expr, visit: &mut impl FnMut(&AttrRef, Option<&Literal>)) {
    match expr {
        Expr::Cmp { lhs, rhs, .. } => match (lhs, rhs) {
            (Operand::Ref(r), Operand::Lit(l)) | (Operand::Lit(l), Operand::Ref(r)) => {
                visit(r, Some(l));
            }
            (Operand::Ref(left), Operand::Ref(right)) => {
                visit(left, None);
                visit(right, None);
            }
            (Operand::Lit(_), Operand::Lit(_)) => {}
        },
        Expr::In { lhs, set } => {
            if let Operand::Ref(r) = lhs {
                if set.is_empty() {
                    visit(r, None);
                }
                for literal in set {
                    visit(r, Some(literal));
                }
            }
        }
        Expr::And(left, right) | Expr::Or(left, right) => {
            walk_expr(left, visit);
            walk_expr(right, visit);
        }
        Expr::Not(inner) => walk_expr(inner, visit),
    }
}

/// Every notion an expression names.
fn expr_notions(expr: &Expr, out: &mut BTreeSet<NotionName>) {
    walk_expr(expr, &mut |r, _| {
        out.insert(r.notion.node.clone());
    });
}

fn action_notions(action: &Action, out: &mut BTreeSet<NotionName>) {
    if let Action::Set { target, .. } = action {
        out.insert(target.notion.node.clone());
    }
}

/// Every notion an intent's statement names -- the `uses` edges of an
/// intent.
fn statement_notions(statement: &Statement) -> BTreeSet<NotionName> {
    let mut out = BTreeSet::new();
    match statement {
        Statement::Ubiquitous { action } | Statement::Optional { action, .. } => {
            action_notions(action, &mut out);
        }
        Statement::EventDriven { event, on, action } => {
            out.insert(event.node.clone());
            if let Some(on) = on {
                out.insert(on.node.clone());
            }
            action_notions(action, &mut out);
        }
        Statement::StateDriven {
            subject, action, ..
        } => {
            out.insert(subject.notion.node.clone());
            action_notions(action, &mut out);
        }
        Statement::Unwanted { condition, action } => {
            expr_notions(condition, &mut out);
            action_notions(action, &mut out);
        }
    }
    out
}

/// Every notion a scenario names -- the `uses` edges of a scenario, which
/// hang off the scenario itself, not off the intent nesting it.
fn scenario_notions(scenario: &Scenario) -> BTreeSet<NotionName> {
    let mut out = BTreeSet::new();
    for step in &scenario.given {
        out.insert(step.notion.node.clone());
    }
    out.insert(scenario.when.notion.node.clone());
    for expr in &scenario.then {
        expr_notions(expr, &mut out);
    }
    out
}

// --- phase 3: the relation graph -----------------------------------------

/// Builds the graph of spec 3.2: the declared relations, plus the derived
/// `uses` edges.
fn relation_graph(model: &TelosModel) -> Graph {
    let mut graph = Graph::default();

    for intent in model.intents.values() {
        let node = NodeRef::Intent(intent.id);
        let declared = [
            (Relation::Refines, &intent.refines),
            (Relation::Requires, &intent.requires),
            (Relation::Excludes, &intent.excludes),
        ];
        for (relation, ids) in declared {
            for id in ids {
                graph.add_edge(node.clone(), relation, NodeRef::Intent(id.node));
            }
        }
        for name in statement_notions(&intent.statement) {
            graph.add_edge(node.clone(), Relation::Uses, NodeRef::Notion(name));
        }
        for scenario in &intent.scenarios {
            let scenario_node = NodeRef::Scenario(scenario.id);
            graph.add_edge(scenario_node.clone(), Relation::Verifies, node.clone());
            for name in scenario_notions(scenario) {
                graph.add_edge(scenario_node.clone(), Relation::Uses, NodeRef::Notion(name));
            }
        }
    }

    for constraint in model.constraints.values() {
        // A global constraint constrains the spec as a whole, which is no
        // edge in particular.
        if let Scope::Intents(ids) = &constraint.scope {
            for id in ids {
                graph.add_edge(
                    NodeRef::Constraint(constraint.id),
                    Relation::Constrains,
                    NodeRef::Intent(id.node),
                );
            }
        }
    }

    for binding in &model.bindings {
        match binding {
            Binding::Implements { path, intent } => graph.add_edge(
                NodeRef::Code(path.clone()),
                Relation::Implements,
                NodeRef::Intent(intent.node),
            ),
            Binding::Proves { test, scenario } => graph.add_edge(
                NodeRef::Test(test.to_string()),
                Relation::Proves,
                NodeRef::Scenario(scenario.node),
            ),
        }
    }

    graph
}

// --- phase 4: cycles -----------------------------------------------------

/// `refines` and `requires` must each be acyclic, independently: an intent
/// cannot end up refining or requiring itself, however long the way round.
fn check_cycles(model: &TelosModel, origins: &Origins, diagnostics: &mut Vec<Diagnostic>) {
    for relation in [Relation::Refines, Relation::Requires] {
        let Some(path) = model.graph.find_cycle(relation) else {
            continue;
        };
        let rendered: Vec<String> = path.iter().map(NodeRef::to_string).collect();
        let file = match path.first() {
            Some(NodeRef::Intent(id)) => origins.intents.get(id).cloned(),
            _ => None,
        };
        diagnostics.push(Diagnostic {
            code: ErrorCode::TelosCycleDetected,
            message: format!("cycle on `{relation}`: {}", rendered.join(" → ")),
            hint: None,
            file,
            line: None,
            col: None,
        });
    }
}

// --- message helpers -----------------------------------------------------

/// Appends « ; closest is `x` » when one of `candidates` is close enough to
/// `word` -- the shape every corrective message in the engine shares with
/// the parser's.
fn with_suggestion(message: String, word: &str, candidates: &[&str]) -> String {
    match closest(word, candidates.iter().copied()) {
        Some(known) => format!("{message}; closest is `{known}`"),
        None => message,
    }
}

fn kind_name(kind: NotionKind) -> &'static str {
    match kind {
        NotionKind::Actor => "actor",
        NotionKind::Entity => "entity",
        NotionKind::Value => "value",
        NotionKind::Event => "event",
        NotionKind::State => "state",
    }
}

fn type_name(ty: &AttrType) -> String {
    match ty {
        AttrType::String => "string".to_string(),
        AttrType::Int => "int".to_string(),
        AttrType::Decimal => "decimal".to_string(),
        AttrType::Money => "money".to_string(),
        AttrType::Bool => "bool".to_string(),
        AttrType::Date => "date".to_string(),
        AttrType::Datetime => "datetime".to_string(),
        AttrType::Enum(_) => "enum".to_string(),
        AttrType::Ref(target) => format!("ref({target})"),
    }
}

fn literal_kind(literal: &Literal) -> &'static str {
    match literal {
        Literal::Str(_) => "a string",
        Literal::Int(_) => "an int",
        Literal::Decimal(_) => "a decimal",
        Literal::Bool(_) => "a bool",
        Literal::Date(_) => "a date",
        Literal::Datetime(_) => "a datetime",
        Literal::Symbol(_) => "a symbol",
    }
}

/// `^\d+\.\d{2} [A-Z]{3}$` -- the money lexeme of Annex B, checked by hand
/// rather than by a regex engine the crate does not depend on.
fn is_money(s: &str) -> bool {
    let Some((amount, currency)) = s.split_once(' ') else {
        return false;
    };
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return false;
    }
    let Some((units, cents)) = amount.split_once('.') else {
        return false;
    };
    !units.is_empty()
        && units.bytes().all(|b| b.is_ascii_digit())
        && cents.len() == 2
        && cents.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTIONS: [&str; 4] = ["Customer", "Invoice", "InvoiceIssued", "PaymentReceived"];

    #[test]
    fn an_unknown_reference_message_suggests_a_case_insensitive_match() {
        // The pinned shape of the reference diagnostic, on the case only
        // `closest` can reach: `invoice` is not a `NotionName` (PascalCase
        // is a lexical rule), so no `.tel` file can carry it -- but the
        // message this helper builds is the very one `require_notion`
        // emits.
        assert_eq!(
            with_suggestion("unknown notion `invoice`".to_string(), "invoice", &NOTIONS),
            "unknown notion `invoice`; closest is `Invoice`"
        );
    }

    #[test]
    fn an_unknown_reference_message_stays_bare_without_a_close_match() {
        assert_eq!(
            with_suggestion("unknown notion `Rogue`".to_string(), "Rogue", &NOTIONS),
            "unknown notion `Rogue`"
        );
    }

    #[test]
    fn money_accepts_the_annex_b_lexeme_only() {
        assert!(is_money("120.00 EUR"));
        assert!(is_money("0.99 USD"));
        assert!(is_money("1234567.89 CHF"));

        assert!(!is_money("120 EUR"), "cents are mandatory");
        assert!(!is_money("120.0 EUR"), "exactly two digits of cents");
        assert!(!is_money("120.000 EUR"), "exactly two digits of cents");
        assert!(!is_money("120.00 eur"), "the currency is uppercase");
        assert!(!is_money("120.00 EU"), "three letters");
        assert!(!is_money("120.00 EURO"), "three letters");
        assert!(!is_money("120.00EUR"), "one space");
        assert!(!is_money("120.00  EUR"), "exactly one space");
        assert!(!is_money(".00 EUR"), "at least one unit digit");
        assert!(!is_money("-1.00 EUR"), "digits only");
        assert!(!is_money(""), "and an empty string is not an amount");
    }

    #[test]
    fn a_type_name_renders_the_attr_type_as_written() {
        assert_eq!(type_name(&AttrType::Money), "money");
        assert_eq!(type_name(&AttrType::Enum(vec!["open".to_string()])), "enum");
        assert_eq!(
            type_name(&AttrType::Ref(NotionName::new("Customer").unwrap())),
            "ref(Customer)"
        );
    }
}
