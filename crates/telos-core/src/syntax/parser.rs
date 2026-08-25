//! Recursive-descent parser for the `.tel` syntax, built on the lexer.
//!
//! Covers notion, intent, constraint, binding, expression, and change files.
//! A change's `op add|edit` forms reuse the entity block rules verbatim,
//! keeping round trips byte-exact, while `run` and `bind` journal lines carry
//! implementation evidence.
//!
//! Diagnostics all share one shape (“ expected X, found `Y` ”), and every
//! file rule recovers from a field-level error by skipping the rest of the
//! offending line, so one pass reports as much as it soundly can. Recovery
//! is brace-depth aware: an error inside a nested block never
//! resynchronizes on that block's `}`.

use std::str::FromStr;

use crate::config::{AgentHost, AgentsCfg, Config, Globs, Policy, TddPolicy, TestCfg};
use crate::error::{Diagnostic, ErrorCode};
use crate::git::Oid;
use crate::ids::{
    CapabilityId, CapabilityRef, ChangeId, ConstraintId, ContextId, EntityRef, FieldName, IntentId,
    NotionName, NotionRef, Owner, RepoPath, ScenarioId,
};
use crate::model::{
    Action, Attr, AttrRef, AttrType, Binding, Capability, Change, ChangeStatus, CmpOp, Constraint,
    ConstraintKind, Context, ContextDependency, ContextKind, ContextMap, Expr, InstanceStep,
    Intent, IntentStatus, JournalEntry, Literal, Notion, NotionKind, NotionMapping, Operand, Rel,
    Rule, Scenario, Scope, StagedOp, Statement, TestRef, TestRun, Witness,
};
use crate::span::{Sp, Span, line_col};
use crate::suggest::closest;

use super::lexer::{TokKind, Token, lex};

/// The five `notion-kind` words, in grammar order.
const NOTION_KINDS: [(&str, NotionKind); 5] = [
    ("actor", NotionKind::Actor),
    ("entity", NotionKind::Entity),
    ("value", NotionKind::Value),
    ("event", NotionKind::Event),
    ("state", NotionKind::State),
];

const CONTEXT_KINDS: [(&str, ContextKind); 3] = [
    ("core", ContextKind::Core),
    ("supporting", ContextKind::Supporting),
    ("generic", ContextKind::Generic),
];

/// The `attr-type` head words, in grammar order.
const ATTR_TYPES: [&str; 9] = [
    "string", "int", "decimal", "money", "bool", "date", "datetime", "enum", "ref",
];

/// The three `status-field` words, in grammar order.
const INTENT_STATUSES: [(&str, IntentStatus); 3] = [
    ("draft", IntentStatus::Draft),
    ("active", IntentStatus::Active),
    ("deprecated", IntentStatus::Deprecated),
];

/// The five EARS `template` words, in grammar order.
const TEMPLATES: [(&str, Template); 5] = [
    ("ubiquitous", Template::Ubiquitous),
    ("event-driven", Template::EventDriven),
    ("state-driven", Template::StateDriven),
    ("unwanted", Template::Unwanted),
    ("optional", Template::Optional),
];

/// The five `constraint-kind` words, in grammar order.
const CONSTRAINT_KINDS: [(&str, ConstraintKind); 5] = [
    ("stack", ConstraintKind::Stack),
    ("architecture", ConstraintKind::Architecture),
    ("quality", ConstraintKind::Quality),
    ("security", ConstraintKind::Security),
    ("convention", ConstraintKind::Convention),
];

/// The five `status-field` words of a change file, in
/// lifecycle order.
const CHANGE_STATUSES: [(&str, ChangeStatus); 5] = [
    ("open", ChangeStatus::Open),
    ("drafted", ChangeStatus::Drafted),
    ("approved", ChangeStatus::Approved),
    ("implementing", ChangeStatus::Implementing),
    ("abandoned", ChangeStatus::Abandoned),
];

/// The two verdicts a `run` line may carry.
const WITNESSES: [(&str, Witness); 2] = [("red", Witness::Red), ("green", Witness::Green)];

/// How an unknown verdict is named back -- the closed set in full, the way
/// [`CHANGE_STATUS_WORDS`] names the statuses.
const WITNESS_WORDS: &str = "`red` or `green`";

/// How an unknown change status is named back: the whole closed set, in the
/// `expected ..., found ...` shape every other diagnostic uses.
const CHANGE_STATUS_WORDS: &str =
    "one of `open`, `drafted`, `approved`, `implementing`, `abandoned`";

/// The tail keywords of an `intent-file` block, in grammar
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

/// Which of the two entity-carrying verbs an `op` announced -- the only
/// thing the shared `entity-decl` rule needs to know to build its
/// [`StagedOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Add,
    Edit,
}

/// Whether `s` is a canonical digest: `sha256:` followed by exactly 64
/// lower-case hex digits.
fn is_canonical_digest(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// How a bracketed comma-separated list ends: the closing token, how to
/// render it, and how to name the list in the separator error message.
struct ListClose {
    kind: TokKind,
    text: &'static str,
    list: &'static str,
}

/// Parses a whole `notion` file.
///
/// Reports every diagnostic found: a syntax error inside a field line is
/// recovered from (skip to the end of the line, then resume at the next
/// field keyword or `}`), so one file can yield several diagnostics.
pub fn parse_notion_file(path: &RepoPath, src: &str) -> Result<Notion, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.notion_file())
}

pub fn parse_context_file(path: &RepoPath, src: &str) -> Result<Context, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.context_file())
}

pub fn parse_capability_file(path: &RepoPath, src: &str) -> Result<Capability, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.capability_file())
}

pub fn parse_context_map_file(path: &RepoPath, src: &str) -> Result<ContextMap, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.context_map_file())
}

pub fn parse_owned_notion_file(
    path: &RepoPath,
    src: &str,
) -> Result<(Owner, Notion), Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.owned_notion_file())
}

/// Parses a whole `intent` file, statement block and scenarios
/// included.
///
/// Recovers like `parse_notion_file`, brace-depth aware: an error inside a
/// nested block (statement, scenario, instance body) resumes at the end of
/// the offending line, never on the nested block's `}`.
pub fn parse_intent_file(path: &RepoPath, src: &str) -> Result<Intent, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.intent_file())
}

pub fn parse_owned_intent_file(
    path: &RepoPath,
    src: &str,
) -> Result<(Owner, Intent), Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.owned_intent_file())
}

/// Parses a whole `constraint` file.
pub fn parse_constraint_file(path: &RepoPath, src: &str) -> Result<Constraint, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.constraint_file())
}

pub fn parse_owned_constraint_file(
    path: &RepoPath,
    src: &str,
) -> Result<(Option<Owner>, Constraint), Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.owned_constraint_file())
}

/// Parses the `bindings.tel` file: zero or more binding lines.
/// An empty file is valid and yields no binding.
pub fn parse_bindings_file(path: &RepoPath, src: &str) -> Result<Vec<Binding>, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.bindings_file())
}

/// Parses a whole `changes/CHG-NNNN.tel` file.
///
/// The nested `entity-decl` of an `op add|edit` is parsed by the very rules
/// [`parse_notion_file`], [`parse_intent_file`] and [`parse_constraint_file`]
/// use, which is what makes the round-trip byte-exact: whatever the
/// emitter writes for an entity, nested or not, this reads back.
///
/// Journal lines may follow the ops, and the model
/// they build takes no part in the digest: a change may be implemented
/// without going stale.
///
/// Three rules live here rather than in the grammar, because they relate two
/// lines rather than describing one: a `digest` belongs to a change that has
/// been approved -- `approved`, or the `implementing` state it moves on to
/// -- and to no other; it is a `sha256:<64 hex>`; and a journal
/// belongs to an `implementing` change and to no other. Everything else
/// about a change -- that its ops are appliable, that no other change claims
/// the same paths, that its witnesses are intact -- is a job for later
/// passes.
pub fn parse_change_file(path: &RepoPath, src: &str) -> Result<Change, Vec<Diagnostic>> {
    parse_file(path, src, |p: &mut P<'_>| p.change_file())
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

/// Parses a single expression from `src`, which must be fully
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

    fn at_offset_kw(&self, offset: usize, keyword: &str) -> bool {
        matches!(
            self.toks.get(self.pos + offset).map(|token| &token.kind),
            Some(TokKind::LowerIdent(word)) if word == keyword
        )
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

    /// Keywords are unreserved -- they are `LowerIdent`s
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

    /// The one syntax-error shape: “ expected X, found `Y` ”.
    fn expected(&self, what: &str) -> Diagnostic {
        self.diag_at(
            self.peek().span,
            format!("expected {what}, found {}", self.found()),
            None,
        )
    }

    /// `expected` over a set of alternatives that depends on how far
    /// through a block we are: “ expected `a`, `b` or `c`, found `X` ”.
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

    fn expect_context_id(&mut self) -> Result<ContextId, Diagnostic> {
        let TokKind::LowerIdent(text) = &self.peek().kind else {
            return Err(self.expected("a context id"));
        };
        let span = self.peek().span;
        let id = ContextId::new(text.clone())
            .map_err(|err| self.diag_at(span, err.message, err.hint))?;
        self.advance();
        Ok(id)
    }

    fn expect_capability_id(&mut self) -> Result<CapabilityId, Diagnostic> {
        let TokKind::LowerIdent(text) = &self.peek().kind else {
            return Err(self.expected("a capability id"));
        };
        let span = self.peek().span;
        let id = CapabilityId::new(text.clone())
            .map_err(|err| self.diag_at(span, err.message, err.hint))?;
        self.advance();
        Ok(id)
    }

    fn capability_ref(&mut self) -> Result<CapabilityRef, Diagnostic> {
        let context = self.expect_context_id()?;
        self.expect(&TokKind::Slash, "`/`")?;
        let capability = self.expect_capability_id()?;
        Ok(CapabilityRef::new(context, capability))
    }

    fn owner_ref(&mut self) -> Result<Owner, Diagnostic> {
        let context = self.expect_context_id()?;
        if self.at(&TokKind::Slash) {
            self.advance();
            let capability = self.expect_capability_id()?;
            Ok(Owner::capability(CapabilityRef::new(context, capability)))
        } else {
            Ok(Owner::context(context))
        }
    }

    fn notion_ref(&mut self) -> Result<NotionRef, Diagnostic> {
        let context = self.expect_context_id()?;
        self.expect(&TokKind::Slash, "`/`")?;
        let notion = self.expect_notion_name()?.node;
        Ok(NotionRef::new(context, notion))
    }

    fn owned_notion_head(&mut self) -> Result<(Owner, NotionName), Diagnostic> {
        let context = self.expect_context_id()?;
        self.expect(&TokKind::Slash, "`/`")?;
        if matches!(self.peek().kind, TokKind::UpperIdent(_)) {
            return Ok((Owner::context(context), self.expect_notion_name()?.node));
        }
        let capability = self.expect_capability_id()?;
        self.expect(&TokKind::Slash, "`/`")?;
        let notion = self.expect_notion_name()?.node;
        Ok((
            Owner::capability(CapabilityRef::new(context, capability)),
            notion,
        ))
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

    /// The `INT-NNNN` form of an `id-lit`.
    fn expect_intent_id(&mut self) -> Result<Sp<IntentId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Intent(id)) = &self.peek().kind else {
            return Err(self.expected("an intent id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// The `SCN-NNNN` form of an `id-lit`.
    fn expect_scenario_id(&mut self) -> Result<Sp<ScenarioId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Scenario(id)) = &self.peek().kind else {
            return Err(self.expected("a scenario id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// The `CON-NNNN` form of an `id-lit`.
    fn expect_constraint_id(&mut self) -> Result<Sp<ConstraintId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Constraint(id)) = &self.peek().kind else {
            return Err(self.expected("a constraint id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    /// A word out of a *small* closed set, reported by listing the whole
    /// set: “ expected `red` or `green`, found `blue` ”, with the closest
    /// known word offered as a hint.
    ///
    /// The two vocabularies written this way -- a change's statuses and a
    /// run's verdicts -- are the ones an agent has no other way to
    /// discover, and each list costs one line. Bigger sets go through
    /// [`P::word_from_set`], which names the *kind* of word expected
    /// instead ("unknown notion kind `x`"): listing nine attribute types in
    /// a message helps nobody.
    ///
    /// `words` is the rendered list, shared with the `found`-less form the
    /// caller needs when the token is not a word at all.
    fn listed_word<T: Copy>(
        &mut self,
        words: &str,
        table: &[(&'static str, T)],
    ) -> Result<T, Diagnostic> {
        let TokKind::LowerIdent(word) = &self.peek().kind else {
            return Err(self.expected(words));
        };
        let span = self.peek().span;
        if let Some(entry) = table.iter().find(|entry| entry.0 == word.as_str()) {
            let value = entry.1;
            self.advance();
            return Ok(value);
        }
        let hint = closest(word, table.iter().map(|entry| entry.0))
            .map(|known| format!("closest is `{known}`"));
        let message = format!("expected {words}, found `{word}`");
        Err(self.diag_at(span, message, hint))
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

    /// `attr-ref = upper-ident , "." , lower-ident`.
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

    // --- strategic domain files ------------------------------

    fn context_file(&mut self) -> Result<Context, Diagnostic> {
        let context = self.context_decl()?;
        self.end_of_file();
        Ok(context)
    }

    fn context_decl(&mut self) -> Result<Context, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("context")?;
        let id = self.expect_context_id()?;
        let kind = self.word_from_set("context kind", &CONTEXT_KINDS)?;
        let title = self.expect_str("a context title")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        self.skip_newlines();
        let def = self.def_field()?;
        self.skip_newlines();
        self.expect(&TokKind::RBrace, "`}`")?;
        Ok(Context {
            id,
            kind,
            title,
            def,
        })
    }

    fn capability_file(&mut self) -> Result<Capability, Diagnostic> {
        let capability = self.capability_decl()?;
        self.end_of_file();
        Ok(capability)
    }

    fn capability_decl(&mut self) -> Result<Capability, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("capability")?;
        let id = self.capability_ref()?;
        let title = self.expect_str("a capability title")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        self.skip_newlines();
        let def = self.def_field()?;
        self.skip_newlines();
        self.expect(&TokKind::RBrace, "`}`")?;
        Ok(Capability { id, title, def })
    }

    fn context_map_file(&mut self) -> Result<ContextMap, Diagnostic> {
        let map = self.context_map_decl()?;
        self.end_of_file();
        Ok(map)
    }

    fn context_map_decl(&mut self) -> Result<ContextMap, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("context-map")?;
        self.expect(&TokKind::LBrace, "`{`")?;
        self.skip_newlines();
        let mut dependencies = Vec::new();
        while !self.at(&TokKind::RBrace) {
            self.expect_lower_kw("dependency")?;
            let consumer = self.expect_context_id()?;
            self.expect_lower_kw("on")?;
            let supplier = self.expect_context_id()?;
            self.expect(&TokKind::LBrace, "`{`")?;
            self.skip_newlines();
            let mut mappings = Vec::new();
            while !self.at(&TokKind::RBrace) {
                self.expect_lower_kw("map")?;
                let from = self.notion_ref()?;
                self.expect(&TokKind::Arrow, "`->`")?;
                let to = self.notion_ref()?;
                self.end_of_field()?;
                self.skip_newlines();
                mappings.push(NotionMapping { from, to });
            }
            self.advance();
            self.end_of_field()?;
            self.skip_newlines();
            dependencies.push(ContextDependency {
                consumer,
                supplier,
                mappings,
            });
        }
        self.advance();
        Ok(ContextMap { dependencies })
    }

    // --- notion files ----------------------------------------

    fn notion_file(&mut self) -> Result<Notion, Diagnostic> {
        let notion = self.notion_decl()?;
        self.end_of_file();
        Ok(notion)
    }

    fn owned_notion_file(&mut self) -> Result<(Owner, Notion), Diagnostic> {
        let owned = self.owned_notion_decl()?;
        self.end_of_file();
        Ok(owned)
    }

    fn owned_notion_decl(&mut self) -> Result<(Owner, Notion), Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("notion")?;
        let (owner, name) = self.owned_notion_head()?;
        let notion = self.notion_body(name)?;
        Ok((owner, notion))
    }

    /// The `notion-file` rule proper, stopping on its own closing `}`.
    ///
    /// Split from [`P::notion_file`] so an `op add|edit` of a change file
    /// can reuse it verbatim for its nested `entity-decl`: a notion
    /// nested in an op differs from one in a file of its own only in what
    /// may follow the block, which is the caller's business.
    fn notion_decl(&mut self) -> Result<Notion, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("notion")?;
        let name = self.expect_notion_name()?.node;
        self.notion_body(name)
    }

    fn notion_body(&mut self, name: NotionName) -> Result<Notion, Diagnostic> {
        let kind = self.notion_kind()?;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;
        self.skip_newlines();

        let def = self.recovered(home, |p| p.def_field()).unwrap_or_default();
        let phrase = self
            .recovered(home, |p| p.phrase_field())
            .unwrap_or_default();

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

        Ok(Notion {
            name,
            kind,
            def,
            phrase,
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

    fn phrase_field(&mut self) -> Result<String, Diagnostic> {
        self.expect_lower_kw("phrase")?;
        let text = self.expect_str("a notion phrase")?;
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

    // --- intent files ----------------------------------------

    fn intent_file(&mut self) -> Result<Intent, Diagnostic> {
        let intent = self.intent_decl()?;
        self.end_of_file();
        Ok(intent)
    }

    fn owned_intent_file(&mut self) -> Result<(Owner, Intent), Diagnostic> {
        let owned = self.owned_intent_decl()?;
        self.end_of_file();
        Ok(owned)
    }

    fn owned_intent_decl(&mut self) -> Result<(Owner, Intent), Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("intent")?;
        let id = self.expect_intent_id()?.node;
        self.expect_lower_kw("in")?;
        let owner = Owner::capability(self.capability_ref()?);
        let title = self.expect_str("an intent title")?.node;
        let intent = self.intent_body(id, title)?;
        Ok((owner, intent))
    }

    /// The `intent-file` rule proper, stopping on its own closing `}` --
    /// nested by an `op add|edit` of a change file, exactly as
    /// [`P::notion_decl`] is.
    fn intent_decl(&mut self) -> Result<Intent, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("intent")?;
        let id = self.expect_intent_id()?.node;
        let title = self.expect_str("an intent title")?.node;
        self.intent_body(id, title)
    }

    fn intent_body(&mut self, id: IntentId, title: String) -> Result<Intent, Diagnostic> {
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

    // --- scenarios -------------------------------------------

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
        // Set as soon as the `given` steps are behind us -- because one was
        // *attempted* (a `when` after a faulty `given` is in its rightful
        // place), or because their absence was already reported once. Either
        // way the steps that follow are judged against `when`/`then`, so a
        // single missing keyword costs a single diagnostic.
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
            if when.is_none() && self.at_kw("when") {
                if !seen_given {
                    // The mandatory `given` steps are missing: report that
                    // once, then take the `when` step anyway so the rest of
                    // the block is still checked against its own rules.
                    let diag = self.expected_one_of(&options);
                    self.diags.push(diag);
                    seen_given = true;
                }
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

    // --- constraint files ------------------------------------

    fn constraint_file(&mut self) -> Result<Constraint, Diagnostic> {
        let constraint = self.constraint_decl()?;
        self.end_of_file();
        Ok(constraint)
    }

    fn owned_constraint_file(&mut self) -> Result<(Option<Owner>, Constraint), Diagnostic> {
        let owned = self.owned_constraint_decl()?;
        self.end_of_file();
        Ok(owned)
    }

    fn owned_constraint_decl(&mut self) -> Result<(Option<Owner>, Constraint), Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("constraint")?;
        let id = self.expect_constraint_id()?.node;
        self.expect_lower_kw("in")?;
        let owner = if self.at_kw("project") {
            self.advance();
            None
        } else if self.at_kw("context") {
            self.advance();
            Some(Owner::context(self.expect_context_id()?))
        } else if self.at_kw("capability") {
            self.advance();
            Some(Owner::capability(self.capability_ref()?))
        } else {
            return Err(self.expected("`project`, `context` or `capability`"));
        };
        let kind = self.word_from_set("constraint kind", &CONSTRAINT_KINDS)?;
        let title = self.expect_str("a constraint title")?.node;
        let constraint = self.constraint_body(id, kind, title, false)?;
        Ok((owner, constraint))
    }

    /// The `constraint-file` rule proper, stopping on its own closing `}`
    /// -- nested by an `op add|edit` of a change file.
    fn constraint_decl(&mut self) -> Result<Constraint, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("constraint")?;
        let id = self.expect_constraint_id()?.node;
        let kind = self.word_from_set("constraint kind", &CONSTRAINT_KINDS)?;
        let title = self.expect_str("a constraint title")?.node;
        self.constraint_body(id, kind, title, true)
    }

    fn constraint_body(
        &mut self,
        id: ConstraintId,
        kind: ConstraintKind,
        title: String,
        has_scope_field: bool,
    ) -> Result<Constraint, Diagnostic> {
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;

        self.skip_newlines();
        let rule = self
            .recovered(home, |p| p.rule_field())
            .unwrap_or_else(|| Rule::Text(String::new()));
        self.skip_newlines();
        let scope = if has_scope_field {
            let scope = self
                .recovered(home, |p| p.scope_field())
                .unwrap_or(Scope::Global);
            self.skip_newlines();
            scope
        } else {
            Scope::Global
        };

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

    // --- bindings file ---------------------------------------

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
                path: RepoPath::parse_outside_telos(path.node)
                    .map_err(|err| self.diag_at(path.span, err.message, err.hint))?,
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

    // --- change files ------------------------------------------

    /// `change-file = "change" , change-id , string-lit , "{" ,
    ///  status-field , [ digest-field ] , { op-decl } , { journal-line } ,
    ///  "}"`.
    ///
    /// The header string is the motivation. The two field lines are
    /// recovered from like any others, and every op and journal line is
    /// recovered from independently, so a file with three broken ops reports
    /// three diagnostics.
    fn change_file(&mut self) -> Result<Change, Diagnostic> {
        self.skip_newlines();
        self.expect_lower_kw("change")?;
        let id = self.expect_change_id()?.node;
        let motivation = self.expect_str("a motivation string")?.node;
        self.expect(&TokKind::LBrace, "`{`")?;
        let home = self.depth;

        self.skip_newlines();
        let status_span = self.peek().span;
        let status = self.recovered(home, |p| p.change_status_field());
        self.skip_newlines();

        // `digest_span` records that the *keyword* was written, whether or
        // not its argument parsed: the coherence checks below are about the
        // line being there, not about it being well-formed.
        let mut digest = None;
        let mut digest_span = None;
        if self.at_kw("digest") {
            digest_span = Some(self.peek().span);
            digest = self.recovered(home, |p| p.digest_field());
            self.skip_newlines();
        }

        // The body is two phases: ops, then journal lines. Once a
        // journal line has been seen, `op` is no longer one of the options
        // -- the canonical order is what makes a change file readable as
        // "what was decided" followed by "what was done".
        //
        // `journal_span` records that the first journal *keyword* was
        // written, whether or not its line parsed: like `digest_span`, the
        // coherence check below is about the line being there.
        let mut ops = Vec::new();
        let mut journal = Vec::new();
        let mut journal_span: Option<Span> = None;
        loop {
            self.skip_newlines();
            if self.at(&TokKind::RBrace) {
                self.advance();
                break;
            }
            let what = if journal_span.is_none() {
                "`op`, `run`, `bind` or `}`"
            } else {
                "`run`, `bind` or `}`"
            };
            if self.at_eof() {
                return Err(self.expected(what));
            }
            if self.at_kw("op") && journal_span.is_none() {
                if let Some(op) = self.recovered(home, |p| p.op_decl()) {
                    ops.push(op);
                }
                continue;
            }
            if self.at_kw("run") || self.at_kw("bind") {
                journal_span.get_or_insert(self.peek().span);
                if let Some(entry) = self.recovered(home, |p| p.journal_line()) {
                    journal.push(entry);
                }
                continue;
            }
            let diag = self.expected(what);
            self.diags.push(diag);
            self.recover_to_newline(home);
        }

        self.end_of_file();

        // Status and digest are one statement written on two lines: the
        // digest *is* the approval, so it is present exactly on the two
        // statuses that carry one: `approved`, and the `implementing` state a
        // change moves on to. Reconcile accepts either status, and an
        // implementing change could not reconcile if that transition dropped
        // its digest.
        //
        // Checked only when the status line itself parsed -- otherwise the
        // complaint would be about a status nobody wrote.
        if let Some(status) = status {
            let carries_digest =
                matches!(status, ChangeStatus::Approved | ChangeStatus::Implementing);
            let diag = match (carries_digest, digest_span) {
                (true, None) => Some(self.diag_at(
                    status_span,
                    "an approved change must carry its digest".to_string(),
                    None,
                )),
                (false, Some(span)) => Some(self.diag_at(
                    span,
                    "digest is only valid on an approved or implementing change".to_string(),
                    None,
                )),
                _ => None,
            };
            if let Some(diag) = diag {
                self.diags.push(diag);
            }

            // A journal records an implementation in flight, and only one
            // status describes that: the first line written by `telos test`
            // or `telos bind` moves an approved
            // change to `implementing`, so a journal anywhere else names a
            // transition that never happened.
            if let Some(span) = journal_span
                && status != ChangeStatus::Implementing
            {
                let diag = self.diag_at(
                    span,
                    "a journal is only valid on an implementing change".to_string(),
                    None,
                );
                self.diags.push(diag);
            }
        }

        Ok(Change {
            id,
            motivation,
            status: status.unwrap_or(ChangeStatus::Open),
            approved_digest: digest,
            ops,
            journal,
        })
    }

    /// The `CHG-NNNN` form of an `id-lit`.
    fn expect_change_id(&mut self) -> Result<Sp<ChangeId>, Diagnostic> {
        let TokKind::IdLit(EntityRef::Change(id)) = &self.peek().kind else {
            return Err(self.expected("a change id"));
        };
        let node = *id;
        let span = self.peek().span;
        self.advance();
        Ok(Sp { node, span })
    }

    fn change_status_field(&mut self) -> Result<ChangeStatus, Diagnostic> {
        self.expect_lower_kw("status")?;
        let status = self.change_status()?;
        self.end_of_field()?;
        Ok(status)
    }

    /// The five statuses of the change lifecycle.
    fn change_status(&mut self) -> Result<ChangeStatus, Diagnostic> {
        self.listed_word(CHANGE_STATUS_WORDS, &CHANGE_STATUSES)
    }

    /// `digest-field = "digest" , string-lit`, the string being the
    /// canonical `"sha256:<64 hex>"` form. It is checked here while the span
    /// of an offending string is still available.
    fn digest_field(&mut self) -> Result<String, Diagnostic> {
        self.expect_lower_kw("digest")?;
        let text = self.expect_str("a digest string")?;
        if !is_canonical_digest(&text.node) {
            return Err(self.diag_at(
                text.span,
                "malformed digest; expected sha256:<64 hex>".to_string(),
                None,
            ));
        }
        self.end_of_field()?;
        Ok(text.node)
    }

    /// `op-decl`: one staged operation.
    fn op_decl(&mut self) -> Result<StagedOp, Diagnostic> {
        self.expect_lower_kw("op")?;
        if self.at_kw("add") {
            self.advance();
            return self.entity_decl(Verb::Add);
        }
        if self.at_kw("edit") {
            self.advance();
            if self.at_kw("config") {
                return self.config_op();
            }
            return self.entity_decl(Verb::Edit);
        }
        if self.at_kw("remove") {
            self.advance();
            return self.remove_op();
        }
        if self.at_kw("accept") {
            self.advance();
            return self.accept_op();
        }
        if self.at_kw("move") {
            self.advance();
            return self.move_op();
        }
        Err(self.expected("`add`, `edit`, `remove`, `move` or `accept`"))
    }

    fn config_op(&mut self) -> Result<StagedOp, Diagnostic> {
        self.expect_lower_kw("config")?;
        self.expect(&TokKind::LBrace, "`{`")?;
        let mut config = Config {
            code: Globs::default(),
            tests: Globs::default(),
            test: TestCfg::default(),
            policy: Policy::default(),
            agents: AgentsCfg::default(),
        };
        self.skip_newlines();
        while !self.at(&TokKind::RBrace) && !self.at(&TokKind::Eof) {
            if self.at_kw("code_glob") {
                self.advance();
                config.code.globs.push(self.expect_str("a code glob")?.node);
            } else if self.at_kw("test_glob") {
                self.advance();
                config
                    .tests
                    .globs
                    .push(self.expect_str("a test glob")?.node);
            } else if self.at_kw("test_cmd") {
                self.advance();
                config.test.cmd = self.expect_str("a test command")?.node;
            } else if self.at_kw("tdd") {
                self.advance();
                config.policy.tdd = if self.at_kw("strict") {
                    self.advance();
                    TddPolicy::Strict
                } else if self.at_kw("advisory") {
                    self.advance();
                    TddPolicy::Advisory
                } else {
                    return Err(self.expected("`strict` or `advisory`"));
                };
            } else if self.at_kw("agent_host") {
                self.advance();
                config.agents.hosts.push(if self.at_kw("claude") {
                    self.advance();
                    AgentHost::Claude
                } else if self.at_kw("codex") {
                    self.advance();
                    AgentHost::Codex
                } else {
                    return Err(self.expected("`claude` or `codex`"));
                });
            } else {
                return Err(self.expected("a config field"));
            }
            self.end_of_field()?;
            self.skip_newlines();
        }
        self.expect(&TokKind::RBrace, "`}`")?;
        self.end_of_field()?;
        config.normalize();
        Ok(StagedOp::EditConfig(config))
    }

    /// Nested `entity-decl = notion-file | intent-file | constraint-file`.
    /// The entity keyword picks the rule that parses the block
    /// exactly as it would at the top of a file of its own.
    fn entity_decl(&mut self, verb: Verb) -> Result<StagedOp, Diagnostic> {
        let op = if self.at_kw("notion") {
            if matches!(
                self.toks.get(self.pos + 1).map(|token| &token.kind),
                Some(TokKind::LowerIdent(_))
            ) {
                let (owner, notion) = self.owned_notion_decl()?;
                match verb {
                    Verb::Add => StagedOp::AddOwnedNotion { owner, notion },
                    Verb::Edit => StagedOp::EditOwnedNotion { owner, notion },
                }
            } else {
                return Err(self.expected("an owner-qualified notion declaration"));
            }
        } else if self.at_kw("intent") {
            if self.at_offset_kw(2, "in") {
                let (owner, intent) = self.owned_intent_decl()?;
                match verb {
                    Verb::Add => StagedOp::AddOwnedIntent { owner, intent },
                    Verb::Edit => StagedOp::EditOwnedIntent { owner, intent },
                }
            } else {
                return Err(self.expected("an owner-qualified intent declaration"));
            }
        } else if self.at_kw("constraint") {
            if self.at_offset_kw(2, "in") {
                let (owner, constraint) = self.owned_constraint_decl()?;
                match verb {
                    Verb::Add => StagedOp::AddOwnedConstraint { owner, constraint },
                    Verb::Edit => StagedOp::EditOwnedConstraint { owner, constraint },
                }
            } else {
                return Err(self.expected("an owner-qualified constraint declaration"));
            }
        } else if self.at_kw("context") {
            let context = self.context_decl()?;
            match verb {
                Verb::Add => StagedOp::AddContext(context),
                Verb::Edit => StagedOp::EditContext(context),
            }
        } else if self.at_kw("capability") {
            let capability = self.capability_decl()?;
            match verb {
                Verb::Add => StagedOp::AddCapability(capability),
                Verb::Edit => StagedOp::EditCapability(capability),
            }
        } else if self.at_kw("context-map") && matches!(verb, Verb::Edit) {
            StagedOp::EditContextMap(self.context_map_decl()?)
        } else {
            return Err(self.expected(
                "`context`, `capability`, `notion`, `intent`, `constraint` or `context-map`",
            ));
        };
        self.end_of_field()?;
        Ok(op)
    }

    fn owner_or_project(&mut self) -> Result<Option<Owner>, Diagnostic> {
        if self.at_kw("project") {
            self.advance();
            Ok(None)
        } else {
            self.owner_ref().map(Some)
        }
    }

    fn move_op(&mut self) -> Result<StagedOp, Diagnostic> {
        self.expect_lower_kw("from")?;
        let from = self.owner_or_project()?;
        self.expect_lower_kw("to")?;
        let to = self.owner_or_project()?;
        let op = if self.at_kw("notion") {
            let (declared, notion) = self.owned_notion_decl()?;
            let (Some(from), Some(to)) = (from, to) else {
                return Err(self.expected("a context or capability owner for a notion move"));
            };
            if declared != to {
                return Err(self.diag_at(
                    self.peek().span,
                    format!("moved notion declares owner `{declared}`, expected `{to}`"),
                    None,
                ));
            }
            StagedOp::MoveNotion { from, to, notion }
        } else if self.at_kw("intent") {
            let (declared, intent) = self.owned_intent_decl()?;
            let (Some(from), Some(to)) = (from, to) else {
                return Err(self.expected("a capability owner for an intent move"));
            };
            if declared != to {
                return Err(self.diag_at(
                    self.peek().span,
                    format!("moved intent declares owner `{declared}`, expected `{to}`"),
                    None,
                ));
            }
            StagedOp::MoveIntent { from, to, intent }
        } else if self.at_kw("constraint") {
            let (declared, constraint) = self.owned_constraint_decl()?;
            if declared != to {
                return Err(self.diag_at(
                    self.peek().span,
                    "moved constraint declaration does not match its destination".to_string(),
                    None,
                ));
            }
            StagedOp::MoveConstraint {
                from,
                to,
                constraint,
            }
        } else {
            return Err(self.expected("`notion`, `intent` or `constraint`"));
        };
        self.end_of_field()?;
        Ok(op)
    }

    /// Parses a canonical removal. Ownership is mandatory for tactical
    /// entities so the claim always resolves to one exact file.
    fn remove_op(&mut self) -> Result<StagedOp, Diagnostic> {
        let op = if self.at_kw("context") {
            self.advance();
            StagedOp::RemoveContext(self.expect_context_id()?)
        } else if self.at_kw("capability") {
            self.advance();
            StagedOp::RemoveCapability(self.capability_ref()?)
        } else if self.at_kw("notion") {
            self.advance();
            let (owner, name) = self.owned_notion_head()?;
            StagedOp::RemoveOwnedNotion { owner, name }
        } else if self.at_kw("intent") {
            self.advance();
            let id = self.expect_intent_id()?.node;
            self.expect_lower_kw("from")?;
            let owner = self.owner_ref()?;
            if owner.capability.is_none() {
                return Err(self.expected("a capability owner"));
            }
            StagedOp::RemoveOwnedIntent { owner, id }
        } else if self.at_kw("constraint") {
            self.advance();
            let id = self.expect_constraint_id()?.node;
            self.expect_lower_kw("from")?;
            let owner = self.owner_or_project()?;
            StagedOp::RemoveOwnedConstraint { owner, id }
        } else {
            return Err(
                self.expected("`context`, `capability`, `notion`, `intent` or `constraint`")
            );
        };
        self.end_of_field()?;
        Ok(op)
    }

    /// `journal-line = run-line | bind-line`.
    ///
    /// Only reached on `run` or `bind`, which the body loop matched to
    /// decide the phase; the final `Err` is unreachable in practice and
    /// kept so the rule reads on its own.
    ///
    /// Both line kinds name a validated path, and neither may name one under
    /// `telos/` -- the rule that
    /// makes [`crate::model::Change::claims`]'s guarantee a property of the
    /// grammar rather than of the commands that usually write these lines.
    fn journal_line(&mut self) -> Result<JournalEntry, Diagnostic> {
        if self.at_kw("run") {
            self.advance();
            let scenario = self.expect_scenario_id()?.node;
            let witness = self.witness()?;
            let text = self.expect_str("a test reference")?;
            let test = TestRef::from_str(&text.node)
                .map_err(|err| self.diag_at(text.span, err.message, err.hint))?;
            // The oid is opaque: 40 hex in a sha1 repository, 64 in a
            // sha256 one, and never anything this parser should adjudicate.
            let oid = self.expect_str("the blob oid string after the test reference")?;
            self.end_of_field()?;
            return Ok(JournalEntry::Run(TestRun {
                scenario,
                witness,
                test,
                oid: Oid(oid.node),
            }));
        }
        if self.at_kw("bind") {
            self.advance();
            let text = self.expect_str("a code path")?;
            let path = RepoPath::parse_outside_telos(text.node)
                .map_err(|err| self.diag_at(text.span, err.message, err.hint))?;
            self.expect(&TokKind::Arrow, "`->`")?;
            let intent = self.expect_intent_id()?.node;
            self.end_of_field()?;
            return Ok(JournalEntry::Bind { path, intent });
        }
        Err(self.expected("`run` or `bind`"))
    }

    /// Refuses a journal path inside the spec tree.
    ///
    /// A journal line names *code*: the test file a run was taken on, the
    /// source file a bind attaches to an intent. Both become claims of the
    /// change, and a claim is a licence to drift the file until this
    /// change reconciles -- which is exactly what `telos/**` must never
    /// hand out. `telos/bindings.tel` is the sharp case: it is sealed, it
    /// is rewritten by *every* reconcile from the folded journal, and
    /// a change claiming it would both lock it against the others and let
    /// unreviewed drift through reconcile's own drift gate. The rest of the
    /// spec tree is no better: an entity file is written by an `op`, whose
    /// content the approval digest covers -- journalling one would be a way
    /// to touch the spec without approving it.
    ///
    /// Enforced here rather than in the commands that normally write these
    /// lines, because a change file is a plain text file an agent may edit
    /// by hand: the invariant has to be a property of what parses.
    /// The two verdicts a run may carry.
    fn witness(&mut self) -> Result<Witness, Diagnostic> {
        self.listed_word(WITNESS_WORDS, &WITNESSES)
    }

    /// `"accept" , string-lit , string-lit`: the path, then the blob oid it
    /// is being sealed at.
    fn accept_op(&mut self) -> Result<StagedOp, Diagnostic> {
        let path = self.expect_str("a repository path")?;
        let oid = self.expect_str("the blob oid string after the path")?;
        self.end_of_field()?;
        Ok(StagedOp::Accept {
            path: RepoPath::parse(path.node)
                .map_err(|err| self.diag_at(path.span, err.message, err.hint))?,
            oid: Oid(oid.node),
        })
    }

    // --- expressions -----------------------------------------

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
    /// parenthesized, so there is no ambiguity here.
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

    #[test]
    fn parses_strategic_domain_declarations_and_owned_entities() {
        let context = parse_context_file(
            &RepoPath::new("telos/contexts/pet/context.tel"),
            "context pet core \"Pet\" {\n  def \"Virtual pet rules.\"\n}\n",
        )
        .unwrap();
        assert_eq!(context.id, ContextId::new("pet").unwrap());
        assert_eq!(context.kind, ContextKind::Core);

        let capability = parse_capability_file(
            &RepoPath::new("telos/contexts/pet/capabilities/care/capability.tel"),
            "capability pet/care \"Care\" {\n  def \"Care for a pet.\"\n}\n",
        )
        .unwrap();
        assert_eq!(capability.id.to_string(), "pet/care");

        let (owner, notion) = parse_owned_notion_file(
            &RepoPath::new("telos/contexts/pet/notions/Pet.tel"),
            "notion pet/Pet entity {\n  def \"A virtual pet.\"\n  phrase \"pet\"\n}\n",
        )
        .unwrap();
        assert_eq!(owner, Owner::context(ContextId::new("pet").unwrap()));
        assert_eq!(notion.name, NotionName::new("Pet").unwrap());

        let (owner, intent) = parse_owned_intent_file(
            &RepoPath::new("telos/contexts/pet/capabilities/care/intents/INT-0002.tel"),
            concat!(
                "intent INT-0002 in pet/care \"Feed a pet\" {\n",
                "  status draft\n",
                "  telos  \"A cared-for pet stays healthy.\"\n",
                "  statement ubiquitous {\n",
                "    system shall \"record a feeding\"\n",
                "  }\n",
                "}\n",
            ),
        )
        .unwrap();
        assert_eq!(owner.to_string(), "pet/care");
        assert_eq!(intent.id, IntentId(2));
    }

    #[test]
    fn parses_directed_context_dependencies_with_explicit_mappings() {
        let map = parse_context_map_file(
            &RepoPath::new("telos/context-map.tel"),
            concat!(
                "context-map {\n",
                "  dependency terminal on pet {\n",
                "    map pet/Pet -> terminal/PetView\n",
                "  }\n",
                "}\n",
            ),
        )
        .unwrap();

        assert_eq!(map.dependencies.len(), 1);
        let dependency = &map.dependencies[0];
        assert_eq!(dependency.consumer, ContextId::new("terminal").unwrap());
        assert_eq!(dependency.supplier, ContextId::new("pet").unwrap());
        assert_eq!(dependency.mappings[0].from.to_string(), "pet/Pet");
        assert_eq!(dependency.mappings[0].to.to_string(), "terminal/PetView");
    }

    #[test]
    fn parses_constraint_scope_from_the_declaration_header() {
        let (owner, constraint) = parse_owned_constraint_file(
            &RepoPath::new("telos/contexts/pet/constraints/CON-0001.tel"),
            concat!(
                "constraint CON-0001 in context pet quality \"Vitals stay bounded\" {\n",
                "  rule  \"Vitals remain between zero and one hundred.\"\n",
                "  check \"python scripts/check_vitals.py\"\n",
                "}\n",
            ),
        )
        .unwrap();

        assert_eq!(owner, Some(Owner::context(ContextId::new("pet").unwrap())));
        assert_eq!(constraint.id, ConstraintId(1));
        assert_eq!(constraint.scope, Scope::Global);
    }

    #[test]
    fn change_files_parse_owned_ops_and_explicit_moves() {
        let src = concat!(
            "change CHG-0001 \"Move invoice ownership\" {\n",
            "  status drafted\n",
            "\n",
            "  op add notion billing/Invoice value {\n",
            "    def \"Invoice identity.\"\n",
            "    phrase \"invoice\"\n",
            "  }\n",
            "\n",
            "  op move from billing/invoicing to billing/settlement intent INT-0017 in billing/settlement \"Issue invoice\" {\n",
            "    status draft\n",
            "    telos  \"Invoices can be issued.\"\n",
            "    statement ubiquitous {\n",
            "      system shall \"issue an invoice\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        );

        let change = parse_change_file(&RepoPath::new("telos/changes/CHG-0001.tel"), src).unwrap();
        assert!(matches!(change.ops[0], StagedOp::AddOwnedNotion { .. }));
        match &change.ops[1] {
            StagedOp::MoveIntent { from, to, intent } => {
                assert_eq!(from.to_string(), "billing/invoicing");
                assert_eq!(to.to_string(), "billing/settlement");
                assert_eq!(intent.id, IntentId(17));
            }
            other => panic!("expected owned intent move, got {other:?}"),
        }
    }

    /// Canonical `telos/notions/Invoice.tel`, byte for byte (the corpus files
    /// themselves are created by mutation commands).
    const INVOICE_TEL: &str = concat!(
        "notion Invoice entity {\n",
        "  def    \"A bill issued to a Customer for delivered work.\"\n",
        "  phrase \"invoice\"\n",
        "  attr   state   enum(open, settled, cancelled)\n",
        "  attr   balance money\n",
        "  rel    issued-to -> Customer\n",
        "}\n",
    );

    /// Canonical `telos/intents/INT-0042.tel`, byte for byte.
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

    /// Canonical `telos/intents/INT-0017.tel`, byte for byte.
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

    /// Canonical `telos/constraints/CON-0003.tel`, byte for byte.
    const CON_0003_TEL: &str = concat!(
        "constraint CON-0003 architecture \"Hexagonal boundaries\" {\n",
        "  rule  \"Domain code must not import adapter modules.\"\n",
        "  scope global\n",
        "  check \"scripts/check-imports.sh --layer domain\"\n",
        "}\n",
    );

    /// Canonical `telos/bindings.tel`, byte for byte.
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
    fn parses_the_payload_schema_invoice_into_the_exact_expected_ast() {
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
            "  def    \"d\"\n",
            "  phrase \"thing\"\n",
            "  attr   a string\n",
            "  attr   b int\n",
            "  attr   c decimal\n",
            "  attr   d money\n",
            "  attr   e bool\n",
            "  attr   f date\n",
            "  attr   g datetime\n",
            "  attr   h ref(Customer)\n",
            "  attr   i enum(one)\n",
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
            let src = format!("notion Thing {word} {{\n  def    \"d\"\n  phrase \"thing\"\n}}\n");
            let notion = parse_notion_file(&path(), &src).unwrap();
            assert_eq!(notion.kind, expected, "kind `{word}`");
        }
    }

    #[test]
    fn a_notion_without_attrs_or_rels_parses() {
        let notion = parse_notion_file(
            &path(),
            "notion Thing value {\n  def    \"d\"\n  phrase \"thing\"\n}\n",
        )
        .unwrap();
        assert!(notion.attrs.is_empty());
        assert!(notion.rels.is_empty());
    }

    #[test]
    fn unknown_notion_kind_suggests_the_closest_one() {
        let src = "notion Invoice entty {\n  def    \"d\"\n  phrase \"invoice\"\n}\n";
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
        let src = "notion Invoice zzzzzz {\n  def    \"d\"\n  phrase \"invoice\"\n}\n";
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
            "  def    \"A bill.\"\n",
            "  phrase \"invoice\"\n",
            "  attr   state enum(open settled)\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(
            diags[0].message,
            "expected `,` or `)` in enum symbol list, found `settled`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(4), Some(26)));
        assert_eq!(diags[0].file.as_ref(), Some(&path()));
    }

    #[test]
    fn an_empty_enum_symbol_list_is_a_syntax_error() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n  attr   state enum()\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected an enum symbol, found `)`");
    }

    #[test]
    fn two_faulty_fields_yield_two_diagnostics_and_parsing_continues() {
        let src = concat!(
            "notion Invoice entity {\n",
            "  def    \"A bill.\"\n",
            "  phrase \"invoice\"\n",
            "  attr   state enum(open settled)\n",
            "  attr   balance\n",
            "  rel    issued-to -> Customer\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 2, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `,` or `)` in enum symbol list, found `settled`"
        );
        assert_eq!((diags[0].line, diags[0].col), (Some(4), Some(26)));
        assert_eq!(
            diags[1].message,
            "expected an attribute type, found end of line"
        );
        assert_eq!((diags[1].line, diags[1].col), (Some(5), Some(17)));
    }

    #[test]
    fn an_attr_after_a_rel_violates_the_grammar_order() {
        let src = concat!(
            "notion Invoice entity {\n",
            "  def    \"d\"\n",
            "  phrase \"invoice\"\n",
            "  rel    issued-to -> Customer\n",
            "  attr   state money\n",
            "}\n",
        );
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected `rel` or `}`, found `attr`");
        assert_eq!((diags[0].line, diags[0].col), (Some(5), Some(3)));
    }

    #[test]
    fn a_missing_def_field_is_a_syntax_error() {
        let src = "notion Invoice entity {\n  phrase \"invoice\"\n  attr   state money\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        // Recovering from the missing `def` consumes the rest of that line --
        // which is the `phrase` line -- so `phrase` is reported missing too.
        assert_eq!(diags.len(), 2, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `def`, found `phrase`");
        assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(3)));
        assert_eq!(diags[1].message, "expected `phrase`, found `attr`");
    }

    #[test]
    fn a_missing_phrase_field_is_a_syntax_error() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  attr   state money\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected `phrase`, found `attr`");
        assert_eq!((diags[0].line, diags[0].col), (Some(3), Some(3)));
    }

    #[test]
    fn an_unknown_attr_type_suggests_the_closest_one() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n  attr   balance mony\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected an attribute type, found `mony`");
        assert_eq!(diags[0].hint.as_deref(), Some("closest is `money`"));
    }

    #[test]
    fn a_ref_attr_type_requires_a_notion_name() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n  attr   owner ref(customer)\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected a notion name, found `customer`");
    }

    #[test]
    fn a_rel_field_requires_an_arrow() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n  rel    issued-to Customer\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags[0].message, "expected `->`, found `Customer`");
    }

    #[test]
    fn blank_lines_between_fields_are_tolerated_by_the_parser() {
        // The emitter's canonical form forbids them, but the parser accepts
        // them; byte-level normalization belongs to round-trip tests.
        let src = concat!(
            "\n",
            "notion Invoice entity {\n",
            "\n",
            "  def    \"A bill.\"\n",
            "  phrase \"invoice\"\n",
            "\n",
            "  attr   balance money\n",
            "\n",
            "  rel    issued-to -> Customer\n",
            "\n",
            "}\n",
        );
        let notion = parse_notion_file(&path(), src).unwrap();
        assert_eq!(notion.attrs.len(), 1);
        assert_eq!(notion.rels.len(), 1);
    }

    #[test]
    fn a_file_without_a_trailing_newline_parses() {
        let notion = parse_notion_file(
            &path(),
            "notion Thing value {\n  def    \"d\"\n  phrase \"thing\"\n}",
        )
        .unwrap();
        assert_eq!(notion.name, nname("Thing"));
    }

    #[test]
    fn content_after_the_closing_brace_is_a_syntax_error() {
        let src = format!(
            "{INVOICE_TEL}notion Other entity {{\n  def    \"d\"\n  phrase \"other\"\n}}\n"
        );
        let diags = parse_notion_file(&path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected end of input, found `notion`");
    }

    #[test]
    fn a_truncated_file_reports_the_unexpected_end_of_input() {
        let src = "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].message,
            "expected `attr`, `rel` or `}`, found end of input"
        );
    }

    #[test]
    fn a_lexer_error_is_reported_against_the_file() {
        let src =
            "notion Invoice entity {\n  def    \"d\"\n  phrase \"invoice\"\n  attr   state @\n}\n";
        let diags = parse_notion_file(&path(), src).unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, ErrorCode::TelosParseError);
        assert_eq!(diags[0].message, "unexpected character `@`");
        assert_eq!(diags[0].file.as_ref(), Some(&path()));
        assert_eq!((diags[0].line, diags[0].col), (Some(4), Some(16)));
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
        // the textual syntax: `or` < `and` < `not` < comparison.
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
        // An expression is always an assertion.
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
        // `attr-ref = upper-ident, ".", lower-ident`: a lone
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
    fn parses_the_payload_schema_int_0042_header_statement_and_relations() {
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
    fn parses_the_payload_schema_int_0017_including_its_empty_instance_body() {
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        // rule, not a syntax rule.
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        // Exactly one: the `when` step is consumed despite the missing
        // `given`, so the `then` line and both `}` are not re-reported.
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected `given`, found `when`");
        assert_eq!((diags[0].line, diags[0].col), (Some(9), Some(5)));
    }

    #[test]
    fn a_scenario_without_a_then_step_is_a_syntax_error() {
        let src = intent_with_scenario(concat!(
            "    given Invoice {}\n",
            "    when  PaymentReceived {}",
        ));
        let diags = parse_intent_file(&intent_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected a scenario id, found `INT-0107`");
    }

    // --- constraint files -------------------------------------------------

    #[test]
    fn parses_the_payload_schema_con_0003_into_the_exact_expected_ast() {
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(
            diags[0].message,
            "expected `global` or an intent id, found `everything`"
        );
    }

    #[test]
    fn a_field_after_the_check_line_is_a_syntax_error() {
        let src = constraint_with("  rule  \"r\"\n  scope global\n  check \"c\"\n  rule  \"r2\"");
        let diags = parse_constraint_file(&constraint_path(), &src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
    fn parses_the_payload_schema_bindings_into_both_variants() {
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
        assert_eq!(diags[0].message, "expected an intent id, found `SCN-0107`");
    }

    #[test]
    fn a_proves_binding_rejects_an_intent_id() {
        let src = "proves     \"tests/billing.rs\" -> INT-0042\n";
        let diags = parse_bindings_file(&bindings_path(), src).unwrap_err();
        assert_eq!(diags.len(), 1, "diagnostics: {diags:#?}");
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

    // --- change files ----------------------------------------------------

    mod changes {
        use super::*;
        use crate::emit::{emit_change, emit_op};
        use crate::ids::ChangeId;
        use crate::model::change::fixtures::{
            CHANGE_EXAMPLE, JOURNAL_EXAMPLE, example_change, example_ops, implementing_change,
        };
        use crate::model::{JournalEntry, TestRun, Witness};

        fn change_path() -> RepoPath {
            RepoPath::new("telos/changes/CHG-0007.tel")
        }

        fn parse(src: &str) -> Change {
            parse_change_file(&change_path(), src)
                .unwrap_or_else(|d| panic!("must parse:\n{src}\n{d:#?}"))
        }

        fn diags(src: &str) -> Vec<Diagnostic> {
            parse_change_file(&change_path(), src).expect_err("must not parse")
        }

        #[test]
        fn parses_the_canonical_example_into_the_expected_change() {
            let change = parse(CHANGE_EXAMPLE);
            let expected = example_change();

            assert_eq!(change.id, ChangeId(7));
            assert_eq!(change.motivation, expected.motivation);
            assert_eq!(change.status, ChangeStatus::Approved);
            assert_eq!(change.approved_digest, expected.approved_digest);

            // Three of the four ops carry no `Sp` at all, so they compare
            // to the in-code fixture as they are...
            let ops = example_ops();
            assert_eq!(change.ops.len(), ops.len());
            assert_eq!(change.ops[0], ops[0]);
            assert_eq!(change.ops[2], ops[2]);
            assert_eq!(change.ops[3], ops[3]);
            // ...while the intent's references were parsed from real
            // positions, which the fixture (built in code) has no way to
            // predict. Their canonical bytes are the span-free identity,
            // and `ops_digest` is that identity for the whole delta.
            let StagedOp::EditOwnedIntent { intent, .. } = &change.ops[1] else {
                panic!("the second op edits an intent");
            };
            assert_eq!(intent.id, IntentId(17));
            assert_eq!(intent.title, "Issuing an invoice opens it");
            assert_eq!(intent.status, IntentStatus::Active);
            assert_eq!(intent.scenarios.len(), 1);
            assert_eq!(emit_op(&change.ops[1]), emit_op(&ops[1]));
            assert_eq!(change.ops_digest(), expected.ops_digest());
            assert_eq!(change.claims(), expected.claims());
        }

        #[test]
        fn the_canonical_example_round_trips_byte_exact() {
            // The emitter defines the canonical form, so its golden source
            // must survive a parse untouched.
            assert_eq!(emit_change(&parse(CHANGE_EXAMPLE)), CHANGE_EXAMPLE);
        }

        #[test]
        fn the_parsed_spans_point_into_the_change_file() {
            // The nested block is parsed in place, so its diagnostics point
            // at the change file's own offsets -- not at offsets in some
            // extracted substring.
            let StagedOp::EditOwnedIntent { intent, .. } = &parse(CHANGE_EXAMPLE).ops[1] else {
                panic!("the second op edits an intent");
            };
            let Statement::EventDriven { event, .. } = &intent.statement else {
                panic!("an event-driven statement");
            };
            let at = CHANGE_EXAMPLE.find("when   InvoiceIssued").unwrap() + "when   ".len();
            assert_eq!(
                event.span,
                Span {
                    start: at as u32,
                    end: (at + "InvoiceIssued".len()) as u32,
                }
            );
        }

        #[test]
        fn every_status_is_accepted() {
            for (word, status) in CHANGE_STATUSES {
                let digest =
                    if matches!(status, ChangeStatus::Approved | ChangeStatus::Implementing) {
                        format!("  digest \"sha256:{}\"\n", "0".repeat(64))
                    } else {
                        String::new()
                    };
                let src = format!("change CHG-0001 \"x\" {{\n  status {word}\n{digest}}}\n");
                assert_eq!(parse(&src).status, status);
            }
        }

        #[test]
        fn an_unknown_status_lists_the_five_and_suggests_the_closest() {
            let found = diags("change CHG-0001 \"x\" {\n  status aproved\n}\n");
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(
                found[0].message,
                "expected one of `open`, `drafted`, `approved`, `implementing`, \
                 `abandoned`, found `aproved`"
            );
            assert_eq!(found[0].hint.as_deref(), Some("closest is `approved`"));
            assert_eq!(found[0].code, ErrorCode::TelosParseError);
            assert_eq!((found[0].line, found[0].col), (Some(2), Some(10)));
        }

        #[test]
        fn a_missing_status_line_is_reported_once() {
            // `status` has a stand-in (`open`), so the ops after it are
            // still parsed rather than abandoned.
            let src = "change CHG-0001 \"x\" {\n  op remove notion billing/Ledger\n}\n";
            let found = diags(src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(found[0].message, "expected `status`, found `op`");
        }

        #[test]
        fn a_header_without_a_change_id_says_so() {
            let found = diags("change INT-0007 \"x\" {\n  status open\n}\n");
            assert_eq!(found[0].message, "expected a change id, found `INT-0007`");
        }

        #[test]
        fn a_status_that_carries_an_approval_must_carry_its_digest() {
            // `implementing` is an approved change in flight, so it
            // owes the same digest -- and one message covers both, since
            // both are approved changes.
            for status in ["approved", "implementing"] {
                let found = diags(&format!(
                    "change CHG-0001 \"x\" {{\n  status {status}\n}}\n"
                ));
                assert_eq!(found.len(), 1, "for `{status}`: {found:#?}");
                assert_eq!(
                    found[0].message, "an approved change must carry its digest",
                    "for `{status}`"
                );
                // Reported at the `status` line: that is the line the
                // writer has to look at to decide which of the two to fix.
                assert_eq!((found[0].line, found[0].col), (Some(2), Some(3)));
            }
        }

        #[test]
        fn a_status_that_carries_no_approval_may_not_carry_a_digest() {
            let digest = format!("  digest \"sha256:{}\"\n", "0".repeat(64));
            for status in ["open", "drafted", "abandoned"] {
                let src = format!("change CHG-0001 \"x\" {{\n  status {status}\n{digest}}}\n");
                let found = diags(&src);
                assert_eq!(found.len(), 1, "for `{status}`: {found:#?}");
                assert_eq!(
                    found[0].message, "digest is only valid on an approved or implementing change",
                    "for `{status}`"
                );
                assert_eq!((found[0].line, found[0].col), (Some(3), Some(3)));
            }
        }

        #[test]
        fn a_malformed_digest_is_reported_where_it_stands() {
            let src = "change CHG-0001 \"x\" {\n  status approved\n  digest \"sha256:beef\"\n}\n";
            let found = diags(src);
            // The digest line was written, so the coherence check stays
            // quiet: one fault, one diagnostic.
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(
                found[0].message,
                "malformed digest; expected sha256:<64 hex>"
            );
            assert_eq!((found[0].line, found[0].col), (Some(3), Some(10)));
        }

        #[test]
        fn is_canonical_digest_accepts_exactly_the_d3_form() {
            let hex = "9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0";
            assert!(is_canonical_digest(&format!("sha256:{hex}")));
            assert!(!is_canonical_digest(hex));
            assert!(!is_canonical_digest(&format!("sha256:{}", &hex[..63])));
            assert!(!is_canonical_digest(&format!("sha256:{hex}0")));
            assert!(!is_canonical_digest(&format!(
                "sha256:{}",
                hex.to_uppercase()
            )));
            assert!(!is_canonical_digest(&format!("SHA256:{hex}")));
            assert!(!is_canonical_digest(""));
        }

        #[test]
        fn every_op_shape_parses() {
            let src = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status drafted\n",
                "\n",
                "  op add notion billing/A entity {\n",
                "    def    \"a\"\n    phrase \"a\"\n",
                "  }\n",
                "\n",
                "  op edit notion billing/B value {\n",
                "    def    \"b\"\n    phrase \"b\"\n",
                "  }\n",
                "\n",
                "  op remove notion billing/C\n",
                "\n",
                "  op add constraint CON-0001 in project stack \"s\" {\n",
                "    rule  \"r\"\n",
                "  }\n",
                "\n",
                "  op edit constraint CON-0002 in context billing quality \"q\" {\n",
                "    rule  \"r\"\n",
                "  }\n",
                "\n",
                "  op remove constraint CON-0003 from billing\n",
                "\n",
                "  op remove intent INT-0004 from billing/invoicing\n",
                "\n",
                "  op accept \"telos/telos.toml\" \"e69de29\"\n",
                "}\n",
            );
            let verbs: Vec<(&str, &str)> = parse(src)
                .ops
                .iter()
                .map(|op| (op.verb(), op.entity()))
                .collect();
            assert_eq!(
                verbs,
                vec![
                    ("add", "notion"),
                    ("edit", "notion"),
                    ("remove", "notion"),
                    ("add", "constraint"),
                    ("edit", "constraint"),
                    ("remove", "constraint"),
                    ("remove", "intent"),
                    ("accept", "file"),
                ]
            );
        }

        #[test]
        fn an_accept_op_takes_exactly_two_strings() {
            let base = "change CHG-0001 \"x\" {\n  status drafted\n\n  op accept";
            let cases = [
                ("", "expected a repository path, found end of line"),
                (
                    " \"telos/telos.toml\"",
                    "expected the blob oid string after the path, found end of line",
                ),
                (
                    " \"telos/telos.toml\" \"e69de29\" \"extra\"",
                    "expected end of line, found `\"extra\"`",
                ),
            ];
            for (tail, expected) in cases {
                let found = diags(&format!("{base}{tail}\n}}\n"));
                assert_eq!(found.len(), 1, "for `{tail}`: {found:#?}");
                assert_eq!(found[0].message, expected, "for `{tail}`");
            }
        }

        #[test]
        fn a_remove_op_wants_the_id_form_its_keyword_announced() {
            let cases = [
                (
                    "notion billing/INT-0001",
                    "expected a capability id, found `INT-0001`",
                ),
                ("intent Ledger", "expected an intent id, found `Ledger`"),
                ("intent CON-0003", "expected an intent id, found `CON-0003`"),
                (
                    "constraint INT-0001",
                    "expected a constraint id, found `INT-0001`",
                ),
                (
                    "binding \"a.rs\"",
                    "expected `context`, `capability`, `notion`, `intent` or `constraint`, found `binding`",
                ),
            ];
            for (tail, expected) in cases {
                let src = format!(
                    "change CHG-0001 \"x\" {{\n  status drafted\n\n  op remove {tail}\n}}\n"
                );
                let found = diags(&src);
                assert_eq!(found.len(), 1, "for `{tail}`: {found:#?}");
                assert_eq!(found[0].message, expected, "for `{tail}`");
            }
        }

        #[test]
        fn an_unknown_verb_or_entity_names_what_was_expected() {
            let src = "change CHG-0001 \"x\" {\n  status drafted\n\n  op stage notion A\n}\n";
            assert_eq!(
                diags(src)[0].message,
                "expected `add`, `edit`, `remove`, `move` or `accept`, found `stage`"
            );
            let src =
                "change CHG-0001 \"x\" {\n  status drafted\n\n  op add scenario SCN-0001\n}\n";
            assert_eq!(
                diags(src)[0].message,
                "expected `context`, `capability`, `notion`, `intent`, `constraint` or `context-map`, found `scenario`"
            );
        }

        #[test]
        fn a_line_that_is_neither_an_op_nor_a_journal_line_is_reported_and_skipped() {
            let src = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status drafted\n",
                "\n",
                "  check \"cargo test\"\n",
                "\n",
                "  op remove notion billing/A\n",
                "}\n",
            );
            let found = diags(src);
            // The body admits four keywords; the op after the bad line is
            // still parsed.
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(
                found[0].message,
                "expected `op`, `run`, `bind` or `}`, found `check`"
            );
        }

        #[test]
        fn a_broken_nested_block_is_recovered_at_the_change_level() {
            // The fatal error is inside a nested intent (no `statement`),
            // two brace levels down. Recovery must climb back out to the
            // change's own body and keep going, or the ops below would
            // vanish silently.
            let src = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status drafted\n",
                "\n",
                "  op add intent INT-0001 in billing/invoicing \"t\" {\n",
                "    status draft\n",
                "    telos  \"why\"\n",
                "  }\n",
                "\n",
                "  op remove notion billing/Ledger\n",
                "\n",
                "  op remove intent Ledger\n",
                "}\n",
            );
            let found = diags(src);
            assert_eq!(found.len(), 2, "diagnostics: {found:#?}");
            assert_eq!(found[0].message, "expected `statement`, found `}`");
            assert_eq!(found[1].message, "expected an intent id, found `Ledger`");
        }

        #[test]
        fn an_unclosed_change_block_is_fatal() {
            let src = "change CHG-0001 \"x\" {\n  status drafted\n";
            let found = diags(src);
            assert_eq!(
                found[0].message,
                "expected `op`, `run`, `bind` or `}`, found end of input"
            );
        }

        #[test]
        fn nothing_may_follow_the_closing_brace() {
            let src = "change CHG-0001 \"x\" {\n  status open\n}\nchange CHG-0002 \"y\" {\n}\n";
            assert_eq!(
                diags(src)[0].message,
                "expected end of input, found `change`"
            );
        }

        #[test]
        fn blank_lines_between_ops_are_layout_only() {
            // The emitter writes exactly one blank line before each op; the
            // parser accepts none, one or many, and emitting normalizes.
            let dense = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status drafted\n",
                "  op remove notion billing/A\n",
                "  op remove notion billing/B\n",
                "}\n",
            );
            let canonical = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status drafted\n",
                "\n",
                "  op remove notion billing/A\n",
                "\n",
                "  op remove notion billing/B\n",
                "}\n",
            );
            assert_eq!(emit_change(&parse(dense)), canonical);
            assert_eq!(emit_change(&parse(canonical)), canonical);
        }

        // --- the journal ----------------------------------------

        /// An implementing change with `body` as its journal block.
        fn journalled(body: &str) -> String {
            format!(
                "change CHG-0001 \"x\" {{\n  status implementing\n  digest \"sha256:{}\"\n\n\
                 {body}}}\n",
                "0".repeat(64)
            )
        }

        #[test]
        fn parses_the_journal_example_into_the_expected_change() {
            let change = parse(JOURNAL_EXAMPLE);
            let expected = implementing_change();
            assert_eq!(change.status, ChangeStatus::Implementing);
            assert_eq!(change.ops.len(), 1);
            assert_eq!(change.journal, expected.journal);
            // The journal is not hashed: the parsed change digests exactly
            // as the same ops with no journal at all would.
            assert_eq!(change.ops_digest(), expected.ops_digest());
        }

        #[test]
        fn the_journal_example_round_trips_byte_exact() {
            assert_eq!(emit_change(&parse(JOURNAL_EXAMPLE)), JOURNAL_EXAMPLE);
        }

        #[test]
        fn a_run_line_carries_the_scenario_the_verdict_the_test_and_the_oid() {
            let src = journalled("  run  SCN-0107 red \"tests/billing.rs::scn_0107\" \"cafe\"\n");
            let change = parse(&src);
            assert_eq!(
                change.journal,
                vec![JournalEntry::Run(TestRun {
                    scenario: ScenarioId(107),
                    witness: Witness::Red,
                    test: "tests/billing.rs::scn_0107".parse().unwrap(),
                    oid: Oid("cafe".to_string()),
                })]
            );
        }

        #[test]
        fn a_test_locator_may_be_a_bare_path() {
            let src = journalled("  run  SCN-0107 green \"tests/billing.rs\" \"cafe\"\n");
            let JournalEntry::Run(run) = &parse(&src).journal[0] else {
                panic!("a run line");
            };
            assert_eq!(run.test.path, RepoPath::new("tests/billing.rs"));
            assert_eq!(run.test.name, None);
            assert_eq!(run.witness, Witness::Green);
        }

        #[test]
        fn a_locator_with_no_path_is_rejected_where_it_stands() {
            let src = journalled("  run  SCN-0107 red \"::scn_0107\" \"cafe\"\n");
            let found = diags(&src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert!(
                found[0]
                    .message
                    .contains("test reference is missing a path"),
                "{:?}",
                found[0].message
            );
        }

        #[test]
        fn the_oid_of_a_run_is_opaque() {
            // Never parsed, only compared (`Oid`): a sha1 repo writes 40
            // hex, a sha256 one 64, and the parser has no business knowing.
            let src = journalled("  run  SCN-0107 red \"tests/b.rs\" \"not-a-real-oid\"\n");
            let JournalEntry::Run(run) = &parse(&src).journal[0] else {
                panic!("a run line");
            };
            assert_eq!(run.oid, Oid("not-a-real-oid".to_string()));
        }

        #[test]
        fn a_bind_line_carries_the_path_and_the_intent() {
            let src = journalled("  bind \"src/billing.rs\" -> INT-0042\n");
            assert_eq!(
                parse(&src).journal,
                vec![JournalEntry::Bind {
                    path: RepoPath::new("src/billing.rs"),
                    intent: IntentId(42),
                }]
            );
        }

        #[test]
        fn a_verdict_outside_red_and_green_names_both_and_hints_the_closest() {
            let src = journalled("  run  SCN-0107 blue \"tests/b.rs\" \"cafe\"\n");
            let found = diags(&src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(found[0].message, "expected `red` or `green`, found `blue`");
            assert_eq!(found[0].code, ErrorCode::TelosParseError);
            assert_eq!((found[0].line, found[0].col), (Some(5), Some(17)));

            // A near miss is named: the closed set is two words wide, so a
            // typo has nowhere to hide.
            let src = journalled("  run  SCN-0107 gren \"tests/b.rs\" \"cafe\"\n");
            let found = diags(&src);
            assert_eq!(found[0].message, "expected `red` or `green`, found `gren`");
            assert_eq!(found[0].hint.as_deref(), Some("closest is `green`"));
        }

        #[test]
        fn a_journal_is_only_valid_on_an_implementing_change() {
            // The grammar of a journal line says nothing about status; this
            // rule relates two lines, so it lives with the digest check.
            for status in ["open", "drafted", "abandoned"] {
                let src = format!(
                    "change CHG-0001 \"x\" {{\n  status {status}\n\n  \
                     bind \"src/b.rs\" -> INT-0001\n}}\n"
                );
                let found = diags(&src);
                assert_eq!(found.len(), 1, "for `{status}`: {found:#?}");
                assert_eq!(
                    found[0].message, "a journal is only valid on an implementing change",
                    "for `{status}`"
                );
                // Reported at the first journal line: that is what has to go
                // (or what the status has to catch up with).
                assert_eq!((found[0].line, found[0].col), (Some(4), Some(3)));
            }
        }

        #[test]
        fn an_approved_change_may_not_carry_a_journal_either() {
            // The first journalled line moves an approved change to
            // `implementing`, so an approved change with a journal is a
            // change whose writer skipped that transition.
            let src = format!(
                "change CHG-0001 \"x\" {{\n  status approved\n  digest \"sha256:{}\"\n\n  \
                 run  SCN-0107 red \"tests/b.rs\" \"cafe\"\n}}\n",
                "0".repeat(64)
            );
            let found = diags(&src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(
                found[0].message,
                "a journal is only valid on an implementing change"
            );
        }

        #[test]
        fn a_journal_line_that_did_not_parse_still_counts_as_a_journal() {
            // The coherence check is about the line being there, not about
            // it being well-formed -- the same rule the digest check uses.
            let src = "change CHG-0001 \"x\" {\n  status drafted\n\n  bind \"src/b.rs\"\n}\n";
            let found = diags(src);
            let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
            assert_eq!(
                messages,
                vec![
                    "expected `->`, found end of line",
                    "a journal is only valid on an implementing change",
                ]
            );
        }

        #[test]
        fn an_op_after_a_journal_line_is_rejected() {
            // the journal format: journal lines come after the last op, so the body is
            // two phases -- once a journal line is seen, no op may follow.
            let src = journalled(concat!(
                "  bind \"src/b.rs\" -> INT-0001\n",
                "\n",
                "  op remove notion billing/A\n",
            ));
            let found = diags(&src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(
                found[0].message,
                "expected `run`, `bind` or `}`, found `op`"
            );
        }

        #[test]
        fn an_unclosed_journal_block_names_what_could_still_come() {
            let src =
                "change CHG-0001 \"x\" {\n  status implementing\n  bind \"a.rs\" -> INT-0001\n";
            let found = diags(src);
            assert_eq!(
                found[0].message,
                "expected `run`, `bind` or `}`, found end of input"
            );
        }

        #[test]
        fn blank_lines_inside_the_journal_are_layout_only() {
            // The emitter writes one blank line before the block and none
            // inside it; the parser accepts any spacing, and emitting
            // normalizes.
            let dense = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status implementing\n",
                "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
                "  bind \"a.rs\" -> INT-0001\n",
                "\n",
                "\n",
                "  run  SCN-0001 red \"tests/b.rs\" \"cafe\"\n",
                "}\n",
            );
            let canonical = concat!(
                "change CHG-0001 \"x\" {\n",
                "  status implementing\n",
                "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
                "\n",
                "  bind \"a.rs\" -> INT-0001\n",
                "  run  SCN-0001 red \"tests/b.rs\" \"cafe\"\n",
                "}\n",
            );
            assert_eq!(emit_change(&parse(dense)), canonical);
            assert_eq!(emit_change(&parse(canonical)), canonical);
        }

        #[test]
        fn a_run_line_wants_a_scenario_id_and_two_strings() {
            let cases = [
                (
                    "run  INT-0001 red \"t.rs\" \"cafe\"",
                    "expected a scenario id, found `INT-0001`",
                ),
                (
                    "run  SCN-0001",
                    "expected `red` or `green`, found end of line",
                ),
                (
                    "run  SCN-0001 red",
                    "expected a test reference, found end of line",
                ),
                (
                    "run  SCN-0001 red \"t.rs\"",
                    "expected the blob oid string after the test reference, found end of line",
                ),
                (
                    "run  SCN-0001 red \"t.rs\" \"cafe\" \"extra\"",
                    "expected end of line, found `\"extra\"`",
                ),
                ("bind \"a.rs\" INT-0001", "expected `->`, found `INT-0001`"),
                (
                    "bind INT-0001 -> INT-0001",
                    "expected a code path, found `INT-0001`",
                ),
                (
                    "bind \"a.rs\" -> SCN-0001",
                    "expected an intent id, found `SCN-0001`",
                ),
            ];
            for (line, expected) in cases {
                let src = journalled(&format!("  {line}\n"));
                let found = diags(&src);
                assert_eq!(found.len(), 1, "for `{line}`: {found:#?}");
                assert_eq!(found[0].message, expected, "for `{line}`");
            }
        }

        #[test]
        fn a_journal_line_may_not_name_a_path_under_telos() {
            // A journal path is a claim, and a claim licenses drift until
            // reconcile -- which `telos/**` must never grant.
            // Enforced by the grammar so a hand-edited change file cannot
            // buy what the commands would never write.
            let cases = [
                "  bind \"telos/bindings.tel\" -> INT-0001",
                "  bind \"telos/telos.toml\" -> INT-0001",
                "  bind \"telos/intents/INT-0001.tel\" -> INT-0001",
                "  run  SCN-0001 green \"telos/bindings.tel\" \"cafe\"",
                "  run  SCN-0001 red \"telos/notions/Invoice.tel::scn_0001\" \"cafe\"",
            ];
            for line in cases {
                let src = journalled(&format!("{line}\n"));
                let found = diags(&src);
                assert_eq!(found.len(), 1, "for `{line}`: {found:#?}");
                assert_eq!(
                    found[0].message, "a journal line cannot name a path under telos/",
                    "for `{line}`"
                );
                assert_eq!(found[0].code, ErrorCode::TelosParseError);
                assert_eq!(
                    found[0].hint.as_deref(),
                    Some(
                        "journal lines name code and test files; \
                         the spec tree is written by ops and by reconcile"
                    )
                );
                // Reported on the offending string, not on the keyword.
                let col = line.find('"').unwrap() + 1;
                assert_eq!((found[0].line, found[0].col), (Some(5), Some(col as u32)));
            }
        }

        #[test]
        fn a_path_that_merely_starts_with_the_letters_telos_is_fine() {
            // The refusal is about the spec directory, not about a prefix:
            // `telosaurus.rs` is ordinary code.
            let src = journalled(concat!(
                "  bind \"telosaurus.rs\" -> INT-0001\n",
                "  bind \"src/telos/adapter.rs\" -> INT-0001\n",
                "  run  SCN-0001 red \"tests/telos_cli.rs\" \"cafe\"\n",
            ));
            assert_eq!(parse(&src).journal.len(), 3);
        }

        #[test]
        fn a_refused_journal_path_never_reaches_claims() {
            // The end the rule serves: reconcile's drift gate waves through
            // any path the change claims, so `telos/**` must not get in.
            let src = journalled("  bind \"telos/bindings.tel\" -> INT-0001\n");
            assert!(parse_change_file(&change_path(), &src).is_err());
        }

        #[test]
        fn a_broken_journal_line_does_not_swallow_the_next_one() {
            let src = journalled(concat!(
                "  run  SCN-0001 blue \"tests/b.rs\" \"cafe\"\n",
                "  bind \"a.rs\" -> INT-0001\n",
            ));
            let found = diags(&src);
            assert_eq!(found.len(), 1, "diagnostics: {found:#?}");
            assert_eq!(found[0].message, "expected `red` or `green`, found `blue`");
        }
    }
}
