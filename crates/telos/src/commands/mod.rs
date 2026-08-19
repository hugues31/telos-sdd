//! One module per command, each exposing a `run` that returns a
//! [`CmdResult`] and prints nothing -- rendering belongs to
//! [`crate::render`], and having exactly one place that writes to a stream is
//! what keeps every command's human and JSON output consistent.

pub mod check;
pub mod init;
pub mod list;
pub mod show;
pub mod status;

use std::path::PathBuf;

use serde_json::json;

use telos_core::error::{Diagnostic, ErrorCode, TelosError};
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

/// Collapses a full diagnostics list into the single [`TelosError`] the
/// envelope carries.
///
/// `code` and `hint` are the first diagnostic's -- the frozen error body
/// has room for exactly one of each, so a command that can find several
/// problems in one pass (`check`, but also `show`/`list` loading the same
/// model) surfaces the first (Annex B). The *message* stays multi-line when
/// there is more than one diagnostic: every diagnostic gets its own `file:
/// message` line (via the same `From<Diagnostic>` conversion, applied to
/// each), so a human reading stderr sees everything the load found in this
/// run. In `--json` mode this means `error.message` can itself carry more
/// than one line -- an agent that only reads the first line still gets the
/// primary diagnosis; this M1 limitation (no `result.diagnostics` array on
/// failure) is documented in `docs/contracts.md`.
pub(crate) fn diagnostics_to_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    let mut iter = diagnostics.into_iter();
    let first = iter
        .next()
        .expect("`load_model` reports at least one diagnostic on `Err`");
    let mut error: TelosError = first.into();
    for diagnostic in iter {
        let extra: TelosError = diagnostic.into();
        error.message.push('\n');
        error.message.push_str(&extra.message);
    }
    error
}
