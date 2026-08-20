//! The change store: filesystem operations over `telos/changes/*.tel`
//! (Annex B), and [`open_change_infos`], the best-effort scan every
//! claim-aware caller (`compute_state`, T5's `change` commands, T6's
//! `reconcile`) starts from.
//!
//! [`emit_change`] is the *only* writer of a change file's bytes -- see its
//! own docs -- so [`write_change`] is a thin wrapper around it, never a
//! second serializer. Reading is the mirror: [`read_change`] always goes
//! through [`parse_change_file`], never partially decodes a file itself.
//!
//! `telos/changes/counters.toml` lives in the same directory as change
//! files but is not one -- [`list_change_ids`] filters to names that parse
//! as a bare `CHG-NNNN.tel` stem, which `counters.toml` never does.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::emit::emit_change;
use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::ids::{ChangeId, RepoPath};
use crate::model::{Change, ChangeStatus};
use crate::syntax::parse_change_file;
use crate::workspace::Workspace;

/// What [`open_change_infos`] reports about one open change: everything a
/// claim-aware caller needs without re-parsing the file itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChangeInfo {
    pub id: ChangeId,
    pub status: ChangeStatus,
    pub claims: BTreeSet<RepoPath>,
    pub obligations: Vec<String>,
}

/// Lists every change id under `telos/changes/`, ascending.
///
/// Scans the directory's direct entries and keeps only the ones whose file
/// name, with a `.tel` suffix stripped, parses as a bare `ChangeId`
/// (`CHG-NNNN`, nothing more) -- `counters.toml`, any non-`.tel` file, and
/// any subdirectory are silently skipped rather than erroring, the same way
/// [`Workspace::spec_files`] treats its own scanned directories. A missing
/// `telos/changes/` directory contributes no id rather than erroring: a
/// project that has never opened a change never created the directory.
pub fn list_change_ids(ws: &Workspace) -> Result<Vec<ChangeId>, TelosError> {
    let dir = changes_dir(ws);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(&dir, e)),
    };

    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_err(&dir, e))?;
        if !entry.path().is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".tel") else {
            continue;
        };
        if let Ok(id) = stem.parse::<ChangeId>() {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}

/// Reads and parses `telos/changes/<id>.tel`.
///
/// A missing file is `TelosReferenceUnknown`, «unknown change `CHG-9999`» --
/// with a `closest is CHG-NNNN` hint when at least one other change exists,
/// by numeric distance to `id`, the same policy `show`/`impact` use for an
/// unknown intent id. A present but unparsable file is reported through
/// [`diagnostics_to_error`], never partially: every [`Diagnostic`]
/// `parse_change_file` collected is folded into the message, one line each.
pub fn read_change(ws: &Workspace, id: ChangeId) -> Result<Change, TelosError> {
    let path = change_path(ws, id);
    let src = match fs::read_to_string(&path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(unknown_change(ws, id)),
        Err(e) => return Err(io_err(&path, e)),
    };
    parse_change_file(&repo_path_for(id), &src).map_err(diagnostics_to_error)
}

/// Writes `c` to `telos/changes/<c.id>.tel`, in canonical form
/// ([`emit_change`]). The only writer of a change file's bytes -- creates
/// `telos/changes/` first if it does not exist yet, the same as
/// [`crate::counters::write_counters`].
pub fn write_change(ws: &Workspace, c: &Change) -> Result<(), TelosError> {
    let dir = changes_dir(ws);
    fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    let path = change_path(ws, c.id);
    fs::write(&path, emit_change(c)).map_err(|e| io_err(&path, e))
}

/// Deletes `telos/changes/<id>.tel` -- the terminal step of both a
/// reconcile and an abandon (D16): both outcomes are "the file is gone",
/// never a stored status. A missing file is the same `TelosReferenceUnknown`
/// [`read_change`] reports: deleting an id the store does not hold is a
/// caller bug, not a no-op to swallow silently.
pub fn delete_change(ws: &Workspace, id: ChangeId) -> Result<(), TelosError> {
    let path = change_path(ws, id);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(unknown_change(ws, id)),
        Err(e) => Err(io_err(&path, e)),
    }
}

/// Every open change, as [`OpenChangeInfo`] -- best-effort, per D15.
///
/// A change file that fails to parse is not an error here: it is reported
/// as an [`OpenChangeInfo`] of its own, `status: Open`, no claim (an
/// unparsable file's ops cannot be trusted enough to claim anything on its
/// behalf), and the single obligation `repair telos/changes/CHG-NNNN.tel
/// (unparseable)` -- so a corrupted change file still keeps the project's
/// state answerable (`compute_state`) instead of blocking every command
/// that needs to know what is open. `id` in that case comes from the file
/// name [`list_change_ids`] already validated, not from the unreadable
/// content.
///
/// "Fails to parse" is read broadly here, on purpose: bytes that are not
/// valid UTF-8 (a truncated write, binary corruption) can never reach
/// [`parse_change_file`] (it takes `&str`) and are exactly the kind of
/// on-disk damage D15 exists to survive, so they get the same repair
/// obligation as a syntax error rather than aborting the whole scan -- the
/// same is true of any other I/O failure reading an individual file (a
/// permission error, say) once its id has already been confirmed present
/// by [`list_change_ids`]. The one exception is the file having vanished
/// (`NotFound`) between that listing and this read: there is nothing left
/// to repair, so that id is silently skipped rather than reported, on the
/// same "list, then read" TOCTOU basis [`Workspace::load_model`] accepts
/// elsewhere in this crate. [`read_change`] is the strict counterpart that
/// still errors loudly on all of this -- only this best-effort enumeration
/// degrades.
pub fn open_change_infos(ws: &Workspace) -> Result<Vec<OpenChangeInfo>, TelosError> {
    let ids = list_change_ids(ws)?;
    Ok(ids
        .into_iter()
        .filter_map(|id| read_open_change_info(ws, id))
        .collect())
}

/// Reads and (best-effort) reports one change file for [`open_change_infos`].
///
/// `None` only for the file having vanished since [`list_change_ids`] saw it
/// -- the "list, then read" TOCTOU every id here is already exposed to --
/// there is nothing left to repair, so that id is dropped rather than
/// reported. Every other outcome is `Some`: a clean parse, a syntax error, a
/// file that is not valid UTF-8 at all (`parse_change_file` takes `&str`,
/// so it can never even be offered such bytes), or any other per-file I/O
/// failure (a permission error, say) all fold into the same best-effort
/// [`unparseable_info`] -- see [`open_change_infos`]'s doc comment for why.
fn read_open_change_info(ws: &Workspace, id: ChangeId) -> Option<OpenChangeInfo> {
    let path = change_path(ws, id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(unparseable_info(id)),
    };
    match String::from_utf8(bytes) {
        Ok(src) => match parse_change_file(&repo_path_for(id), &src) {
            Ok(change) => Some(OpenChangeInfo {
                id: change.id,
                status: change.status,
                claims: change.claims(),
                obligations: change.obligations(),
            }),
            Err(_diagnostics) => Some(unparseable_info(id)),
        },
        Err(_not_utf8) => Some(unparseable_info(id)),
    }
}

/// The best-effort [`OpenChangeInfo`] for a change file that could not be
/// read as canonical `.tel` source at all -- invalid UTF-8, a parse
/// failure, or any other per-file I/O error -- per D15.
fn unparseable_info(id: ChangeId) -> OpenChangeInfo {
    OpenChangeInfo {
        id,
        status: ChangeStatus::Open,
        claims: BTreeSet::new(),
        obligations: vec![format!("repair telos/changes/{id}.tel (unparseable)")],
    }
}

// --- shared helpers ----------------------------------------------------

fn changes_dir(ws: &Workspace) -> PathBuf {
    ws.telos_dir.join("changes")
}

fn change_path(ws: &Workspace, id: ChangeId) -> PathBuf {
    changes_dir(ws).join(format!("{id}.tel"))
}

fn repo_path_for(id: ChangeId) -> RepoPath {
    RepoPath::new(format!("telos/changes/{id}.tel"))
}

fn io_err(path: &Path, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to access {}: {e}", path.display()),
    )
}

/// «unknown change `CHG-9999`», with a numeric-nearest-id hint when at
/// least one other change exists -- the same shape and algorithm as the
/// CLI's `nearest_id` for an unknown intent/scenario/constraint id, kept
/// here rather than shared because it is `telos-core` that owns the change
/// store and cannot depend back on the `telos` binary crate.
fn unknown_change(ws: &Workspace, id: ChangeId) -> TelosError {
    let hint = list_change_ids(ws)
        .unwrap_or_default()
        .into_iter()
        .min_by_key(|other| id.0.abs_diff(other.0))
        .map(|nearest| format!("closest is {nearest}"));
    let error = TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        format!("unknown change `{id}`"),
    );
    match hint {
        Some(hint) => error.hint(hint),
        None => error,
    }
}

/// Collapses a full diagnostics list into one [`TelosError`], mirroring the
/// CLI's own `diagnostics_to_error` policy (`crates/telos/src/commands/
/// mod.rs`): the first diagnostic supplies `code` and `hint` -- the frozen
/// error body has room for exactly one of each -- and every diagnostic's
/// message is appended on its own line, so a human reading stderr still
/// sees everything the parse found. Duplicated rather than shared for the
/// same reason as [`unknown_change`]: `telos-core` cannot depend on the
/// `telos` binary crate that owns the original. Shared inside the crate with
/// [`crate::reconcile`], which collapses the overlay's diagnostics the same
/// way.
pub(crate) fn diagnostics_to_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    let mut iter = diagnostics.into_iter();
    let first = iter
        .next()
        .expect("a diagnostics `Err` always carries at least one diagnostic");
    let mut error: TelosError = first.into();
    for diagnostic in iter {
        let extra: TelosError = diagnostic.into();
        error.message.push('\n');
        error.message.push_str(&extra.message);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A `Workspace` built directly rather than through `discover` -- no
    /// `telos/telos.toml` needs to exist on disk for these unit-level
    /// tests, which exercise `read_open_change_info` on its own, one
    /// filesystem outcome at a time.
    fn workspace(root: &std::path::Path) -> Workspace {
        Workspace {
            repo_root: root.to_path_buf(),
            telos_dir: root.join("telos"),
            config: Config::default(),
        }
    }

    /// The TOCTOU case black-box tests cannot force deterministically: a
    /// file `list_change_ids` never even listed (the same observable state
    /// as one that vanished right after being listed) is skipped, not
    /// reported.
    #[test]
    fn read_open_change_info_skips_a_file_that_is_not_there() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());

        assert_eq!(read_open_change_info(&ws, ChangeId(1)), None);
    }

    #[test]
    fn read_open_change_info_is_best_effort_on_invalid_utf8() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path());
        fs::create_dir_all(changes_dir(&ws)).unwrap();
        fs::write(
            change_path(&ws, ChangeId(1)),
            b"\xff\xfe garbage".as_slice(),
        )
        .unwrap();

        let info = read_open_change_info(&ws, ChangeId(1)).unwrap();

        assert_eq!(
            info,
            OpenChangeInfo {
                id: ChangeId(1),
                status: ChangeStatus::Open,
                claims: BTreeSet::new(),
                obligations: vec!["repair telos/changes/CHG-0001.tel (unparseable)".to_string()],
            }
        );
    }
}
