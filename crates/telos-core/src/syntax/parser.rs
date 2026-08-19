//! Recursive-descent parser for the `.tel` syntax, built on the Task 4
//! lexer.
//!
//! Covers every rule of Annex C.2 (`notion-file`, `intent-file` with its
//! EARS `statement-block` and its `scenario-decl`s, `constraint-file`,
//! `bindings-file`) and Annex C.3's expression mini-language.
//!
//! Diagnostics all share one shape (« expected X, found `Y` »), and every
//! file rule recovers from a field-level error by skipping the rest of the
//! offending line, so one pass reports as much as it soundly can. Recovery
//! is brace-depth aware: an error inside a nested block never
//! resynchronizes on that block's `}`.

use std::str::FromStr;

use crate::error::{Diagnostic, ErrorCode};
use crate::ids::{ConstraintId, EntityRef, FieldName, IntentId, NotionName, RepoPath, ScenarioId};
use crate::model::{
    Action, Attr, AttrRef, AttrType, Binding, CmpOp, Constraint, ConstraintKind, Expr,
    InstanceStep, Intent, IntentStatus, Literal, Notion, NotionKind, Operand, Rel, Rule, Scenario,
    Scope, Statement, TestRef,
};
use crate::span::{Sp, Span, line_col};
use crate::suggest::closest;

use super::lexer::{TokKind, Token, lex};

/// The five `notion-kind` words (Annex C.2), in grammar order.
const NOTION_KINDS: [(&str, NotionKind); 5] = [
    ("actor", NotionKind::Actor),
    ("entity", NotionKind::Entity),
    ("value", NotionKind::Value),
    ("event", NotionKind::Event),
    ("state", NotionKind::State),
];

/// The `attr-type` head words (Annex C.2), in grammar order.
const ATTR_TYPES: [&str; 9] = [
    "string", "int", "decimal", "money", "bool", "date", "datetime", "enum", "ref",
];

/// The three `status-field` words (Annex C.2), in grammar order.
const INTENT_STATUSES: [(&str, IntentStatus); 3] = [
    ("draft", IntentStatus::Draft),
    ("active", IntentStatus::Active),
    ("deprecated", IntentStatus::Deprecated),
];

/// The five EARS `template` words (Annex C.2), in grammar order.
const TEMPLATES: [(&str, Template); 5] = [
    ("ubiquitous", Template::Ubiquitous),
    ("event-driven", Template::EventDriven),
    ("state-driven", Template::StateDriven),
    ("unwanted", Template::Unwanted),
    ("optional", Template::Optional),
];

/// The five `constraint-kind` words (Annex C.2), in grammar order.
const CONSTRAINT_KINDS: [(&str, ConstraintKind); 5] = [
    ("stack", ConstraintKind::Stack),
    ("architecture", ConstraintKind::Architecture),
    ("quality", ConstraintKind::Quality),
    ("security", ConstraintKind::Security),
    ("convention", ConstraintKind::Convention),
];

/// The tail keywords of an `intent-file` block (Annex C.2), in grammar
/// order: each may appear zero or more times, but never before the
/// previous one.
const INTENT_TAIL: [&str; 4] = ["refines", "requires", "excludes", "scenario"];

/// The keywords still accepted at `stage` of an intent block, plus `}`.
fn tail_options(stage: usize) -> Vec<String> {
    INTENT_TAIL[stage..]
        .iter()
        .map(|kw| format!("`{kw}`"))
        .chain(["`}`".to_string()])
        .collect()
}

/// The step keywords still accepted inside a scenario block, plus `}` once
/// the block could legally close.
fn step_options(seen_given: bool, seen_when: bool, no_then: bool) -> Vec<String> {
    let mut options = Vec::new();
    if seen_when {
        options.push("`then`".to_string());
        if !no_then {
            options.push("`}`".to_string());
        }
    } else {
        options.push("`given`".to_string());
        if seen_given {
            options.push("`when`".to_string());
        }
    }
    options
}

/// The EARS template a `statement` block announces, which decides the shape
/// of its `template-body`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Template {
    Ubiquitous,
    EventDriven,
    StateDriven,
    Unwanted,
    Optional,
}

/// How a bracketed comma-separated list ends: the closing token, how to
/// render it, and how to name the list in the separator error message.
struct ListClose {
    kind: TokKind,
    text: &'static str,
    list: &'static str,
}

/// Parses a whole `notion` file (Annex C.2).
///
/// Reports every diagnostic found: a syntax error inside a field line is
/// recovered from (skip to the end of the line, then resume at the next
/// field keyword or `}`), so one file can yield several diagnostics.
pub fn parse_notion_file(path: &RepoPath, src: &str) -> Result<Notion, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.notion_file())
}

/// Parses a whole `intent` file (Annex C.2), statement block and scenarios
/// included.
///
/// Recovers like `parse_notion_file`, brace-depth aware: an error inside a
/// nested block (statement, scenario, instance body) resumes at the end of
/// the offending line, never on the nested block's `}`.
pub fn parse_intent_file(path: &RepoPath, src: &str) -> Result<Intent, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.intent_file())
}

/// Parses a whole `constraint` file (Annex C.2).
pub fn parse_constraint_file(path: &RepoPath, src: &str) -> Result<Constraint, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.constraint_file())
}

/// Parses the `bindings.tel` file (Annex C.2): zero or more binding lines.
/// An empty file is valid and yields no binding.
pub fn parse_bindings_file(path: &RepoPath, src: &str) -> Result<Vec<Binding>, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.bindings_file())
}

/// Runs one file-level rule over `src`: lexes, parses, and returns either
/// the node (when nothing at all was reported) or every diagnostic
/// collected, the fatal one last.
fn parse_file<T>(
    path: &RepoPath,
    src: &str,
    rule: impl FnOnce(&mut P<'_>) -> Result<T, Diagnostic>,
) -> Result<T, Vec<Diagnostic>> {
    let mut p = match P::new(src, Some(path)) {
        Ok(p) => p,
        Err(diag) => return Err(vec![diag]),
    };
    match rule(&mut p) {
        Ok(node) if p.diags.is_empty() => Ok(node),
        Ok(_) => Err(p.diags),
        Err(diag) => {
            p.diags.push(diag);
            Err(p.diags)
        }
    }
}

/// Parses a single expression (Annex C.3) from `src`, which must be fully
/// consumed. Returns the first error; recovery is a whole-file concern, so
/// there is none here.
pub fn parse_expr(src: &str) -> Result<Expr, Diagnostic> {
    let mut p = P::new(src, None)?;
    p.skip_newlines();
    let expr = p.expr()?;
    p.skip_newlines();
    if !p.at_eof() {
        return Err(p.expected("end of input"));
    }
    Ok(expr)
}

/// Parser state: the token stream plus the cursor, the current brace
/// nesting depth (error recovery needs it), the source (for positions and
/// for rendering the token a diagnostic was found at), the file the
/// diagnostics belong to, and the diagnostics collected so far.
struct P<'a> {
    toks: Vec<Token>,
    pos: usize,
    depth: u32,
    src: &'a str,
    path: Option<&'a RepoPath>,
    diags: Vec<Diagnostic>,
}

impl<'a> P<'a> {
    /// Lexes `src`. A lexical error is returned straight away, tagged with
    /// `path` (the lexer knows no file).
    fn new(src: &'a str, path: Option<&'a RepoPath>) -> Result<Self, Diagnostic> {
        let toks = lex(src).map_err(|mut diag| {
            diag.file = path.cloned();
            diag
        })?;
        Ok(Self {
            toks,
            pos: 0,
            depth: 0,
            src,
            path,
            diags: Vec::new(),
        })
    }

    // --- cursor -----------------------------------------------------------

    /// The current token. `lex` always terminates the stream with `Eof` and
    /// `advance` never steps past it, so this never panics.
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    /// Consumes the current token, keeping `depth` in step with the braces
    /// crossed -- the single choke point through which tokens are read, so
    /// the count cannot drift.
    fn advance(&mut self) {
        match self.peek().kind {
            TokKind::LBrace => self.depth += 1,
            TokKind::RBrace => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
    }

    fn at(&self, kind: &TokKind) -> bool {
        &self.peek().kind == kind
    }

    /// Keywords are unreserved (Annex C.1) -- they are `LowerIdent`s
    /// matched contextually, here.
    fn at_kw(&self, kw: &str) -> bool {
        matches!(&self.peek().kind, TokKind::LowerIdent(word) if word == kw)
    }

    fn at_eof(&self) -> bool {
        self.at(&TokKind::Eof)
    }

    /// Blank lines are a layout concern (the emitter's and the round-trip
    /// test's job); the parser silently accepts runs of them.
    fn skip_newlines(&mut self) {
        while self.at(&TokKind::Newline) {
            self.advance();
        }
    }

    // --- diagnostics ------------------------------------------------------

    fn diag_at(&self, span: Span, message: String, hint: Option<String>) -> Diagnostic {
        let (line, col) = line_col(self.src, span.start);
        Diagnostic {
            code: ErrorCode::TelosParseError,
            message,
            hint,
            file: self.path.cloned(),
            line: Some(line),
            col: Some(col),
        }
    }

    /// Renders the current token for the `found ...` half of a message.
    fn found(&self) -> String {
        let tok = self.peek();
        match tok.kind {
            TokKind::Eof => "end of input".to_string(),
            TokKind::Newline => "end of line".to_string(),
            _ => format!(
                "`{}`",
                &self.src[tok.span.start as usize..tok.span.end as usize]
            ),
        }
    }

    /// The one syntax-error shape: « expected X, found `Y` ».
    fn expected(&self, what: &str) -> Diagnostic {
        self.diag_at(
            self.peek().span,
            format!("expected {what}, found {}", self.found()),
            None,
        )
    }

    /// `expected` over a set of alternatives that depends on how far
    /// through a block we are: « expected `a`, `b` or `c`, found `X` ».
    fn expected_one_of(&self, options: &[String]) -> Diagnostic {
        let what = match options {
            [] => String::new(),
            [only] => only.clone(),
            [head @ .., last] => format!("{} or {last}", head.join(", ")),
        };
        self.expected(&what)
    }

    /// Error recovery: swallow the rest of the offending line so parsing
    /// can resume at the next field keyword or `}`.
    ///
    /// `home` is the brace depth of the block being recovered inside, so a
    /// nested block is crossed whole: only a newline *at* `home` ends the
    /// line, and only a `}` at `home` is left for the block's own loop to
    /// see. At `home == 0` (the bindings file, which has no block at all)
    /// there is no `}` to protect, so braces are skipped like any other
    /// token -- which also guarantees progress on every call.
    fn recover_to_newline(&mut self, home: u32) {
        loop {
            if self.at_eof() {
                return;
            }
            if home > 0 && self.depth <= home && self.at(&TokKind::RBrace) {
                return;
            }
            let done = self.depth <= home && self.at(&TokKind::Newline);
            self.advance();
            if done {
                return;
            }
        }
    }

    /// Runs a field-level rule, recovering from its failure: the
    /// diagnostic is collected and the rest of the offending line skipped,
    /// so the fields that follow are still checked.
    fn recovered<T>(
        &mut self,
        home: u32,
        rule: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
    ) -> Option<T> {
        match rule(self) {
            Ok(node) => Some(node),
            Err(diag) => {
                self.diags.push(diag);
                self.recover_to_newline(home);
                None
            }
        }
    }

    /// After a file's closing `}`: nothing but blank lines may follow.
    fn end_of_file(&mut self) {
        self.skip_newlines();
        if !self.at_eof() {
            let diag = self.expected("end of input");
            self.diags.push(diag);
        }
    }

    // --- token-level helpers ---------------------------------------------

    fn expect(&mut self, kind: &TokKind, what: &str) -> Result<Span, Diagnostic> {
        if !self.at(kind) {
            return Err(self.expected(what));
        }
        let span = self.peek().span;
        self.advance();
        Ok(span)
    }

    fn expect_lower_kw(&mut self, kw: &str) -> Result<Span, Diagnostic> {
        if !self.at_kw(kw) {
            return Err(self.expected(&format!("`{kw}`")));
        }
        let span = self.peek().span;
        self.advance();
        Ok(span)
    }

    fn expect_str(&mut self, what: &str) -> Result<Sp<String>, Diagnostic> {
        let TokKind::Str(text) = &self.peek().kind else {
            return Err(self.expected(what));
        };
        let node = text.clone();
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    fn expect_notion_name(&mut self) -> Result<Sp<NotionName>, Diagnostic> {
        let TokKind::UpperIdent(text) = &self.peek().kind else {
            return Err(self.expected("a notion name"));
        };
        let span = self.peek().span;
        let node = NotionName::new(text.clone())
            .map_err(|err| self.diag_at(span, err.message, err.hint))?;
        self.advance();
        Ok(Sp { node, span })
    }

    fn expect_field_name(&mut self) -> Result<Sp<FieldName>, Diagnostic> {
        let TokKind::LowerIdent(text) = &self.peek().kind else {
            return Err(self.expected("a field name"));
        };
        let span = self.peek().span;
        let node = FieldName::new(text.clone())
            .map_err(|err| self.diag_at(span, err.message, err.hint))?;
        self.advance();
        Ok(Sp { node, span })
    }

    /// A bare `lower-ident` used as an enum symbol.
    fn expect_symbol(&mut self, what: &str) -> Result<String, Diagnostic> {
        let TokKind::LowerIdent(text) = &self.peek().kind else {
            return Err(self.expected(what));
        };
        let symbol = text.clone();
        self.advance();
        Ok(symbol)
    }

    /// The `INT-NNNN` form of an `id-lit` (Annex C.1).
    fn expect_intent_id(&mut self) -> Result<Sp<IntentId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Intent(id)) = &self.peek().kind else {
            return Err(self.expected("an intent id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// The `SCN-NNNN` form of an `id-lit` (Annex C.1).
    fn expect_scenario_id(&mut self) -> Result<Sp<ScenarioId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Scenario(id)) = &self.peek().kind else {
            return Err(self.expected("a scenario id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// The `CON-NNNN` form of an `id-lit` (Annex C.1).
    fn expect_constraint_id(&mut self) -> Result<Sp<ConstraintId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Constraint(id)) = &self.peek().kind else {
            return Err(self.expected("a constraint id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// A word out of a closed set (notion kind, intent status, statement
    /// template, constraint kind), reported with the closest known word
    /// when it is not one of them.
    fn word_from_set<T: Copy>(
        &mut self,
        noun: &str,
        table: &[(&'static str, T)],
    ) -> Result<T, Diagnostic> {
        let TokKind::LowerIdent(word) = &self.peek().kind else {
            return Err(self.expected(&format!("a {noun}")));
        };
        let span = self.peek().span;
        if let Some(entry) = table.iter().find(|entry| entry.0 == word.as_str()) {
            let value = entry.1;
            self.advance();
            return Ok(value);
        }
        let message = match closest(word, table.iter().map(|entry| entry.0)) {
            Some(known) => format!("unknown {noun} `{word}`; closest is `{known}`"),
            None => format!("unknown {noun} `{word}`"),
        };
        Err(self.diag_at(span, message, None))
    }

    /// `attr-ref = upper-ident , "." , lower-ident` (Annex C.3).
    fn attr_ref(&mut self) -> Result<AttrRef, Diagnostic> {
        let notion = self.expect_notion_name()?;
        self.expect(&TokKind::Dot, "`.`")?;
        let attr = self.expect_field_name()?;
        Ok(AttrRef { notion, attr })
    }

    /// The `elem , { "," , elem }` shape shared by enum symbol lists, `in`
    /// literal sets, constraint scopes and instance bodies.
    ///
    /// `close` names the token that ends the list; `None` means the list
    /// runs to the end of the line (the `scope` field), and the caller
    /// decides what may follow. `allow_empty` permits a list with no
    /// element at all (the empty instance body `{}`).
    fn comma_list<T>(
        &mut self,
        close: Option<ListClose>,
        allow_empty: bool,
        mut elem: impl FnMut(&mut Self) -> Result<T, Diagnostic>,
    ) -> Result<Vec<T>, Diagnostic> {
        let mut items = Vec::new();
        if let Some(close) = &close
            && allow_empty
            && self.at(&close.kind)
        {
            self.advance();
            return Ok(items);
        }
        items.push(elem(self)?);
        loop {
            if let Some(close) = &close {
                if self.at(&close.kind) {
                    self.advance();
                    return Ok(items);
                }
                if !self.at(&TokKind::Comma) {
                    return Err(
                        self.expected(&format!("`,` or `{}` in {}", close.text, close.list))
                    );
                }
            } else if !self.at(&TokKind::Comma) {
                return Ok(items);
            }
            self.advance();
            items.push(elem(self)?);
        }
    }

    /// Fields are newline-separated; `}` and end of input close the last
    /// one without a newline of their own.
    fn end_of_field(&mut self) -> Result<(), Diagnostic> {
        if self.at(&TokKind::Newline) {
            self.advance();
            return Ok(());
        }
        if self.at_eof() || self.at(&TokKind::RBrace) {
            return Ok(());
        }
        Err(self.expected("end of line"))
    }

    // --- notion files (Annex C.2) ----------------------------------------

    fn notion_file(&mut self) -> Result<Notion, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("notion")?;
        let name = self.expect_notion_name()?;
        let kind = self.notion_kind()?;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;
        self.skip_newlines();

        let def = self.recovered(home, |p| p.def_field()).unwrap_or_default();

        let mut attrs = Vec::new();
        let mut rels = Vec::new();
        // The grammar order (`def`, then attrs, then rels) is enforced: an
        // `attr` after a `rel` is a syntax error, not a reordering.
        let mut in_rels = false;

        loop {
            self.skip_newlines();
            let what = if in_rels {
                "`rel` or `}`"
            } else {
                "`attr`, `rel` or `}`"
            };
            if self.at(&TokKind::RBrace) {
                self.advance();
                break;
            }
            if self.at_eof() {
                return Err(self.expected(what));
            }
            if self.at_kw("attr") && !in_rels {
                if let Some(attr) = self.recovered(home, |p| p.attr_field()) {
                    attrs.push(attr);
                }
                continue;
            }
            if self.at_kw("rel") {
                in_rels = true;
                if let Some(rel) = self.recovered(home, |p| p.rel_field()) {
                    rels.push(rel);
                }
                continue;
            }
            let diag = self.expected(what);
            self.diags.push(diag);
            self.recover_to_newline(home);
        }

        self.end_of_file();

        Ok(Notion {
            name: name.node,
            kind,
            def,
            attrs,
            rels,
        })
    }

    fn notion_kind(&mut self) -> Result<NotionKind, Diagnostic> {
        self.word_from_set("notion kind", &NOTION_KINDS)
    }

    fn def_field(&mut self) -> Result<String, Diagnostic> {
        self.expect_lower_kw("def")?;
        let text = self.expect_str("a definition string")?;
        self.end_of_field()?;
        Ok(text.node)
    }

    fn attr_field(&mut self) -> Result<Attr, Diagnostic> {
        self.expect_lower_kw("attr")?;
        let name = self.expect_field_name()?;
        let ty = self.attr_type()?;
        self.end_of_field()?;
        Ok(Attr {
            name: name.node,
            ty,
        })
    }

    fn attr_type(&mut self) -> Result<AttrType, Diagnostic> {
        let TokKind::LowerIdent(word) = &self.peek().kind else {
            return Err(self.expected("an attribute type"));
        };
        let span = self.peek().span;
        let ty = match word.as_str() {
            "string" => AttrType::String,
            "int" => AttrType::Int,
            "decimal" => AttrType::Decimal,
            "money" => AttrType::Money,
            "bool" => AttrType::Bool,
            "date" => AttrType::Date,
            "datetime" => AttrType::Datetime,
            "enum" => {
                self.advance();
                return self.enum_type();
            }
            "ref" => {
                self.advance();
                return self.ref_type();
            }
            unknown => {
                let hint =
                    closest(unknown, ATTR_TYPES).map(|known| format!("closest is `{known}`"));
                return Err(self.diag_at(
                    span,
                    format!("expected an attribute type, found `{unknown}`"),
                    hint,
                ));
            }
        };
        self.advance();
        Ok(ty)
    }

    /// `enum` is already consumed: `"(" , lower-ident , { "," , lower-ident } , ")"`.
    fn enum_type(&mut self) -> Result<AttrType, Diagnostic> {
        self.expect(&TokKind::LParen, "`(`")?;
        let symbols = self.comma_list(
            Some(ListClose {
                kind: TokKind::RParen,
                text: ")",
                list: "enum symbol list",
            }),
            false,
            |p| p.expect_symbol("an enum symbol"),
        )?;
        Ok(AttrType::Enum(symbols))
    }

    /// `ref` is already consumed: `"(" , upper-ident , ")"`.
    fn ref_type(&mut self) -> Result<AttrType, Diagnostic> {
        self.expect(&TokKind::LParen, "`(`")?;
        let target = self.expect_notion_name()?;
        self.expect(&TokKind::RParen, "`)`")?;
        Ok(AttrType::Ref(target.node))
    }

    fn rel_field(&mut self) -> Result<Rel, Diagnostic> {
        self.expect_lower_kw("rel")?;
        let name = self.expect_field_name()?;
        self.expect(&TokKind::Arrow, "`->`")?;
        let target = self.expect_notion_name()?;
        self.end_of_field()?;
        Ok(Rel {
            name: name.node,
            target,
        })
    }

    // --- intent files (Annex C.2) ----------------------------------------

    fn intent_file(&mut self) -> Result<Intent, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("intent")?;
        let id = self.expect_intent_id()?.node;
        let title = self.expect_str("an intent title")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;

        self.skip_newlines();
        let status = self
            .recovered(home, |p| p.status_field())
            .unwrap_or(IntentStatus::Draft);
        self.skip_newlines();
        let telos = self
            .recovered(home, |p| p.telos_field())
            .unwrap_or_default();
        self.skip_newlines();
        // Unlike the fields above, a statement has no stand-in: an intent
        // without one is not an intent, so the error is fatal.
        let statement = self.statement_block()?;

        let mut refines = Vec::new();
        let mut requires = Vec::new();
        let mut excludes = Vec::new();
        let mut scenarios = Vec::new();
        // The grammar order (refines, requires, excludes, scenarios) is
        // enforced: `stage` only ever moves forward.
        let mut stage = 0usize;

        loop {
            self.skip_newlines();
            if self.at(&TokKind::RBrace) {
                self.advance();
                break;
            }
            if self.at_eof() {
                return Err(self.expected_one_of(&tail_options(stage)));
            }
            let found = INTENT_TAIL[stage..].iter().position(|kw| self.at_kw(kw));
            let Some(offset) = found else {
                let diag = self.expected_one_of(&tail_options(stage));
                self.diags.push(diag);
                self.recover_to_newline(home);
                continue;
            };
            stage += offset;
            match stage {
                0 => {
                    if let Some(id) = self.recovered(home, |p| p.relation_line("refines")) {
                        refines.push(id);
                    }
                }
                1 => {
                    if let Some(id) = self.recovered(home, |p| p.relation_line("requires")) {
                        requires.push(id);
                    }
                }
                2 => {
                    if let Some(id) = self.recovered(home, |p| p.relation_line("excludes")) {
                        excludes.push(id);
                    }
                }
                _ => {
                    if let Some(scenario) = self.recovered(home, |p| p.scenario_decl()) {
                        scenarios.push(scenario);
                    }
                }
            }
        }

        self.end_of_file();

        Ok(Intent {
            id,
            title,
            status,
            telos,
            statement,
            refines,
            requires,
            excludes,
            scenarios,
        })
    }

    fn status_field(&mut self) -> Result<IntentStatus, Diagnostic> {
        self.expect_lower_kw("status")?;
        let status = self.word_from_set("status", &INTENT_STATUSES)?;
        self.end_of_field()?;
        Ok(status)
    }

    fn telos_field(&mut self) -> Result<String, Diagnostic> {
        self.expect_lower_kw("telos")?;
        let text = self.expect_str("a telos string")?;
        self.end_of_field()?;
        Ok(text.node)
    }

    /// `refines-line` / `requires-line` / `excludes-line`: one intent id
    /// per line.
    fn relation_line(&mut self, kw: &str) -> Result<Sp<IntentId>, Diagnostic> {
        self.expect_lower_kw(kw)?;
        let id = self.expect_intent_id()?;
        self.end_of_field()?;
        Ok(id)
    }

    /// `statement-block = "statement" , template , "{" , template-body , "}"`.
    fn statement_block(&mut self) -> Result<Statement, Diagnostic> {
        self.expect_lower_kw("statement")?;
        let template = self.word_from_set("statement template", &TEMPLATES)?;
        self.expect(&TokKind::LBrace, "`{`")?;
        self.skip_newlines();
        let statement = self.template_body(template)?;
        self.skip_newlines();
        self.expect(&TokKind::RBrace, "`}`")?;
        self.end_of_field()?;
        Ok(statement)
    }

    /// The `template-body` the announced template calls for: each of the
    /// four non-ubiquitous templates opens with its own head line, then
    /// they all end on the shall-line.
    fn template_body(&mut self, template: Template) -> Result<Statement, Diagnostic> {
        match template {
            Template::Ubiquitous => Ok(Statement::Ubiquitous {
                action: self.shall_line()?,
            }),
            Template::EventDriven => {
                self.expect_body_head("event-driven", "when")?;
                let event = self.expect_notion_name()?;
                let on = if self.at_kw("on") {
                    self.advance();
                    Some(self.expect_notion_name()?)
                } else {
                    None
                };
                self.end_of_field()?;
                Ok(Statement::EventDriven {
                    event,
                    on,
                    action: self.shall_line()?,
                })
            }
            Template::StateDriven => {
                self.expect_body_head("state-driven", "while")?;
                let subject = self.attr_ref()?;
                self.expect(&TokKind::EqEq, "`==`")?;
                let value = self.parse_literal("a literal")?;
                self.end_of_field()?;
                Ok(Statement::StateDriven {
                    subject,
                    value,
                    action: self.shall_line()?,
                })
            }
            Template::Unwanted => {
                self.expect_body_head("unwanted", "if")?;
                let condition = self.expr()?;
                self.end_of_field()?;
                Ok(Statement::Unwanted {
                    condition,
                    action: self.shall_line()?,
                })
            }
            Template::Optional => {
                self.expect_body_head("optional", "where")?;
                let feature = self.expect_field_name()?;
                self.end_of_field()?;
                Ok(Statement::Optional {
                    feature: feature.node,
                    action: self.shall_line()?,
                })
            }
        }
    }

    /// The head keyword a `template-body` must open with. Its own error
    /// shape names the template, since a body that opens on the wrong word
    /// is a mismatch, not a missing token.
    fn expect_body_head(&mut self, template: &str, head: &str) -> Result<(), Diagnostic> {
        if self.at_kw(head) {
            self.advance();
            return Ok(());
        }
        Err(self.diag_at(
            self.peek().span,
            format!(
                "template `{template}` expects `{head} …`, found {}",
                self.found()
            ),
            None,
        ))
    }

    /// `shall-line = "system" , "shall" , action`.
    fn shall_line(&mut self) -> Result<Action, Diagnostic> {
        self.expect_lower_kw("system")?;
        self.expect_lower_kw("shall")?;
        let action = self.action()?;
        self.end_of_field()?;
        Ok(action)
    }

    /// `action = "set" , attr-ref , "=" , literal | string-lit` -- the free
    /// clause is the one place a spec may fall back to prose.
    fn action(&mut self) -> Result<Action, Diagnostic> {
        if self.at_kw("set") {
            self.advance();
            let target = self.attr_ref()?;
            self.expect(&TokKind::Assign, "`=`")?;
            let value = self.parse_literal("a literal")?;
            return Ok(Action::Set { target, value });
        }
        let text = self.expect_str("`set` or a quoted clause")?;
        Ok(Action::Free(text.node))
    }

    // --- scenarios (Annex C.2) -------------------------------------------

    /// `scenario-decl`: at least one `given`, exactly one `when`, at least
    /// one `then`, in that order.
    fn scenario_decl(&mut self) -> Result<Scenario, Diagnostic> {
        self.expect_lower_kw("scenario")?;
        let id = self.expect_scenario_id()?.node;
        let title = self.expect_str("a scenario title")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;

        let mut given = Vec::new();
        let mut when: Option<InstanceStep> = None;
        let mut then = Vec::new();
        // Set as soon as a `given` line is *attempted*: a `when` that
        // follows a faulty `given` is in its rightful place, and must not
        // be reported a second time.
        let mut seen_given = false;

        loop {
            self.skip_newlines();
            let options = step_options(seen_given, when.is_some(), then.is_empty());
            if self.at(&TokKind::RBrace) {
                let Some(when) = when else {
                    return Err(self.expected_one_of(&options));
                };
                if then.is_empty() {
                    return Err(self.expected_one_of(&options));
                }
                self.advance();
                return Ok(Scenario {
                    id,
                    title,
                    given,
                    when,
                    then,
                });
            }
            if self.at_eof() {
                return Err(self.expected_one_of(&options));
            }
            if when.is_none() && self.at_kw("given") {
                seen_given = true;
                if let Some(step) = self.recovered(home, |p| p.instance_step("given")) {
                    given.push(step);
                }
                continue;
            }
            if when.is_none() && seen_given && self.at_kw("when") {
                // Fatal, unlike the other steps: a scenario has no shape
                // without the event that triggers it.
                when = Some(self.instance_step("when")?);
                continue;
            }
            if when.is_some() && self.at_kw("then") {
                if let Some(expr) = self.recovered(home, |p| p.then_step()) {
                    then.push(expr);
                }
                continue;
            }
            let diag = self.expected_one_of(&options);
            self.diags.push(diag);
            self.recover_to_newline(home);
        }
    }

    /// `given-step` / `when-step`: a notion name plus its instance body.
    fn instance_step(&mut self, kw: &str) -> Result<InstanceStep, Diagnostic> {
        self.expect_lower_kw(kw)?;
        let notion = self.expect_notion_name()?;
        let fields = self.instance_body()?;
        self.end_of_field()?;
        Ok(InstanceStep { notion, fields })
    }

    /// `instance-body = "{" , [ field-val , { "," , field-val } ] , "}"`.
    fn instance_body(&mut self) -> Result<Vec<(Sp<FieldName>, Literal)>, Diagnostic> {
        self.expect(&TokKind::LBrace, "`{`")?;
        self.comma_list(
            Some(ListClose {
                kind: TokKind::RBrace,
                text: "}",
                list: "instance body",
            }),
            true,
            |p| p.field_val(),
        )
    }

    /// `field-val = lower-ident , ":" , literal`.
    fn field_val(&mut self) -> Result<(Sp<FieldName>, Literal), Diagnostic> {
        let name = self.expect_field_name()?;
        self.expect(&TokKind::Colon, "`:`")?;
        let value = self.parse_literal("a literal")?;
        Ok((name, value))
    }

    /// `then-step = "then" , expr`.
    fn then_step(&mut self) -> Result<Expr, Diagnostic> {
        self.expect_lower_kw("then")?;
        let expr = self.expr()?;
        self.end_of_field()?;
        Ok(expr)
    }

    // --- constraint files (Annex C.2) ------------------------------------

    fn constraint_file(&mut self) -> Result<Constraint, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("constraint")?;
        let id = self.expect_constraint_id()?.node;
        let kind = self.word_from_set("constraint kind", &CONSTRAINT_KINDS)?;
        let title = self.expect_str("a constraint title")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;

        self.skip_newlines();
        let rule = self
            .recovered(home, |p| p.rule_field())
            .unwrap_or_else(|| Rule::Text(String::new()));
        self.skip_newlines();
        let scope = self
            .recovered(home, |p| p.scope_field())
            .unwrap_or(Scope::Global);
        self.skip_newlines();

        let mut check = None;
        if self.at_kw("check") {
            check = self.recovered(home, |p| p.check_field());
            self.skip_newlines();
        }

        if !self.at(&TokKind::RBrace) {
            let mut options = Vec::new();
            if check.is_none() {
                options.push("`check`".to_string());
            }
            options.push("`}`".to_string());
            return Err(self.expected_one_of(&options));
        }
        self.advance();
        self.end_of_file();

        Ok(Constraint {
            id,
            kind,
            title,
            rule,
            scope,
            check,
        })
    }

    /// `rule-field = "rule" , ( string-lit | expr )`: quoted is prose, bare
    /// is machine-checkable.
    fn rule_field(&mut self) -> Result<Rule, Diagnostic> {
        self.expect_lower_kw("rule")?;
        let rule = if matches!(self.peek().kind, TokKind::Str(_)) {
            Rule::Text(self.expect_str("a rule string")?.node)
        } else {
            Rule::Machine(self.expr()?)
        };
        self.end_of_field()?;
        Ok(rule)
    }

    /// `scope-field = "scope" , ( "global" | intent-id , { "," , intent-id } )`.
    fn scope_field(&mut self) -> Result<Scope, Diagnostic> {
        self.expect_lower_kw("scope")?;
        let scope = if self.at_kw("global") {
            self.advance();
            Scope::Global
        } else if matches!(self.peek().kind, TokKind::IdLit(_)) {
            Scope::Intents(self.comma_list(None, false, |p| p.expect_intent_id())?)
        } else {
            return Err(self.expected("`global` or an intent id"));
        };
        self.end_of_field()?;
        Ok(scope)
    }

    fn check_field(&mut self) -> Result<String, Diagnostic> {
        self.expect_lower_kw("check")?;
        let cmd = self.expect_str("a check command")?;
        self.end_of_field()?;
        Ok(cmd.node)
    }

    // --- bindings file (Annex C.2) ---------------------------------------

    /// `bindings-file = { binding-line }` -- an empty file is valid.
    fn bindings_file(&mut self) -> Result<Vec<Binding>, Diagnostic> {
        let mut bindings = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                return Ok(bindings);
            }
            // `home` is 0: a bindings file has no block, so recovery
            // always consumes at least the offending token.
            if let Some(binding) = self.recovered(0, |p| p.binding_line()) {
                bindings.push(binding);
            }
        }
    }

    fn binding_line(&mut self) -> Result<Binding, Diagnostic> {
        if self.at_kw("implements") {
            self.advance();
            let path = self.expect_str("a code path")?;
            self.expect(&TokKind::Arrow, "`->`")?;
            let intent = self.expect_intent_id()?;
            self.end_of_field()?;
            return Ok(Binding::Implements {
                path: RepoPath::new(path.node),
                intent,
            });
        }
        if self.at_kw("proves") {
            self.advance();
            let text = self.expect_str("a test reference")?;
            let test = TestRef::from_str(&text.node)
                .map_err(|err| self.diag_at(text.span, err.message, err.hint))?;
            self.expect(&TokKind::Arrow, "`->`")?;
            let scenario = self.expect_scenario_id()?;
            self.end_of_field()?;
            return Ok(Binding::Proves { test, scenario });
        }
        Err(self.expected("`implements` or `proves`"))
    }

    // --- expressions (Annex C.3) -----------------------------------------

    /// `expr = or-expr`; precedence `or` < `and` < `not` < comparison.
    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.and_expr()?;
        while self.at_kw("or") {
            self.advance();
            let rhs = self.and_expr()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.unary()?;
        while self.at_kw("and") {
            self.advance();
            let rhs = self.unary()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        if self.at_kw("not") {
            self.advance();
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    /// A parenthesized sub-expression, or a comparison. Operands are never
    /// parenthesized (Annex C.3), so there is no ambiguity here.
    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        if self.at(&TokKind::LParen) {
            self.advance();
            let inner = self.expr()?;
            self.expect(&TokKind::RParen, "`)`")?;
            return Ok(inner);
        }
        self.comparison()
    }

    /// An expression is always an assertion: a bare operand is an error.
    fn comparison(&mut self) -> Result<Expr, Diagnostic> {
        let lhs = self.parse_attr_ref_or_literal()?;
        if self.at_kw("in") {
            self.advance();
            self.expect(&TokKind::LParen, "`(`")?;
            let set = self.comma_list(
                Some(ListClose {
                    kind: TokKind::RParen,
                    text: ")",
                    list: "literal list",
                }),
                false,
                |p| p.parse_literal("a literal"),
            )?;
            return Ok(Expr::In { lhs, set });
        }
        let Some(op) = self.cmp_op() else {
            return Err(self.expected("a comparison operator or `in`"));
        };
        self.advance();
        let rhs = self.parse_attr_ref_or_literal()?;
        Ok(Expr::Cmp { op, lhs, rhs })
    }

    fn cmp_op(&self) -> Option<CmpOp> {
        Some(match self.peek().kind {
            TokKind::EqEq => CmpOp::Eq,
            TokKind::Ne => CmpOp::Ne,
            TokKind::Lt => CmpOp::Lt,
            TokKind::Le => CmpOp::Le,
            TokKind::Gt => CmpOp::Gt,
            TokKind::Ge => CmpOp::Ge,
            _ => return None,
        })
    }

    /// `operand = attr-ref | literal`, where `attr-ref = upper-ident, ".",
    /// lower-ident` -- a lone `lower-ident` is an enum symbol literal.
    fn parse_attr_ref_or_literal(&mut self) -> Result<Operand, Diagnostic> {
        if matches!(self.peek().kind, TokKind::UpperIdent(_)) {
            return Ok(Operand::Ref(self.attr_ref()?));
        }
        Ok(Operand::Lit(
            self.parse_literal("an attribute reference or a literal")?,
        ))
    }

    fn parse_literal(&mut self, what: &str) -> Result<Literal, Diagnostic> {
        let span = self.peek().span;
        let literal = match &self.peek().kind {
            TokKind::Str(text) => Literal::Str(text.clone()),
            TokKind::Int(value) => Literal::Int(*value),
            TokKind::Decimal(lexeme) => Literal::Decimal(lexeme.clone()),
            TokKind::Date(lexeme) => Literal::Date(lexeme.clone()),
            TokKind::Datetime(lexeme) => Literal::Datetime(lexeme.clone()),
            TokKind::LowerIdent(word) if word == "true" => Literal::Bool(true),
            TokKind::LowerIdent(word) if word == "false" => Literal::Bool(false),
            TokKind::LowerIdent(word) => Literal::Symbol(Sp {
                node: word.clone(),
                span,
            }),
            _ => return Err(self.expected(what)),
        };
        self.advance();
        Ok(literal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Annex D, `telos/notions/Invoice.tel`, byte for byte (the corpus
    /// files themselves are created in Task 7).
    const INVOICE_TEL: &str = concat!(
        "notion Invoice entity {\n",
        "  def  \"A bill issued to a Customer for delivered work.\"\n",
        "  attr state   enum(open, settled, cancelled)\n",
        "  attr balance money\n",
        "  rel  issued-to -> Customer\n",
        "}\n",
    );

    /// Annex D, `telos/intents/INT-0042.tel`, byte for byte.
    const INT_0042_TEL: &str = concat!(
        "intent INT-0042 \"Invoice payment marks it settled\" {\n",
        "  status active\n",
        "  telos  \"Customers must see immediately that their debt is cleared.\"\n",
        "  statement event-driven {\n",
        "    when   PaymentReceived on Invoice\n",
        "    system shall set Invoice.state = settled\n",
        "  }\n",
        "  requires INT-0017\n",
        "\n",
        "  scenario SCN-0107 \"full payment settles the invoice\" {\n",
        "    given Invoice { state: open, balance: \"120.00 EUR\" }\n",
        "    when  PaymentReceived { amount: \"120.00 EUR\" }\n",
        "    then  Invoice.state == settled\n",
        "  }\n",
        "}\n",
    );

    /// Annex D, `telos/intents/INT-0017.tel`, byte for byte.
    const INT_0017_TEL: &str = concat!(
        "intent INT-0017 \"Issuing an invoice opens it\" {\n",
        "  status active\n",
        "  telos  \"An invoice must start its life open and unpaid.\"\n",
        "  statement event-driven {\n",
        "    when   InvoiceIssued on Invoice\n",
        "    system shall set Invoice.state = open\n",
        "  }\n",
        "\n",
        "  scenario SCN-0091 \"a newly issued invoice is open\" {\n",
        "    given Customer { name: \"ACME\" }\n",
        "    when  InvoiceIssued {}\n",
        "    then  Invoice.state == open\n",
        "  }\n",
        "}\n",
    );

    /// Annex D, `telos/constraints/CON-0003.tel`, byte for byte.
    const CON_0003_TEL: &str = concat!(
        "constraint CON-0003 architecture \"Hexagonal boundaries\" {\n",
        "  rule  \"Domain code must not import adapter modules.\"\n",
        "  scope global\n",
        "  check \"scripts/check-imports.sh --layer domain\"\n",
        "}\n",
    );

    /// Annex D, `telos/bindings.tel`, byte for byte.
    const BINDINGS_TEL: &str = concat!(
        "implements \"src/billing/invoice.rs\" -> INT-0042\n",
        "proves     \"tests/billing.rs::scn_0107_full_payment_settles_the_invoice\" -> SCN-0107\n",
    );

    fn path() -> RepoPath {
        RepoPath::new("telos/notions/Invoice.tel")
    }

    fn intent_path() -> RepoPath {
        RepoPath::new("telos/intents/INT-0042.tel")
    }

    fn constraint_path() -> RepoPath {
        RepoPath::new("telos/constraints/CON-0003.tel")
    }

    fn bindings_path() -> RepoPath {
        RepoPath::new("telos/bindings.tel")
    }

    fn nname(s: &str) -> NotionName {
        NotionName::new(s).unwrap()
    }

    fn fname(s: &str) -> FieldName {
        FieldName::new(s).unwrap()
    }

    fn syms(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// The span of the first `word` at or after the first `anchor` in `src`
    /// -- how the tests pin the exact position a node was parsed from
    /// without counting occurrences by hand.
    fn span_after(src: &str, anchor: &str, word: &str) -> Span {
        let from = src.find(anchor).expect("anchor not found");
        let start = (from + src[from..].find(word).expect("word not found")) as u32;
        Span {
            start,
            end: start + word.len() as u32,
        }
    }

    fn notion_at(src: &str, anchor: &str, word: &str) -> Sp<NotionName> {
        Sp {
            node: nname(word),
            span: span_after(src, anchor, word),
        }
    }

    fn field_at(src: &str, anchor: &str, word: &str) -> Sp<FieldName> {
        Sp {
            node: fname(word),
            span: span_after(src, anchor, word),
        }
    }

    fn symbol_at(src: &str, anchor: &str, word: &str) -> Literal {
        Literal::Symbol(Sp {
            node: word.to_string(),
            span: span_after(src, anchor, word),
        })
    }

    /// A minimal intent whose statement block is the one under test.
    fn intent_with_statement(template: &str, body: &str) -> String {
        format!(
            concat!(
                "intent INT-0001 \"t\" {{\n",
                "  status draft\n",
                "  telos  \"why\"\n",
                "  statement {} {{\n",
                "{}\n",
                "  }}\n",
                "}}\n",
            ),
            template, body
        )
    }

    /// A minimal intent whose only scenario is the one under test.
    fn intent_with_scenario(steps: &str) -> String {
        format!(
            concat!(
                "intent INT-0001 \"t\" {{\n",
                "  status draft\n",
                "  telos  \"why\"\n",
                "  statement ubiquitous {{\n",
                "    system shall \"do it\"\n",
                "  }}\n",
                "\n",
                "  scenario SCN-0107 \"s\" {{\n",
                "{}\n",
                "  }}\n",
                "}}\n",
            ),
            steps
        )
    }

    /// A minimal constraint whose `rule`/`scope`/`check` lines are the ones
    /// under test.
    fn constraint_with(fields: &str) -> String {
        format!("constraint CON-0003 architecture \"t\" {{\n{fields}\n}}\n")
    }

    // --- notion files -----------------------------------------------------

    #[test]
    fn parses_the_annex_d_invoice_into_the_exact_expected_ast() {
        let notion = parse_notion_file(&path(), INVOICE_TEL).unwrap();

        assert_eq!(notion.name, nname("Invoice"));
        assert_eq!(notion.kind, NotionKind::Entity);
        assert_eq!(
            notion.def,
            "A bill issued to a Customer for delivered work."
        );
        assert_eq!(
            notion.attrs,
            vec![
                Attr {
                    name: fname("state"),
                    ty: AttrType::Enum(syms(&["open", "settled", "cancelled"])),
                },
                Attr {
                    name: fname("balance"),
                    ty: AttrType::Money,
                },
            ]
        );
        assert_eq!(notion.rels.len(), 1);
        assert_eq!(notion.rels[0].name, fname("issued-to"));
        assert_eq!(notion.rels[0].target.node, nname("Customer"));
        // The span points at the `Customer` of the `rel` line (the last
        // occurrence -- the def string mentions one too).
        let at = INVOICE_TEL.rfind("Customer").unwrap() as u32;
        assert_eq!(
            notion.rels[0].target.span,
            Span {
                start: at,
                end: at + 8
            }
        );
    }

    #[test]
    fn parses_every_attr_type() {
        let src = concat!(
            "notion Thing value {\n",
            "  def  \"d\"\n",
            "  attr a string\n",
            "  attr b int\n",
            "  attr c decimal\n",
            "  attr d money\n",
            "  attr e bool\n",
            "  attr f date\n",
            "  attr g datetime\n",
            "  attr h ref(Customer)\n",
            "  attr i enum(one)\n",
            "}\n",
        );
        let notion = parse_notion_file(&path(), src).unwrap();
        let types: Vec<AttrType> = notion.attrs.iter().map(|a| a.ty.clone()).collect();
        assert_eq!(
            types,
            vec![
                AttrType::String,
                AttrType::Int,
                AttrType::Decimal,
                AttrType::Money,
                AttrType::Bool,
                AttrType::Date,
                AttrType::Datetime,
                AttrType::Ref(nname("Customer")),
                AttrType::Enum(syms(&["one"])),
            ]
        );
    }

    #[test]
    fn parses_every_notion_kind() {
        let cases = [
            ("actor", NotionKind::Actor),
            ("entity", NotionKind::Entity),
            ("value", NotionKind::Value),
            ("event", NotionKind::Event),
            ("state", NotionKind::State),
        ];
        for (word, expected) in cases {
            let src = format!("notion Thing {word} {{\n  def  \"d\"\n}}\n");
            let notion = parse_notion_file(&path(), &src).unwrap();
            assert_eq!(notion.kind, expected, "kind `{word}`");
        }
    }

    #[test]
    fn a_notion_without_attrs_or_rels_parses() {
        let notion = parse_notion_file(&path(), "notion Thing value {\n  def  \"d\"\n}\n").unwrap();
        assert!(notion.attrs.is_empty());
        assert!(notion.rels.is_empty());
    }

    #[test]
    fn unknown_notion_kind_suggests_the_closest_one() {
        let src = "notion Invoice entty {\n  def  \"d\"\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(
            diags[0].message,
            "unknown notion kind `entty`; closest is `entity`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(1), Some(16)));
    }

    #[test]
    fn unknown_notion_kind_without_a_close_match_is_reported_plainly() {
        let src = "notion Invoice zzzzzz {\n  def  \"d\"\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags[0].message, "unknown notion kind `zzzzzz`");
    }

    #[test]
    fn enum_symbol_list_without_a_comma_reports_the_exact_message_and_position() {
        // Line 3, col 24 is the `s` of `settled`:
        // `  attr state enum(open settled)`
        //  123456789...
        let src = concat!(
            "notion Invoice entity {\n",
            "  def  \"A bill.\"\n",
            "  attr state enum(open settled)\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(
            diags[0].message,
            "expected `,` or `)` in enum symbol list, found `settled`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(3), Some(24)));
        assert_eq!(diags[0].file.as_ref(), Some(&path()));
    }

    #[test]
    fn an_empty_enum_symbol_list_is_a_syntax_error() {
        let src = "notion Invoice entity {\n  def  \"d\"\n  attr state enum()\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected an enum symbol, found `)`");
    }

    #[test]
    fn two_faulty_fields_yield_two_diagnostics_and_parsing_continues() {
        let src = concat!(
            "notion Invoice entity {\n",
            "  def  \"A bill.\"\n",
            "  attr state enum(open settled)\n",
            "  attr balance\n",
            "  rel  issued-to -> Customer\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 2, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `,` or `)` in enum symbol list, found `settled`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(3), Some(24)));
        assert_eq!(
            diags[1].message,
            "expected an attribute type, found end of line"
        );
        assert_eq!((diags[1].line, diags[1].col), (Some(4), Some(15)));
    }

    #[test]
    fn an_attr_after_a_rel_violates_the_grammar_order() {
        let src = concat!(
            "notion Invoice entity {\n",
            "  def  \"d\"\n",
            "  rel  issued-to -> Customer\n",
            "  attr state money\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected `rel` or `}`, found `attr`");
        assert_eq!((diags[0].line, diags[0].col), (Some(4), Some(3)));
    }

    #[test]
    fn a_missing_def_field_is_a_syntax_error() {
        let src = "notion Invoice entity {\n  attr state money\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected `def`, found `attr`");
        assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(3)));
    }

    #[test]
    fn an_unknown_attr_type_suggests_the_closest_one() {
        let src = "notion Invoice entity {\n  def  \"d\"\n  attr balance mony\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected an attribute type, found `mony`");
        assert_eq!(diags[0].hint.as_deref(), Some("closest is `money`"));
    }

    #[test]
    fn a_ref_attr_type_requires_a_notion_name() {
        let src = "notion Invoice entity {\n  def  \"d\"\n  attr owner ref(customer)\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected a notion name, found `customer`");
    }

    #[test]
    fn a_rel_field_requires_an_arrow() {
        let src = "notion Invoice entity {\n  def  \"d\"\n  rel  issued-to Customer\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected `->`, found `Customer`");
    }

    #[test]
    fn blank_lines_between_fields_are_tolerated_by_the_parser() {
        // Canonical form forbids them (Annex C.4 §4) -- that is the
        // emitter's and the round-trip test's job, not the parser's.
        let src = concat!(
            "\n",
            "notion Invoice entity {\n",
            "\n",
            "  def  \"A bill.\"\n",
            "\n",
            "  attr balance money\n",
            "\n",
            "  rel  issued-to -> Customer\n",
            "\n",
            "}\n",
        );
        let notion = parse_notion_file(&path(), src).unwrap();
        assert_eq!(notion.attrs.len(), 1);
        assert_eq!(notion.rels.len(), 1);
    }

    #[test]
    fn a_file_without_a_trailing_newline_parses() {
        let notion = parse_notion_file(&path(), "notion Thing value {\n  def  \"d\"\n}").unwrap();
        assert_eq!(notion.name, nname("Thing"));
    }

    #[test]
    fn content_after_the_closing_brace_is_a_syntax_error() {
        let src = format!("{INVOICE_TEL}notion Other entity {{\n  def  \"d\"\n}}\n");
        let diags = parse_notion_file(&path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected end of input, found `notion`");
    }

    #[test]
    fn a_truncated_file_reports_the_unexpected_end_of_input() {
        let src = "notion Invoice entity {\n  def  \"d\"\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].message,
            "expected `attr`, `rel` or `}`, found end of input"
        );
    }

    #[test]
    fn a_lexer_error_is_reported_against_the_file() {
        let src = "notion Invoice entity {\n  def  \"d\"\n  attr state @\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(diags[0].message, "unexpected character `@`");
        assert_eq!(diags[0].file.as_ref(), Some(&path()));
        assert_eq!((diags[0].line, diags[0].col), (Some(3), Some(14)));
    }

    // --- expressions ------------------------------------------------------

    #[test]
    fn parse_expr_builds_a_comparison_of_a_ref_against_an_enum_symbol() {
        let src = "Invoice.state == settled";
        let expr = parse_expr(src).unwrap();
        let symbol_at = src.find("settled").unwrap() as u32;
        assert_eq!(
            expr,
            Expr::Cmp {
                op: CmpOp::Eq,
                lhs: Operand::Ref(AttrRef {
                    notion: Sp {
                        node: nname("Invoice"),
                        span: Span { start: 0, end: 7 },
                    },
                    attr: Sp {
                        node: fname("state"),
                        span: Span { start: 8, end: 13 },
                    },
                }),
                rhs: Operand::Lit(Literal::Symbol(Sp {
                    node: "settled".to_string(),
                    span: Span {
                        start: symbol_at,
                        end: symbol_at + 7,
                    },
                })),
            }
        );
    }

    #[test]
    fn parse_expr_binds_not_tighter_than_and() {
        // Annex C.3: `or` < `and` < `not` < comparison.
        let expr = parse_expr("not A.b == 1 and C.d in (1, 2)").unwrap();
        let Expr::And(lhs, rhs) = expr else {
            panic!("expected And, got {expr:?}");
        };
        assert!(matches!(*lhs, Expr::Not(inner) if matches!(*inner, Expr::Cmp { .. })));
        let Expr::In { set, .. } = *rhs else {
            panic!("expected In, got {rhs:?}");
        };
        assert_eq!(set, vec![Literal::Int(1), Literal::Int(2)]);
    }

    #[test]
    fn parse_expr_respects_parentheses() {
        let expr = parse_expr("(A.b == 1 or C.d == 2) and E.f == 3").unwrap();
        let Expr::And(lhs, rhs) = expr else {
            panic!("expected And, got {expr:?}");
        };
        assert!(matches!(*lhs, Expr::Or(..)));
        assert!(matches!(*rhs, Expr::Cmp { .. }));
    }

    #[test]
    fn parse_expr_binds_and_tighter_than_or_without_parentheses() {
        let expr = parse_expr("A.b == 1 or C.d == 2 and E.f == 3").unwrap();
        let Expr::Or(lhs, rhs) = expr else {
            panic!("expected Or, got {expr:?}");
        };
        assert!(matches!(*lhs, Expr::Cmp { .. }));
        assert!(matches!(*rhs, Expr::And(..)));
    }

    #[test]
    fn parse_expr_makes_boolean_operators_left_associative() {
        let expr = parse_expr("A.a == 1 and B.b == 2 and C.c == 3").unwrap();
        let Expr::And(lhs, rhs) = expr else {
            panic!("expected And, got {expr:?}");
        };
        assert!(matches!(*lhs, Expr::And(..)));
        assert!(matches!(*rhs, Expr::Cmp { .. }));
    }

    #[test]
    fn parse_expr_accepts_double_negation() {
        let expr = parse_expr("not not A.b == 1").unwrap();
        let Expr::Not(inner) = expr else {
            panic!("expected Not, got {expr:?}");
        };
        assert!(matches!(*inner, Expr::Not(cmp) if matches!(*cmp, Expr::Cmp { .. })));
    }

    #[test]
    fn parse_expr_reads_true_and_false_as_bool_literals() {
        for (text, value) in [("true", true), ("false", false)] {
            let expr = parse_expr(&format!("Invoice.paid == {text}")).unwrap();
            let Expr::Cmp { rhs, .. } = expr else {
                panic!("expected Cmp");
            };
            assert_eq!(rhs, Operand::Lit(Literal::Bool(value)));
        }
    }

    #[test]
    fn parse_expr_supports_every_comparison_operator() {
        let cases = [
            ("==", CmpOp::Eq),
            ("!=", CmpOp::Ne),
            ("<", CmpOp::Lt),
            ("<=", CmpOp::Le),
            (">", CmpOp::Gt),
            (">=", CmpOp::Ge),
        ];
        for (text, expected) in cases {
            let expr = parse_expr(&format!("Invoice.balance {text} 0")).unwrap();
            let Expr::Cmp { op, .. } = expr else {
                panic!("expected Cmp for `{text}`");
            };
            assert_eq!(op, expected, "operator `{text}`");
        }
    }

    #[test]
    fn parse_expr_keeps_every_literal_kind() {
        let cases = [
            (
                "Invoice.label == \"120.00 EUR\"",
                Literal::Str("120.00 EUR".to_string()),
            ),
            (
                "Invoice.total == 120.50",
                Literal::Decimal("120.50".to_string()),
            ),
            ("Invoice.count == -3", Literal::Int(-3)),
            (
                "Invoice.due == 2026-01-31",
                Literal::Date("2026-01-31".to_string()),
            ),
            (
                "Invoice.at == 2026-01-31T10:00:00Z",
                Literal::Datetime("2026-01-31T10:00:00Z".to_string()),
            ),
        ];
        for (src, expected) in cases {
            let expr = parse_expr(src).unwrap();
            let Expr::Cmp { rhs, .. } = expr else {
                panic!("expected Cmp for `{src}`");
            };
            assert_eq!(rhs, Operand::Lit(expected), "source `{src}`");
        }
    }

    #[test]
    fn parse_expr_accepts_a_literal_on_the_left_of_a_comparison() {
        let expr = parse_expr("0 < Invoice.balance").unwrap();
        let Expr::Cmp { lhs, .. } = expr else {
            panic!("expected Cmp");
        };
        assert_eq!(lhs, Operand::Lit(Literal::Int(0)));
    }

    #[test]
    fn parse_expr_accepts_an_in_list_of_one_literal() {
        let expr = parse_expr("Invoice.state in (open)").unwrap();
        let Expr::In { set, .. } = expr else {
            panic!("expected In");
        };
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn parse_expr_rejects_a_bare_operand() {
        // An expression is always an assertion (Annex C.3).
        let diag = parse_expr("Invoice.state").unwrap_err();
        assert_eq!(diag.code, ErrorCode::TelosParseError);
        assert_eq!(
            diag.message,
            "expected a comparison operator or `in`, found end of input"
        );
        assert_eq!(diag.file, None);
        assert_eq!((diag.line, diag.col), (Some(1), Some(14)));
    }

    #[test]
    fn parse_expr_rejects_trailing_garbage() {
        let diag = parse_expr("A.b == 1 x").unwrap_err();
        assert_eq!(diag.message, "expected end of input, found `x`");
    }

    #[test]
    fn parse_expr_requires_an_upper_ident_before_the_dot_of_an_attr_ref() {
        // `attr-ref = upper-ident, ".", lower-ident` (Annex C.3): a lone
        // lower-ident is an enum symbol, so `a.b` is not an attr-ref.
        let diag = parse_expr("a.b == 1").unwrap_err();
        assert_eq!(
            diag.message,
            "expected a comparison operator or `in`, found `.`"
        );
    }

    #[test]
    fn parse_expr_rejects_an_unclosed_parenthesis() {
        let diag = parse_expr("(A.b == 1").unwrap_err();
        assert_eq!(diag.message, "expected `)`, found end of input");
    }

    #[test]
    fn parse_expr_in_list_without_a_comma_is_a_syntax_error() {
        let diag = parse_expr("A.b in (1 2)").unwrap_err();
        assert_eq!(
            diag.message,
            "expected `,` or `)` in literal list, found `2`"
        );
    }

    #[test]
    fn parse_expr_requires_a_field_name_after_the_dot() {
        let diag = parse_expr("Invoice.State == 1").unwrap_err();
        assert_eq!(diag.message, "expected a field name, found `State`");
    }

    #[test]
    fn parse_expr_rejects_an_operand_that_is_neither_a_ref_nor_a_literal() {
        let diag = parse_expr("== 1").unwrap_err();
        assert_eq!(
            diag.message,
            "expected an attribute reference or a literal, found `==`"
        );
    }

    #[test]
    fn parse_expr_reports_lexer_errors_without_a_file() {
        let diag = parse_expr("A.b == @").unwrap_err();
        assert_eq!(diag.code, ErrorCode::TelosParseError);
        assert_eq!(diag.message, "unexpected character `@`");
        assert_eq!(diag.file, None);
    }

    // --- intent files -----------------------------------------------------

    #[test]
    fn parses_the_annex_d_int_0042_header_statement_and_relations() {
        let intent = parse_intent_file(&intent_path(), INT_0042_TEL).unwrap();

        assert_eq!(intent.id, IntentId(42));
        assert_eq!(intent.title, "Invoice payment marks it settled");
        assert_eq!(intent.status, IntentStatus::Active);
        assert_eq!(
            intent.telos,
            "Customers must see immediately that their debt is cleared."
        );
        assert_eq!(
            intent.statement,
            Statement::EventDriven {
                event: notion_at(INT_0042_TEL, "statement event-driven", "PaymentReceived"),
                on: Some(notion_at(INT_0042_TEL, "on Invoice", "Invoice")),
                action: Action::Set {
                    target: AttrRef {
                        notion: notion_at(INT_0042_TEL, "shall set", "Invoice"),
                        attr: field_at(INT_0042_TEL, "shall set", "state"),
                    },
                    value: symbol_at(INT_0042_TEL, "shall set", "settled"),
                },
            }
        );
        assert!(intent.refines.is_empty());
        assert!(intent.excludes.is_empty());
        assert_eq!(intent.requires.len(), 1);
        assert_eq!(intent.requires[0].node, IntentId(17));
        assert_eq!(
            intent.requires[0].span,
            span_after(INT_0042_TEL, "requires", "INT-0017")
        );
    }

    #[test]
    fn parses_the_int_0042_scenario_with_exact_instance_fields() {
        let intent = parse_intent_file(&intent_path(), INT_0042_TEL).unwrap();
        assert_eq!(intent.scenarios.len(), 1);
        let scenario = &intent.scenarios[0];

        assert_eq!(scenario.id, ScenarioId(107));
        assert_eq!(scenario.title, "full payment settles the invoice");
        assert_eq!(scenario.given.len(), 1);
        assert_eq!(
            scenario.given[0].notion,
            notion_at(INT_0042_TEL, "given Invoice", "Invoice")
        );
        assert_eq!(
            scenario.given[0].fields,
            vec![
                (
                    field_at(INT_0042_TEL, "given Invoice", "state"),
                    symbol_at(INT_0042_TEL, "given Invoice", "open"),
                ),
                (
                    field_at(INT_0042_TEL, "given Invoice", "balance"),
                    Literal::Str("120.00 EUR".to_string()),
                ),
            ]
        );
        assert_eq!(
            scenario.when.notion,
            notion_at(INT_0042_TEL, "when  PaymentReceived", "PaymentReceived")
        );
        assert_eq!(
            scenario.when.fields,
            vec![(
                field_at(INT_0042_TEL, "when  PaymentReceived", "amount"),
                Literal::Str("120.00 EUR".to_string()),
            )]
        );
        assert_eq!(
            scenario.then,
            vec![Expr::Cmp {
                op: CmpOp::Eq,
                lhs: Operand::Ref(AttrRef {
                    notion: notion_at(INT_0042_TEL, "then", "Invoice"),
                    attr: field_at(INT_0042_TEL, "then", "state"),
                }),
                rhs: Operand::Lit(symbol_at(INT_0042_TEL, "then", "settled")),
            }]
        );
    }

    #[test]
    fn parses_the_annex_d_int_0017_including_its_empty_instance_body() {
        let intent = parse_intent_file(&intent_path(), INT_0017_TEL).unwrap();

        assert_eq!(intent.id, IntentId(17));
        assert_eq!(intent.title, "Issuing an invoice opens it");
        assert!(intent.requires.is_empty());
        assert_eq!(
            intent.statement,
            Statement::EventDriven {
                event: notion_at(INT_0017_TEL, "statement event-driven", "InvoiceIssued"),
                on: Some(notion_at(INT_0017_TEL, "on Invoice", "Invoice")),
                action: Action::Set {
                    target: AttrRef {
                        notion: notion_at(INT_0017_TEL, "shall set", "Invoice"),
                        attr: field_at(INT_0017_TEL, "shall set", "state"),
                    },
                    value: symbol_at(INT_0017_TEL, "shall set", "open"),
                },
            }
        );

        let scenario = &intent.scenarios[0];
        assert_eq!(scenario.id, ScenarioId(91));
        assert_eq!(scenario.given[0].notion.node, nname("Customer"));
        assert_eq!(
            scenario.given[0].fields[0].1,
            Literal::Str("ACME".to_string())
        );
        assert_eq!(scenario.when.notion.node, nname("InvoiceIssued"));
        assert!(
            scenario.when.fields.is_empty(),
            "`{{}}` is an empty instance body"
        );
    }

    #[test]
    fn parses_a_ubiquitous_statement_with_a_free_clause_action() {
        let src = intent_with_statement("ubiquitous", "    system shall \"log every request\"");
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        assert_eq!(
            intent.statement,
            Statement::Ubiquitous {
                action: Action::Free("log every request".to_string()),
            }
        );
    }

    #[test]
    fn parses_a_state_driven_statement() {
        let src = intent_with_statement(
            "state-driven",
            "    while  Invoice.state == open\n    system shall \"send a reminder\"",
        );
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        assert_eq!(
            intent.statement,
            Statement::StateDriven {
                subject: AttrRef {
                    notion: notion_at(&src, "while", "Invoice"),
                    attr: field_at(&src, "while", "state"),
                },
                value: symbol_at(&src, "while", "open"),
                action: Action::Free("send a reminder".to_string()),
            }
        );
    }

    #[test]
    fn parses_an_unwanted_statement() {
        let src = intent_with_statement(
            "unwanted",
            "    if     Invoice.balance < 0\n    system shall \"block the payment\"",
        );
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        assert_eq!(
            intent.statement,
            Statement::Unwanted {
                condition: Expr::Cmp {
                    op: CmpOp::Lt,
                    lhs: Operand::Ref(AttrRef {
                        notion: notion_at(&src, "if", "Invoice"),
                        attr: field_at(&src, "if", "balance"),
                    }),
                    rhs: Operand::Lit(Literal::Int(0)),
                },
                action: Action::Free("block the payment".to_string()),
            }
        );
    }

    #[test]
    fn parses_an_optional_statement() {
        let src = intent_with_statement(
            "optional",
            "    where  dark-mode\n    system shall \"use the dark palette\"",
        );
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        assert_eq!(
            intent.statement,
            Statement::Optional {
                feature: fname("dark-mode"),
                action: Action::Free("use the dark palette".to_string()),
            }
        );
    }

    #[test]
    fn a_template_body_that_does_not_match_its_template_is_a_syntax_error() {
        let src = intent_with_statement(
            "event-driven",
            "    while  Invoice.state == open\n    system shall \"x\"",
        );
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(
            diags[0].message,
            "template `event-driven` expects `when …`, found `while`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(5), Some(5)));
        assert_eq!(diags[0].file.as_ref(), Some(&intent_path()));
    }

    #[test]
    fn an_unknown_statement_template_suggests_the_closest_one() {
        let src = intent_with_statement("event-drive", "    when   Paid\n    system shall \"x\"");
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(
            diags[0].message,
            "unknown statement template `event-drive`; closest is `event-driven`"
        );
    }

    #[test]
    fn an_event_driven_statement_without_an_on_clause_parses() {
        let src = intent_with_statement(
            "event-driven",
            "    when   PaymentReceived\n    system shall \"archive the invoice\"",
        );
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        let Statement::EventDriven { on, .. } = intent.statement else {
            panic!("expected EventDriven");
        };
        assert_eq!(on, None);
    }

    #[test]
    fn an_intent_without_scenarios_parses() {
        // "an active intent needs at least one scenario" is an integrity
        // rule (Task 8), not a syntax rule.
        let src = intent_with_statement("ubiquitous", "    system shall \"log every request\"");
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        assert!(intent.scenarios.is_empty());
    }

    #[test]
    fn parses_every_intent_status() {
        let cases = [
            ("draft", IntentStatus::Draft),
            ("active", IntentStatus::Active),
            ("deprecated", IntentStatus::Deprecated),
        ];
        for (word, expected) in cases {
            let src = format!(
                "intent INT-0001 \"t\" {{\n  status {word}\n  telos  \"why\"\n  statement ubiquitous {{\n    system shall \"x\"\n  }}\n}}\n"
            );
            let intent = parse_intent_file(&intent_path(), &src).unwrap();
            assert_eq!(intent.status, expected, "status `{word}`");
        }
    }

    #[test]
    fn an_unknown_status_suggests_the_closest_one() {
        let src = "intent INT-0001 \"t\" {\n  status activ\n  telos  \"why\"\n  statement ubiquitous {\n    system shall \"x\"\n  }\n}\n";
        let diags = parse_intent_file(&intent_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "unknown status `activ`; closest is `active`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(10)));
    }

    #[test]
    fn parses_every_relation_line_in_grammar_order() {
        let src = concat!(
            "intent INT-0001 \"t\" {\n",
            "  status draft\n",
            "  telos  \"why\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"x\"\n",
            "  }\n",
            "  refines  INT-0002\n",
            "  refines  INT-0003\n",
            "  requires INT-0004\n",
            "  excludes INT-0005\n",
            "}\n",
        );
        let intent = parse_intent_file(&intent_path(), src).unwrap();
        let ids = |list: &[Sp<IntentId>]| list.iter().map(|id| id.node).collect::<Vec<_>>();
        assert_eq!(ids(&intent.refines), vec![IntentId(2), IntentId(3)]);
        assert_eq!(ids(&intent.requires), vec![IntentId(4)]);
        assert_eq!(ids(&intent.excludes), vec![IntentId(5)]);
    }

    #[test]
    fn a_refines_line_after_a_requires_line_violates_the_grammar_order() {
        let src = concat!(
            "intent INT-0001 \"t\" {\n",
            "  status draft\n",
            "  telos  \"why\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"x\"\n",
            "  }\n",
            "  requires INT-0004\n",
            "  refines  INT-0002\n",
            "}\n",
        );
        let diags = parse_intent_file(&intent_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `requires`, `excludes`, `scenario` or `}`, found `refines`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(8), Some(3)));
    }

    #[test]
    fn a_relation_line_rejects_a_scenario_id() {
        let src = concat!(
            "intent INT-0001 \"t\" {\n",
            "  status draft\n",
            "  telos  \"why\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"x\"\n",
            "  }\n",
            "  requires SCN-0001\n",
            "}\n",
        );
        let diags = parse_intent_file(&intent_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected an intent id, found `SCN-0001`");
        assert_eq!((diags[0].line, diags[0].col), (Some(7), Some(12)));
    }

    #[test]
    fn a_truncated_intent_file_reports_the_unexpected_end_of_input() {
        let src = concat!(
            "intent INT-0001 \"t\" {\n",
            "  status draft\n",
            "  telos  \"why\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"x\"\n",
            "  }\n",
        );
        let diags = parse_intent_file(&intent_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `refines`, `requires`, `excludes`, `scenario` or `}`, found end of input"
        );
    }

    #[test]
    fn an_intent_header_rejects_a_constraint_id() {
        let src = "intent CON-0001 \"t\" {\n";
        let diags = parse_intent_file(&intent_path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected an intent id, found `CON-0001`");
    }

    // --- scenarios --------------------------------------------------------

    #[test]
    fn parses_several_given_and_then_steps() {
        let src = intent_with_scenario(concat!(
            "    given Invoice { state: open }\n",
            "    given Customer { name: \"ACME\" }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled\n",
            "    then  Invoice.balance == 0",
        ));
        let intent = parse_intent_file(&intent_path(), &src).unwrap();
        let scenario = &intent.scenarios[0];
        assert_eq!(scenario.given.len(), 2);
        assert_eq!(scenario.then.len(), 2);
    }

    #[test]
    fn a_given_step_after_the_when_step_violates_the_grammar_order() {
        let src = intent_with_scenario(concat!(
            "    given Invoice { state: open }\n",
            "    when  PaymentReceived {}\n",
            "    given Customer {}\n",
            "    then  Invoice.state == settled",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `then`, found `given`");
        assert_eq!((diags[0].line, diags[0].col), (Some(11), Some(5)));
    }

    #[test]
    fn a_scenario_without_a_given_step_is_a_syntax_error() {
        let src = intent_with_scenario(concat!(
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags[0].message, "expected `given`, found `when`");
    }

    #[test]
    fn a_scenario_without_a_then_step_is_a_syntax_error() {
        let src = intent_with_scenario(concat!(
            "    given Invoice {}\n",
            "    when  PaymentReceived {}",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags[0].message, "expected `then`, found `}`");
    }

    #[test]
    fn a_faulty_then_line_is_recovered_and_the_next_one_still_parses() {
        let src = intent_with_scenario(concat!(
            "    given Invoice { state: open }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state\n",
            "    then  Invoice.balance == 0",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        // Exactly one: recovery resumes on the next `then` line, and the
        // scenario's and the intent's `}` still close their own block.
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected a comparison operator or `in`, found end of line"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(11), Some(24)));
    }

    #[test]
    fn an_error_inside_an_instance_body_does_not_resynchronize_on_its_closing_brace() {
        // Recovery is brace-depth aware: the `}` of the instance body must
        // not be mistaken for the end of the scenario block, which would
        // cascade into spurious diagnostics for every following line.
        let src = intent_with_scenario(concat!(
            "    given Invoice { state: }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected a literal, found `}`");
        assert_eq!((diags[0].line, diags[0].col), (Some(9), Some(28)));
    }

    #[test]
    fn an_instance_body_field_requires_a_colon() {
        let src = intent_with_scenario(concat!(
            "    given Invoice { state open }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `:`, found `open`");
    }

    #[test]
    fn an_instance_body_without_a_comma_reports_the_list_it_belongs_to() {
        let src = intent_with_scenario(concat!(
            "    given Invoice { state: open balance: 0 }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(
            diags[0].message,
            "expected `,` or `}` in instance body, found `balance`"
        );
    }

    #[test]
    fn a_scenario_header_rejects_an_intent_id() {
        let src = intent_with_scenario("    given Invoice {}");
        let src = src.replace("SCN-0107", "INT-0107");
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags[0].message, "expected a scenario id, found `INT-0107`");
    }

    // --- constraint files -------------------------------------------------

    #[test]
    fn parses_the_annex_d_con_0003_into_the_exact_expected_ast() {
        let constraint = parse_constraint_file(&constraint_path(), CON_0003_TEL).unwrap();
        assert_eq!(constraint.id, ConstraintId(3));
        assert_eq!(constraint.kind, ConstraintKind::Architecture);
        assert_eq!(constraint.title, "Hexagonal boundaries");
        assert_eq!(
            constraint.rule,
            Rule::Text("Domain code must not import adapter modules.".to_string())
        );
        assert_eq!(constraint.scope, Scope::Global);
        assert_eq!(
            constraint.check.as_deref(),
            Some("scripts/check-imports.sh --layer domain")
        );
    }

    #[test]
    fn an_unquoted_rule_is_a_machine_checkable_expression() {
        let src = constraint_with("  rule  Invoice.balance >= 0\n  scope global");
        let constraint = parse_constraint_file(&constraint_path(), &src).unwrap();
        assert_eq!(
            constraint.rule,
            Rule::Machine(Expr::Cmp {
                op: CmpOp::Ge,
                lhs: Operand::Ref(AttrRef {
                    notion: notion_at(&src, "rule", "Invoice"),
                    attr: field_at(&src, "rule", "balance"),
                }),
                rhs: Operand::Lit(Literal::Int(0)),
            })
        );
        assert_eq!(constraint.check, None);
    }

    #[test]
    fn a_scope_of_intent_ids_parses_as_a_comma_separated_list() {
        let src = constraint_with("  rule  \"r\"\n  scope INT-0042, INT-0017");
        let constraint = parse_constraint_file(&constraint_path(), &src).unwrap();
        assert_eq!(
            constraint.scope,
            Scope::Intents(vec![
                Sp {
                    node: IntentId(42),
                    span: span_after(&src, "scope", "INT-0042"),
                },
                Sp {
                    node: IntentId(17),
                    span: span_after(&src, "scope", "INT-0017"),
                },
            ])
        );
    }

    #[test]
    fn parses_every_constraint_kind() {
        let cases = [
            ("stack", ConstraintKind::Stack),
            ("architecture", ConstraintKind::Architecture),
            ("quality", ConstraintKind::Quality),
            ("security", ConstraintKind::Security),
            ("convention", ConstraintKind::Convention),
        ];
        for (word, expected) in cases {
            let src =
                format!("constraint CON-0003 {word} \"t\" {{\n  rule  \"r\"\n  scope global\n}}\n");
            let constraint = parse_constraint_file(&constraint_path(), &src).unwrap();
            assert_eq!(constraint.kind, expected, "kind `{word}`");
        }
    }

    #[test]
    fn an_unknown_constraint_kind_suggests_the_closest_one() {
        let src = "constraint CON-0003 quallity \"t\" {\n  rule  \"r\"\n  scope global\n}\n";
        let diags = parse_constraint_file(&constraint_path(), src).unwrap_err();
        assert_eq!(
            diags[0].message,
            "unknown constraint kind `quallity`; closest is `quality`"
        );
    }

    #[test]
    fn a_constraint_without_a_scope_is_a_syntax_error() {
        let src = "constraint CON-0003 quality \"t\" {\n  rule  \"r\"\n}\n";
        let diags = parse_constraint_file(&constraint_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `scope`, found `}`");
    }

    #[test]
    fn a_scope_that_is_neither_global_nor_an_intent_id_is_a_syntax_error() {
        let src = constraint_with("  rule  \"r\"\n  scope everything");
        let diags = parse_constraint_file(&constraint_path(), &src).unwrap_err();
        assert_eq!(
            diags[0].message,
            "expected `global` or an intent id, found `everything`"
        );
    }

    #[test]
    fn a_field_after_the_check_line_is_a_syntax_error() {
        let src = constraint_with("  rule  \"r\"\n  scope global\n  check \"c\"\n  rule  \"r2\"");
        let diags = parse_constraint_file(&constraint_path(), &src).unwrap_err();
        assert_eq!(diags[0].message, "expected `}`, found `rule`");
    }

    #[test]
    fn two_faulty_constraint_fields_yield_two_diagnostics() {
        let src = "constraint CON-0003 quality \"t\" {\n  rule\n  scope 42\n}\n";
        let diags = parse_constraint_file(&constraint_path(), src).unwrap_err();
        assert_eq!(diags.len(), 2, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected an attribute reference or a literal, found end of line"
        );
        assert_eq!(
            diags[1].message,
            "expected `global` or an intent id, found `42`"
        );
    }

    // --- bindings files ---------------------------------------------------

    #[test]
    fn parses_the_annex_d_bindings_into_both_variants() {
        let bindings = parse_bindings_file(&bindings_path(), BINDINGS_TEL).unwrap();
        assert_eq!(
            bindings,
            vec![
                Binding::Implements {
                    path: RepoPath::new("src/billing/invoice.rs"),
                    intent: Sp {
                        node: IntentId(42),
                        span: span_after(BINDINGS_TEL, "implements", "INT-0042"),
                    },
                },
                Binding::Proves {
                    test: TestRef {
                        path: RepoPath::new("tests/billing.rs"),
                        name: Some("scn_0107_full_payment_settles_the_invoice".to_string()),
                    },
                    scenario: Sp {
                        node: ScenarioId(107),
                        span: span_after(BINDINGS_TEL, "proves", "SCN-0107"),
                    },
                },
            ]
        );
    }

    #[test]
    fn an_empty_bindings_file_parses_to_no_bindings() {
        assert_eq!(parse_bindings_file(&bindings_path(), "").unwrap(), vec![]);
    }

    #[test]
    fn a_bindings_file_of_blank_lines_parses_to_no_bindings() {
        assert_eq!(
            parse_bindings_file(&bindings_path(), "\n\n\n").unwrap(),
            vec![]
        );
    }

    #[test]
    fn a_proves_binding_without_a_test_name_keeps_the_bare_path() {
        let src = "proves     \"tests/billing.rs\" -> SCN-0107\n";
        let bindings = parse_bindings_file(&bindings_path(), src).unwrap();
        let Binding::Proves { test, .. } = &bindings[0] else {
            panic!("expected Proves, got {:?}", bindings[0]);
        };
        assert_eq!(test.path, RepoPath::new("tests/billing.rs"));
        assert_eq!(test.name, None);
    }

    #[test]
    fn a_proves_binding_rejects_an_empty_path() {
        let src = "proves     \"::scn_0107\" -> SCN-0107\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(
            diags[0].message,
            "test reference is missing a path: `::scn_0107`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(1), Some(12)));
    }

    #[test]
    fn an_implements_binding_rejects_a_scenario_id() {
        let src = "implements \"src/billing/invoice.rs\" -> SCN-0107\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected an intent id, found `SCN-0107`");
    }

    #[test]
    fn a_proves_binding_rejects_an_intent_id() {
        let src = "proves     \"tests/billing.rs\" -> INT-0042\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected a scenario id, found `INT-0042`");
    }

    #[test]
    fn an_unknown_binding_keyword_is_a_syntax_error() {
        let src = "implements \"a.rs\" -> INT-0001\nproofs \"b.rs\" -> SCN-0001\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `implements` or `proves`, found `proofs`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(1)));
    }

    #[test]
    fn a_stray_brace_in_a_bindings_file_is_reported_without_stalling() {
        // A bindings file has no block, so recovery has no `}` to leave
        // for an enclosing loop: it must consume it and move on.
        let src = "}\nimplements \"a.rs\" -> INT-0001\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `implements` or `proves`, found `}`"
        );
    }

    #[test]
    fn a_faulty_binding_line_is_recovered_and_the_next_one_still_parses() {
        let src = concat!(
            "implements \"a.rs\" INT-0001\n",
            "implements \"b.rs\" -> INT-0002\n",
        );
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `->`, found `INT-0001`");
    }
}
