//! Recursive-descent parser for the `.tel` syntax, built on the Task 4
//! lexer.
//!
//! Task 5 covers Annex C.2's `notion-file` rule and Annex C.3's expression
//! mini-language; intents, scenarios, constraints and bindings land in
//! Task 6 on top of the same helpers.

use crate::error::{Diagnostic, ErrorCode};
use crate::ids::{FieldName, NotionName, RepoPath};
use crate::model::{
    Attr, AttrRef, AttrType, CmpOp, Expr, Literal, Notion, NotionKind, Operand, Rel,
};
use crate::span::{Sp, Span, line_col};
use crate::suggest::closest;

use super::lexer::{TokKind, Token, lex};

/// The five `notion-kind` words (Annex C.2), in grammar order.
const NOTION_KINDS: [&str; 5] = ["actor", "entity", "value", "event", "state"];

/// The `attr-type` head words (Annex C.2), in grammar order.
const ATTR_TYPES: [&str; 9] = [
    "string", "int", "decimal", "money", "bool", "date", "datetime", "enum", "ref",
];

/// Parses a whole `notion` file (Annex C.2).
///
/// Reports every diagnostic found: a syntax error inside a field line is
/// recovered from (skip to the end of the line, then resume at the next
/// field keyword or `}`), so one file can yield several diagnostics.
pub fn parse_notion_file(path: &RepoPath, src: &str) -> Result<Notion, Vec<Diagnostic>> {
    let mut p = match P::new(src, Some(path)) {
        Ok(p) => p,
        Err(diag) => return Err(vec![diag]),
    };
    match p.notion_file() {
        Ok(notion) if p.diags.is_empty() => Ok(notion),
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

/// Parser state: the token stream plus the cursor, the source (for
/// positions and for rendering the token a diagnostic was found at), the
/// file the diagnostics belong to, and the diagnostics collected so far.
struct P<'a> {
    toks: Vec<Token>,
    pos: usize,
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

    fn advance(&mut self) {
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

    /// Error recovery: swallow the rest of the offending line so parsing
    /// can resume at the next field keyword or `}`.
    fn recover_to_newline(&mut self) {
        loop {
            if self.at_eof() || self.at(&TokKind::RBrace) {
                return;
            }
            let done = self.at(&TokKind::Newline);
            self.advance();
            if done {
                return;
            }
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
        self.skip_newlines();

        let mut def = String::new();
        match self.def_field() {
            Ok(text) => def = text,
            Err(diag) => {
                self.diags.push(diag);
                self.recover_to_newline();
            }
        }

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
                match self.attr_field() {
                    Ok(attr) => attrs.push(attr),
                    Err(diag) => {
                        self.diags.push(diag);
                        self.recover_to_newline();
                    }
                }
                continue;
            }
            if self.at_kw("rel") {
                in_rels = true;
                match self.rel_field() {
                    Ok(rel) => rels.push(rel),
                    Err(diag) => {
                        self.diags.push(diag);
                        self.recover_to_newline();
                    }
                }
                continue;
            }
            let diag = self.expected(what);
            self.diags.push(diag);
            self.recover_to_newline();
        }

        self.skip_newlines();
        if !self.at_eof() {
            let diag = self.expected("end of input");
            self.diags.push(diag);
        }

        Ok(Notion {
            name: name.node,
            kind,
            def,
            attrs,
            rels,
        })
    }

    fn notion_kind(&mut self) -> Result<NotionKind, Diagnostic> {
        let TokKind::LowerIdent(word) = &self.peek().kind else {
            return Err(self.expected("a notion kind"));
        };
        let span = self.peek().span;
        let kind = match word.as_str() {
            "actor" => NotionKind::Actor,
            "entity" => NotionKind::Entity,
            "value" => NotionKind::Value,
            "event" => NotionKind::Event,
            "state" => NotionKind::State,
            unknown => {
                let message = match closest(unknown, NOTION_KINDS) {
                    Some(known) => format!("unknown notion kind `{unknown}`; closest is `{known}`"),
                    None => format!("unknown notion kind `{unknown}`"),
                };
                return Err(self.diag_at(span, message, None));
            }
        };
        self.advance();
        Ok(kind)
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
        let mut symbols = vec![self.expect_symbol("an enum symbol")?];
        loop {
            if self.at(&TokKind::RParen) {
                self.advance();
                break;
            }
            if self.at(&TokKind::Comma) {
                self.advance();
                symbols.push(self.expect_symbol("an enum symbol")?);
                continue;
            }
            return Err(self.expected("`,` or `)` in enum symbol list"));
        }
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
            let mut set = vec![self.parse_literal("a literal")?];
            loop {
                if self.at(&TokKind::RParen) {
                    self.advance();
                    break;
                }
                if self.at(&TokKind::Comma) {
                    self.advance();
                    set.push(self.parse_literal("a literal")?);
                    continue;
                }
                return Err(self.expected("`,` or `)` in literal list"));
            }
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
            let notion = self.expect_notion_name()?;
            self.expect(&TokKind::Dot, "`.`")?;
            let attr = self.expect_field_name()?;
            return Ok(Operand::Ref(AttrRef { notion, attr }));
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

    fn path() -> RepoPath {
        RepoPath::new("telos/notions/Invoice.tel")
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
}
