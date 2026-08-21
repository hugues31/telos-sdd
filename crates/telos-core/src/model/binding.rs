//! Bindings: the link between spec and code -- a source file implements an
//! intent, or a test proves a scenario.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use crate::error::{ErrorCode, TelosError};
use crate::ids::{IntentId, RepoPath, ScenarioId};
use crate::span::Sp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Binding {
    Implements {
        path: RepoPath,
        intent: Sp<IntentId>,
    },
    Proves {
        test: TestRef,
        scenario: Sp<ScenarioId>,
    },
}

impl Binding {
    /// The file to seal: the implementing source file, or the file holding
    /// the proving test.
    pub fn code_path(&self) -> &RepoPath {
        match self {
            Binding::Implements { path, .. } => path,
            Binding::Proves { test, .. } => &test.path,
        }
    }
}

/// A test locator: `path` (mandatory) and an optional test `name`, joined by
/// `"::"` (e.g. `"tests/billing.rs::scn_0107"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct TestRef {
    pub path: RepoPath,
    pub name: Option<String>,
}

impl FromStr for TestRef {
    type Err = TelosError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (path, name) = match s.split_once("::") {
            Some((path, name)) => (path, Some(name.to_string())),
            None => (s, None),
        };
        if path.is_empty() {
            return Err(TelosError::new(
                ErrorCode::TelosParseError,
                format!("test reference is missing a path: `{s}`"),
            ));
        }
        Ok(TestRef {
            path: RepoPath::parse_outside_telos(path)?,
            name,
        })
    }
}

impl fmt::Display for TestRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{}::{name}", self.path),
            None => write!(f, "{}", self.path),
        }
    }
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

    fn scenario_id(n: u32) -> Sp<ScenarioId> {
        Sp {
            node: ScenarioId(n),
            span: Span::default(),
        }
    }

    #[test]
    fn implements_code_path_is_the_source_path() {
        let b = Binding::Implements {
            path: RepoPath::new("src/invoice.rs"),
            intent: intent_id(42),
        };
        assert_eq!(b.code_path(), &RepoPath::new("src/invoice.rs"));
    }

    #[test]
    fn proves_code_path_is_the_test_files_path() {
        let b = Binding::Proves {
            test: TestRef {
                path: RepoPath::new("tests/billing.rs"),
                name: Some("scn_0107".to_string()),
            },
            scenario: scenario_id(107),
        };
        assert_eq!(b.code_path(), &RepoPath::new("tests/billing.rs"));
    }

    #[test]
    fn binding_serializes_tagged_by_lowercase_kind() {
        let b = Binding::Implements {
            path: RepoPath::new("src/invoice.rs"),
            intent: intent_id(42),
        };
        assert_eq!(
            serde_json::to_string(&b).unwrap(),
            "{\"kind\":\"implements\",\"path\":\"src/invoice.rs\",\"intent\":\"INT-0042\"}"
        );
    }

    #[test]
    fn test_ref_parses_path_and_name_split_on_double_colon() {
        let parsed: TestRef = "tests/billing.rs::scn_0107".parse().unwrap();
        assert_eq!(parsed.path, RepoPath::new("tests/billing.rs"));
        assert_eq!(parsed.name.as_deref(), Some("scn_0107"));
    }

    #[test]
    fn test_ref_parses_path_only_with_no_name() {
        let parsed: TestRef = "tests/billing.rs".parse().unwrap();
        assert_eq!(parsed.path, RepoPath::new("tests/billing.rs"));
        assert_eq!(parsed.name, None);
    }

    #[test]
    fn test_ref_rejects_an_empty_path_before_the_separator() {
        assert!("::scn_0107".parse::<TestRef>().is_err());
    }

    #[test]
    fn test_ref_rejects_repository_escapes_and_spec_paths() {
        for reference in ["../outside.rs", "tests/../outside.rs", "telos/a.tel"] {
            assert!(
                reference.parse::<TestRef>().is_err(),
                "accepted {reference}"
            );
        }
    }

    #[test]
    fn test_ref_display_rejoins_path_and_name() {
        let r = TestRef {
            path: RepoPath::new("tests/billing.rs"),
            name: Some("scn_0107".to_string()),
        };
        assert_eq!(r.to_string(), "tests/billing.rs::scn_0107");
    }

    #[test]
    fn test_ref_display_without_name_is_bare_path() {
        let r = TestRef {
            path: RepoPath::new("tests/billing.rs"),
            name: None,
        };
        assert_eq!(r.to_string(), "tests/billing.rs");
    }
}
