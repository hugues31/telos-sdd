//! Intents: the goals a spec commits to, expressed as a `Statement`
//! (ubiquitous / event-driven / state-driven / unwanted / optional
//! template) plus relations to other intents and the scenarios that verify
//! it.

use serde::Serialize;

use crate::ids::{FieldName, IntentId, NotionName};
use crate::span::Sp;

use super::expr::{AttrRef, Expr, Literal};
use super::scenario::Scenario;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentStatus {
    Draft,
    Active,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "template", rename_all = "kebab-case")]
pub enum Statement {
    Ubiquitous {
        action: Action,
    },
    EventDriven {
        event: Sp<NotionName>,
        on: Option<Sp<NotionName>>,
        action: Action,
    },
    /// `while X.y == v`
    StateDriven {
        subject: AttrRef,
        value: Literal,
        action: Action,
    },
    /// `if <expr>`
    Unwanted {
        condition: Expr,
        action: Action,
    },
    /// `where <flag>`
    Optional {
        feature: FieldName,
        action: Action,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Action {
    /// `set Invoice.state = settled`
    Set { target: AttrRef, value: Literal },
    /// `system shall "notify the auditor"`
    Free(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Intent {
    pub id: IntentId,
    pub title: String,
    pub status: IntentStatus,
    pub telos: String,
    pub statement: Statement,
    pub refines: Vec<Sp<IntentId>>,
    pub requires: Vec<Sp<IntentId>>,
    /// Sorted by id in canonical form.
    pub excludes: Vec<Sp<IntentId>>,
    /// Sorted by id.
    pub scenarios: Vec<Scenario>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn intent_id(n: u32) -> Sp<IntentId> {
        Sp {
            node: IntentId(n),
            span: Span::default(),
        }
    }

    #[test]
    fn statement_ubiquitous_serializes_with_kebab_case_template_tag() {
        let s = Statement::Ubiquitous {
            action: Action::Free("notify the auditor".to_string()),
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            "{\"template\":\"ubiquitous\",\"action\":{\"Free\":\"notify the auditor\"}}"
        );
    }

    #[test]
    fn statement_event_driven_serializes_with_kebab_case_variant_name() {
        let s = Statement::EventDriven {
            event: Sp {
                node: NotionName::new("Paid").unwrap(),
                span: Span::default(),
            },
            on: None,
            action: Action::Free("archive".to_string()),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"template\":\"event-driven\""));
    }

    #[test]
    fn intent_orders_refines_requires_excludes_and_scenarios() {
        let intent = Intent {
            id: IntentId(42),
            title: "Invoices settle once paid".to_string(),
            status: IntentStatus::Active,
            telos: "customers trust the ledger".to_string(),
            statement: Statement::Ubiquitous {
                action: Action::Free("track invoices".to_string()),
            },
            refines: vec![intent_id(1)],
            requires: vec![intent_id(2), intent_id(3)],
            excludes: vec![],
            scenarios: vec![],
        };
        assert_eq!(intent.id, IntentId(42));
        assert_eq!(intent.requires.len(), 2);
        assert_eq!(intent.requires[0].node, IntentId(2));
    }
}
