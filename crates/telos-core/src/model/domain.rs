use serde::Serialize;

use crate::ids::{CapabilityRef, ContextId, NotionRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextKind {
    Core,
    Supporting,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Context {
    pub id: ContextId,
    pub kind: ContextKind,
    pub title: String,
    pub def: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub id: CapabilityRef,
    pub title: String,
    pub def: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotionMapping {
    pub from: NotionRef,
    pub to: NotionRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDependency {
    pub consumer: ContextId,
    pub supplier: ContextId,
    pub mappings: Vec<NotionMapping>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ContextMap {
    pub dependencies: Vec<ContextDependency>,
}
