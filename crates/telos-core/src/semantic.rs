//! The semantic pass: folds parsed `.tel` files into a `TelosModel`, checks
//! the integrity conditions that hold at read time, and derives the relation
//! graph.
//!
//! Every reference must resolve; an `active` intent must be verified by at
//! least one scenario; a `when` step must name an event; and literals must
//! agree with the attribute types they meet. Referential deletion safety and
//! code coverage belong to the change flow, not to model loading.
//!
//! Two properties are deliberate:
//!
//! - **Every diagnostic is collected.** A spec with four faults reports four
//!   findings in one pass, in a stable order (notions, then intents, then
//!   constraints, then bindings, then cycles), never "the first error".
//! - **No I/O.** Files arrive already parsed; this module never reads a
//!   path, which is what lets the same pass serve the CLI, the tests and
//!   an in-memory spec that exists nowhere on disk.
//!
//! One consequence of the second point: a `Diagnostic` from here carries the
//! file it came from but no line or column. Positions are byte offsets
//! (`Span`) that only the source text they were cut from can turn into a
//! line and a column, and the source text is exactly what a parsed
//! `TelFile` no longer holds.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Diagnostic, ErrorCode};
use crate::graph::{Graph, NodeRef, Relation};
use crate::ids::{
    CapabilityRef, ConstraintId, ContextId, FieldName, IntentId, NotionName, NotionRef, Owner,
    RepoPath, ScenarioId,
};
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
        context: None,
        diagnostics: Vec::new(),
    };
    checker.run();
    diagnostics.extend(checker.diagnostics);
    check_domain_boundaries(&model, &origins, &mut diagnostics);

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
    contexts: BTreeMap<ContextId, RepoPath>,
    capabilities: BTreeMap<CapabilityRef, RepoPath>,
    domain_notions: BTreeMap<NotionRef, RepoPath>,
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
            TelFile::Context(context) => {
                model
                    .sources
                    .insert(path.clone(), SourceKind::Context(context.id.clone()));
                match origins.contexts.get(&context.id) {
                    Some(first) => diagnostics.push(duplicate(
                        format!("context `{}`", context.id),
                        first,
                        &path,
                    )),
                    None => {
                        origins.contexts.insert(context.id.clone(), path);
                        model.contexts.insert(context.id.clone(), context);
                    }
                }
            }
            TelFile::Capability(capability) => {
                model
                    .sources
                    .insert(path.clone(), SourceKind::Capability(capability.id.clone()));
                match origins.capabilities.get(&capability.id) {
                    Some(first) => diagnostics.push(duplicate(
                        format!("capability `{}`", capability.id),
                        first,
                        &path,
                    )),
                    None => {
                        origins.capabilities.insert(capability.id.clone(), path);
                        model.capabilities.insert(capability.id.clone(), capability);
                    }
                }
            }
            TelFile::OwnedNotion { owner, notion } => {
                let reference = NotionRef::new(owner.context.clone(), notion.name.clone());
                model
                    .sources
                    .insert(path.clone(), SourceKind::QualifiedNotion(reference.clone()));
                match origins.domain_notions.get(&reference) {
                    Some(first) => {
                        diagnostics.push(duplicate(format!("notion `{reference}`"), first, &path))
                    }
                    None => {
                        origins.domain_notions.insert(reference.clone(), path);
                        model
                            .notions
                            .entry(notion.name.clone())
                            .or_insert_with(|| notion.clone());
                        model.notion_owners.insert(reference.clone(), owner);
                        model.domain_notions.insert(reference, notion);
                    }
                }
            }
            TelFile::OwnedIntent { owner, intent } => {
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
                        model.intent_owners.insert(intent.id, owner);
                        model.intents.insert(intent.id, intent);
                    }
                }
            }
            TelFile::OwnedConstraint { owner, constraint } => {
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
                        model.constraint_owners.insert(constraint.id, owner);
                        model.constraints.insert(constraint.id, constraint);
                    }
                }
            }
            TelFile::ContextBindings { context, bindings } => {
                model.sources.insert(path.clone(), SourceKind::Bindings);
                for binding in bindings {
                    origins.bindings.push(path.clone());
                    model.binding_contexts.push(context.clone());
                    model.bindings.push(binding);
                }
            }
            TelFile::ContextMap(context_map) => {
                model.sources.insert(path, SourceKind::ContextMap);
                model.context_map = context_map;
            }
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

/// “ <entity> is declared twice: <first file> and <second file> ”.
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
    /// Unqualified tactical references always resolve in this context.
    context: Option<ContextId>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn run(&mut self) {
        let model = self.model;
        if model.domain_notions.is_empty() {
            for (name, notion) in &model.notions {
                self.context = None;
                self.file = self.origins.notions.get(name).cloned();
                self.check_notion(notion);
            }
        } else {
            for (reference, notion) in &model.domain_notions {
                self.context = Some(reference.context.clone());
                self.file = self.origins.domain_notions.get(reference).cloned();
                self.check_notion(notion);
            }
        }
        for (id, intent) in &model.intents {
            self.context = model
                .intent_owners
                .get(id)
                .map(|owner| owner.context.clone());
            self.file = self.origins.intents.get(id).cloned();
            self.check_intent(intent);
        }
        for (id, constraint) in &model.constraints {
            self.context = model
                .constraint_owners
                .get(id)
                .and_then(Option::as_ref)
                .map(|owner| owner.context.clone());
            self.file = self.origins.constraints.get(id).cloned();
            self.check_constraint(constraint);
        }
        for (binding, file) in model.bindings.iter().zip(&self.origins.bindings) {
            self.context = None;
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
        if let Some(context) = &self.context {
            let reference = NotionRef::new(context.clone(), name.clone());
            if let Some(notion) = model.domain_notions.get(&reference) {
                return Some(notion);
            }
            let known: Vec<&str> = model
                .domain_notions
                .keys()
                .filter(|candidate| candidate.context == *context)
                .map(|candidate| candidate.notion.as_str())
                .collect();
            self.reference(with_suggestion(
                format!("unknown notion `{name}`"),
                name.as_str(),
                &known,
            ));
            return None;
        }
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
    /// `decimal` attribute -- with five types that are more than a variant
    /// match: `money`, `date` and `datetime` are strings of a fixed lexical
    /// form, `enum` admits only its declared symbols, and `ref(...)` has no
    /// literal form at all (nothing to check).
    ///
    /// The three lexeme checks are not redundant with the lexer, which only
    /// ever produces well-formed ones: a literal can also come
    /// from a JSON payload ([`crate::payload`]), which copies the string
    /// verbatim because it has no attribute type in hand at that point. This
    /// is where that string finally meets its type.
    fn check_value(&mut self, notion: &NotionName, attr: &Attr, value: &Literal) {
        let qualified = format!("{notion}.{}", attr.name);
        let matches = match (&attr.ty, value) {
            (AttrType::String, Literal::Str(_))
            | (AttrType::Int, Literal::Int(_))
            | (AttrType::Decimal, Literal::Decimal(_))
            | (AttrType::Bool, Literal::Bool(_)) => true,
            (AttrType::Money, Literal::Str(amount)) => {
                if !is_money(amount) {
                    self.integrity(format!(
                        "attribute `{qualified}` has type `money`, \
                         but `{amount}` is not an amount of the form `0.00 EUR`"
                    ));
                }
                return;
            }
            (AttrType::Date, Literal::Date(lexeme)) => {
                if !is_date(lexeme) {
                    self.integrity(format!(
                        "attribute `{qualified}` has type `date`, \
                         but `{lexeme}` is not a date of the form `2026-08-19`"
                    ));
                }
                return;
            }
            (AttrType::Datetime, Literal::Datetime(lexeme)) => {
                if !is_datetime(lexeme) {
                    self.integrity(format!(
                        "attribute `{qualified}` has type `datetime`, \
                         but `{lexeme}` is not a timestamp of the form \
                         `2026-08-19T12:00:00Z`"
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
        self.check_phrase(notion);
        for rel in &notion.rels {
            let _ = self.require_notion(&rel.target.node);
        }
        for attr in &notion.attrs {
            if let AttrType::Ref(target) = &attr.ty {
                let _ = self.require_notion(target);
            }
        }
    }

    /// A phrase is the term's surface form, used mid-sentence after `the `.
    ///
    /// No leading-capital rule: a phrase is never sentence-initial, so `SLA`
    /// is correct as written. The single-line rule lives in
    /// `payload::notion_from_obj`, the only boundary a newline can cross.
    fn check_phrase(&mut self, notion: &'a Notion) {
        let name = &notion.name;
        let phrase = notion.phrase.as_str();

        if phrase.trim().is_empty() {
            self.integrity(format!("notion {name} has an empty phrase"));
            return;
        }
        let lowered = phrase.to_lowercase();
        if ["a ", "an ", "the "]
            .iter()
            .any(|article| lowered.starts_with(article))
        {
            self.integrity(format!(
                "notion {name} phrase must not start with an article: `{phrase}`"
            ));
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
/// without a literal -- the semantic pass does not check that two attributes have
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
///
/// `pub(crate)` for [`crate::overlay`], which answers the same question in
/// the other direction: not "what does this entity use?" but "does anything
/// still use the notion this op removes?" (the referential-deletion check).
pub(crate) fn expr_notions(expr: &Expr, out: &mut BTreeSet<NotionName>) {
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
/// intent. `pub(crate)` for the same reason as [`expr_notions`].
pub(crate) fn statement_notions(statement: &Statement) -> BTreeSet<NotionName> {
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
/// `pub(crate)` for the same reason as [`expr_notions`].
pub(crate) fn scenario_notions(scenario: &Scenario) -> BTreeSet<NotionName> {
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

// --- strategic domain boundaries ---------------------------------------

fn check_domain_boundaries(
    model: &TelosModel,
    origins: &Origins,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if model.contexts.is_empty()
        && model.capabilities.is_empty()
        && model.intent_owners.is_empty()
        && model.domain_notions.is_empty()
    {
        return;
    }

    for (id, capability) in &model.capabilities {
        if !model.contexts.contains_key(&id.context) {
            diagnostics.push(boundary(
                format!(
                    "capability `{}` belongs to unknown context `{}`",
                    id, id.context
                ),
                origins.capabilities.get(id).cloned(),
            ));
        }
        debug_assert_eq!(&capability.id, id);
    }

    for (reference, owner) in &model.notion_owners {
        check_owner_exists(
            model,
            owner,
            format!("notion `{reference}`"),
            origins.domain_notions.get(reference).cloned(),
            diagnostics,
        );
        if let Some(notion) = model.domain_notions.get(reference) {
            let mut targets: BTreeSet<NotionName> = notion
                .rels
                .iter()
                .map(|relation| relation.target.node.clone())
                .collect();
            targets.extend(
                notion
                    .attrs
                    .iter()
                    .filter_map(|attribute| match &attribute.ty {
                        AttrType::Ref(target) => Some(target.clone()),
                        _ => None,
                    }),
            );
            for target in targets {
                check_local_notion(
                    model,
                    &owner.context,
                    &target,
                    format!("notion `{reference}`"),
                    origins.domain_notions.get(reference).cloned(),
                    diagnostics,
                );
            }
        }
    }
    for (id, owner) in &model.intent_owners {
        check_owner_exists(
            model,
            owner,
            format!("intent {id}"),
            origins.intents.get(id).cloned(),
            diagnostics,
        );
    }
    for (id, owner) in &model.constraint_owners {
        if let Some(owner) = owner {
            check_owner_exists(
                model,
                owner,
                format!("constraint {id}"),
                origins.constraints.get(id).cloned(),
                diagnostics,
            );
        }
    }

    let map_file = model
        .sources
        .iter()
        .find_map(|(path, kind)| matches!(kind, SourceKind::ContextMap).then(|| path.clone()));
    let mut dependency_graph = Graph::default();
    let declared_dependencies: BTreeSet<(ContextId, ContextId)> = model
        .context_map
        .dependencies
        .iter()
        .map(|dependency| (dependency.consumer.clone(), dependency.supplier.clone()))
        .collect();

    for dependency in &model.context_map.dependencies {
        for (role, context) in [
            ("consumer", &dependency.consumer),
            ("supplier", &dependency.supplier),
        ] {
            if !model.contexts.contains_key(context) {
                diagnostics.push(boundary(
                    format!("dependency {role} `{context}` is not a declared context"),
                    map_file.clone(),
                ));
            }
        }
        dependency_graph.add_edge(
            NodeRef::Context(dependency.consumer.clone()),
            Relation::DependsOn,
            NodeRef::Context(dependency.supplier.clone()),
        );
        for mapping in &dependency.mappings {
            if mapping.from.context != dependency.supplier
                || mapping.to.context != dependency.consumer
            {
                diagnostics.push(boundary(
                    format!(
                        "mapping must point from supplier `{}` to consumer `{}`: {} -> {}",
                        dependency.supplier, dependency.consumer, mapping.from, mapping.to
                    ),
                    map_file.clone(),
                ));
            }
            for reference in [&mapping.from, &mapping.to] {
                if !model.domain_notions.contains_key(reference) {
                    diagnostics.push(boundary(
                        format!("mapping references unknown notion `{reference}`"),
                        map_file.clone(),
                    ));
                }
            }
        }
    }
    if let Some(cycle) = dependency_graph.find_cycle(Relation::DependsOn) {
        let rendered: Vec<String> = cycle.iter().map(NodeRef::to_string).collect();
        diagnostics.push(boundary(
            format!("context dependency cycle: {}", rendered.join(" → ")),
            map_file,
        ));
    }

    for (id, intent) in &model.intents {
        let Some(owner) = model.intent_owners.get(id) else {
            continue;
        };
        let mut used = statement_notions(&intent.statement);
        for scenario in &intent.scenarios {
            used.extend(scenario_notions(scenario));
        }
        for notion in used {
            check_local_notion(
                model,
                &owner.context,
                &notion,
                format!("intent {id}"),
                origins.intents.get(id).cloned(),
                diagnostics,
            );
        }
        for relation in &intent.refines {
            check_intent_context_relation(
                model,
                owner,
                relation.node,
                "refines",
                false,
                &declared_dependencies,
                origins.intents.get(id).cloned(),
                diagnostics,
            );
        }
        for relation in &intent.excludes {
            check_intent_context_relation(
                model,
                owner,
                relation.node,
                "excludes",
                false,
                &declared_dependencies,
                origins.intents.get(id).cloned(),
                diagnostics,
            );
        }
        for relation in &intent.requires {
            check_intent_context_relation(
                model,
                owner,
                relation.node,
                "requires",
                true,
                &declared_dependencies,
                origins.intents.get(id).cloned(),
                diagnostics,
            );
        }
    }

    let mut production_owners: BTreeMap<&RepoPath, ContextId> = BTreeMap::new();
    for (index, binding) in model.bindings.iter().enumerate() {
        let Some(binding_context) = model.binding_contexts.get(index) else {
            continue;
        };
        if let Binding::Implements { path, intent } = binding {
            if let Some(intent_owner) = model.intent_owners.get(&intent.node)
                && intent_owner.context != *binding_context
            {
                diagnostics.push(boundary(
                    format!(
                        "binding in context `{binding_context}` implements {} owned by `{}`",
                        intent.node, intent_owner.context
                    ),
                    origins.bindings.get(index).cloned(),
                ));
            }
            if let Some(first) = production_owners.get(path) {
                if first != binding_context {
                    diagnostics.push(boundary(
                        format!(
                            "production file `{path}` implements intents from contexts `{first}` and `{binding_context}`"
                        ),
                        origins.bindings.get(index).cloned(),
                    ));
                }
            } else {
                production_owners.insert(path, binding_context.clone());
            }
        }
    }
}

fn check_local_notion(
    model: &TelosModel,
    context: &ContextId,
    notion: &NotionName,
    referrer: String,
    file: Option<RepoPath>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let reference = NotionRef::new(context.clone(), notion.clone());
    if !model.domain_notions.contains_key(&reference)
        && model
            .domain_notions
            .keys()
            .any(|candidate| candidate.notion == *notion)
    {
        diagnostics.push(boundary(
            format!(
                "{referrer} references `{reference}`, which is not owned by context `{context}`"
            ),
            file,
        ));
    }
}

fn check_owner_exists(
    model: &TelosModel,
    owner: &Owner,
    entity: String,
    file: Option<RepoPath>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !model.contexts.contains_key(&owner.context) {
        diagnostics.push(boundary(
            format!("{entity} belongs to unknown context `{}`", owner.context),
            file.clone(),
        ));
    }
    if let Some(capability) = owner.capability_ref()
        && !model.capabilities.contains_key(&capability)
    {
        diagnostics.push(boundary(
            format!("{entity} belongs to unknown capability `{capability}`"),
            file,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn check_intent_context_relation(
    model: &TelosModel,
    source: &Owner,
    target_id: IntentId,
    relation: &str,
    dependency_allowed: bool,
    dependencies: &BTreeSet<(ContextId, ContextId)>,
    file: Option<RepoPath>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(target) = model.intent_owners.get(&target_id) else {
        return;
    };
    if source.context == target.context {
        return;
    }
    let allowed = dependency_allowed
        && dependencies.contains(&(source.context.clone(), target.context.clone()));
    if !allowed {
        diagnostics.push(boundary(
            format!(
                "cross-context `{relation}` from `{}` to `{}` is not allowed",
                source.context, target.context
            ),
            file,
        ));
    }
}

fn boundary(message: String, file: Option<RepoPath>) -> Diagnostic {
    Diagnostic {
        code: ErrorCode::TelosContextBoundaryViolation,
        message,
        hint: Some(
            "declare the ownership/dependency explicitly or keep the relation inside one context"
                .to_string(),
        ),
        file,
        line: None,
        col: None,
    }
}

// --- phase 3: the relation graph -----------------------------------------

/// Builds the graph from declared relations plus derived `uses` edges.
fn relation_graph(model: &TelosModel) -> Graph {
    let mut graph = Graph::default();

    for capability in model.capabilities.keys() {
        graph.add_edge(
            NodeRef::Capability(capability.clone()),
            Relation::BelongsTo,
            NodeRef::Context(capability.context.clone()),
        );
    }
    for (notion, owner) in &model.notion_owners {
        let target = match owner.capability_ref() {
            Some(capability) => NodeRef::Capability(capability),
            None => NodeRef::Context(owner.context.clone()),
        };
        graph.add_edge(
            NodeRef::QualifiedNotion(notion.clone()),
            Relation::BelongsTo,
            target,
        );
    }
    for (intent, owner) in &model.intent_owners {
        if let Some(capability) = owner.capability_ref() {
            graph.add_edge(
                NodeRef::Intent(*intent),
                Relation::BelongsTo,
                NodeRef::Capability(capability),
            );
        }
    }
    for (constraint, owner) in &model.constraint_owners {
        if let Some(owner) = owner {
            let target = match owner.capability_ref() {
                Some(capability) => NodeRef::Capability(capability),
                None => NodeRef::Context(owner.context.clone()),
            };
            graph.add_edge(
                NodeRef::Constraint(*constraint),
                Relation::BelongsTo,
                target,
            );
        }
    }
    for dependency in &model.context_map.dependencies {
        graph.add_edge(
            NodeRef::Context(dependency.consumer.clone()),
            Relation::DependsOn,
            NodeRef::Context(dependency.supplier.clone()),
        );
        for mapping in &dependency.mappings {
            graph.add_edge(
                NodeRef::QualifiedNotion(mapping.from.clone()),
                Relation::MapsTo,
                NodeRef::QualifiedNotion(mapping.to.clone()),
            );
        }
    }

    for intent in model.intents.values() {
        let node = NodeRef::Intent(intent.id);
        let context = model
            .intent_owners
            .get(&intent.id)
            .map(|owner| owner.context.clone());
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
            let target = match &context {
                Some(context) => NodeRef::QualifiedNotion(NotionRef::new(context.clone(), name)),
                None => NodeRef::Notion(name),
            };
            graph.add_edge(node.clone(), Relation::Uses, target);
        }
        for scenario in &intent.scenarios {
            let scenario_node = NodeRef::Scenario(scenario.id);
            graph.add_edge(scenario_node.clone(), Relation::Verifies, node.clone());
            for name in scenario_notions(scenario) {
                let target = match &context {
                    Some(context) => {
                        NodeRef::QualifiedNotion(NotionRef::new(context.clone(), name))
                    }
                    None => NodeRef::Notion(name),
                };
                graph.add_edge(scenario_node.clone(), Relation::Uses, target);
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

/// Appends “ ; closest is `x` ” when one of `candidates` is close enough to
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

/// `^\d+\.\d{2} [A-Z]{3}$` -- the canonical money lexeme, checked by hand
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

/// `^\d{4}-\d{2}-\d{2}$` -- the canonical `date-lit` lexeme
///
/// A *shape*, not a calendar: `2026-13-99` passes, exactly as it does
/// through the lexer, which reads the same positional shape. Rejecting an
/// impossible month is a different check, and one no other layer of the
/// engine makes either.
fn is_date(s: &str) -> bool {
    matches_shape(s, "dddd-dd-dd")
}

/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z?$` -- the canonical
/// `datetime-lit` lexeme, whose trailing `Z` is optional.
fn is_datetime(s: &str) -> bool {
    matches_shape(s.strip_suffix('Z').unwrap_or(s), "dddd-dd-ddTdd:dd:dd")
}

/// Matches `s` against a positional shape in which `d` stands for one ASCII
/// digit and every other byte stands for itself -- the hand-rolled stand-in
/// for the two anchored regexes above, which the crate depends on no engine
/// to run.
fn matches_shape(s: &str, shape: &str) -> bool {
    s.len() == shape.len()
        && s.bytes()
            .zip(shape.bytes())
            .all(|(byte, expected)| match expected {
                b'd' => byte.is_ascii_digit(),
                other => byte == other,
            })
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
    fn money_accepts_the_public_schema_lexeme_only() {
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
    fn date_accepts_the_canonical_lexeme_only() {
        assert!(is_date("2026-08-19"));
        // A shape, not a calendar -- the same reading the lexer makes.
        assert!(is_date("2026-13-99"));

        assert!(!is_date("2026-8-19"), "two digits of month");
        assert!(!is_date("26-08-19"), "four digits of year");
        assert!(!is_date("2026-08-19T12:00:00Z"), "a date carries no time");
        assert!(!is_date("2026/08/19"), "dashes, not slashes");
        assert!(!is_date("2026-08-19 "), "no trailing space");
        assert!(!is_date("aaaa-bb-cc"), "digits only");
        assert!(!is_date(""), "and an empty string is not a date");
    }

    #[test]
    fn datetime_accepts_the_canonical_lexeme_only() {
        assert!(is_datetime("2026-08-19T12:00:00Z"));
        assert!(is_datetime("2026-08-19T12:00:00"), "the `Z` is optional");

        assert!(!is_datetime("2026-08-19"), "a timestamp carries a time");
        assert!(!is_datetime("2026-08-19 12:00:00Z"), "`T`, not a space");
        assert!(!is_datetime("2026-08-19T12:00Z"), "seconds are mandatory");
        assert!(!is_datetime("2026-08-19T12:00:00z"), "an uppercase `Z`");
        assert!(!is_datetime("2026-08-19T12:00:00+02:00"), "no offset form");
        assert!(!is_datetime(""), "and an empty string is not a timestamp");
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
