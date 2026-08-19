//! Typed identifiers shared across the engine.
//!
//! Only `RepoPath` is defined here for now, because `error::Diagnostic`
//! needs it. The rest of this module (`IntentId`, `ScenarioId`,
//! `ConstraintId`, `ChangeId`, `NotionName`, `FieldName`, `EntityRef`, ...)
//! is Task 3's responsibility, per Annex B's `ids.rs` section.

use std::fmt;

use serde::Serialize;

/// Repo-relative path. The separator is always `/`; conversion to/from the
/// host OS path representation happens only at I/O boundaries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    /// Builds a `RepoPath` from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Borrows the underlying path as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_as_str_round_trip() {
        let path = RepoPath::new("telos/notions/Invoice.tel");
        assert_eq!(path.as_str(), "telos/notions/Invoice.tel");
    }

    #[test]
    fn display_matches_as_str() {
        let path = RepoPath::new("telos/telos.toml");
        assert_eq!(path.to_string(), "telos/telos.toml");
    }

    #[test]
    fn serializes_as_a_plain_json_string() {
        let path = RepoPath::new("telos/telos.toml");
        assert_eq!(
            serde_json::to_string(&path).unwrap(),
            "\"telos/telos.toml\""
        );
    }
}
