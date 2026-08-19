//! Scenarios: concrete given/when/then examples that verify an intent.

use serde::Serialize;

use crate::ids::{FieldName, NotionName, ScenarioId};
use crate::span::Sp;

use super::expr::{Expr, Literal};

/// One instance line: a notion together with the field values that
/// characterize this particular instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceStep {
    pub notion: Sp<NotionName>,
    pub fields: Vec<(Sp<FieldName>, Literal)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scenario {
    pub id: ScenarioId,
    pub title: String,
    /// At least 1; multiple lines combine as `And`.
    pub given: Vec<InstanceStep>,
    /// Exactly 1; its notion must be `kind = event`.
    pub when: InstanceStep,
    /// At least 1.
    pub then: Vec<Expr>,
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
    fn scenario_holds_given_when_then() {
        let scenario = Scenario {
            id: ScenarioId(107),
            title: "invoice is settled".to_string(),
            given: vec![InstanceStep {
                notion: notion("Invoice"),
                fields: vec![(field("state"), Literal::Str("draft".to_string()))],
            }],
            when: InstanceStep {
                notion: notion("Paid"),
                fields: vec![],
            },
            then: vec![],
        };
        assert_eq!(scenario.id, ScenarioId(107));
        assert_eq!(scenario.given.len(), 1);
        assert_eq!(scenario.when.notion.node, NotionName::new("Paid").unwrap());
    }

    #[test]
    fn instance_step_serializes_fields_as_name_value_pairs() {
        let step = InstanceStep {
            notion: notion("Invoice"),
            fields: vec![(field("state"), Literal::Str("draft".to_string()))],
        };
        assert_eq!(
            serde_json::to_string(&step).unwrap(),
            "{\"notion\":\"Invoice\",\"fields\":[[\"state\",{\"str\":\"draft\"}]]}"
        );
    }
}
