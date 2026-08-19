//! One module per command, each exposing a `run` that returns a
//! [`CmdResult`] and prints nothing -- rendering belongs to
//! [`crate::render`], and having exactly one place that writes to a stream is
//! what keeps every command's human and JSON output consistent.

pub mod check;
pub mod init;
pub mod status;

use std::path::PathBuf;

use serde_json::json;

use telos_core::error::{ErrorCode, TelosError};
use telos_core::lock::Lock;
use telos_core::workspace::Workspace;

use crate::envelope::{CmdResult, Outcome};

/// What a command knows about where it runs. Discovery (git repository,
/// workspace) starts from `cwd`.
pub struct Ctx {
    pub cwd: PathBuf,
}

/// Reads `ws`'s `telos.lock`, turning a missing file into a domain error.
///
/// A discovered workspace (`telos/telos.toml` exists) without a
/// `telos.lock` is not simply "unsealed" -- `telos init` always seals, so
/// there is no code path that leaves a real project in that state on
/// purpose. It is an abnormal state distinct from "no `telos/` at all"
/// ([`Workspace::discover`]'s own `TelosNotInitialized`, with its own
/// hint), so it gets its own message and hint here rather than reusing
/// `discover`'s.
fn require_lock(ws: &Workspace) -> Result<Lock, TelosError> {
    Lock::read(&ws.lock_path())?.ok_or_else(|| {
        TelosError::new(ErrorCode::TelosNotInitialized, "telos.lock is missing").hint(
            "the project was never sealed; run `telos init` in a fresh repository or restore telos.lock from git",
        )
    })
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
