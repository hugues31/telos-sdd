//! One module per command, each exposing a `run` that returns a
//! [`CmdResult`] and prints nothing -- rendering belongs to
//! [`crate::render`], and having exactly one place that writes to a stream is
//! what keeps every command's human and JSON output consistent.

pub mod init;

use std::path::PathBuf;

use serde_json::json;

use crate::envelope::{CmdResult, Outcome};

/// What a command knows about where it runs. Discovery (git repository,
/// workspace) starts from `cwd`.
pub struct Ctx {
    pub cwd: PathBuf,
}

/// `telos version` -- the same version `--version` prints, in the envelope.
///
/// Small enough to live here rather than in its own module: it reads nothing
/// and touches nothing.
pub fn version() -> CmdResult {
    Ok(Outcome {
        result: json!({ "version": telos_core::VERSION }),
        human: format!("telos {}", telos_core::VERSION),
        next_actions: Vec::new(),
    })
}
