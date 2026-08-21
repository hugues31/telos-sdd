//! The canonical `.tel` emitter.
//!
//! This module is the *only* writer of `.tel` syntax in the engine: the
//! canonical form is not a style guide checked after the fact, it is
//! whatever these functions produce. Every mutation path re-emits a
//! whole file from the model rather than editing text, so a spec tree can
//! never drift into a second, almost-canonical dialect.
//!
//! The contract that makes that safe is byte-level idempotence:
//! `emit(parse(s)) == s` for every canonical `s` (proved on the corpus in
//! `tests/roundtrip.rs`). Three consequences shape the code below:
//!
//! - **Padding is per group, with fixed widths.** A keyword is
//!   padded to the longest keyword its group *could* hold -- not the longest
//!   one actually present -- so adding a `check` line to a constraint, or
//!   dropping a `rel` from a notion, never reflows the columns of its
//!   neighbours, and a diff stays the size of the edit.
//! - **Order is normalized, not echoed.** `attrs` and `rels` keep
//!   insertion order because their order is data; relation ids, scenarios
//!   and bindings are sorted, because theirs is not.
//! - **Parentheses are minimal:** one is emitted exactly when a
//!   child binds looser than its parent.
//!
//! Every file-level function returns a `String` ending in exactly one `\n`
//! (the empty bindings file excepted -- an empty file is zero bytes).
//! Nothing here touches the filesystem: callers decide where bytes go.

use crate::ids::{FieldName, IntentId, ScenarioId};
use crate::model::{
    Action, Attr, AttrRef, AttrType, Binding, Change, CmpOp, Constraint, ConstraintKind, Expr,
    InstanceStep, Intent, IntentStatus, JournalEntry, Literal, Notion, NotionKind, Operand, Rel,
    Rule, Scenario, Scope, StagedOp, Statement, TelFile,
};
use crate::{
    config::Config,
    error::{ErrorCode, TelosError},
};

/// Emits a project's configuration in the canonical TOML representation.
pub fn emit_config(config: &Config) -> Result<String, TelosError> {
    let mut config = config.clone();
    config.normalize();
    let mut text = toml::to_string(&config).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to emit telos.toml: {e}"),
        )
    })?;
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    Ok(text)
}
use crate::span::Sp;

/// `write!` into a `String`: `fmt::Write` on a `String` is infallible, so
/// the `Result` carries no information worth threading through.
macro_rules! w {
    ($out:expr, $($arg:tt)*) => {{
        use std::fmt::Write as _;
        let _ = write!($out, $($arg)*);
    }};
}

/// One indentation level.
const INDENT: &str = "  ";

/// Keyword padding widths, one per field group. Each is the length
/// of the longest keyword the group admits, so the width is a property of
/// the grammar, not of the file being emitted.
mod width {
    /// `def`, `attr`, `rel`.
    pub const NOTION: usize = 4;
    /// `status`, `telos`.
    pub const INTENT: usize = 6;
    /// `when`, `while`, `if`, `where`, `system`.
    pub const STATEMENT: usize = 6;
    /// `given`, `when`, `then`.
    pub const SCENARIO: usize = 5;
    /// `rule`, `scope`, `check`.
    pub const CONSTRAINT: usize = 5;
    /// `implements`, `proves`.
    pub const BINDING: usize = 10;
    /// `status`, `digest`.
    pub const CHANGE: usize = 6;
    /// `run`, `bind` -- the two journal lines of a change.
    pub const JOURNAL: usize = 4;
}

/// Emits one parsed file in canonical form.
pub fn emit_file(f: &TelFile) -> String {
    match f {
        TelFile::Notion(n) => emit_notion(n),
        TelFile::Intent(i) => emit_intent(i),
        TelFile::Constraint(c) => emit_constraint(c),
        TelFile::Bindings(b) => emit_bindings(b),
    }
}

// --- notions -------------------------------------------------------------

/// Emits a `notion` file.
///
/// Two paddings stack here: the field keyword to the group width,
/// then -- inside the `attr` block and inside the `rel` block independently
/// -- the field *name* to the longest name of its own block, so the
/// types line up in a column.
pub fn emit_notion(n: &Notion) -> String {
    let mut out = String::new();
    w!(out, "notion {} {} {{\n", n.name, notion_kind(n.kind));

    keyword(&mut out, 1, "def", width::NOTION);
    w!(out, "{}\n", quote(&n.def));

    let attr_width = longest(n.attrs.iter().map(|a| a.name.as_str()));
    for Attr { name, ty } in &n.attrs {
        keyword(&mut out, 1, "attr", width::NOTION);
        pad_name(&mut out, name, attr_width);
        w!(out, "{}\n", attr_type(ty));
    }

    let rel_width = longest(n.rels.iter().map(|r| r.name.as_str()));
    for Rel { name, target } in &n.rels {
        keyword(&mut out, 1, "rel", width::NOTION);
        pad_name(&mut out, name, rel_width);
        w!(out, "-> {}\n", target.node);
    }

    out.push_str("}\n");
    out
}

fn notion_kind(k: NotionKind) -> &'static str {
    match k {
        NotionKind::Actor => "actor",
        NotionKind::Entity => "entity",
        NotionKind::Value => "value",
        NotionKind::Event => "event",
        NotionKind::State => "state",
    }
}

fn attr_type(ty: &AttrType) -> String {
    match ty {
        AttrType::String => "string".to_string(),
        AttrType::Int => "int".to_string(),
        AttrType::Decimal => "decimal".to_string(),
        AttrType::Money => "money".to_string(),
        AttrType::Bool => "bool".to_string(),
        AttrType::Date => "date".to_string(),
        AttrType::Datetime => "datetime".to_string(),
        // No space after `enum`; `, ` separates symbols.
        AttrType::Enum(symbols) => format!("enum({})", symbols.join(", ")),
        AttrType::Ref(target) => format!("ref({target})"),
    }
}

// --- intents -------------------------------------------------------------

/// Emits an `intent` file, statement block, relation
/// lines and scenarios included.
///
/// Blank lines appear in exactly one place: before each scenario,
/// which separates the header from the first one and the scenarios from each
/// other in a single rule.
pub fn emit_intent(i: &Intent) -> String {
    let mut out = String::new();
    w!(out, "intent {} {} {{\n", i.id, quote(&i.title));

    keyword(&mut out, 1, "status", width::INTENT);
    w!(out, "{}\n", intent_status(i.status));
    keyword(&mut out, 1, "telos", width::INTENT);
    w!(out, "{}\n", quote(&i.telos));

    emit_statement(&mut out, &i.statement);

    // Relation lines take no padding, and each group is sorted by id; the
    // groups themselves stay in grammar order.
    relations(&mut out, "refines", &i.refines);
    relations(&mut out, "requires", &i.requires);
    relations(&mut out, "excludes", &i.excludes);

    let mut scenarios: Vec<&Scenario> = i.scenarios.iter().collect();
    scenarios.sort_by_key(|s| s.id);
    for scenario in scenarios {
        out.push('\n');
        emit_scenario(&mut out, scenario);
    }

    out.push_str("}\n");
    out
}

fn intent_status(s: IntentStatus) -> &'static str {
    match s {
        IntentStatus::Draft => "draft",
        IntentStatus::Active => "active",
        IntentStatus::Deprecated => "deprecated",
    }
}

fn relations(out: &mut String, kw: &str, ids: &[Sp<IntentId>]) {
    let mut sorted: Vec<IntentId> = ids.iter().map(|id| id.node).collect();
    sorted.sort();
    for id in sorted {
        w!(out, "{INDENT}{kw} {id}\n");
    }
}

/// `statement-block`: the template word on the opening line, then the head
/// line the template calls for (all templates but `ubiquitous`), then the
/// shall-line -- both padded to the statement group's width.
fn emit_statement(out: &mut String, s: &Statement) {
    w!(out, "{INDENT}statement {} {{\n", template(s));
    match s {
        Statement::Ubiquitous { .. } => {}
        Statement::EventDriven { event, on, .. } => {
            keyword(out, 2, "when", width::STATEMENT);
            w!(out, "{}", event.node);
            if let Some(on) = on {
                w!(out, " on {}", on.node);
            }
            out.push('\n');
        }
        Statement::StateDriven { subject, value, .. } => {
            keyword(out, 2, "while", width::STATEMENT);
            w!(out, "{} == {}\n", attr_ref(subject), emit_literal(value));
        }
        Statement::Unwanted { condition, .. } => {
            keyword(out, 2, "if", width::STATEMENT);
            w!(out, "{}\n", emit_expr(condition));
        }
        Statement::Optional { feature, .. } => {
            keyword(out, 2, "where", width::STATEMENT);
            w!(out, "{feature}\n");
        }
    }
    keyword(out, 2, "system", width::STATEMENT);
    match action(s) {
        Action::Set { target, value } => {
            w!(
                out,
                "shall set {} = {}\n",
                attr_ref(target),
                emit_literal(value)
            );
        }
        Action::Free(text) => w!(out, "shall {}\n", quote(text)),
    }
    w!(out, "{INDENT}}}\n");
}

fn template(s: &Statement) -> &'static str {
    match s {
        Statement::Ubiquitous { .. } => "ubiquitous",
        Statement::EventDriven { .. } => "event-driven",
        Statement::StateDriven { .. } => "state-driven",
        Statement::Unwanted { .. } => "unwanted",
        Statement::Optional { .. } => "optional",
    }
}

/// Every template ends on a shall-line, so the action is reachable
/// uniformly.
fn action(s: &Statement) -> &Action {
    match s {
        Statement::Ubiquitous { action }
        | Statement::EventDriven { action, .. }
        | Statement::StateDriven { action, .. }
        | Statement::Unwanted { action, .. }
        | Statement::Optional { action, .. } => action,
    }
}

// --- scenarios -----------------------------------------------------------

fn emit_scenario(out: &mut String, s: &Scenario) {
    w!(out, "{INDENT}scenario {} {} {{\n", s.id, quote(&s.title));
    for given in &s.given {
        instance_step(out, "given", given);
    }
    instance_step(out, "when", &s.when);
    for then in &s.then {
        keyword(out, 2, "then", width::SCENARIO);
        w!(out, "{}\n", emit_expr(then));
    }
    w!(out, "{INDENT}}}\n");
}

fn instance_step(out: &mut String, kw: &str, step: &InstanceStep) {
    keyword(out, 2, kw, width::SCENARIO);
    w!(
        out,
        "{} {}\n",
        step.notion.node,
        instance_body(&step.fields)
    );
}

/// `instance-body` on one line; an empty payload is `{}`, with no
/// inner space to trim.
fn instance_body(fields: &[(Sp<FieldName>, Literal)]) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }
    let pairs: Vec<String> = fields
        .iter()
        .map(|(name, value)| format!("{}: {}", name.node, emit_literal(value)))
        .collect();
    format!("{{ {} }}", pairs.join(", "))
}

// --- constraints ---------------------------------------------------------

/// Emits a `constraint` file. `check` is optional;
/// its absence changes no other line, since the padding width is fixed.
pub fn emit_constraint(c: &Constraint) -> String {
    let mut out = String::new();
    w!(
        out,
        "constraint {} {} {} {{\n",
        c.id,
        constraint_kind(c.kind),
        quote(&c.title)
    );

    keyword(&mut out, 1, "rule", width::CONSTRAINT);
    match &c.rule {
        // A quoted rule is prose; a bare one is machine-checkable.
        Rule::Text(text) => w!(out, "{}\n", quote(text)),
        Rule::Machine(expr) => w!(out, "{}\n", emit_expr(expr)),
    }

    keyword(&mut out, 1, "scope", width::CONSTRAINT);
    match &c.scope {
        Scope::Global => out.push_str("global\n"),
        Scope::Intents(ids) => {
            // Sorted ascending by id regardless of model order:
            // this is a canonicalization, not an echo of source order.
            let mut ids: Vec<IntentId> = ids.iter().map(|id| id.node).collect();
            ids.sort();
            let list: Vec<String> = ids.iter().map(IntentId::to_string).collect();
            w!(out, "{}\n", list.join(", "));
        }
    }

    if let Some(check) = &c.check {
        keyword(&mut out, 1, "check", width::CONSTRAINT);
        w!(out, "{}\n", quote(check));
    }

    out.push_str("}\n");
    out
}

fn constraint_kind(k: ConstraintKind) -> &'static str {
    match k {
        ConstraintKind::Stack => "stack",
        ConstraintKind::Architecture => "architecture",
        ConstraintKind::Quality => "quality",
        ConstraintKind::Security => "security",
        ConstraintKind::Convention => "convention",
    }
}

// --- bindings ------------------------------------------------------------

/// Emits `bindings.tel`: every `implements` line
/// first, sorted by (path, intent id), then every `proves` line, sorted by
/// (test locator, scenario id).
///
/// The sort happens here rather than at the call site so that no caller can
/// write an out-of-order bindings file. An empty binding list yields an
/// empty string: an empty file is valid, and an empty file is zero bytes,
/// not a lone newline.
pub fn emit_bindings(bindings: &[Binding]) -> String {
    let mut implements: Vec<(&str, IntentId)> = Vec::new();
    let mut proves: Vec<(String, ScenarioId)> = Vec::new();
    for binding in bindings {
        match binding {
            Binding::Implements { path, intent } => implements.push((path.as_str(), intent.node)),
            // The locator is sorted (and emitted) as the single string it is
            // written as, `path` or `path::test_name`.
            Binding::Proves { test, scenario } => proves.push((test.to_string(), scenario.node)),
        }
    }
    implements.sort();
    proves.sort();

    let mut out = String::new();
    for (path, intent) in implements {
        keyword(&mut out, 0, "implements", width::BINDING);
        w!(out, "{} -> {intent}\n", quote(path));
    }
    for (test, scenario) in proves {
        keyword(&mut out, 0, "proves", width::BINDING);
        w!(out, "{} -> {scenario}\n", quote(&test));
    }
    out
}

// --- changes -------------------------------------------------------------

/// Emits a `changes/CHG-NNNN.tel` file.
///
/// The header string is the change's motivation; `status` and `digest`
/// share one padding group, and `digest` is written exactly when the change
/// carries an approval. Then one blank line before each op -- the same
/// single rule the intent emitter uses for scenarios, which separates the
/// header from the first op and the ops from each other at once.
///
/// Ops are emitted in staged order and **never sorted**: unlike a
/// constraint's scope or an intent's `requires`, the order of a
/// transaction's operations is data. Each op is [`emit_op`]'s output shifted
/// one level right, which is what nests a whole entity block -- header line,
/// body and closing brace -- inside the change without the entity emitters
/// knowing anything about changes.
///
/// The journal comes last, separated from the ops by a single
/// blank line and written contiguously in append order -- also never sorted,
/// and for a stronger reason: it is a chronology, and the change journal records no
/// timestamps precisely because the order of the lines *is* the time.
pub fn emit_change(c: &Change) -> String {
    let mut out = String::new();
    w!(out, "change {} {} {{\n", c.id, quote(&c.motivation));

    keyword(&mut out, 1, "status", width::CHANGE);
    w!(out, "{}\n", c.status.as_str());
    if let Some(digest) = &c.approved_digest {
        keyword(&mut out, 1, "digest", width::CHANGE);
        w!(out, "{}\n", quote(digest));
    }

    for op in &c.ops {
        out.push('\n');
        out.push_str(&indent(&emit_op(op), 1));
    }

    // The journal closes the block: one blank line separates it
    // from whatever precedes it -- the last op, or the header of a change
    // that has none -- and its lines are contiguous, in append order. A
    // change with no journal keeps the canonical transaction shape.
    if !c.journal.is_empty() {
        out.push('\n');
        for entry in &c.journal {
            out.push_str(&indent(&emit_journal_entry(entry), 1));
        }
    }

    out.push_str("}\n");
    out
}

/// Emits one staged operation at indentation level 0, ending in exactly one
/// `\n`. A file-level fragment, and the unit of input of
/// [`crate::model::Change::ops_digest`].
///
/// `add` and `edit` fuse `op <verb> ` onto the first line of the entity's
/// own canonical block, so the entity is written by `emit_notion` /
/// `emit_intent` / `emit_constraint` verbatim -- kind, title and all. That
/// verbatim reuse is what lets the parser hand the nested block
/// straight back to the entity block parsers, and what keeps `edit` a complete
/// post-state rather than a lossy summary.
///
/// `remove` and `accept` have no entity block and are one line each.
pub fn emit_op(op: &StagedOp) -> String {
    match op {
        StagedOp::AddNotion(n) => format!("op add {}", emit_notion(n)),
        StagedOp::EditNotion(n) => format!("op edit {}", emit_notion(n)),
        StagedOp::RemoveNotion(name) => format!("op remove notion {name}\n"),
        StagedOp::AddIntent(i) => format!("op add {}", emit_intent(i)),
        StagedOp::EditIntent(i) => format!("op edit {}", emit_intent(i)),
        StagedOp::RemoveIntent(id) => format!("op remove intent {id}\n"),
        StagedOp::AddConstraint(c) => format!("op add {}", emit_constraint(c)),
        StagedOp::EditConstraint(c) => format!("op edit {}", emit_constraint(c)),
        StagedOp::RemoveConstraint(id) => format!("op remove constraint {id}\n"),
        StagedOp::EditConfig(config) => {
            let mut config = config.clone();
            config.normalize();
            let mut out = String::from("op edit config {\n");
            for glob in &config.code.globs {
                w!(out, "  code_glob  {}\n", quote(glob));
            }
            for glob in &config.tests.globs {
                w!(out, "  test_glob  {}\n", quote(glob));
            }
            w!(out, "  test_cmd   {}\n", quote(&config.test.cmd));
            w!(
                out,
                "  tdd        {}\n",
                match config.policy.tdd {
                    crate::config::TddPolicy::Strict => "strict",
                    crate::config::TddPolicy::Advisory => "advisory",
                }
            );
            for host in &config.agents.hosts {
                w!(
                    out,
                    "  agent_host {}\n",
                    match host {
                        crate::config::AgentHost::Claude => "claude",
                        crate::config::AgentHost::Codex => "codex",
                    }
                );
            }
            out.push_str("}\n");
            out
        }
        StagedOp::Accept { path, oid } => {
            format!("op accept {} {}\n", quote(path.as_str()), quote(&oid.0))
        }
    }
}

/// Emits one journal line at indentation level 0, ending in exactly one
/// `\n`.
///
/// `run` and `bind` share one padding group of width 4, so the two line
/// kinds keep a common column and adding a `bind` to a journal of `run`s
/// reflows nothing. Both strings are quoted like any other: a test locator
/// is written as the single `path[::name]` string [`crate::model::TestRef`]
/// displays, and the oid is written verbatim -- it is opaque, never
/// parsed, only compared.
///
/// Unlike [`emit_op`], this is *not* an input of any digest: the journal is
/// written after the approval it must not move.
pub fn emit_journal_entry(e: &JournalEntry) -> String {
    let mut out = String::new();
    match e {
        JournalEntry::Run(run) => {
            keyword(&mut out, 0, "run", width::JOURNAL);
            w!(
                out,
                "{} {} {} {}\n",
                run.scenario,
                run.witness.as_str(),
                quote(&run.test.to_string()),
                quote(&run.oid.0)
            );
        }
        JournalEntry::Bind { path, intent } => {
            keyword(&mut out, 0, "bind", width::JOURNAL);
            w!(out, "{} -> {intent}\n", quote(path.as_str()));
        }
    }
    out
}

/// The canonical block of one scenario, alone: exactly the bytes
/// [`emit_intent`] writes for it, indentation included.
///
/// This is a *fingerprint*, not a file: the witness logic compares the
/// fragment of a scenario in the base against the same scenario in a
/// change's post-model to decide whether it moved. Emission is what
/// makes that comparison sound -- spans differ between a parsed scenario and
/// a staged one and would defeat structural equality, and the emitter has no
/// notion of them.
pub(crate) fn emit_scenario_fragment(s: &Scenario) -> String {
    let mut out = String::new();
    emit_scenario(&mut out, s);
    out
}

// --- expressions ---------------------------------------------------------

/// Emits an expression with minimal parentheses. A fragment: no
/// trailing newline.
pub fn emit_expr(e: &Expr) -> String {
    let mut out = String::new();
    // 0 is looser than every operator, so the outermost node is never
    // parenthesized.
    write_expr(&mut out, e, 0);
    out
}

/// Binding power, loosest first: `or` < `and` < `not` < comparison.
fn precedence(e: &Expr) -> u8 {
    match e {
        Expr::Or(..) => 1,
        Expr::And(..) => 2,
        Expr::Not(..) => 3,
        Expr::Cmp { .. } | Expr::In { .. } => 4,
    }
}

/// Writes `e` as a child of a node of binding power `parent`.
///
/// Parentheses go in exactly when the child binds strictly looser than its
/// parent -- the minimal set that survives a re-parse. Equal power needs
/// none: `and` and `or` chains re-associate to the left on re-parse, which
/// prints identically, so idempotence holds without them.
fn write_expr(out: &mut String, e: &Expr, parent: u8) {
    let here = precedence(e);
    let parens = here < parent;
    if parens {
        out.push('(');
    }
    match e {
        Expr::Or(lhs, rhs) => {
            write_expr(out, lhs, here);
            out.push_str(" or ");
            write_expr(out, rhs, here);
        }
        Expr::And(lhs, rhs) => {
            write_expr(out, lhs, here);
            out.push_str(" and ");
            write_expr(out, rhs, here);
        }
        Expr::Not(inner) => {
            out.push_str("not ");
            write_expr(out, inner, here);
        }
        Expr::Cmp { op, lhs, rhs } => {
            w!(out, "{} {} {}", operand(lhs), cmp_op(*op), operand(rhs));
        }
        Expr::In { lhs, set } => {
            let literals: Vec<String> = set.iter().map(emit_literal).collect();
            w!(out, "{} in ({})", operand(lhs), literals.join(", "));
        }
    }
    if parens {
        out.push(')');
    }
}

fn cmp_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn operand(o: &Operand) -> String {
    match o {
        Operand::Ref(r) => attr_ref(r),
        Operand::Lit(l) => emit_literal(l),
    }
}

fn attr_ref(r: &AttrRef) -> String {
    format!("{}.{}", r.notion.node, r.attr.node)
}

/// Emits a literal. A fragment: no trailing newline.
///
/// `decimal`, `date` and `datetime` re-emit the lexeme they were parsed
/// from the canonical literal rules -- a decimal amount never goes through a float, so
/// `120.50` can never come back as `120.49999999999999`.
pub fn emit_literal(l: &Literal) -> String {
    match l {
        Literal::Str(s) => quote(s),
        Literal::Int(n) => n.to_string(),
        Literal::Decimal(lexeme) | Literal::Date(lexeme) | Literal::Datetime(lexeme) => {
            lexeme.clone()
        }
        Literal::Bool(b) => b.to_string(),
        Literal::Symbol(s) => s.node.clone(),
    }
}

// --- shared layout helpers ----------------------------------------------

/// Writes `level` indents, then `word` right-padded to `width`, then the one
/// space that separates a keyword from its value.
fn keyword(out: &mut String, level: usize, word: &str, width: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
    out.push_str(word);
    for _ in 0..width.saturating_sub(word.len()) {
        out.push(' ');
    }
    out.push(' ');
}

/// Writes a field name right-padded to the width of its block, plus the
/// separating space.
fn pad_name(out: &mut String, name: &FieldName, width: usize) {
    out.push_str(name.as_str());
    for _ in 0..width.saturating_sub(name.as_str().len()) {
        out.push(' ');
    }
    out.push(' ');
}

/// The longest name of a block, which is the column its values start at.
fn longest<'a>(names: impl Iterator<Item = &'a str>) -> usize {
    names.map(str::len).max().unwrap_or(0)
}

/// Shifts a whole emitted block `level` indentation levels to the right.
///
/// Only non-empty lines are prefixed: a blank separator line inside a nested
/// block stays zero bytes wide, because canonical `.tel` never carries
/// trailing whitespace and indenting an empty line would create some. The
/// block's own trailing newline is preserved (`str::lines` drops it, and one
/// `\n` is written back after every line), so the result still ends in
/// exactly one `\n`.
fn indent(block: &str, level: usize) -> String {
    let mut out = String::with_capacity(block.len() + block.len() / 4);
    for line in block.lines() {
        if !line.is_empty() {
            for _ in 0..level {
                out.push_str(INDENT);
            }
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Quotes a string: double quotes, `\"` and `\\` are the only escapes.
/// A newline can never appear in a `.tel` string, so none is emitted.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ConstraintId, NotionName, RepoPath};
    use crate::span::{Sp, Span};

    fn notion() -> Notion {
        Notion {
            name: NotionName::new("Invoice").unwrap(),
            kind: NotionKind::Entity,
            def: "A bill.".to_string(),
            attrs: vec![],
            rels: vec![],
        }
    }

    fn constraint() -> Constraint {
        Constraint {
            id: ConstraintId(3),
            kind: ConstraintKind::Architecture,
            title: "Hexagonal boundaries".to_string(),
            rule: Rule::Text("No adapter imports.".to_string()),
            scope: Scope::Global,
            check: None,
        }
    }

    fn intent() -> Intent {
        Intent {
            id: IntentId(42),
            title: "Invoices settle".to_string(),
            status: IntentStatus::Active,
            telos: "Trust.".to_string(),
            statement: Statement::Ubiquitous {
                action: Action::Free("track invoices".to_string()),
            },
            refines: vec![],
            requires: vec![],
            excludes: vec![],
            scenarios: vec![],
        }
    }

    #[test]
    fn emit_file_routes_each_variant_to_its_emitter() {
        assert_eq!(
            emit_file(&TelFile::Notion(notion())),
            emit_notion(&notion())
        );
        assert_eq!(
            emit_file(&TelFile::Intent(intent())),
            emit_intent(&intent())
        );
        assert_eq!(
            emit_file(&TelFile::Constraint(constraint())),
            emit_constraint(&constraint())
        );
        let bindings = vec![Binding::Implements {
            path: RepoPath::new("src/a.rs"),
            intent: Sp {
                node: IntentId(1),
                span: Span::default(),
            },
        }];
        assert_eq!(
            emit_file(&TelFile::Bindings(bindings.clone())),
            emit_bindings(&bindings)
        );
    }

    #[test]
    fn keyword_padding_is_the_group_width_plus_one_space() {
        // The padding widths, spelled out as the strings they produce.
        let mut out = String::new();
        keyword(&mut out, 1, "def", width::NOTION);
        keyword(&mut out, 0, "telos", width::INTENT);
        keyword(&mut out, 0, "if", width::STATEMENT);
        keyword(&mut out, 0, "then", width::SCENARIO);
        keyword(&mut out, 0, "rule", width::CONSTRAINT);
        keyword(&mut out, 0, "proves", width::BINDING);
        assert_eq!(out, "  def  telos  if     then  rule  proves     ");
    }

    #[test]
    fn a_keyword_as_long_as_its_group_gets_a_single_space() {
        let mut out = String::new();
        keyword(&mut out, 0, "status", width::INTENT);
        keyword(&mut out, 0, "implements", width::BINDING);
        assert_eq!(out, "status implements ");
    }

    #[test]
    fn quote_escapes_only_quotes_and_backslashes() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("a \"b\""), "\"a \\\"b\\\"\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        // Tabs and other characters pass through untouched: the lexer never
        // produced an escape for them, so the emitter must not invent one.
        assert_eq!(quote("a\tb"), "\"a\tb\"");
    }

    // --- changes ---------------------------------------------------------

    mod changes {
        use super::*;
        use crate::git::Oid;
        use crate::ids::ChangeId;
        use crate::model::ChangeStatus;
        use crate::model::change::fixtures::{
            CHANGE_EXAMPLE, JOURNAL_EXAMPLE, con_0003, empty_change, example_change,
            implementing_change, int_0017, invoice, notion_name, run, run_oid,
        };
        use crate::model::{TestRun, Witness};

        #[test]
        fn emit_change_reproduces_the_canonical_example_byte_for_byte() {
            assert_eq!(emit_change(&example_change()), CHANGE_EXAMPLE);
        }

        // --- the journal ----------------------------------------

        #[test]
        fn emit_change_reproduces_the_journal_example_byte_for_byte() {
            assert_eq!(emit_change(&implementing_change()), JOURNAL_EXAMPLE);
        }

        #[test]
        fn a_run_line_is_the_keyword_the_scenario_the_verdict_the_test_and_the_oid() {
            assert_eq!(
                emit_journal_entry(&run(Witness::Red)),
                "run  SCN-0001 red \"tests/billing.rs::scn_0001_a_full_payment_settles_the_invoice\" \
                 \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n"
            );
            assert_eq!(
                emit_journal_entry(&run(Witness::Green)),
                "run  SCN-0001 green \"tests/billing.rs::scn_0001_a_full_payment_settles_the_invoice\" \
                 \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n"
            );
        }

        #[test]
        fn a_bind_line_is_the_keyword_the_path_an_arrow_and_the_intent() {
            assert_eq!(
                emit_journal_entry(&JournalEntry::Bind {
                    path: RepoPath::new("src/billing.rs"),
                    intent: IntentId(1),
                }),
                "bind \"src/billing.rs\" -> INT-0001\n"
            );
        }

        #[test]
        fn run_and_bind_share_one_padding_group_of_width_four() {
            // `run` is padded to `bind`'s length, so the two line kinds line
            // up in a column and neither reflows the other.
            let run_line = emit_journal_entry(&run(Witness::Red));
            let bind_line = emit_journal_entry(&JournalEntry::Bind {
                path: RepoPath::new("src/billing.rs"),
                intent: IntentId(1),
            });
            assert!(run_line.starts_with("run  SCN-"), "{run_line}");
            assert!(bind_line.starts_with("bind \""), "{bind_line}");
            assert_eq!(width::JOURNAL, 4);
        }

        #[test]
        fn a_test_reference_without_a_name_is_emitted_as_a_bare_path() {
            let entry = JournalEntry::Run(TestRun {
                scenario: ScenarioId(7),
                witness: Witness::Green,
                test: "tests/billing.rs".parse().unwrap(),
                oid: run_oid(),
            });
            assert!(
                emit_journal_entry(&entry)
                    .starts_with("run  SCN-0007 green \"tests/billing.rs\" \"e69de29"),
                "{}",
                emit_journal_entry(&entry)
            );
        }

        #[test]
        fn every_journal_line_ends_in_exactly_one_newline() {
            for entry in &implementing_change().journal {
                let emitted = emit_journal_entry(entry);
                assert!(emitted.ends_with('\n'), "{emitted:?}");
                assert!(!emitted.ends_with("\n\n"), "{emitted:?}");
            }
        }

        #[test]
        fn one_blank_line_opens_the_journal_block_and_none_splits_it() {
            let emitted = emit_change(&implementing_change());
            let (_, journal) = emitted.split_once("  }\n").expect("the op block closes");
            assert_eq!(
                journal,
                concat!(
                    "\n",
                    "  run  SCN-0001 red \"tests/billing.rs::scn_0001_a_full_payment_settles_the_invoice\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
                    "  run  SCN-0001 green \"tests/billing.rs::scn_0001_a_full_payment_settles_the_invoice\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
                    "  bind \"src/billing.rs\" -> INT-0001\n",
                    "}\n",
                )
            );
        }

        #[test]
        fn an_empty_journal_adds_no_blank_line_at_all() {
            // The transaction shape must be untouched: a change with no
            // journal is byte-identical to the canonical example.
            assert_eq!(emit_change(&example_change()), CHANGE_EXAMPLE);
            let mut change = empty_change();
            change.journal = vec![];
            assert_eq!(
                emit_change(&change),
                "change CHG-0001 \"x\" {\n  status open\n}\n"
            );
        }

        #[test]
        fn a_journal_on_a_change_with_no_op_still_gets_its_blank_line() {
            // The blank line separates the journal from whatever precedes
            // it, which here is the status line rather than an op.
            let mut change = empty_change();
            change.journal = vec![JournalEntry::Bind {
                path: RepoPath::new("src/billing.rs"),
                intent: IntentId(1),
            }];
            assert_eq!(
                emit_change(&change),
                concat!(
                    "change CHG-0001 \"x\" {\n",
                    "  status open\n",
                    "\n",
                    "  bind \"src/billing.rs\" -> INT-0001\n",
                    "}\n",
                )
            );
        }

        #[test]
        fn journal_lines_are_emitted_in_append_order_never_sorted() {
            let mut change = implementing_change();
            change.journal.reverse();
            let emitted = emit_change(&change);
            let kinds: Vec<String> = emitted
                .lines()
                .filter(|l| l.starts_with("  run ") || l.starts_with("  bind "))
                .map(|l| l.trim_start().split(' ').next().unwrap().to_string())
                .collect();
            assert_eq!(kinds, vec!["bind", "run", "run"]);
            // Within the runs, green now precedes red -- the append order
            // is data, and reversing it must show.
            assert!(emitted.find(" green ") < emitted.find(" red "), "{emitted}");
        }

        #[test]
        fn a_journal_line_never_carries_trailing_whitespace() {
            for line in emit_change(&implementing_change()).lines() {
                assert_eq!(line.trim_end(), line, "trailing whitespace: {line:?}");
            }
        }

        // --- emit_scenario_fragment ---------------------------------------

        #[test]
        fn a_scenario_fragment_is_the_scenario_block_the_intent_emitter_writes() {
            let intent = int_0017();
            let scenario = &intent.scenarios[0];
            let fragment = emit_scenario_fragment(scenario);
            assert!(
                emit_intent(&intent).contains(&fragment),
                "the fragment must be the intent's own bytes:\n{fragment}"
            );
            assert!(fragment.starts_with("  scenario SCN-0091 "), "{fragment}");
            assert!(fragment.ends_with("  }\n"), "{fragment}");
        }

        #[test]
        fn a_scenario_fragment_ignores_spans_by_construction() {
            // Two scenarios equal but for their spans emit the same bytes,
            // which is what makes the fragment a fingerprint.
            let mut a = int_0017();
            let mut b = int_0017();
            a.scenarios[0].given[0].notion.span = crate::span::Span { start: 1, end: 2 };
            b.scenarios[0].given[0].notion.span = crate::span::Span { start: 90, end: 99 };
            assert_ne!(a.scenarios[0], b.scenarios[0]);
            assert_eq!(
                emit_scenario_fragment(&a.scenarios[0]),
                emit_scenario_fragment(&b.scenarios[0])
            );
        }

        #[test]
        fn a_change_with_no_op_is_a_header_a_status_and_a_brace() {
            assert_eq!(
                emit_change(&empty_change()),
                "change CHG-0001 \"x\" {\n  status open\n}\n"
            );
        }

        #[test]
        fn the_digest_line_is_absent_when_the_change_is_not_approved() {
            let mut change = example_change();
            change.status = ChangeStatus::Drafted;
            change.approved_digest = None;
            let emitted = emit_change(&change);
            assert!(!emitted.contains("digest"), "{emitted}");
            // Dropping `digest` must not reflow `status`: the padding width
            // is a property of the group, not of the lines present.
            assert!(emitted.starts_with(
                "change CHG-0007 \"Invoices can be settled\" {\n  status drafted\n\n  op add"
            ));
        }

        #[test]
        fn every_status_is_written_as_its_keyword() {
            for status in [
                ChangeStatus::Open,
                ChangeStatus::Drafted,
                ChangeStatus::Approved,
                ChangeStatus::Implementing,
                ChangeStatus::Abandoned,
            ] {
                let mut change = empty_change();
                change.status = status;
                assert_eq!(
                    emit_change(&change),
                    format!(
                        "change CHG-0001 \"x\" {{\n  status {}\n}}\n",
                        status.as_str()
                    )
                );
            }
        }

        #[test]
        fn ops_are_emitted_in_staged_order_never_sorted() {
            let mut change = example_change();
            change.ops.reverse();
            let emitted = emit_change(&change);
            let order: Vec<&str> = emitted.lines().filter(|l| l.starts_with("  op ")).collect();
            assert_eq!(
                order,
                vec![
                    "  op accept \"telos/telos.toml\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"",
                    "  op remove constraint CON-0003",
                    "  op edit intent INT-0017 \"Issuing an invoice opens it\" {",
                    "  op add notion Invoice entity {",
                ]
            );
        }

        #[test]
        fn exactly_one_blank_line_precedes_each_op() {
            let emitted = emit_change(&example_change());
            for (i, line) in emitted.lines().enumerate() {
                if line.starts_with("  op ") {
                    let previous = emitted.lines().nth(i - 1).unwrap();
                    assert_eq!(previous, "", "line {i} is `{line}`");
                }
            }
            assert!(!emitted.contains("\n\n\n"), "no double blank line");
        }

        #[test]
        fn no_line_of_a_change_ever_carries_trailing_whitespace() {
            // The intent block nested in the example has an internal blank
            // line (before its scenario): indenting it must leave it empty.
            let emitted = emit_change(&example_change());
            for line in emitted.lines() {
                assert_eq!(line.trim_end(), line, "trailing whitespace: {line:?}");
            }
            assert!(emitted.contains("    }\n\n    scenario SCN-0091"));
        }

        // --- emit_op ------------------------------------------------------

        #[test]
        fn emit_op_add_fuses_the_verb_onto_the_entity_block_at_level_zero() {
            assert_eq!(
                emit_op(&StagedOp::AddNotion(invoice())),
                format!("op add {}", emit_notion(&invoice()))
            );
            assert_eq!(
                emit_op(&StagedOp::AddIntent(int_0017())),
                format!("op add {}", emit_intent(&int_0017()))
            );
            assert_eq!(
                emit_op(&StagedOp::AddConstraint(con_0003())),
                format!("op add {}", emit_constraint(&con_0003()))
            );
        }

        #[test]
        fn emit_op_edit_differs_from_add_only_in_the_verb() {
            let add = emit_op(&StagedOp::AddIntent(int_0017()));
            let edit = emit_op(&StagedOp::EditIntent(int_0017()));
            assert_eq!(add.replacen("op add ", "op edit ", 1), edit);
        }

        #[test]
        fn emit_op_remove_is_a_single_line_per_entity_kind() {
            assert_eq!(
                emit_op(&StagedOp::RemoveNotion(notion_name("Invoice"))),
                "op remove notion Invoice\n"
            );
            assert_eq!(
                emit_op(&StagedOp::RemoveIntent(IntentId(42))),
                "op remove intent INT-0042\n"
            );
            assert_eq!(
                emit_op(&StagedOp::RemoveConstraint(ConstraintId(3))),
                "op remove constraint CON-0003\n"
            );
        }

        #[test]
        fn emit_op_accept_is_a_single_line_of_two_quoted_strings() {
            assert_eq!(
                emit_op(&StagedOp::Accept {
                    path: RepoPath::new("telos/telos.toml"),
                    oid: Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
                }),
                "op accept \"telos/telos.toml\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n"
            );
        }

        #[test]
        fn emit_op_quotes_a_path_that_needs_escaping() {
            assert_eq!(
                emit_op(&StagedOp::Accept {
                    path: RepoPath::new("src/say \"hi\".rs"),
                    oid: Oid("abc".to_string()),
                }),
                "op accept \"src/say \\\"hi\\\".rs\" \"abc\"\n"
            );
        }

        #[test]
        fn every_op_ends_in_exactly_one_newline() {
            let mut ops = example_change().ops;
            ops.push(StagedOp::RemoveNotion(notion_name("Invoice")));
            ops.push(StagedOp::AddConstraint(con_0003()));
            for op in &ops {
                let emitted = emit_op(op);
                assert!(emitted.ends_with('\n'), "{emitted:?}");
                assert!(!emitted.ends_with("\n\n"), "{emitted:?}");
            }
        }

        #[test]
        fn a_change_is_its_ops_indented_one_level_between_header_and_brace() {
            // The relation the digest depends on: what `emit_change` writes
            // for an op is exactly `emit_op`'s bytes, shifted.
            let change = example_change();
            let emitted = emit_change(&change);
            for op in &change.ops {
                assert!(
                    emitted.contains(&indent(&emit_op(op), 1)),
                    "missing op block:\n{}",
                    emit_op(op)
                );
            }
        }

        // --- indent -------------------------------------------------------

        #[test]
        fn indent_shifts_non_empty_lines_and_leaves_blank_ones_empty() {
            assert_eq!(indent("a\n\nb\n", 1), "  a\n\n  b\n");
            assert_eq!(indent("a\n", 2), "    a\n");
            assert_eq!(indent("a\n", 0), "a\n");
        }

        #[test]
        fn indent_preserves_relative_indentation() {
            assert_eq!(indent("a {\n  b\n}\n", 1), "  a {\n    b\n  }\n");
        }

        #[test]
        fn indent_of_an_empty_block_is_empty() {
            assert_eq!(indent("", 1), "");
        }

        #[test]
        fn change_id_is_written_in_its_display_form() {
            let mut change = empty_change();
            change.id = ChangeId(7);
            assert!(emit_change(&change).starts_with("change CHG-0007 "));
        }

        #[test]
        fn a_motivation_that_needs_escaping_is_quoted() {
            let mut change = empty_change();
            change.motivation = "say \"hi\"".to_string();
            assert!(emit_change(&change).starts_with("change CHG-0001 \"say \\\"hi\\\"\" {\n"));
        }
    }
}
