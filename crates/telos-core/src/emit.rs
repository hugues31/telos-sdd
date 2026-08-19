//! The canonical `.tel` emitter (Annex C.4).
//!
//! This module is the *only* writer of `.tel` syntax in the engine: the
//! canonical form is not a style guide checked after the fact, it is
//! whatever these functions produce. Every mutation path (M2) re-emits a
//! whole file from the model rather than editing text, so a spec tree can
//! never drift into a second, almost-canonical dialect.
//!
//! The contract that makes that safe is byte-level idempotence:
//! `emit(parse(s)) == s` for every canonical `s` (proved on the Annex D
//! corpus in `tests/roundtrip.rs`). Three consequences shape the code below:
//!
//! - **Padding is per group, with fixed widths** (C.4.5). A keyword is
//!   padded to the longest keyword its group *could* hold -- not the longest
//!   one actually present -- so adding a `check` line to a constraint, or
//!   dropping a `rel` from a notion, never reflows the columns of its
//!   neighbours, and a diff stays the size of the edit.
//! - **Order is normalized, not echoed** (C.4.3). `attrs` and `rels` keep
//!   insertion order because their order is data; relation ids, scenarios
//!   and bindings are sorted, because theirs is not.
//! - **Parentheses are minimal** (C.4.9): one is emitted exactly when a
//!   child binds looser than its parent.
//!
//! Every file-level function returns a `String` ending in exactly one `\n`
//! (the empty bindings file excepted -- an empty file is zero bytes).
//! Nothing here touches the filesystem: callers decide where bytes go.

use crate::ids::{FieldName, IntentId, ScenarioId};
use crate::model::{
    Action, Attr, AttrRef, AttrType, Binding, CmpOp, Constraint, ConstraintKind, Expr,
    InstanceStep, Intent, IntentStatus, Literal, Notion, NotionKind, Operand, Rel, Rule, Scenario,
    Scope, Statement, TelFile,
};
use crate::span::Sp;

/// `write!` into a `String`: `fmt::Write` on a `String` is infallible, so
/// the `Result` carries no information worth threading through.
macro_rules! w {
    ($out:expr, $($arg:tt)*) => {{
        use std::fmt::Write as _;
        let _ = write!($out, $($arg)*);
    }};
}

/// One indentation level (C.4.1).
const INDENT: &str = "  ";

/// Keyword padding widths, one per field group (C.4.5). Each is the length
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

/// Emits a `notion` file (C.2 `notion-file`).
///
/// Two paddings stack here: the field keyword to the group width (C.4.5),
/// then -- inside the `attr` block and inside the `rel` block independently
/// -- the field *name* to the longest name of its own block (C.4.6), so the
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
        // C.4.10: no space after `enum`, `, ` between symbols.
        AttrType::Enum(symbols) => format!("enum({})", symbols.join(", ")),
        AttrType::Ref(target) => format!("ref({target})"),
    }
}

// --- intents -------------------------------------------------------------

/// Emits an `intent` file (C.2 `intent-file`), statement block, relation
/// lines and scenarios included.
///
/// Blank lines appear in exactly one place (C.4.4): before each scenario,
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

    // Relation lines take no padding (C.4.5), and each group is sorted by
    // id (C.4.3); the groups themselves stay in grammar order.
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

/// `instance-body` on one line (C.4.7); an empty payload is `{}`, with no
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

/// Emits a `constraint` file (C.2 `constraint-file`). `check` is optional;
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
        // A quoted rule is prose, a bare one is machine-checkable (C.2).
        Rule::Text(text) => w!(out, "{}\n", quote(text)),
        Rule::Machine(expr) => w!(out, "{}\n", emit_expr(expr)),
    }

    keyword(&mut out, 1, "scope", width::CONSTRAINT);
    match &c.scope {
        Scope::Global => out.push_str("global\n"),
        Scope::Intents(ids) => {
            let list: Vec<String> = ids.iter().map(|id| id.node.to_string()).collect();
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

/// Emits `bindings.tel` (C.2 `bindings-file`): every `implements` line
/// first, sorted by (path, intent id), then every `proves` line, sorted by
/// (test locator, scenario id) (C.4.2).
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

// --- expressions ---------------------------------------------------------

/// Emits an expression (C.3) with minimal parentheses. A fragment: no
/// trailing newline.
pub fn emit_expr(e: &Expr) -> String {
    let mut out = String::new();
    // 0 is looser than every operator, so the outermost node is never
    // parenthesized.
    write_expr(&mut out, e, 0);
    out
}

/// Binding power, loosest first: `or` < `and` < `not` < comparison (C.3).
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

/// Emits a literal (C.3). A fragment: no trailing newline.
///
/// `decimal`, `date` and `datetime` re-emit the lexeme they were parsed
/// from (C.4.10) -- a decimal amount never goes through a float, so
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
/// space that separates a keyword from its value (C.4.5).
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
/// separating space (C.4.6).
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

/// Quotes a string (C.4.8): double quotes, `\"` and `\\` the only escapes.
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
        // The widths of C.4.5, spelled out as the strings they produce.
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
}
