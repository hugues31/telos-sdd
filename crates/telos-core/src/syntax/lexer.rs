//! Lexer for the `.tel` textual syntax.
//!
//! Produces a flat token stream for the recursive-descent parser. Keywords are
//! not reserved and always lex as `LowerIdent` (the parser matches them
//! contextually); a dash inside an
//! identifier continues it only when followed by an alphanumeric (`a->b`
//! lexes as `a`, `->`, `b`; `issued-to` lexes as one identifier). On the
//! first lexical error, `lex` stops and returns the `Diagnostic` --
//! recovery is the parser's job, not the lexer's.

use std::str::FromStr;

use crate::error::{Diagnostic, ErrorCode};
use crate::ids::EntityRef;
use crate::span::{Span, line_col};

/// A lexed token: its kind plus the byte-offset span it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: TokKind,
    pub span: Span,
}

/// The kind of a lexed token. Identifiers and literals carry
/// their lexeme; `Decimal`/`Date`/`Datetime` keep the source text verbatim
/// rather than parsing to a numeric/temporal type so re-emission preserves
/// the original lexeme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokKind {
    UpperIdent(String),
    LowerIdent(String),
    IdLit(EntityRef),
    Str(String),
    Int(i64),
    Decimal(String),
    Date(String),
    Datetime(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Colon,
    Dot,
    Arrow,
    Assign,
    EqEq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
    Newline,
    Eof,
}

/// Lexes `src` into a flat token stream, terminated by `Eof`.
///
/// Stops and returns the first `Diagnostic` encountered; error recovery
/// (skipping to a sync point and continuing) is the parser's job, not the
/// lexer's.
pub(crate) fn lex(src: &str) -> Result<Vec<Token>, Diagnostic> {
    Lexer::new(src).tokenize()
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            chars: src.char_indices().collect(),
            pos: 0,
        }
    }

    // --- cursor primitives ---

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).map(|&(_, c)| c)
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).map(|&(_, c)| c)
    }

    fn is_digit_at(&self, n: usize) -> bool {
        matches!(self.peek_at(n), Some(c) if c.is_ascii_digit())
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// The byte offset of the cursor: the start of the next unread char, or
    /// `src.len()` at end of input.
    fn byte_pos(&self) -> u32 {
        self.chars
            .get(self.pos)
            .map_or(self.src.len() as u32, |&(i, _)| i as u32)
    }

    fn slice(&self, start: u32, end: u32) -> &'a str {
        &self.src[start as usize..end as usize]
    }

    fn make_span(&self, start: u32) -> Span {
        Span {
            start,
            end: self.byte_pos(),
        }
    }

    fn diag(&self, code: ErrorCode, message: String, span: Span) -> Diagnostic {
        let (line, col) = line_col(self.src, span.start);
        Diagnostic {
            code,
            message,
            hint: None,
            file: None,
            line: Some(line),
            col: Some(col),
        }
    }

    fn skip_inline_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\r')) {
            self.advance();
        }
    }

    // --- driver ---

    fn tokenize(&mut self) -> Result<Vec<Token>, Diagnostic> {
        let mut tokens = Vec::new();
        loop {
            self.skip_inline_whitespace();
            let start = self.byte_pos();
            let Some(c) = self.peek() else {
                tokens.push(Token {
                    kind: TokKind::Eof,
                    span: Span { start, end: start },
                });
                return Ok(tokens);
            };
            let token = match c {
                '\n' => {
                    self.advance();
                    Token {
                        kind: TokKind::Newline,
                        span: self.make_span(start),
                    }
                }
                '"' => self.lex_string(start)?,
                '{' => self.punct(TokKind::LBrace, start),
                '}' => self.punct(TokKind::RBrace, start),
                '(' => self.punct(TokKind::LParen, start),
                ')' => self.punct(TokKind::RParen, start),
                ',' => self.punct(TokKind::Comma, start),
                ':' => self.punct(TokKind::Colon, start),
                '.' => self.punct(TokKind::Dot, start),
                '=' => self.lex_eq(start),
                '!' => self.lex_bang(start)?,
                '<' => self.lex_lt(start),
                '>' => self.lex_gt(start),
                '-' => self.lex_dash(start)?,
                c if c.is_ascii_uppercase() => self.lex_upper(start)?,
                c if c.is_ascii_lowercase() => self.lex_lower(start),
                c if c.is_ascii_digit() => self.lex_number(start)?,
                other => {
                    return Err(self.diag(
                        ErrorCode::TelosParseError,
                        format!("unexpected character `{other}`"),
                        Span {
                            start,
                            end: start + other.len_utf8() as u32,
                        },
                    ));
                }
            };
            tokens.push(token);
        }
    }

    // --- punctuation ---

    fn punct(&mut self, kind: TokKind, start: u32) -> Token {
        self.advance();
        Token {
            kind,
            span: self.make_span(start),
        }
    }

    fn lex_eq(&mut self, start: u32) -> Token {
        self.advance();
        let kind = if self.peek() == Some('=') {
            self.advance();
            TokKind::EqEq
        } else {
            TokKind::Assign
        };
        Token {
            kind,
            span: self.make_span(start),
        }
    }

    fn lex_bang(&mut self, start: u32) -> Result<Token, Diagnostic> {
        self.advance();
        if self.peek() == Some('=') {
            self.advance();
            Ok(Token {
                kind: TokKind::Ne,
                span: self.make_span(start),
            })
        } else {
            Err(self.diag(
                ErrorCode::TelosParseError,
                "unexpected character `!` (expected `!=`)".to_string(),
                Span {
                    start,
                    end: start + 1,
                },
            ))
        }
    }

    fn lex_lt(&mut self, start: u32) -> Token {
        self.advance();
        let kind = if self.peek() == Some('=') {
            self.advance();
            TokKind::Le
        } else {
            TokKind::Lt
        };
        Token {
            kind,
            span: self.make_span(start),
        }
    }

    fn lex_gt(&mut self, start: u32) -> Token {
        self.advance();
        let kind = if self.peek() == Some('=') {
            self.advance();
            TokKind::Ge
        } else {
            TokKind::Gt
        };
        Token {
            kind,
            span: self.make_span(start),
        }
    }

    /// A `-` is either the start of `->` (arrow), the sign of a negative
    /// number (`-` immediately followed by a digit), or -- since standalone
    /// `-` is not a punctuation token -- a lexical error.
    fn lex_dash(&mut self, start: u32) -> Result<Token, Diagnostic> {
        if self.peek_at(1) == Some('>') {
            self.advance();
            self.advance();
            Ok(Token {
                kind: TokKind::Arrow,
                span: self.make_span(start),
            })
        } else if self.is_digit_at(1) {
            self.lex_number(start)
        } else {
            Err(self.diag(
                ErrorCode::TelosParseError,
                "unexpected character `-` (expected `->` or a negative number)".to_string(),
                Span {
                    start,
                    end: start + 1,
                },
            ))
        }
    }

    // --- strings ---

    fn lex_string(&mut self, start: u32) -> Result<Token, Diagnostic> {
        self.advance(); // opening quote
        let mut buf = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(self.diag(
                        ErrorCode::TelosParseError,
                        "unterminated string literal".to_string(),
                        Span {
                            start,
                            end: start + 1,
                        },
                    ));
                }
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\n') => {
                    let at = self.byte_pos();
                    return Err(self.diag(
                        ErrorCode::TelosParseError,
                        "newline in string literal".to_string(),
                        Span {
                            start: at,
                            end: at + 1,
                        },
                    ));
                }
                Some('\\') => {
                    let esc_start = self.byte_pos();
                    self.advance();
                    match self.peek() {
                        Some('"') => {
                            buf.push('"');
                            self.advance();
                        }
                        Some('\\') => {
                            buf.push('\\');
                            self.advance();
                        }
                        _ => {
                            return Err(self.diag(
                                ErrorCode::TelosParseError,
                                "invalid escape sequence (only `\\\"` and `\\\\` are allowed)"
                                    .to_string(),
                                Span {
                                    start: esc_start,
                                    end: esc_start + 1,
                                },
                            ));
                        }
                    }
                }
                Some(other) => {
                    buf.push(other);
                    self.advance();
                }
            }
        }
        Ok(Token {
            kind: TokKind::Str(buf),
            span: self.make_span(start),
        })
    }

    // --- identifiers and id literals ---

    /// `lower-ident = LOWER, {LOWER|DIGIT}, { "-", (LOWER|DIGIT), {LOWER|DIGIT} }`.
    /// The dash-continuation loop implements identifier disambiguation:
    /// a dash continues the identifier only when followed by an
    /// alphanumeric, so `issued-to` is one identifier but `a->b` is not.
    fn lex_lower(&mut self, start: u32) -> Token {
        self.advance();
        while matches!(self.peek(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            self.advance();
        }
        while self.peek() == Some('-')
            && matches!(self.peek_at(1), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            self.advance(); // '-'
            self.advance(); // first char of the segment (checked above)
            while matches!(self.peek(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                self.advance();
            }
        }
        let span = self.make_span(start);
        Token {
            kind: TokKind::LowerIdent(self.slice(start, span.end).to_string()),
            span,
        }
    }

    /// `upper-ident = UPPER, {ALPHA|DIGIT}` -- no dash extension, unlike
    /// `lower-ident`. When the base run is exactly one of the four id
    /// prefixes (`INT`/`SCN`/`CON`/`CHG`) and is immediately followed by
    /// `-`, this instead attempts an id literal (`INT-0042` etc.).
    fn lex_upper(&mut self, start: u32) -> Result<Token, Diagnostic> {
        self.advance();
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c.is_ascii_digit()) {
            self.advance();
        }
        let base_end = self.byte_pos();
        let base = self.slice(start, base_end);

        if self.peek() == Some('-') && matches!(base, "INT" | "SCN" | "CON" | "CHG") {
            return self.lex_id_lit(start);
        }

        Ok(Token {
            kind: TokKind::UpperIdent(base.to_string()),
            span: Span {
                start,
                end: base_end,
            },
        })
    }

    /// Consumes `-` and a run of digits after a confirmed `INT`/`SCN`/
    /// `CON`/`CHG` base, then hands the full run to `EntityRef::from_str`.
    ///
    /// Design decision (malformed id runs, e.g. `INT-42` with fewer than
    /// the 4 digits `digit4plus` requires): `upper-ident` cannot contain
    /// `-`, and standalone `-` is not in the punctuation table, so there
    /// is no valid token split for a run like `INT-42` -- it becomes a
    /// `TelosParseError` diagnostic (reusing `EntityRef`/the typed id's
    /// `FromStr` error, which already names the expected `PREFIX-NNNN`
    /// form) rather than being silently re-lexed as separate tokens.
    fn lex_id_lit(&mut self, start: u32) -> Result<Token, Diagnostic> {
        self.advance(); // '-'
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.advance();
        }
        let span = self.make_span(start);
        let lexeme = self.slice(start, span.end);
        match EntityRef::from_str(lexeme) {
            Ok(entity_ref) => Ok(Token {
                kind: TokKind::IdLit(entity_ref),
                span,
            }),
            Err(err) => Err(self.diag(err.code, err.message, span)),
        }
    }

    // --- numbers, dates, datetimes ---

    /// Handles `int-lit`, `decimal-lit`, `date-lit`, and `datetime-lit`
    /// (all digit-starting literals), including an optional
    /// leading `-` sign for `int-lit`/`decimal-lit`.
    ///
    /// Lookahead: a run of exactly 4 digits followed by `-DD-DD` is a date
    /// (extended to a datetime if `THH:MM:SS` and an optional `Z` follow);
    /// dates never carry a sign. Otherwise the digit run is `Int`, unless
    /// followed by `.` and at least one digit, which makes it `Decimal`
    /// (lexeme kept verbatim for canonical re-emission).
    fn lex_number(&mut self, start: u32) -> Result<Token, Diagnostic> {
        let negative = self.peek() == Some('-');
        if negative {
            self.advance();
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.advance();
        }
        let int_digits = self.pos - digits_start;

        if !negative && int_digits == 4 && self.looks_like_date_tail() {
            for _ in 0..6 {
                self.advance();
            }
            if self.looks_like_datetime_tail() {
                for _ in 0..9 {
                    self.advance();
                }
                if self.peek() == Some('Z') {
                    self.advance();
                }
                let span = self.make_span(start);
                return Ok(Token {
                    kind: TokKind::Datetime(self.slice(start, span.end).to_string()),
                    span,
                });
            }
            let span = self.make_span(start);
            return Ok(Token {
                kind: TokKind::Date(self.slice(start, span.end).to_string()),
                span,
            });
        }

        if self.peek() == Some('.') && self.is_digit_at(1) {
            self.advance(); // '.'
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.advance();
            }
            let span = self.make_span(start);
            return Ok(Token {
                kind: TokKind::Decimal(self.slice(start, span.end).to_string()),
                span,
            });
        }

        let span = self.make_span(start);
        let lexeme = self.slice(start, span.end);
        let value: i64 = lexeme.parse().map_err(|_| {
            self.diag(
                ErrorCode::TelosParseError,
                format!("integer literal out of range: `{lexeme}`"),
                span,
            )
        })?;
        Ok(Token {
            kind: TokKind::Int(value),
            span,
        })
    }

    /// Peeks (without consuming) whether the cursor sits right before
    /// `-DD-DD` -- the month/day tail of a `date-lit`, assuming a 4-digit
    /// year was just consumed.
    fn looks_like_date_tail(&self) -> bool {
        self.peek() == Some('-')
            && self.is_digit_at(1)
            && self.is_digit_at(2)
            && self.peek_at(3) == Some('-')
            && self.is_digit_at(4)
            && self.is_digit_at(5)
    }

    /// Peeks (without consuming) whether the cursor sits right before
    /// `THH:MM:SS` -- the time tail of a `datetime-lit`, assuming a
    /// `date-lit` was just consumed.
    fn looks_like_datetime_tail(&self) -> bool {
        self.peek() == Some('T')
            && self.is_digit_at(1)
            && self.is_digit_at(2)
            && self.peek_at(3) == Some(':')
            && self.is_digit_at(4)
            && self.is_digit_at(5)
            && self.peek_at(6) == Some(':')
            && self.is_digit_at(7)
            && self.is_digit_at(8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::ids::{EntityRef, IntentId};

    #[test]
    fn dash_disambiguates_arrow_from_kebab_ident() {
        let tokens = lex("rel  issued-to -> Customer").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LowerIdent("rel".to_string()),
                TokKind::LowerIdent("issued-to".to_string()),
                TokKind::Arrow,
                TokKind::UpperIdent("Customer".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn comparison_operators_and_zero_lex_correctly() {
        let tokens = lex("balance >= 0 and state == open").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&TokKind::Ge));
        assert!(kinds.contains(&TokKind::Int(0)));
        assert!(kinds.contains(&TokKind::EqEq));
    }

    #[test]
    fn plain_string_literal() {
        let tokens = lex("\"120.00 EUR\"").unwrap();
        assert_eq!(tokens[0].kind, TokKind::Str("120.00 EUR".to_string()));
    }

    #[test]
    fn string_literal_with_escaped_quotes() {
        let tokens = lex("def \"a \\\"b\\\"\"").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LowerIdent("def".to_string()),
                TokKind::Str("a \"b\"".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_string_errors_with_span_on_opening_quote() {
        let err = lex("\"unterminated").unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert_eq!(err.line, Some(1));
        assert_eq!(err.col, Some(1));
    }

    #[test]
    fn date_literal() {
        let tokens = lex("2026-08-19").unwrap();
        assert_eq!(tokens[0].kind, TokKind::Date("2026-08-19".to_string()));
        assert_eq!(tokens[1].kind, TokKind::Eof);
    }

    #[test]
    fn datetime_literal() {
        let tokens = lex("2026-08-19T12:00:00Z").unwrap();
        assert_eq!(
            tokens[0].kind,
            TokKind::Datetime("2026-08-19T12:00:00Z".to_string())
        );
    }

    #[test]
    fn intent_id_literal() {
        let tokens = lex("INT-0042").unwrap();
        assert_eq!(
            tokens[0].kind,
            TokKind::IdLit(EntityRef::Intent(IntentId(42)))
        );
    }

    #[test]
    fn newline_is_emitted_as_a_field_separator() {
        let tokens = lex("a\nb").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LowerIdent("a".to_string()),
                TokKind::Newline,
                TokKind::LowerIdent("b".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn consecutive_blank_lines_emit_one_newline_each_not_collapsed() {
        let tokens = lex("a\n\nb").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LowerIdent("a".to_string()),
                TokKind::Newline,
                TokKind::Newline,
                TokKind::LowerIdent("b".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn span_is_the_exact_byte_range_of_the_token() {
        let tokens = lex("  Invoice").unwrap();
        assert_eq!(tokens[0].kind, TokKind::UpperIdent("Invoice".to_string()));
        assert_eq!(tokens[0].span, Span { start: 2, end: 9 });
    }

    #[test]
    fn negative_decimal_literal() {
        let tokens = lex("-3.14").unwrap();
        assert_eq!(tokens[0].kind, TokKind::Decimal("-3.14".to_string()));
    }

    #[test]
    fn negative_int_literal() {
        let tokens = lex("-3").unwrap();
        assert_eq!(tokens[0].kind, TokKind::Int(-3));
    }

    #[test]
    fn plain_int_and_decimal_lexemes() {
        let tokens = lex("120 120.50").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::Int(120),
                TokKind::Decimal("120.50".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn malformed_id_literal_is_a_diagnostic_not_a_token_split() {
        // `INT-42` has only 2 digits (< 4 required by `digit4plus`). An
        // `upper-ident` cannot contain `-`, so there is no valid
        // token split for the trailing `-42` either (no bare `-` in the
        // punct table) -- this must be a diagnostic, not silently
        // re-lexed as separate tokens.
        let err = lex("INT-42").unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert!(
            err.message.contains("INT-42"),
            "message should name the offending input: {}",
            err.message
        );
    }

    #[test]
    fn all_punctuation_tokens() {
        let tokens = lex("{ } ( ) , : . = == != <= >= < >").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LBrace,
                TokKind::RBrace,
                TokKind::LParen,
                TokKind::RParen,
                TokKind::Comma,
                TokKind::Colon,
                TokKind::Dot,
                TokKind::Assign,
                TokKind::EqEq,
                TokKind::Ne,
                TokKind::Le,
                TokKind::Ge,
                TokKind::Lt,
                TokKind::Gt,
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn attr_ref_dot_notation() {
        let tokens = lex("Invoice.total").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::UpperIdent("Invoice".to_string()),
                TokKind::Dot,
                TokKind::LowerIdent("total".to_string()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn backslash_escape_is_preserved() {
        let tokens = lex("\"a\\\\b\"").unwrap();
        assert_eq!(tokens[0].kind, TokKind::Str("a\\b".to_string()));
    }

    #[test]
    fn invalid_escape_sequence_is_an_error() {
        let err = lex("\"a\\nb\"").unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
    }

    #[test]
    fn newline_inside_string_is_an_error() {
        let err = lex("\"a\nb\"").unwrap_err();
        assert_eq!(err.code, ErrorCode::TelosParseError);
    }

    #[test]
    fn other_id_kinds_lex_correctly() {
        use crate::ids::{ChangeId, ConstraintId, ScenarioId};
        assert_eq!(
            lex("SCN-0107").unwrap()[0].kind,
            TokKind::IdLit(EntityRef::Scenario(ScenarioId(107)))
        );
        assert_eq!(
            lex("CON-0003").unwrap()[0].kind,
            TokKind::IdLit(EntityRef::Constraint(ConstraintId(3)))
        );
        assert_eq!(
            lex("CHG-0007").unwrap()[0].kind,
            TokKind::IdLit(EntityRef::Change(ChangeId(7)))
        );
    }

    #[test]
    fn keywords_are_not_reserved_and_lex_as_lower_ident() {
        let tokens = lex("notion attr when true false").unwrap();
        let kinds: Vec<TokKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokKind::LowerIdent("notion".to_string()),
                TokKind::LowerIdent("attr".to_string()),
                TokKind::LowerIdent("when".to_string()),
                TokKind::LowerIdent("true".to_string()),
                TokKind::LowerIdent("false".to_string()),
                TokKind::Eof,
            ]
        );
    }
}
