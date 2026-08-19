//! The `--json` envelope (Annex B): the single shape every command answers
//! with, success or failure.
//!
//! The contract is frozen, and the freeze is about *absence* as much as
//! presence: all five keys are serialized every time -- `result` is
//! explicitly `null` on failure, `error` explicitly `null` on success, and
//! `hint` explicitly `null` when the error has none. No
//! `skip_serializing_if` anywhere. A consumer can therefore index every key
//! unconditionally instead of branching on whether it exists, which is the
//! whole point for the agent tooling that reads this blind.
//!
//! Key order in the output is struct field order (`serde_json` preserves
//! it), so the envelope is also stable enough to diff.

use serde::Serialize;
use serde_json::Value;

use telos_core::error::{ErrorCode, TelosError};

/// The JSON answer to one command invocation.
#[derive(Debug, Serialize)]
pub struct Envelope {
    pub ok: bool,
    /// The invoked command, e.g. `"init"`.
    pub command: String,
    /// The command's payload; `null` when `ok` is false.
    pub result: Option<Value>,
    /// The failure; `null` when `ok` is true.
    pub error: Option<ErrorBody>,
    /// Suggested follow-up invocations, e.g. `["telos status"]`.
    pub next_actions: Vec<String>,
}

/// The serialized form of a [`TelosError`].
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    /// `null` rather than absent when the error carries no hint.
    pub hint: Option<String>,
}

impl Envelope {
    /// The envelope for a command that succeeded.
    pub fn success(command: &str, outcome: Outcome) -> Self {
        Envelope {
            ok: true,
            command: command.to_string(),
            result: Some(outcome.result),
            error: None,
            next_actions: outcome.next_actions,
        }
    }

    /// The envelope for a command that failed. A failure suggests no
    /// follow-up: `next_actions` is empty, never absent.
    pub fn failure(command: &str, error: TelosError) -> Self {
        Envelope {
            ok: false,
            command: command.to_string(),
            result: None,
            error: Some(ErrorBody {
                code: error.code,
                message: error.message,
                hint: error.hint,
            }),
            next_actions: Vec::new(),
        }
    }
}

/// What a command returns when it succeeds: the same information twice, once
/// for machines (`result`) and once for humans (`human`), plus what to
/// suggest doing next.
#[derive(Debug)]
pub struct Outcome {
    pub result: Value,
    /// The human-mode text, without its trailing newline.
    pub human: String,
    pub next_actions: Vec<String>,
}

/// Every command's return type. The renderer is what turns it into output.
pub type CmdResult = Result<Outcome, TelosError>;
