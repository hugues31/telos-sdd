//! Constraints: cross-cutting rules (stack, architecture, quality, security,
//! convention) that apply globally or to a subset of intents.

use serde::Serialize;

use crate::ids::{ConstraintId, IntentId};
use crate::span::Sp;

use super::expr::Expr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConstraintKind {
    Stack,
    Architecture,
    Quality,
    Security,
    Convention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Rule {
    Text(String),
    Machine(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Scope {
    Global,
    Intents(Vec<Sp<IntentId>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Constraint {
    pub id: ConstraintId,
    pub kind: ConstraintKind,
    pub title: String,
    pub rule: Rule,
    pub scope: Scope,
    /// Shell command; opaque in M1.
    pub check: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn constraint_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ConstraintKind::Architecture).unwrap(),
            "\"architecture\""
        );
    }

    #[test]
    fn scope_global_vs_intents() {
        assert_eq!(Scope::Global, Scope::Global);
        let scoped = Scope::Intents(vec![Sp {
            node: IntentId(1),
            span: Span::default(),
        }]);
        assert_ne!(scoped, Scope::Global);
    }

    #[test]
    fn constraint_holds_a_rule_and_a_scope() {
        let c = Constraint {
            id: ConstraintId(1),
            kind: ConstraintKind::Stack,
            title: "use Rust".to_string(),
            rule: Rule::Text("the engine is written in Rust".to_string()),
            scope: Scope::Global,
            check: None,
        };
        assert_eq!(c.id, ConstraintId(1));
        assert_eq!(c.kind, ConstraintKind::Stack);
        assert_eq!(c.check, None);
    }
}
