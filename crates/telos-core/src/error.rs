//! Frozen error contract: `ErrorCode` is stable API for agent tooling and
//! must never have its serialized variant names changed once published;
//! `TelosError` and `Diagnostic` are the two error representations built
//! on top of it.

use serde::Serialize;

use crate::ids::RepoPath;

/// A stable, machine-readable error code.
///
/// Serializes to `SCREAMING_SNAKE_CASE` (e.g. `TelosDriftDetected` becomes
/// `"TELOS_DRIFT_DETECTED"`). This is a frozen external contract consumed by
/// agent tooling: variants are never renamed or removed, only added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // Frozen by spec §8 (present from M1 even if only emitted in M2+).
    TelosDriftDetected,
    TelosApprovalStale,
    TelosReferenceUnknown,
    TelosScenarioRedExpected,
    TelosTestSealed,
    TelosOrphanCode,
    TelosConstraintFailed,
    TelosChangeStateInvalid,
    TelosFileClaimed,
    // M1 extensions (frozen in turn as of their publication).
    /// No `telos/` directory or no `telos.lock`.
    TelosNotInitialized,
    /// `init` run on an already-initialized project.
    TelosAlreadyInitialized,
    /// `.tel` file is syntactically invalid.
    TelosParseError,
    /// Rule §3.3 violated (other than unknown reference / cycle).
    TelosIntegrityViolation,
    /// Cycle detected on `requires`/`refines`.
    TelosCycleDetected,
    /// git absent, not a repo, or a plumbing command failed.
    TelosGitError,
    /// Internal invariant broken (a bug).
    TelosInternal,
    // M3 extension.
    /// No test runner configured, or `telos test`/`witness_verdict`'s test
    /// discovery (D4) found zero or more than one candidate.
    TelosTestNotFound,
}

/// A non-localized engine error: a code, a human-readable message, and an
/// optional actionable hint.
#[derive(Debug, Clone)]
pub struct TelosError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,
}

impl TelosError {
    /// Builds a new error with no hint.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    /// Attaches a hint, consuming and returning `self` for chaining.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// A localized finding produced by a check (a checker can emit several;
/// the error envelope surfaces only the first one).
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,
    pub file: Option<RepoPath>,
    /// 1-indexed.
    pub line: Option<u32>,
    /// 1-indexed.
    pub col: Option<u32>,
}

impl From<Diagnostic> for TelosError {
    /// Keeps `code`/`message`/`hint`, prefixing `message` with the
    /// available position information (`"file:line:col: "`, degrading
    /// gracefully as fewer position fields are present).
    fn from(diag: Diagnostic) -> Self {
        let message = match (&diag.file, diag.line, diag.col) {
            (Some(file), Some(line), Some(col)) => format!("{file}:{line}:{col}: {}", diag.message),
            (Some(file), Some(line), None) => format!("{file}:{line}: {}", diag.message),
            (Some(file), None, _) => format!("{file}: {}", diag.message),
            (None, _, _) => diag.message,
        };
        TelosError {
            code: diag.code,
            message,
            hint: diag.hint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_serialization_is_frozen() -> Result<(), serde_json::Error> {
        // One assertion per variant -- this list IS the freeze. Annex B's
        // `error.rs` enumerates 17 identifiers (9 codes frozen by spec §8 +
        // 7 M1 extensions + 1 M3 extension); every one of them is checked
        // here.
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosDriftDetected)?,
            "\"TELOS_DRIFT_DETECTED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosApprovalStale)?,
            "\"TELOS_APPROVAL_STALE\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosReferenceUnknown)?,
            "\"TELOS_REFERENCE_UNKNOWN\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosScenarioRedExpected)?,
            "\"TELOS_SCENARIO_RED_EXPECTED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosTestSealed)?,
            "\"TELOS_TEST_SEALED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosOrphanCode)?,
            "\"TELOS_ORPHAN_CODE\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosConstraintFailed)?,
            "\"TELOS_CONSTRAINT_FAILED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosChangeStateInvalid)?,
            "\"TELOS_CHANGE_STATE_INVALID\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosFileClaimed)?,
            "\"TELOS_FILE_CLAIMED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosNotInitialized)?,
            "\"TELOS_NOT_INITIALIZED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosAlreadyInitialized)?,
            "\"TELOS_ALREADY_INITIALIZED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosParseError)?,
            "\"TELOS_PARSE_ERROR\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosIntegrityViolation)?,
            "\"TELOS_INTEGRITY_VIOLATION\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosCycleDetected)?,
            "\"TELOS_CYCLE_DETECTED\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosGitError)?,
            "\"TELOS_GIT_ERROR\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosInternal)?,
            "\"TELOS_INTERNAL\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::TelosTestNotFound)?,
            "\"TELOS_TEST_NOT_FOUND\""
        );
        Ok(())
    }

    #[test]
    fn telos_error_new_has_no_hint() {
        let err = TelosError::new(ErrorCode::TelosInternal, "boom");
        assert_eq!(err.code, ErrorCode::TelosInternal);
        assert_eq!(err.message, "boom");
        assert_eq!(err.hint, None);
    }

    #[test]
    fn telos_error_hint_builder_sets_hint() {
        let err = TelosError::new(ErrorCode::TelosNotInitialized, "no telos/ dir")
            .hint("run `telos init` at the repository root");
        assert_eq!(
            err.hint.as_deref(),
            Some("run `telos init` at the repository root")
        );
    }

    #[test]
    fn diagnostic_into_telos_error_prefixes_file_line_col() {
        let diag = Diagnostic {
            code: ErrorCode::TelosParseError,
            message: "unexpected token".to_string(),
            hint: None,
            file: Some(RepoPath::new("telos/notions/Invoice.tel")),
            line: Some(3),
            col: Some(7),
        };
        let err: TelosError = diag.into();
        assert_eq!(err.code, ErrorCode::TelosParseError);
        assert_eq!(
            err.message,
            "telos/notions/Invoice.tel:3:7: unexpected token"
        );
    }

    #[test]
    fn diagnostic_into_telos_error_without_position_keeps_message_bare() {
        let diag = Diagnostic {
            code: ErrorCode::TelosInternal,
            message: "no position available".to_string(),
            hint: Some("check the logs".to_string()),
            file: None,
            line: None,
            col: None,
        };
        let err: TelosError = diag.into();
        assert_eq!(err.message, "no position available");
        assert_eq!(err.hint.as_deref(), Some("check the logs"));
    }

    #[test]
    fn diagnostic_into_telos_error_degrades_without_col() {
        let diag = Diagnostic {
            code: ErrorCode::TelosParseError,
            message: "bad line".to_string(),
            hint: None,
            file: Some(RepoPath::new("telos/telos.toml")),
            line: Some(9),
            col: None,
        };
        let err: TelosError = diag.into();
        assert_eq!(err.message, "telos/telos.toml:9: bad line");
    }

    #[test]
    fn diagnostic_serializes_with_frozen_error_code() {
        let diag = Diagnostic {
            code: ErrorCode::TelosCycleDetected,
            message: "cycle".to_string(),
            hint: None,
            file: None,
            line: None,
            col: None,
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"code\":\"TELOS_CYCLE_DETECTED\""));
    }
}
