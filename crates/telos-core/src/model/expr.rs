//! Expressions: comparisons and boolean combinators used in `then` clauses
//! (scenarios), `if <expr>` conditions (unwanted-behavior intents), and
//! machine-checkable constraint rules.

use serde::Serialize;

use crate::ids::{FieldName, NotionName};
use crate::span::Sp;

/// A reference to a notion's attribute, e.g. `Invoice.state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttrRef {
    pub notion: Sp<NotionName>,
    pub attr: Sp<FieldName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Literal {
    Str(String),
    Int(i64),
    /// The lexeme is kept verbatim (e.g. `"120.50"`) -- never converted to a
    /// float, to avoid binary floating-point rounding of decimal amounts.
    Decimal(String),
    Bool(bool),
    /// Formats validated at lex time.
    Date(String),
    Datetime(String),
    /// A bare enum symbol (as written), resolved during the semantic pass.
    Symbol(Sp<String>),
    // No `Money` variant: a money amount is a `Str` lexeme of the form
    // `^\d+\.\d{2} [A-Z]{3}$` (e.g. "120.00 EUR"), validated by the
    // semantic pass when the target attribute's type is `money`.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Operand {
    Ref(AttrRef),
    Lit(Literal),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Expr {
    Cmp {
        op: CmpOp,
        lhs: Operand,
        rhs: Operand,
    },
    In {
        lhs: Operand,
        set: Vec<Literal>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn notion(s: &str) -> Sp<NotionName> {
        Sp {
            node: NotionName::new(s).unwrap(),
            span: Span::default(),
        }
    }

    fn field(s: &str) -> Sp<FieldName> {
        Sp {
            node: FieldName::new(s).unwrap(),
            span: Span::default(),
        }
    }

    #[test]
    fn literal_decimal_keeps_the_lexeme_verbatim() {
        let l = Literal::Decimal("120.50".to_string());
        assert_eq!(l, Literal::Decimal("120.50".to_string()));
    }

    #[test]
    fn literal_serializes_tagged_by_lowercase_variant_name() {
        assert_eq!(
            serde_json::to_string(&Literal::Bool(true)).unwrap(),
            "{\"bool\":true}"
        );
        assert_eq!(
            serde_json::to_string(&Literal::Int(42)).unwrap(),
            "{\"int\":42}"
        );
    }

    #[test]
    fn expr_cmp_builds_a_comparison_between_a_ref_and_a_literal() {
        let expr = Expr::Cmp {
            op: CmpOp::Eq,
            lhs: Operand::Ref(AttrRef {
                notion: notion("Invoice"),
                attr: field("state"),
            }),
            rhs: Operand::Lit(Literal::Symbol(Sp {
                node: "settled".to_string(),
                span: Span::default(),
            })),
        };
        match expr {
            Expr::Cmp { op, .. } => assert_eq!(op, CmpOp::Eq),
            _ => panic!("expected Expr::Cmp"),
        }
    }

    #[test]
    fn expr_and_or_not_combine_boxed_sub_expressions() {
        let a = Expr::Cmp {
            op: CmpOp::Eq,
            lhs: Operand::Lit(Literal::Bool(true)),
            rhs: Operand::Lit(Literal::Bool(true)),
        };
        let b = a.clone();
        let combined = Expr::Not(Box::new(Expr::And(Box::new(a), Box::new(b))));
        assert!(matches!(combined, Expr::Not(_)));
    }

    #[test]
    fn attr_ref_serializes_notion_and_attr_as_bare_strings() {
        let r = AttrRef {
            notion: notion("Invoice"),
            attr: field("state"),
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            "{\"notion\":\"Invoice\",\"attr\":\"state\"}"
        );
    }
}
