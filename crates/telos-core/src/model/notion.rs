//! Notions: the domain nouns (actors, entities, values, events, states) a
//! spec is written about, with their attributes and relations.

use serde::Serialize;

use crate::ids::{FieldName, NotionName};
use crate::span::Sp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotionKind {
    Actor,
    Entity,
    Value,
    Event,
    State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttrType {
    String,
    Int,
    Decimal,
    Money,
    Bool,
    Date,
    Datetime,
    /// Ordered enum symbols -- the order is semantic (e.g. state progression).
    Enum(Vec<String>),
    Ref(NotionName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attr {
    pub name: FieldName,
    pub ty: AttrType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rel {
    pub name: FieldName,
    pub target: Sp<NotionName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notion {
    pub name: NotionName,
    pub kind: NotionKind,
    pub def: String,
    /// The term's surface form, used mid-sentence: `"invoice"`, `"payment is
    /// received"`. Carries no article -- consumers prepend `the `.
    pub phrase: String,
    /// Insertion order is the canonical order.
    pub attrs: Vec<Attr>,
    pub rels: Vec<Rel>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn name(s: &str) -> NotionName {
        NotionName::new(s).unwrap()
    }

    fn field(s: &str) -> FieldName {
        FieldName::new(s).unwrap()
    }

    #[test]
    fn notion_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&NotionKind::Entity).unwrap(),
            "\"entity\""
        );
        assert_eq!(
            serde_json::to_string(&NotionKind::Event).unwrap(),
            "\"event\""
        );
    }

    #[test]
    fn attr_type_enum_variant_serializes_with_ordered_symbols() {
        let ty = AttrType::Enum(vec!["draft".to_string(), "settled".to_string()]);
        assert_eq!(
            serde_json::to_string(&ty).unwrap(),
            "{\"enum\":[\"draft\",\"settled\"]}"
        );
    }

    #[test]
    fn attr_type_ref_variant_wraps_a_notion_name() {
        let ty = AttrType::Ref(name("Customer"));
        assert_eq!(ty, AttrType::Ref(name("Customer")));
    }

    #[test]
    fn notion_holds_attrs_and_rels_in_insertion_order() {
        let notion = Notion {
            name: name("Invoice"),
            kind: NotionKind::Entity,
            def: "A billing document.".to_string(),
            phrase: "invoice".to_string(),
            attrs: vec![
                Attr {
                    name: field("state"),
                    ty: AttrType::Enum(vec!["draft".to_string(), "settled".to_string()]),
                },
                Attr {
                    name: field("amount"),
                    ty: AttrType::Money,
                },
            ],
            rels: vec![Rel {
                name: field("billed-to"),
                target: Sp {
                    node: name("Customer"),
                    span: Span { start: 0, end: 8 },
                },
            }],
        };
        assert_eq!(notion.attrs[0].name, field("state"));
        assert_eq!(notion.attrs[1].name, field("amount"));
        assert_eq!(notion.rels[0].target.node, name("Customer"));
    }

    #[test]
    fn rel_serializes_target_as_bare_notion_name_dropping_span() {
        let rel = Rel {
            name: field("billed-to"),
            target: Sp {
                node: name("Customer"),
                span: Span { start: 0, end: 8 },
            },
        };
        assert_eq!(
            serde_json::to_string(&rel).unwrap(),
            "{\"name\":\"billed-to\",\"target\":\"Customer\"}"
        );
    }
}
