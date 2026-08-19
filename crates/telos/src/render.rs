//! The single exit point: every command result -- success or failure, JSON
//! or human -- becomes output text and a process exit code here, and nowhere
//! else. Commands never print.
//!
//! JSON mode emits *compact* JSON (`serde_json::to_string`, one line, no
//! indentation): the envelope is machine input first, and a one-line answer
//! is what pipes, logs and `jq` want. Human mode is the terminal's: the
//! outcome's own text on success, a `error[CODE]: message` line (plus an
//! indented `hint:` line when there is one) on failure.
//!
//! Exit codes: 0 on success, 1 on any domain error. Usage errors exit 2, but
//! never reach here -- clap exits on its own before a command ever runs.

use std::process::ExitCode;

use serde_json::Value;
use telos_core::error::ErrorCode;

use crate::envelope::{CmdResult, Envelope};

/// Turns `res` into the text to print and the code to exit with.
///
/// The caller chooses the stream: JSON always goes to stdout (a consumer
/// reads one stream, whatever happened), while in human mode success goes to
/// stdout and errors to stderr. See [`crate::cli::run`].
pub fn render(command: &str, res: CmdResult, json: bool) -> (String, ExitCode) {
    if json {
        return match res {
            Ok(outcome) => (
                to_json(&Envelope::success(command, outcome)),
                ExitCode::SUCCESS,
            ),
            Err(error) => (
                to_json(&Envelope::failure(command, error)),
                ExitCode::FAILURE,
            ),
        };
    }

    match res {
        Ok(outcome) => (outcome.human, ExitCode::SUCCESS),
        Err(error) => {
            let mut text = format!("error[{}]: {}", code_name(error.code), error.message);
            if let Some(hint) = &error.hint {
                text.push_str(&format!("\n  hint: {hint}"));
            }
            (text, ExitCode::FAILURE)
        }
    }
}

/// The answer of last resort, used only if serializing an envelope fails --
/// which it cannot: an `Envelope` is `bool`s, `String`s and
/// `serde_json::Value`s, none of which can error. A constant rather than a
/// panic, so that the one thing every consumer relies on -- stdout in JSON
/// mode is a five-key envelope -- holds even in a case that cannot happen.
const SERIALIZATION_FAILURE: &str = r#"{"ok":false,"command":"","result":null,"error":{"code":"TELOS_INTERNAL","message":"failed to serialize the envelope","hint":null},"next_actions":[]}"#;

/// Serializes an envelope, compactly.
fn to_json(envelope: &Envelope) -> String {
    serde_json::to_string(envelope).unwrap_or_else(|_| SERIALIZATION_FAILURE.to_string())
}

/// The frozen `SCREAMING_SNAKE_CASE` spelling of `code`, read from the same
/// `Serialize` impl the JSON envelope uses so human and machine output can
/// never disagree about what an error is called.
fn code_name(code: ErrorCode) -> String {
    match serde_json::to_value(code) {
        Ok(Value::String(name)) => name,
        // Unreachable: `ErrorCode` is a unit-variant enum, which serde
        // always renders as a string.
        _ => format!("{code:?}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use telos_core::error::TelosError;

    use super::*;
    use crate::envelope::Outcome;

    fn outcome() -> Outcome {
        Outcome {
            result: json!({ "version": "0.7.0" }),
            human: "telos 0.7.0".to_string(),
            next_actions: Vec::new(),
        }
    }

    fn hintless() -> TelosError {
        TelosError::new(ErrorCode::TelosInternal, "boom")
    }

    /// Locks the key order (struct field order) and the compact, one-line
    /// form -- both are what a consumer diffs and pipes.
    #[test]
    fn a_success_envelope_is_one_compact_line_in_field_order() {
        let (text, _) = render("version", Ok(outcome()), true);
        assert_eq!(
            text,
            r#"{"ok":true,"command":"version","result":{"version":"0.7.0"},"error":null,"next_actions":[]}"#
        );
    }

    /// A hintless error still carries `hint`, as an explicit null: the key is
    /// never absent, so a consumer reads it without checking.
    #[test]
    fn a_hintless_error_still_carries_a_null_hint() {
        let (text, _) = render("version", Err(hintless()), true);
        assert_eq!(
            text,
            r#"{"ok":false,"command":"version","result":null,"error":{"code":"TELOS_INTERNAL","message":"boom","hint":null},"next_actions":[]}"#
        );
    }

    #[test]
    fn a_human_error_without_a_hint_is_a_single_line() {
        let (text, _) = render("version", Err(hintless()), false);
        assert_eq!(text, "error[TELOS_INTERNAL]: boom");
    }

    #[test]
    fn a_human_error_with_a_hint_indents_it_on_its_own_line() {
        let (text, _) = render("version", Err(hintless().hint("try again")), false);
        assert_eq!(text, "error[TELOS_INTERNAL]: boom\n  hint: try again");
    }

    #[test]
    fn a_human_success_is_the_outcome_text_alone() {
        let (text, _) = render("version", Ok(outcome()), false);
        assert_eq!(text, "telos 0.7.0");
    }
}
