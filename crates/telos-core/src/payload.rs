//! Frozen JSON payload schemas: the `stdin` shape `add`/`edit`
//! read, translated into the typed model of [`crate::model`].
//!
//! This module sits at the same altitude as [`crate::syntax`]'s parser --
//! translating an external representation into the AST -- except the
//! external representation here is JSON, not the `.tel` mini-language.
//! Consequently it draws the same line the parser does: a payload function
//! resolves what it structurally must (a literal's shape needs the target
//! attribute's type, so `fields` are typed here; a `given`/`when` step needs
//! its notion resolved for the same reason) and leaves referential
//! integrity that does not gate typing -- whether a `refines` id exists,
//! whether an event notion really has kind `event` -- to the semantic pass
//! that runs once the whole model is assembled.
//!
//! Two exceptions are handled here because
//! resolving a `fields` literal is impossible without them: an unknown
//! notion named in `given`/`when`, and an unknown attribute named in
//! `fields`, are both reported here as `TELOS_REFERENCE_UNKNOWN` with a
//! `closest`-match suggestion.
//!
//! `add`/`edit` share one schema per entity kind, differing only in which
//! keys are mandatory: edits accept the same keys, all optional.
//! [`resolve`] captures that once: a key present in the JSON always wins, a
//! key absent falls back to the base entity's current value (`edit`) or a
//! per-field default (`add`), and a field with neither is a missing-field
//! error -- which only ever fires for `add`, since every `patch_*` call
//! supplies a base.
//!
//! One grammar rule -- `action = "set" , attr-ref , "=" , literal | prose`
//! -- is needed twice: once inside [`crate::syntax::parser`] for `.tel`
//! files, once here for a payload's bare action string. The parser's own
//! implementation (`P::action`) is not reachable from here: `P` and its
//! methods are private to `syntax::parser`, which does not `pub`-export the
//! struct. Rather than widen that module's visibility for this one caller,
//! [`parse_action`] rebuilds the small slice of the grammar it needs
//! directly on top of the lexer (`crate::syntax::lexer`, `pub(crate)` and
//! therefore already crate-visible), matching `P::action`/`P::attr_ref`/
//! `P::parse_literal` token-for-token.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::counters::Alloc;
use crate::error::{ErrorCode, TelosError};
use crate::ids::{FieldName, IntentId, NotionName, ScenarioId, is_lower_kebab};
use crate::model::{
    Action, Attr, AttrRef, AttrType, CmpOp, Constraint, ConstraintKind, Expr, InstanceStep, Intent,
    IntentStatus, Literal, Notion, NotionKind, Operand, Rel, Rule, Scenario, Scope, Statement,
};
use crate::span::{Sp, Span};
use crate::suggest::closest;
use crate::syntax::lexer::{TokKind, lex};
use crate::syntax::parse_expr;

// --- notion --------------------------------------------------------------

const NOTION_KEYS: &[&str] = &["name", "kind", "def", "attrs", "rels"];

const NOTION_KIND_WORDS: [(&str, NotionKind); 5] = [
    ("actor", NotionKind::Actor),
    ("entity", NotionKind::Entity),
    ("value", NotionKind::Value),
    ("event", NotionKind::Event),
    ("state", NotionKind::State),
];

const ATTR_TYPE_WORDS: [&str; 9] = [
    "string", "int", "decimal", "money", "bool", "date", "datetime", "enum", "ref",
];

/// Builds a `Notion` from an `add notion` payload.
pub fn notion_from_json(v: &Value) -> Result<Notion, TelosError> {
    let obj = as_object(v, "a notion payload")?;
    check_unknown_keys(obj, NOTION_KEYS, "notion")?;
    notion_from_obj(obj, None)
}

/// Applies an `edit notion` payload on top of `base`: every key is
/// optional, and a key present replaces that field wholesale.
pub fn patch_notion(base: &Notion, v: &Value) -> Result<Notion, TelosError> {
    let obj = as_object(v, "a notion payload")?;
    check_unknown_keys(obj, NOTION_KEYS, "notion")?;
    notion_from_obj(obj, Some(base))
}

fn notion_from_obj(obj: &Map<String, Value>, base: Option<&Notion>) -> Result<Notion, TelosError> {
    let name = resolve(
        obj,
        "name",
        "notion",
        base.map(|b| b.name.clone()),
        None,
        |v| NotionName::new(as_str(v, "name")?),
    )?;
    let kind = resolve(obj, "kind", "notion", base.map(|b| b.kind), None, |v| {
        notion_kind_from_str(as_str(v, "kind")?)
    })?;
    let def = resolve(
        obj,
        "def",
        "notion",
        base.map(|b| b.def.clone()),
        None,
        |v| Ok(as_str(v, "def")?.to_string()),
    )?;
    let attrs = resolve(
        obj,
        "attrs",
        "notion",
        base.map(|b| b.attrs.clone()),
        Some(Vec::new()),
        |v| {
            as_array(v, "attrs")?
                .iter()
                .map(|attr| attr_from_json(attr, &name))
                .collect()
        },
    )?;
    let rels = resolve(
        obj,
        "rels",
        "notion",
        base.map(|b| b.rels.clone()),
        Some(Vec::new()),
        |v| as_array(v, "rels")?.iter().map(rel_from_json).collect(),
    )?;
    Ok(Notion {
        name,
        kind,
        def,
        attrs,
        rels,
    })
}

fn notion_kind_from_str(s: &str) -> Result<NotionKind, TelosError> {
    NOTION_KIND_WORDS
        .iter()
        .find(|(word, _)| *word == s)
        .map(|(_, kind)| *kind)
        .ok_or_else(|| closed_set_err("notion kind", s, &NOTION_KIND_WORDS.map(|(word, _)| word)))
}

fn attr_from_json(v: &Value, notion: &NotionName) -> Result<Attr, TelosError> {
    let obj = as_object(v, "a notion attr")?;
    check_unknown_keys(obj, &["name", "type", "values", "target"], "notion attr")?;
    let name = FieldName::new(as_str(required(obj, "name", "notion attr")?, "name")?)?;
    let ty_word = as_str(required(obj, "type", "notion attr")?, "type")?;
    let ty = attr_type_from_json(ty_word, obj, notion, &name)?;
    Ok(Attr { name, ty })
}

/// The hint an enum symbol refused at staging carries: the grammar of what
/// `enum(...)` accepts in a `.tel` file, since the symbol is written there
/// verbatim. A caller that only ever sees JSON has no other way to learn
/// that a symbol is an identifier of the mini-language, not free text.
const ENUM_SYMBOL_HINT: &str = "an enum symbol is ASCII lower-kebab-case, exactly what a `.tel` \
file accepts inside `enum(...)`: a lowercase letter, then lowercase letters or digits, segments \
joined by single `-` (`playing`, `x-wins`)";

/// Types one attribute declaration. The symbols of an `enum` are the one
/// part of a declaration this boundary checks lexically: they are emitted
/// bare into the change file and read back by the parser as `lower-ident`s,
/// so a symbol spelled any other way (`X`) would be accepted here, written,
/// and then break every later parse of the change -- with no command left
/// to repair or abandon it. `FieldName`'s grammar is exactly that of a
/// `lower-ident`, so the same predicate serves both.
fn attr_type_from_json(
    word: &str,
    obj: &Map<String, Value>,
    notion: &NotionName,
    attr: &FieldName,
) -> Result<AttrType, TelosError> {
    match word {
        "string" => Ok(AttrType::String),
        "int" => Ok(AttrType::Int),
        "decimal" => Ok(AttrType::Decimal),
        "money" => Ok(AttrType::Money),
        "bool" => Ok(AttrType::Bool),
        "date" => Ok(AttrType::Date),
        "datetime" => Ok(AttrType::Datetime),
        "enum" => {
            let values = as_array(required(obj, "values", "notion attr")?, "values")?;
            if values.is_empty() {
                return Err(shape_err(
                    "`enum` attribute type requires a non-empty `values` array",
                ));
            }
            let symbols = values
                .iter()
                .map(|item| {
                    let symbol = as_str(item, "values")?;
                    if !is_lower_kebab(symbol) {
                        return Err(shape_err(format!(
                            "attribute `{notion}.{attr}` has type `enum`, but `{symbol}` is not \
                             an enum symbol; symbols are lower-kebab-case like `x-wins`"
                        ))
                        .hint(ENUM_SYMBOL_HINT));
                    }
                    Ok(symbol.to_string())
                })
                .collect::<Result<Vec<_>, TelosError>>()?;
            Ok(AttrType::Enum(symbols))
        }
        "ref" => {
            let target = required(obj, "target", "notion attr")?;
            Ok(AttrType::Ref(NotionName::new(as_str(target, "target")?)?))
        }
        unknown => Err(closed_set_err("attribute type", unknown, &ATTR_TYPE_WORDS)),
    }
}

fn rel_from_json(v: &Value) -> Result<Rel, TelosError> {
    let obj = as_object(v, "a notion rel")?;
    check_unknown_keys(obj, &["name", "target"], "notion rel")?;
    let name = FieldName::new(as_str(required(obj, "name", "notion rel")?, "name")?)?;
    let target = NotionName::new(as_str(required(obj, "target", "notion rel")?, "target")?)?;
    Ok(Rel {
        name,
        target: sp(target),
    })
}

// --- intent ----------------------------------------------------------------

const INTENT_KEYS: &[&str] = &[
    "title",
    "status",
    "telos",
    "statement",
    "refines",
    "requires",
    "excludes",
    "scenarios",
];

const INTENT_STATUS_WORDS: [(&str, IntentStatus); 3] = [
    ("draft", IntentStatus::Draft),
    ("active", IntentStatus::Active),
    ("deprecated", IntentStatus::Deprecated),
];

/// Builds an `(Intent, Vec<ScenarioId>)` from an `add intent` payload. The
/// intent id and every scenario id are freshly allocated
/// from `alloc`, since an `add` payload never carries one. `notions` resolves
/// the attribute types `given`/`when` field
/// literals are typed against.
pub fn intent_from_json(
    v: &Value,
    notions: &BTreeMap<NotionName, Notion>,
    alloc: &mut Alloc,
) -> Result<(Intent, Vec<ScenarioId>), TelosError> {
    let obj = as_object(v, "an intent payload")?;
    check_unknown_keys(obj, INTENT_KEYS, "intent")?;

    let id = alloc.next_intent();
    let title = as_str(required(obj, "title", "intent")?, "title")?.to_string();
    let status = intent_status_from_str(as_str(required(obj, "status", "intent")?, "status")?)?;
    let telos = as_str(required(obj, "telos", "intent")?, "telos")?.to_string();
    let statement = statement_from_json(required(obj, "statement", "intent")?)?;
    let refines = resolve(obj, "refines", "intent", None, Some(Vec::new()), |v| {
        intent_ids_from_json(v, "refines")
    })?;
    let requires = resolve(obj, "requires", "intent", None, Some(Vec::new()), |v| {
        intent_ids_from_json(v, "requires")
    })?;
    let excludes = resolve(obj, "excludes", "intent", None, Some(Vec::new()), |v| {
        intent_ids_from_json(v, "excludes")
    })?;
    let scenarios_json = match obj.get("scenarios") {
        Some(v) => as_array(v, "scenarios")?.as_slice(),
        None => &[],
    };
    let (scenarios, allocated) = scenarios_from_json(scenarios_json, notions, alloc, None)?;

    let intent = Intent {
        id,
        title,
        status,
        telos,
        statement,
        refines,
        requires,
        excludes,
        scenarios,
    };
    Ok((intent, allocated))
}

/// Applies an `edit intent` payload on top of `base`: every key is
/// optional and replaces its field wholesale when present; `scenarios`,
/// when present, replaces the whole list -- entries with an `id` replace
/// the scenario of that id, entries without one are newly allocated, and
/// any scenario of `base` absent from the list is dropped.
pub fn patch_intent(
    base: &Intent,
    v: &Value,
    notions: &BTreeMap<NotionName, Notion>,
    alloc: &mut Alloc,
) -> Result<(Intent, Vec<ScenarioId>), TelosError> {
    let obj = as_object(v, "an intent payload")?;
    check_unknown_keys(obj, INTENT_KEYS, "intent")?;

    let title = resolve(
        obj,
        "title",
        "intent",
        Some(base.title.clone()),
        None,
        |v| Ok(as_str(v, "title")?.to_string()),
    )?;
    let status = resolve(obj, "status", "intent", Some(base.status), None, |v| {
        intent_status_from_str(as_str(v, "status")?)
    })?;
    let telos = resolve(
        obj,
        "telos",
        "intent",
        Some(base.telos.clone()),
        None,
        |v| Ok(as_str(v, "telos")?.to_string()),
    )?;
    let statement = resolve(
        obj,
        "statement",
        "intent",
        Some(base.statement.clone()),
        None,
        statement_from_json,
    )?;
    let refines = resolve(
        obj,
        "refines",
        "intent",
        Some(base.refines.clone()),
        None,
        |v| intent_ids_from_json(v, "refines"),
    )?;
    let requires = resolve(
        obj,
        "requires",
        "intent",
        Some(base.requires.clone()),
        None,
        |v| intent_ids_from_json(v, "requires"),
    )?;
    let excludes = resolve(
        obj,
        "excludes",
        "intent",
        Some(base.excludes.clone()),
        None,
        |v| intent_ids_from_json(v, "excludes"),
    )?;
    let (scenarios, allocated) = match obj.get("scenarios") {
        Some(v) => scenarios_from_json(
            as_array(v, "scenarios")?,
            notions,
            alloc,
            Some(&base.scenarios),
        )?,
        None => (base.scenarios.clone(), Vec::new()),
    };

    let intent = Intent {
        id: base.id,
        title,
        status,
        telos,
        statement,
        refines,
        requires,
        excludes,
        scenarios,
    };
    Ok((intent, allocated))
}

fn intent_status_from_str(s: &str) -> Result<IntentStatus, TelosError> {
    INTENT_STATUS_WORDS
        .iter()
        .find(|(word, _)| *word == s)
        .map(|(_, status)| *status)
        .ok_or_else(|| closed_set_err("intent status", s, &INTENT_STATUS_WORDS.map(|(w, _)| w)))
}

fn intent_ids_from_json(v: &Value, field: &str) -> Result<Vec<Sp<IntentId>>, TelosError> {
    as_array(v, field)?
        .iter()
        .map(|item| {
            let id: IntentId = as_str(item, field)?.parse()?;
            Ok(sp(id))
        })
        .collect()
}

// --- statement / action ---------------------------------------------------

fn statement_from_json(v: &Value) -> Result<Statement, TelosError> {
    let obj = as_object(v, "a statement")?;
    let template = as_str(required(obj, "template", "statement")?, "template")?;
    match template {
        "ubiquitous" => {
            check_unknown_keys(obj, &["template", "action"], "statement")?;
            Ok(Statement::Ubiquitous {
                action: action_from_json(obj)?,
            })
        }
        "event-driven" => {
            check_unknown_keys(obj, &["template", "when", "on", "action"], "statement")?;
            let event = NotionName::new(as_str(required(obj, "when", "statement")?, "when")?)?;
            let on = match obj.get("on") {
                Some(v) => Some(sp(NotionName::new(as_str(v, "on")?)?)),
                None => None,
            };
            Ok(Statement::EventDriven {
                event: sp(event),
                on,
                action: action_from_json(obj)?,
            })
        }
        "state-driven" => {
            check_unknown_keys(obj, &["template", "while", "action"], "statement")?;
            let text = as_str(required(obj, "while", "statement")?, "while")?;
            let expr = parse_expr_field(text, "statement.while")?;
            let (subject, value) = state_driven_shape(expr)?;
            Ok(Statement::StateDriven {
                subject,
                value,
                action: action_from_json(obj)?,
            })
        }
        "unwanted" => {
            check_unknown_keys(obj, &["template", "if", "action"], "statement")?;
            let text = as_str(required(obj, "if", "statement")?, "if")?;
            let condition = parse_expr_field(text, "statement.if")?;
            Ok(Statement::Unwanted {
                condition,
                action: action_from_json(obj)?,
            })
        }
        "optional" => {
            check_unknown_keys(obj, &["template", "where", "action"], "statement")?;
            let text = as_str(required(obj, "where", "statement")?, "where")?;
            let feature = FieldName::new(text)?;
            Ok(Statement::Optional {
                feature,
                action: action_from_json(obj)?,
            })
        }
        unknown => Err(closed_set_err(
            "statement template",
            unknown,
            &[
                "ubiquitous",
                "event-driven",
                "state-driven",
                "unwanted",
                "optional",
            ],
        )),
    }
}

/// The `while` expression of a `state-driven` statement must parse to
/// exactly `Notion.attr == literal`: any other shape -- a
/// different operator, a bare literal on the left, `and`/`or`/`not` -- is a
/// clear, dedicated error rather than a silent `_ => unreachable` mismatch.
fn state_driven_shape(expr: Expr) -> Result<(AttrRef, Literal), TelosError> {
    match expr {
        Expr::Cmp {
            op: CmpOp::Eq,
            lhs: Operand::Ref(subject),
            rhs: Operand::Lit(value),
        } => Ok((subject, value)),
        _ => Err(shape_err(
            "a state-driven `while` must parse to exactly `Notion.attr == literal`",
        )),
    }
}

fn action_from_json(obj: &Map<String, Value>) -> Result<Action, TelosError> {
    parse_action(as_str(required(obj, "action", "statement")?, "action")?)
}

/// Parses a bare action string: `set <Notion>.<attr> = <literal>`
/// when it starts with `"set "`, a free clause otherwise. See the module
/// doc comment for why this rebuilds the grammar instead of reusing
/// `syntax::parser::P::action`.
fn parse_action(text: &str) -> Result<Action, TelosError> {
    if !text.starts_with("set ") {
        return Ok(Action::Free(text.to_string()));
    }
    let malformed = || {
        TelosError::new(
            ErrorCode::TelosParseError,
            "an action starting with `set ` must be a formal assignment".to_string(),
        )
    };
    let toks: Vec<TokKind> = lex(text)
        .map_err(|_| malformed())?
        .into_iter()
        .map(|t| t.kind)
        .collect();
    let mut it = toks.into_iter();

    match it.next() {
        Some(TokKind::LowerIdent(w)) if w == "set" => {}
        _ => return Err(malformed()),
    }
    let notion = match it.next() {
        Some(TokKind::UpperIdent(s)) => NotionName::new(s).map_err(|_| malformed())?,
        _ => return Err(malformed()),
    };
    match it.next() {
        Some(TokKind::Dot) => {}
        _ => return Err(malformed()),
    }
    let attr = match it.next() {
        Some(TokKind::LowerIdent(s)) => FieldName::new(s).map_err(|_| malformed())?,
        _ => return Err(malformed()),
    };
    match it.next() {
        Some(TokKind::Assign) => {}
        _ => return Err(malformed()),
    }
    let value = it
        .next()
        .and_then(|t| literal_from_tok(&t))
        .ok_or_else(malformed)?;
    match it.next() {
        Some(TokKind::Eof) => {}
        _ => return Err(malformed()),
    }

    Ok(Action::Set {
        target: AttrRef {
            notion: sp(notion),
            attr: sp(attr),
        },
        value,
    })
}

/// Mirrors `syntax::parser::P::parse_literal`'s token-to-literal mapping,
/// restricted to the tokens an action's right-hand side can be.
fn literal_from_tok(tok: &TokKind) -> Option<Literal> {
    Some(match tok {
        TokKind::Str(s) => Literal::Str(s.clone()),
        TokKind::Int(n) => Literal::Int(*n),
        TokKind::Decimal(s) => Literal::Decimal(s.clone()),
        TokKind::Date(s) => Literal::Date(s.clone()),
        TokKind::Datetime(s) => Literal::Datetime(s.clone()),
        TokKind::LowerIdent(w) if w == "true" => Literal::Bool(true),
        TokKind::LowerIdent(w) if w == "false" => Literal::Bool(false),
        TokKind::LowerIdent(w) => Literal::Symbol(sp(w.clone())),
        _ => return None,
    })
}

// --- scenarios ---------------------------------------------------------

/// Parses `arr` into scenarios, allocating a fresh id for every entry that
/// carries none. `existing` distinguishes `add` (`None`: no entry may carry
/// an `id`, every scenario is new) from `edit` (`Some(base scenarios)`: an
/// entry may carry an `id` to replace that scenario in place; a scenario of
/// `base` missing from `arr` is simply not in the returned list, which is
/// what "replaces the field wholesale" means for `scenarios`). Returns the
/// full scenario list plus the ids freshly allocated along the way.
fn scenarios_from_json(
    arr: &[Value],
    notions: &BTreeMap<NotionName, Notion>,
    alloc: &mut Alloc,
    existing: Option<&[Scenario]>,
) -> Result<(Vec<Scenario>, Vec<ScenarioId>), TelosError> {
    let allow_id = existing.is_some();
    let allowed_keys: &[&str] = if allow_id {
        &["id", "title", "given", "when", "then"]
    } else {
        &["title", "given", "when", "then"]
    };

    let mut scenarios = Vec::new();
    let mut allocated = Vec::new();
    for (index, item) in arr.iter().enumerate() {
        let obj = as_object(item, "a scenario")?;
        check_unknown_keys(obj, allowed_keys, "scenario")?;

        let id = match obj.get("id") {
            Some(v) => as_str(v, "id")?.parse()?,
            None => {
                let id = alloc.next_scenario();
                allocated.push(id);
                id
            }
        };
        scenarios.push(scenario_fields_from_json(obj, notions, id, index)?);
    }
    Ok((scenarios, allocated))
}

fn scenario_fields_from_json(
    obj: &Map<String, Value>,
    notions: &BTreeMap<NotionName, Notion>,
    id: ScenarioId,
    index: usize,
) -> Result<Scenario, TelosError> {
    let title = as_str(required(obj, "title", "scenario")?, "title")?.to_string();
    let given: Vec<InstanceStep> = as_array(required(obj, "given", "scenario")?, "given")?
        .iter()
        .map(|g| instance_step_from_json(g, notions))
        .collect::<Result<_, _>>()?;
    if given.is_empty() {
        return Err(shape_err(format!(
            "scenario `{title}` must have at least one `given` step"
        )));
    }
    let when = instance_step_from_json(required(obj, "when", "scenario")?, notions)?;
    let then: Vec<Expr> = as_array(required(obj, "then", "scenario")?, "then")?
        .iter()
        .enumerate()
        .map(|(i, t)| {
            parse_expr_field(as_str(t, "then")?, &format!("scenarios[{index}].then[{i}]"))
        })
        .collect::<Result<_, _>>()?;
    if then.is_empty() {
        return Err(shape_err(format!(
            "scenario `{title}` must have at least one `then` expression"
        )));
    }
    Ok(Scenario {
        id,
        title,
        given,
        when,
        then,
    })
}

/// A `given`/`when` step: `{"notion": "Invoice", "fields": {...}}`. The
/// notion must be known since resolving each
/// field's literal needs its declared attribute type.
fn instance_step_from_json(
    v: &Value,
    notions: &BTreeMap<NotionName, Notion>,
) -> Result<InstanceStep, TelosError> {
    let obj = as_object(v, "an instance step")?;
    check_unknown_keys(obj, &["notion", "fields"], "instance step")?;

    let name = NotionName::new(as_str(required(obj, "notion", "instance step")?, "notion")?)?;
    let notion = notions
        .get(&name)
        .ok_or_else(|| unknown_notion_err(&name, notions))?;

    let fields = match obj.get("fields") {
        Some(v) => {
            let fields_obj = as_object(v, "field `fields`")?;
            let mut fields = Vec::new();
            for (key, val) in fields_obj {
                let field_name = FieldName::new(key.as_str())?;
                let attr = notion
                    .attrs
                    .iter()
                    .find(|a| a.name == field_name)
                    .ok_or_else(|| unknown_attr_err(&field_name, notion))?;
                let literal = literal_from_field_json(val, attr, &field_name, &notion.name)?;
                fields.push((sp(field_name), literal));
            }
            fields
        }
        None => Vec::new(),
    };

    Ok(InstanceStep {
        notion: sp(name),
        fields,
    })
}

/// Types one `fields` entry against the target attribute's declared type.
/// Structural typing only -- membership of an `enum` symbol
/// among its declared values, the `^\d+\.\d{2} [A-Z]{3}$` shape of a
/// `money` amount -- is the semantic pass's job, once the whole model
/// exists to check it against; this boundary only ever needs to decide,
/// from the JSON value alone, which `Literal` variant it becomes.
///
/// The one lexeme this boundary checks in a `fields` value is a `decimal`'s
/// (an enum's declared symbols get the same treatment in
/// [`attr_type_from_json`], for the same reason). The string
/// is stored verbatim, emitted verbatim into the change file, and read
/// back by the lexer -- so anything but a `decimal-lit` would come back as
/// a different variant (`"2"` as an `Int` the semantic pass rejects at
/// reconcile, long after staging accepted it) or not lex at all. Nothing
/// downstream ever sees that string as a decimal again, so the check has
/// to happen here, where the attribute is still in hand to name.
fn literal_from_field_json(
    v: &Value,
    attr: &Attr,
    field: &FieldName,
    notion: &NotionName,
) -> Result<Literal, TelosError> {
    match &attr.ty {
        AttrType::String => Ok(Literal::Str(as_str(v, field.as_str())?.to_string())),
        AttrType::Int => v
            .as_i64()
            .map(Literal::Int)
            .ok_or_else(|| shape_err(format!("field `{notion}.{field}` must be a JSON integer"))),
        AttrType::Decimal => {
            if v.is_number() {
                return Err(TelosError::new(
                    ErrorCode::TelosParseError,
                    "decimal values must be JSON strings to avoid float hazards".to_string(),
                ));
            }
            let lexeme = as_str(v, field.as_str())?;
            if !is_decimal_lexeme(lexeme) {
                return Err(shape_err(format!(
                    "field `{notion}.{field}` has type `decimal`, but `{lexeme}` is not \
                     a decimal of the form `120.50`; a whole number is written `2.0`"
                )));
            }
            Ok(Literal::Decimal(lexeme.to_string()))
        }
        AttrType::Money => Ok(Literal::Str(as_str(v, field.as_str())?.to_string())),
        AttrType::Bool => Ok(Literal::Bool(as_bool(v, field.as_str())?)),
        AttrType::Date => Ok(Literal::Date(as_str(v, field.as_str())?.to_string())),
        AttrType::Datetime => Ok(Literal::Datetime(as_str(v, field.as_str())?.to_string())),
        AttrType::Enum(_) => Ok(Literal::Symbol(sp(as_str(v, field.as_str())?.to_string()))),
        AttrType::Ref(_) => Err(TelosError::new(
            ErrorCode::TelosParseError,
            "ref attributes cannot be set from payloads".to_string(),
        )),
    }
}

/// `^-?\d+\.\d+$` -- the canonical `decimal-lit` lexeme, exactly the
/// strings the lexer turns into one `Decimal` token (see
/// `syntax::lexer`'s `lex_number`): an optional `-`, at least one ASCII
/// digit, a `.`, at least one ASCII digit. No exponent, no leading `+`, no
/// bare `.5` or `2.`, and no surrounding whitespace either, since the string
/// is stored and emitted as is.
fn is_decimal_lexeme(s: &str) -> bool {
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let Some((units, fraction)) = unsigned.split_once('.') else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(units) && digits(fraction)
}

fn unknown_notion_err(name: &NotionName, notions: &BTreeMap<NotionName, Notion>) -> TelosError {
    let known: Vec<&str> = notions.keys().map(NotionName::as_str).collect();
    TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        with_suggestion(format!("unknown notion `{name}`"), name.as_str(), &known),
    )
}

fn unknown_attr_err(field: &FieldName, notion: &Notion) -> TelosError {
    let known: Vec<&str> = notion.attrs.iter().map(|a| a.name.as_str()).collect();
    TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        with_suggestion(
            format!("unknown attribute `{field}` on notion `{}`", notion.name),
            field.as_str(),
            &known,
        ),
    )
}

// --- constraint ----------------------------------------------------------

const CONSTRAINT_KEYS: &[&str] = &["kind", "title", "rule", "scope", "check"];

const CONSTRAINT_KIND_WORDS: [(&str, ConstraintKind); 5] = [
    ("stack", ConstraintKind::Stack),
    ("architecture", ConstraintKind::Architecture),
    ("quality", ConstraintKind::Quality),
    ("security", ConstraintKind::Security),
    ("convention", ConstraintKind::Convention),
];

/// Builds a `Constraint` from an `add constraint` payload,
/// allocating its id from `alloc`.
pub fn constraint_from_json(v: &Value, alloc: &mut Alloc) -> Result<Constraint, TelosError> {
    let obj = as_object(v, "a constraint payload")?;
    check_unknown_keys(obj, CONSTRAINT_KEYS, "constraint")?;

    let id = alloc.next_constraint();
    let kind = constraint_kind_from_str(as_str(required(obj, "kind", "constraint")?, "kind")?)?;
    let title = as_str(required(obj, "title", "constraint")?, "title")?.to_string();
    let rule = rule_from_json(required(obj, "rule", "constraint")?)?;
    let scope = scope_from_json(required(obj, "scope", "constraint")?)?;
    let check = match obj.get("check") {
        Some(Value::Null) | None => None,
        Some(v) => Some(as_str(v, "check")?.to_string()),
    };

    Ok(Constraint {
        id,
        kind,
        title,
        rule,
        scope,
        check,
    })
}

/// Applies an `edit constraint` payload on top of `base`: every
/// key is optional and replaces its field wholesale when present;
/// `"check": null` explicitly clears it, while an absent `check` leaves it
/// untouched.
pub fn patch_constraint(base: &Constraint, v: &Value) -> Result<Constraint, TelosError> {
    let obj = as_object(v, "a constraint payload")?;
    check_unknown_keys(obj, CONSTRAINT_KEYS, "constraint")?;

    let kind = resolve(obj, "kind", "constraint", Some(base.kind), None, |v| {
        constraint_kind_from_str(as_str(v, "kind")?)
    })?;
    let title = resolve(
        obj,
        "title",
        "constraint",
        Some(base.title.clone()),
        None,
        |v| Ok(as_str(v, "title")?.to_string()),
    )?;
    let rule = resolve(
        obj,
        "rule",
        "constraint",
        Some(base.rule.clone()),
        None,
        rule_from_json,
    )?;
    let scope = resolve(
        obj,
        "scope",
        "constraint",
        Some(base.scope.clone()),
        None,
        scope_from_json,
    )?;
    let check = match obj.get("check") {
        Some(Value::Null) => None,
        Some(v) => Some(as_str(v, "check")?.to_string()),
        None => base.check.clone(),
    };

    Ok(Constraint {
        id: base.id,
        kind,
        title,
        rule,
        scope,
        check,
    })
}

fn constraint_kind_from_str(s: &str) -> Result<ConstraintKind, TelosError> {
    CONSTRAINT_KIND_WORDS
        .iter()
        .find(|(word, _)| *word == s)
        .map(|(_, kind)| *kind)
        .ok_or_else(|| closed_set_err("constraint kind", s, &CONSTRAINT_KIND_WORDS.map(|(w, _)| w)))
}

fn rule_from_json(v: &Value) -> Result<Rule, TelosError> {
    let obj = as_object(v, "a rule")?;
    check_unknown_keys(obj, &["text", "expr"], "rule")?;
    match (obj.get("text"), obj.get("expr")) {
        (Some(t), None) => Ok(Rule::Text(as_str(t, "text")?.to_string())),
        (None, Some(e)) => Ok(Rule::Machine(parse_expr_field(
            as_str(e, "expr")?,
            "rule.expr",
        )?)),
        (Some(_), Some(_)) => Err(shape_err(
            "`rule` must have exactly one of `text` or `expr`, not both",
        )),
        (None, None) => Err(shape_err(
            "`rule` must have exactly one of `text` or `expr`",
        )),
    }
}

fn scope_from_json(v: &Value) -> Result<Scope, TelosError> {
    if let Some(s) = v.as_str() {
        return if s == "global" {
            Ok(Scope::Global)
        } else {
            Err(shape_err(format!(
                "field `scope` must be `\"global\"` or an array of intent ids, found string `{s}`"
            )))
        };
    }
    Ok(Scope::Intents(intent_ids_from_json(v, "scope")?))
}

// --- JSON plumbing ---------------------------------------------------------

/// A "shape" error: the payload does not match the schema at all (wrong
/// JSON type for a key, a required key missing, `rule` naming neither
/// `text` nor `expr`...). Always `TELOS_PARSE_ERROR`, prefixed `"payload:
/// "` per the public schema -- distinct from the handful of exact messages
/// frozen for unknown attribute types, the decimal-as-number refusal,
/// a malformed `set` action, an unknown key), none of which carry this
/// prefix.
fn shape_err(msg: impl Into<String>) -> TelosError {
    TelosError::new(
        ErrorCode::TelosParseError,
        format!("payload: {}", msg.into()),
    )
}

/// The hint every payload expression field carries when it fails to parse.
/// A caller that only ever sees JSON gets nothing actionable from a bare
/// lexer or parser message (`` unexpected character `é` ``): it has to be
/// told the field is a grammar, not free text. Set unconditionally because
/// neither the lexer nor the parser ever produces a hint of its own.
const EXPR_HINT: &str = "this field holds an expression of the `.tel` \
mini-language, not prose: `Notion.attr == literal` or `Notion.attr in \
(a, b)`, combined with `and`/`or`/`not`; identifiers are ASCII";

/// Parses one payload field holding a mini-language expression, tagging the
/// diagnostic with that field's path in the payload (`scenarios[0].then[1]`,
/// `statement.while`...) and [`EXPR_HINT`]. The diagnostic itself carries no
/// file, so without the path the caller cannot tell which of a payload's
/// several expression fields it came from.
fn parse_expr_field(text: &str, field: &str) -> Result<Expr, TelosError> {
    parse_expr(text).map_err(|diag| {
        let err = TelosError::from(diag);
        TelosError::new(err.code, format!("payload.{field}: {}", err.message)).hint(EXPR_HINT)
    })
}

fn missing_field_err(key: &str, kind: &str) -> TelosError {
    shape_err(format!("missing required field `{key}` in {kind} payload"))
}

/// The exact wording for an unknown key: no `"payload:
/// "` prefix, `{kind}` names the payload ("notion", "intent", "scenario"...).
fn unknown_key_err(key: &str, kind: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosParseError,
        format!("unknown key `{key}` in {kind} payload"),
    )
}

/// `unknown {noun} \`{word}\`; expected one of {options}` -- the shape
/// used for attribute-type errors and every other closed-set field (notion
/// kind, intent status, constraint kind, statement template).
fn closed_set_err(noun: &str, word: &str, options: &[&str]) -> TelosError {
    TelosError::new(
        ErrorCode::TelosParseError,
        format!(
            "unknown {noun} `{word}`; expected one of {}",
            options.join(", ")
        ),
    )
}

/// Appends `; closest is \`x\`` when one of `candidates` is close enough to
/// `word` -- mirrors `semantic`'s private helper of the same name, which is
/// not reachable from here.
fn with_suggestion(message: String, word: &str, candidates: &[&str]) -> String {
    match closest(word, candidates.iter().copied()) {
        Some(known) => format!("{message}; closest is `{known}`"),
        None => message,
    }
}

fn sp<T>(node: T) -> Sp<T> {
    Sp {
        node,
        span: Span::default(),
    }
}

/// Resolves one payload field: the JSON value when the key is present
/// (parsed via `parse`); otherwise `base` (`edit`: leave the field as it
/// was) when given; otherwise `default` (`add`: the field's fallback, e.g.
/// `attrs` defaulting to `[]`); otherwise a missing-field error (`add`,
/// mandatory field, no fallback).
fn resolve<T>(
    obj: &Map<String, Value>,
    key: &str,
    kind: &str,
    base: Option<T>,
    default: Option<T>,
    parse: impl FnOnce(&Value) -> Result<T, TelosError>,
) -> Result<T, TelosError> {
    match obj.get(key) {
        Some(v) => parse(v),
        None => base.or(default).ok_or_else(|| missing_field_err(key, kind)),
    }
}

fn as_object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, TelosError> {
    v.as_object()
        .ok_or_else(|| shape_err(format!("expected a JSON object for {what}")))
}

fn as_array<'a>(v: &'a Value, field: &str) -> Result<&'a Vec<Value>, TelosError> {
    v.as_array()
        .ok_or_else(|| shape_err(format!("field `{field}` must be an array")))
}

fn as_str<'a>(v: &'a Value, field: &str) -> Result<&'a str, TelosError> {
    v.as_str()
        .ok_or_else(|| shape_err(format!("field `{field}` must be a string")))
}

fn as_bool(v: &Value, field: &str) -> Result<bool, TelosError> {
    v.as_bool()
        .ok_or_else(|| shape_err(format!("field `{field}` must be a bool")))
}

fn check_unknown_keys(
    obj: &Map<String, Value>,
    allowed: &[&str],
    kind: &str,
) -> Result<(), TelosError> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(unknown_key_err(key, kind));
        }
    }
    Ok(())
}

fn required<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    kind: &str,
) -> Result<&'a Value, TelosError> {
    obj.get(key).ok_or_else(|| missing_field_err(key, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::Counters;
    use crate::ids::ScenarioId;
    use crate::model::{Rule as ModelRule, Scope as ModelScope};

    // --- fixtures ----------------------------------------------------------

    fn name(s: &str) -> NotionName {
        NotionName::new(s).unwrap()
    }

    fn field(s: &str) -> FieldName {
        FieldName::new(s).unwrap()
    }

    fn invoice_and_payment_notions() -> BTreeMap<NotionName, Notion> {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Invoice"),
            Notion {
                name: name("Invoice"),
                kind: NotionKind::Entity,
                def: "A bill.".to_string(),
                attrs: vec![
                    Attr {
                        name: field("state"),
                        ty: AttrType::Enum(vec!["open".to_string(), "settled".to_string()]),
                    },
                    Attr {
                        name: field("balance"),
                        ty: AttrType::Money,
                    },
                ],
                rels: vec![],
            },
        );
        notions.insert(
            name("PaymentReceived"),
            Notion {
                name: name("PaymentReceived"),
                kind: NotionKind::Event,
                def: "A payment arrived.".to_string(),
                attrs: vec![Attr {
                    name: field("amount"),
                    ty: AttrType::Money,
                }],
                rels: vec![],
            },
        );
        notions
    }

    fn sample_intent() -> Intent {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "Invoices can be settled", "status": "active",
            "telos": "Customers must see immediately that their debt is cleared.",
            "statement": { "template": "event-driven", "when": "PaymentReceived",
                           "on": "Invoice", "action": "set Invoice.state = settled" },
            "refines": [], "requires": ["INT-0017"], "excludes": [],
            "scenarios": [
                { "title": "a full payment settles the invoice",
                  "given": [ {"notion": "Invoice", "fields": {"state": "open", "balance": "120.00 EUR"}} ],
                  "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
                  "then":  ["Invoice.state == settled"] } ] });
        intent_from_json(&json, &notions, &mut alloc).unwrap().0
    }

    // --- notion_from_json ----------------------------------------------

    #[test]
    fn notion_from_json_parses_the_full_payload_schema_example() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity",
            "def": "A bill issued to a Customer for delivered work.",
            "attrs": [
                {"name": "state", "type": "enum", "values": ["open", "settled"]},
                {"name": "balance", "type": "money"},
                {"name": "customer", "type": "ref", "target": "Customer"}
            ],
            "rels": [ {"name": "issued-to", "target": "Customer"} ]
        });

        let notion = notion_from_json(&json).unwrap();

        assert_eq!(
            notion,
            Notion {
                name: name("Invoice"),
                kind: NotionKind::Entity,
                def: "A bill issued to a Customer for delivered work.".to_string(),
                attrs: vec![
                    Attr {
                        name: field("state"),
                        ty: AttrType::Enum(vec!["open".to_string(), "settled".to_string()]),
                    },
                    Attr {
                        name: field("balance"),
                        ty: AttrType::Money,
                    },
                    Attr {
                        name: field("customer"),
                        ty: AttrType::Ref(name("Customer")),
                    },
                ],
                rels: vec![Rel {
                    name: field("issued-to"),
                    target: sp(name("Customer")),
                }],
            }
        );
    }

    #[test]
    fn notion_from_json_defaults_attrs_and_rels_to_empty() {
        let json = serde_json::json!({"name": "Customer", "kind": "actor", "def": "x"});
        let notion = notion_from_json(&json).unwrap();
        assert_eq!(notion.attrs, vec![]);
        assert_eq!(notion.rels, vec![]);
    }

    #[test]
    fn notion_from_json_rejects_unknown_attribute_type_with_exact_message() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "x",
            "attrs": [ {"name": "state", "type": "txt"} ]
        });
        let err = notion_from_json(&json).unwrap_err();
        assert_eq!(
            err.message,
            "unknown attribute type `txt`; expected one of string, int, decimal, money, bool, date, datetime, enum, ref"
        );
        assert_eq!(err.code, ErrorCode::TelosParseError);
    }

    #[test]
    fn notion_from_json_rejects_enum_attribute_without_values() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "x",
            "attrs": [ {"name": "state", "type": "enum"} ]
        });
        assert!(notion_from_json(&json).is_err());
    }

    #[test]
    fn notion_from_json_rejects_enum_attribute_with_empty_values() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "x",
            "attrs": [ {"name": "state", "type": "enum", "values": []} ]
        });
        assert!(notion_from_json(&json).is_err());
    }

    /// An enum symbol is written bare into the change file and read back by
    /// the parser as a `lower-ident`: `X` would be accepted here, emitted as
    /// `enum(playing, X, o, draw)`, and make the whole change unparseable
    /// for every later command (#29). The boundary refuses it, naming the
    /// attribute, the value and the accepted spelling.
    #[test]
    fn notion_from_json_rejects_enum_symbol_that_is_not_lower_kebab_naming_the_attribute() {
        let json = serde_json::json!({
            "name": "Board", "kind": "entity", "def": "x",
            "attrs": [ {"name": "outcome", "type": "enum",
                        "values": ["playing", "X", "o", "draw"]} ]
        });
        let err = notion_from_json(&json).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert_eq!(
            err.message,
            "payload: attribute `Board.outcome` has type `enum`, but `X` is not an enum \
             symbol; symbols are lower-kebab-case like `x-wins`"
        );
        assert_eq!(
            err.hint.as_deref(),
            Some(
                "an enum symbol is ASCII lower-kebab-case, exactly what a `.tel` file \
                 accepts inside `enum(...)`: a lowercase letter, then lowercase letters or \
                 digits, segments joined by single `-` (`playing`, `x-wins`)"
            )
        );
    }

    /// What the boundary accepts as an enum symbol always lexes back as a
    /// single `LowerIdent` token covering the whole string -- the round-trip
    /// invariant the check exists for. The relation is inclusion, not
    /// equality: the lexer also tolerates `_` inside a segment, which the
    /// documented `lower-ident` grammar does not, so a refused symbol may
    /// still lex while an accepted one always does.
    #[test]
    fn is_lower_kebab_accepts_only_what_lexes_as_one_lower_ident_token() {
        let cases = [
            ("playing", true),
            ("x-wins", true),
            ("o", true),
            ("a1", true),
            ("v2-final", true),
            ("X", false),
            ("Draw", false),
            ("x_wins", false),
            ("-x", false),
            ("x-", false),
            ("x--wins", false),
            ("1x", false),
            ("x wins", false),
            ("x.y", false),
            ("", false),
            ("\u{E9}", false),
            ("x\n", false),
        ];
        for (symbol, expected) in cases {
            assert_eq!(is_lower_kebab(symbol), expected, "{symbol:?}");
            if expected {
                let toks = lex(symbol).unwrap_or_else(|d| panic!("{symbol:?} does not lex: {d:?}"));
                assert!(
                    matches!(
                        toks.as_slice(),
                        [first, eof]
                            if first.kind == TokKind::LowerIdent(symbol.to_string())
                                && eof.kind == TokKind::Eof
                    ),
                    "{symbol:?} does not lex as one `LowerIdent`: {toks:?}"
                );
            }
        }
    }

    #[test]
    fn notion_from_json_rejects_ref_attribute_without_target() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "x",
            "attrs": [ {"name": "customer", "type": "ref"} ]
        });
        assert!(notion_from_json(&json).is_err());
    }

    #[test]
    fn notion_from_json_rejects_an_unknown_top_level_key_with_exact_message() {
        let json = serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "x", "titel": "oops"
        });
        let err = notion_from_json(&json).unwrap_err();
        assert_eq!(err.message, "unknown key `titel` in notion payload");
    }

    #[test]
    fn notion_from_json_rejects_a_missing_required_field() {
        let json = serde_json::json!({"kind": "entity", "def": "x"});
        assert!(notion_from_json(&json).is_err());
    }

    // --- patch_notion --------------------------------------------------

    #[test]
    fn patch_notion_with_only_def_touches_only_that_field() {
        let base = notion_from_json(&serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "old def",
            "attrs": [{"name": "state", "type": "string"}],
        }))
        .unwrap();

        let patched = patch_notion(&base, &serde_json::json!({"def": "new def"})).unwrap();

        assert_eq!(patched.def, "new def");
        assert_eq!(patched.name, base.name);
        assert_eq!(patched.kind, base.kind);
        assert_eq!(patched.attrs, base.attrs);
        assert_eq!(patched.rels, base.rels);
    }

    #[test]
    fn patch_notion_replaces_attrs_wholesale() {
        let base = notion_from_json(&serde_json::json!({
            "name": "Invoice", "kind": "entity", "def": "d",
            "attrs": [{"name": "state", "type": "string"}, {"name": "balance", "type": "money"}],
        }))
        .unwrap();

        let patched = patch_notion(
            &base,
            &serde_json::json!({"attrs": [{"name": "balance", "type": "money"}]}),
        )
        .unwrap();

        assert_eq!(
            patched.attrs,
            vec![Attr {
                name: field("balance"),
                ty: AttrType::Money
            }]
        );
    }

    // --- intent_from_json / statements -----------------------------------

    #[test]
    fn intent_from_json_allocates_ids_past_the_corpus_floor() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: 42,
                scenario: 107,
                constraint: 3,
                change: 0,
            },
        );
        let json = serde_json::json!({
            "title": "Invoices can be settled", "status": "active",
            "telos": "Customers must see immediately that their debt is cleared.",
            "statement": { "template": "event-driven", "when": "PaymentReceived",
                           "on": "Invoice", "action": "set Invoice.state = settled" },
            "refines": [], "requires": ["INT-0017"], "excludes": [],
            "scenarios": [
                { "title": "a full payment settles the invoice",
                  "given": [ {"notion": "Invoice", "fields": {"state": "open", "balance": "120.00 EUR"}} ],
                  "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
                  "then":  ["Invoice.state == settled"] } ] });

        let (intent, scenario_ids) = intent_from_json(&json, &notions, &mut alloc).unwrap();

        assert_eq!(intent.id, IntentId(43));
        assert_eq!(scenario_ids, vec![ScenarioId(108)]);
        assert_eq!(intent.scenarios[0].id, ScenarioId(108));
        assert_eq!(intent.requires, vec![sp(IntentId(17))]);
        match &intent.statement {
            Statement::EventDriven { event, on, action } => {
                assert_eq!(event.node, name("PaymentReceived"));
                assert_eq!(on.as_ref().map(|s| s.node.clone()), Some(name("Invoice")));
                assert!(matches!(action, Action::Set { .. }));
            }
            other => panic!("expected EventDriven, got {other:?}"),
        }
    }

    #[test]
    fn intent_from_json_rejects_an_unknown_top_level_key_with_exact_message() {
        let notions = BTreeMap::new();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({"titel": "oops"});
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.message, "unknown key `titel` in intent payload");
    }

    #[test]
    fn intent_from_json_unwanted_template_parses_the_if_expr() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "no negative balances", "status": "active", "telos": "t",
            "statement": {"template": "unwanted", "if": "Invoice.balance < 0", "action": "reject the write"},
            "scenarios": [
                {"title": "s",
                 "given": [{"notion": "Invoice", "fields": {}}],
                 "when": {"notion": "PaymentReceived", "fields": {}},
                 "then": ["Invoice.state == open"]}
            ]
        });
        let (intent, _) = intent_from_json(&json, &notions, &mut alloc).unwrap();
        match intent.statement {
            Statement::Unwanted { condition, action } => {
                assert!(matches!(condition, Expr::Cmp { op: CmpOp::Lt, .. }));
                assert_eq!(action, Action::Free("reject the write".to_string()));
            }
            other => panic!("expected Unwanted, got {other:?}"),
        }
    }

    #[test]
    fn intent_from_json_state_driven_template_extracts_subject_and_value() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "state-driven", "while": "Invoice.state == open", "action": "notify"},
            "scenarios": [
                {"title": "s",
                 "given": [{"notion": "Invoice", "fields": {}}],
                 "when": {"notion": "PaymentReceived", "fields": {}},
                 "then": ["Invoice.state == open"]}
            ]
        });
        let (intent, _) = intent_from_json(&json, &notions, &mut alloc).unwrap();
        match intent.statement {
            Statement::StateDriven { subject, value, .. } => {
                assert_eq!(subject.notion.node, name("Invoice"));
                assert_eq!(subject.attr.node, field("state"));
                // `while` is parsed by `parse_expr` from real source text, so
                // the symbol's span reflects its position in that string --
                // unlike a `fields` literal (built straight from JSON, no
                // source text to point at), so only the node is compared.
                match value {
                    Literal::Symbol(sym) => assert_eq!(sym.node, "open"),
                    other => panic!("expected Symbol, got {other:?}"),
                }
            }
            other => panic!("expected StateDriven, got {other:?}"),
        }
    }

    #[test]
    fn intent_from_json_state_driven_rejects_a_while_that_is_not_ref_eq_literal() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "state-driven", "while": "Invoice.balance > 0", "action": "notify"},
            "scenarios": [
                {"title": "s",
                 "given": [{"notion": "Invoice", "fields": {}}],
                 "when": {"notion": "PaymentReceived", "fields": {}},
                 "then": ["Invoice.state == open"]}
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert!(err.message.contains("state-driven"));
    }

    #[test]
    fn intent_from_json_optional_template_parses_the_where_feature() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "optional", "where": "dark-mode", "action": "show dark mode"},
            "scenarios": [
                {"title": "s",
                 "given": [{"notion": "Invoice", "fields": {}}],
                 "when": {"notion": "PaymentReceived", "fields": {}},
                 "then": ["Invoice.state == open"]}
            ]
        });
        let (intent, _) = intent_from_json(&json, &notions, &mut alloc).unwrap();
        match intent.statement {
            Statement::Optional { feature, .. } => assert_eq!(feature, field("dark-mode")),
            other => panic!("expected Optional, got {other:?}"),
        }
    }

    #[test]
    fn scenario_then_expr_surfaces_the_parse_expr_diagnostic() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "ubiquitous", "action": "do it"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoice","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert!(
            err.message.contains("comparison operator"),
            "expected the parse_expr diagnostic to surface, got: {}",
            err.message
        );
    }

    #[test]
    fn scenario_then_expr_error_names_its_field_path_and_hints_the_grammar() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "ubiquitous", "action": "do it"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoice","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open", "la facture est réglée"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert!(
            err.message.starts_with("payload.scenarios[0].then[1]: "),
            "expected the failing field's path, got: {}",
            err.message
        );
        assert!(
            err.message.contains("unexpected character `é`"),
            "expected the lexer diagnostic to survive the prefix, got: {}",
            err.message
        );
        assert!(
            err.hint
                .as_deref()
                .is_some_and(|h| h.contains("mini-language") && h.contains("not prose")),
            "expected the mini-language hint, got: {:?}",
            err.hint
        );
    }

    #[test]
    fn statement_while_expr_error_names_its_field_path() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "state-driven", "while": "la facture est ouverte",
                          "action": "notify"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoice","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert!(
            err.message.starts_with("payload.statement.while: "),
            "expected the failing field's path, got: {}",
            err.message
        );
        assert!(err.hint.is_some(), "expected the mini-language hint");
    }

    #[test]
    fn statement_if_expr_error_names_its_field_path() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "unwanted", "if": "le solde est négatif",
                          "action": "refuse"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoice","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert!(
            err.message.starts_with("payload.statement.if: "),
            "expected the failing field's path, got: {}",
            err.message
        );
        assert!(err.hint.is_some(), "expected the mini-language hint");
    }

    #[test]
    fn constraint_rule_expr_error_names_its_field_path() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "kind": "architecture", "title": "t",
            "rule": {"expr": "le solde reste positif"},
            "scope": "global"
        });
        let err = constraint_from_json(&json, &mut alloc).unwrap_err();
        assert!(
            err.message.starts_with("payload.rule.expr: "),
            "expected the failing field's path, got: {}",
            err.message
        );
        assert!(err.hint.is_some(), "expected the mini-language hint");
    }

    #[test]
    fn intent_from_json_rejects_an_unknown_notion_in_given_with_a_suggestion() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "ubiquitous", "action": "do it"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoic","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
        assert!(
            err.message.contains("closest is `Invoice`"),
            "{}",
            err.message
        );
    }

    #[test]
    fn intent_from_json_rejects_an_unknown_attribute_in_fields_with_a_suggestion() {
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "ubiquitous", "action": "do it"},
            "scenarios": [
                { "title": "s",
                  "given": [{"notion":"Invoice","fields":{"stat": "open"}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
        assert!(
            err.message.contains("closest is `state`"),
            "{}",
            err.message
        );
    }

    // --- fields literal typing -------------------------------------------

    #[test]
    fn given_field_enum_value_resolves_to_symbol() {
        let notions = invoice_and_payment_notions();
        let json = serde_json::json!({"notion": "Invoice", "fields": {"state": "open"}});
        let step = instance_step_from_json(&json, &notions).unwrap();
        assert_eq!(
            step.fields,
            vec![(sp(field("state")), Literal::Symbol(sp("open".to_string())))]
        );
    }

    #[test]
    fn given_field_string_value_resolves_to_str() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Customer"),
            Notion {
                name: name("Customer"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("name"),
                    ty: AttrType::String,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Customer", "fields": {"name": "ACME"}});
        let step = instance_step_from_json(&json, &notions).unwrap();
        assert_eq!(
            step.fields,
            vec![(sp(field("name")), Literal::Str("ACME".to_string()))]
        );
    }

    #[test]
    fn given_field_int_value_resolves_to_int() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Order"),
            Notion {
                name: name("Order"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("quantity"),
                    ty: AttrType::Int,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Order", "fields": {"quantity": 3}});
        let step = instance_step_from_json(&json, &notions).unwrap();
        assert_eq!(step.fields, vec![(sp(field("quantity")), Literal::Int(3))]);
    }

    #[test]
    fn given_field_int_as_json_string_is_rejected() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Order"),
            Notion {
                name: name("Order"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("quantity"),
                    ty: AttrType::Int,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Order", "fields": {"quantity": "3"}});
        assert!(instance_step_from_json(&json, &notions).is_err());
    }

    #[test]
    fn given_field_decimal_as_json_number_is_rejected_with_exact_message() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Product"),
            Notion {
                name: name("Product"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("price"),
                    ty: AttrType::Decimal,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Product", "fields": {"price": 120.50}});
        let err = instance_step_from_json(&json, &notions).unwrap_err();
        assert_eq!(
            err.message,
            "decimal values must be JSON strings to avoid float hazards"
        );
        assert_eq!(err.code, ErrorCode::TelosParseError);
    }

    #[test]
    fn given_field_decimal_as_json_string_resolves_to_decimal() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Product"),
            Notion {
                name: name("Product"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("price"),
                    ty: AttrType::Decimal,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Product", "fields": {"price": "120.50"}});
        let step = instance_step_from_json(&json, &notions).unwrap();
        assert_eq!(
            step.fields,
            vec![(sp(field("price")), Literal::Decimal("120.50".to_string()))]
        );
    }

    /// The lexeme is stored and re-emitted verbatim, then read back by the
    /// lexer: `"2"` would come back as an `Int` and fail typing at reconcile
    /// after staging had accepted it. The boundary refuses it, naming the
    /// attribute and the fix.
    #[test]
    fn given_field_decimal_string_that_is_not_a_decimal_lexeme_is_rejected_naming_the_attribute() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Product"),
            Notion {
                name: name("Product"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("price"),
                    ty: AttrType::Decimal,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Product", "fields": {"price": "2"}});
        let err = instance_step_from_json(&json, &notions).unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert_eq!(
            err.message,
            "payload: field `Product.price` has type `decimal`, but `2` is not a decimal \
             of the form `120.50`; a whole number is written `2.0`"
        );
    }

    /// What the boundary accepts is exactly what the lexer reads back as a
    /// single `Decimal` token covering the whole string -- the round-trip
    /// invariant the check exists for, pinned so the two cannot drift.
    #[test]
    fn is_decimal_lexeme_accepts_exactly_what_lexes_as_one_decimal_token() {
        let cases = [
            ("120.50", true),
            ("-0.10", true),
            ("2.0", true),
            ("0.5", true),
            ("2", false),
            ("-2", false),
            (".5", false),
            ("2.", false),
            ("+1.0", false),
            ("--1.0", false),
            ("1e5", false),
            ("1.5.5", false),
            ("1.5 2", false),
            (" 1.5", false),
            ("1.5\n", false),
            ("2026-08-19", false),
            ("abc", false),
            ("", false),
            ("\u{FF11}.\u{FF15}", false),
        ];
        for (lexeme, expected) in cases {
            assert_eq!(is_decimal_lexeme(lexeme), expected, "{lexeme:?}");
            let lexes_as_one_decimal = match lex(lexeme) {
                Ok(toks) => matches!(
                    toks.as_slice(),
                    [first, eof]
                        if first.kind == TokKind::Decimal(lexeme.to_string())
                            && eof.kind == TokKind::Eof
                ),
                Err(_) => false,
            };
            assert_eq!(
                lexes_as_one_decimal, expected,
                "lexer disagrees on {lexeme:?}"
            );
        }
    }

    #[test]
    fn given_field_bool_value_resolves_to_bool() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Flag"),
            Notion {
                name: name("Flag"),
                kind: NotionKind::Value,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("enabled"),
                    ty: AttrType::Bool,
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Flag", "fields": {"enabled": true}});
        let step = instance_step_from_json(&json, &notions).unwrap();
        assert_eq!(
            step.fields,
            vec![(sp(field("enabled")), Literal::Bool(true))]
        );
    }

    #[test]
    fn given_field_ref_attribute_is_refused() {
        let mut notions = BTreeMap::new();
        notions.insert(
            name("Invoice"),
            Notion {
                name: name("Invoice"),
                kind: NotionKind::Entity,
                def: "x".to_string(),
                attrs: vec![Attr {
                    name: field("customer"),
                    ty: AttrType::Ref(name("Customer")),
                }],
                rels: vec![],
            },
        );
        let json = serde_json::json!({"notion": "Invoice", "fields": {"customer": "Customer/1"}});
        let err = instance_step_from_json(&json, &notions).unwrap_err();
        assert_eq!(err.message, "ref attributes cannot be set from payloads");
    }

    // --- action -----------------------------------------------------------

    #[test]
    fn action_starting_with_set_parses_to_a_formal_assignment() {
        let action = parse_action("set Invoice.state = settled").unwrap();
        match action {
            Action::Set { target, value } => {
                assert_eq!(target.notion.node, name("Invoice"));
                assert_eq!(target.attr.node, field("state"));
                assert_eq!(value, Literal::Symbol(sp("settled".to_string())));
            }
            other => panic!("expected Action::Set, got {other:?}"),
        }
    }

    #[test]
    fn action_not_starting_with_set_is_a_free_clause() {
        assert_eq!(
            parse_action("notify the auditor").unwrap(),
            Action::Free("notify the auditor".to_string())
        );
    }

    #[test]
    fn action_starting_with_set_but_not_an_assignment_is_rejected_with_exact_message() {
        let err = parse_action("set in stone").unwrap_err();
        assert_eq!(
            err.message,
            "an action starting with `set ` must be a formal assignment"
        );
        assert_eq!(err.code, ErrorCode::TelosParseError);
    }

    // --- patch_intent -------------------------------------------------

    #[test]
    fn patch_intent_with_only_telos_touches_only_that_field() {
        let base = sample_intent();
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: base.id.0,
                scenario: base.scenarios.iter().map(|s| s.id.0).max().unwrap_or(0),
                constraint: 0,
                change: 0,
            },
        );

        let json = serde_json::json!({"telos": "x"});
        let (patched, allocated) = patch_intent(&base, &json, &notions, &mut alloc).unwrap();

        assert_eq!(patched.telos, "x");
        assert_eq!(patched.id, base.id);
        assert_eq!(patched.title, base.title);
        assert_eq!(patched.status, base.status);
        assert_eq!(patched.statement, base.statement);
        assert_eq!(patched.refines, base.refines);
        assert_eq!(patched.requires, base.requires);
        assert_eq!(patched.excludes, base.excludes);
        assert_eq!(patched.scenarios, base.scenarios);
        assert!(allocated.is_empty());
    }

    #[test]
    fn patch_intent_scenario_with_id_replaces_that_scenario_in_place() {
        let base = sample_intent();
        let existing_id = base.scenarios[0].id;
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: base.id.0,
                scenario: existing_id.0,
                constraint: 0,
                change: 0,
            },
        );

        let json = serde_json::json!({
            "scenarios": [
                { "id": existing_id.to_string(), "title": "renamed",
                  "given": [ {"notion": "Invoice", "fields": {"state": "open"}} ],
                  "when":  {"notion": "PaymentReceived", "fields": {}},
                  "then":  ["Invoice.state == settled"] }
            ]
        });
        let (patched, allocated) = patch_intent(&base, &json, &notions, &mut alloc).unwrap();

        assert_eq!(patched.scenarios.len(), 1);
        assert_eq!(patched.scenarios[0].id, existing_id);
        assert_eq!(patched.scenarios[0].title, "renamed");
        assert!(allocated.is_empty());
    }

    #[test]
    fn patch_intent_scenario_without_id_is_newly_allocated() {
        let base = sample_intent();
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: base.id.0,
                scenario: base.scenarios[0].id.0,
                constraint: 0,
                change: 0,
            },
        );

        let json = serde_json::json!({
            "scenarios": [
                { "title": "brand new",
                  "given": [ {"notion": "Invoice", "fields": {}} ],
                  "when":  {"notion": "PaymentReceived", "fields": {}},
                  "then":  ["Invoice.state == open"] }
            ]
        });
        let (patched, allocated) = patch_intent(&base, &json, &notions, &mut alloc).unwrap();

        assert_eq!(allocated.len(), 1);
        assert_eq!(patched.scenarios[0].id, allocated[0]);
        assert_ne!(patched.scenarios[0].id, base.scenarios[0].id);
    }

    #[test]
    fn patch_intent_scenario_add_payload_rejects_an_id_key() {
        // add-style scenario parsing (intent_from_json) never allows "id".
        let notions = invoice_and_payment_notions();
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "title": "t", "status": "active", "telos": "t",
            "statement": {"template": "ubiquitous", "action": "do it"},
            "scenarios": [
                { "id": "SCN-0001", "title": "s",
                  "given": [{"notion":"Invoice","fields":{}}],
                  "when": {"notion":"PaymentReceived","fields":{}},
                  "then": ["Invoice.state == open"] }
            ]
        });
        let err = intent_from_json(&json, &notions, &mut alloc).unwrap_err();
        assert_eq!(err.message, "unknown key `id` in scenario payload");
    }

    // --- constraint_from_json / patch_constraint --------------------------

    #[test]
    fn constraint_from_json_parses_the_payload_schema_example() {
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: 0,
                scenario: 0,
                constraint: 3,
                change: 0,
            },
        );
        let json = serde_json::json!({
            "kind": "architecture", "title": "Hexagonal boundaries",
            "rule": {"text": "Domain code must not import adapter modules."},
            "scope": "global", "check": "scripts/check-imports.sh --layer domain"
        });
        let constraint = constraint_from_json(&json, &mut alloc).unwrap();
        assert_eq!(constraint.id, crate::ids::ConstraintId(4));
        assert_eq!(constraint.kind, ConstraintKind::Architecture);
        assert_eq!(
            constraint.rule,
            ModelRule::Text("Domain code must not import adapter modules.".to_string())
        );
        assert_eq!(constraint.scope, ModelScope::Global);
        assert_eq!(
            constraint.check.as_deref(),
            Some("scripts/check-imports.sh --layer domain")
        );
    }

    #[test]
    fn constraint_from_json_rule_expr_parses_via_parse_expr() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "kind": "architecture", "title": "t",
            "rule": {"expr": "Invoice.balance >= 0"},
            "scope": ["INT-0017"]
        });
        let constraint = constraint_from_json(&json, &mut alloc).unwrap();
        assert!(matches!(constraint.rule, ModelRule::Machine(_)));
        assert_eq!(
            constraint.scope,
            ModelScope::Intents(vec![sp(IntentId(17))])
        );
        assert_eq!(constraint.check, None);
    }

    #[test]
    fn constraint_from_json_rejects_a_rule_with_both_text_and_expr() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let json = serde_json::json!({
            "kind": "architecture", "title": "t",
            "rule": {"text": "x", "expr": "Invoice.balance >= 0"},
            "scope": "global"
        });
        assert!(constraint_from_json(&json, &mut alloc).is_err());
    }

    #[test]
    fn patch_constraint_null_check_clears_it() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let base = constraint_from_json(
            &serde_json::json!({
                "kind": "architecture", "title": "t",
                "rule": {"text": "x"}, "scope": "global", "check": "run.sh"
            }),
            &mut alloc,
        )
        .unwrap();

        let patched = patch_constraint(&base, &serde_json::json!({"check": null})).unwrap();
        assert_eq!(patched.check, None);
        assert_eq!(patched.title, base.title);
    }

    #[test]
    fn patch_constraint_absent_check_leaves_it_untouched() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        let base = constraint_from_json(
            &serde_json::json!({
                "kind": "architecture", "title": "t",
                "rule": {"text": "x"}, "scope": "global", "check": "run.sh"
            }),
            &mut alloc,
        )
        .unwrap();

        let patched = patch_constraint(&base, &serde_json::json!({"title": "renamed"})).unwrap();
        assert_eq!(patched.check.as_deref(), Some("run.sh"));
        assert_eq!(patched.title, "renamed");
    }
}
