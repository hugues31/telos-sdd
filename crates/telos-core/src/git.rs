//! Git blob OIDs: the bridge between the working tree and content-addressed
//! sealing.
//!
//! [`GitRepo::blob_oids`] and [`GitRepo::store_blobs`] are each deliberately
//! a *single* `git hash-object --stdin-paths` process, and crucially neither
//! passes `--no-filters`. Letting clean filters run is the whole point -- a
//! repo's `.gitattributes` (e.g. `* text eol=lf`) then gives the same OID
//! for the same logical content regardless of which OS checked the file out
//! with which line endings, which is what makes a lock file sealed on Linux
//! verifiable, byte for byte, on Windows.
//!
//! The two differ in exactly one flag. `blob_oids` never passes `-w`: it
//! only hashes, and since it runs on every `status`, it must not litter the
//! object store. `store_blobs` passes `-w`: it is what a *seal* uses, so
//! that every OID a lock records names an object the store actually holds,
//! commit or no commit -- which is what lets `telos revert` restore a
//! project that was sealed but never committed. The objects it writes stay
//! unreachable until a commit references them; git only prunes unreachable
//! objects past `gc.pruneExpire`, two weeks by default.
//!
//! [`GitRepo::ensure_matches_workspace_root`] verifies that a
//! [`crate::workspace::Workspace`]'s `repo_root` and this [`GitRepo`]'s
//! `root` name the same directory (canonicalized on both sides, so a
//! symlinked route to the same place -- e.g. macOS's `/tmp` -- still
//! compares equal). The check itself lives here, but the *calling* is done
//! by [`crate::lock::seal`] and [`crate::state::compute_state`] -- every
//! command built on either (`init`, `status`, `check --sealed`, and the
//! transaction commands) inherits it from there rather than needing to
//! call it itself. Without it, a `Workspace` and a `GitRepo` discovered
//! independently from `cwd` (as `status` and `check --sealed` do) can
//! silently name two different repositories -- a nested git repo under an
//! initialized workspace, say -- and `compute_state` would then hash blobs
//! from the wrong tree and report bogus drift.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use serde::Serialize;

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::repo_fs::RepoFs;

/// The frozen hint of the one `TELOS_GIT_ERROR` [`GitRepo::cat_blob`]
/// raises: an OID the seal records that the object store does not hold --
/// a lock sealed by a `telos` older than [`GitRepo::store_blobs`], or an
/// object git has since pruned.
///
/// It lives here, next to the command that produces it, because the caller
/// that surfaces it -- `telos revert` -- is in another crate, and the
/// remedy it names ("commit the sealed state") is about git, not about
/// reverting.
pub const MISSING_BLOB_HINT: &str = "the sealed content is not in the git object store; commit the sealed state or restore the file by hand";

/// An opaque git object id: 40 hex characters for a sha1 repository, 64 for
/// a sha256 one. Never parsed or interpreted, only compared and displayed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Oid(pub String);

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A discovered git repository: just its worktree root.
#[derive(Debug)]
pub struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    /// Runs `git rev-parse --show-toplevel` with `cwd = start` and stores
    /// the result as `root`.
    ///
    /// A non-zero exit (not inside a git repository, most commonly) is
    /// reported as `TelosGitError` with the hint `"not a git repository;
    /// run \`git init\`"`. An I/O error spawning `git` itself (the binary
    /// is missing from `PATH`) is reported as `TelosGitError` with a
    /// message saying so, since there is no git output to explain it.
    pub fn discover(start: &Path) -> Result<Self, TelosError> {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .current_dir(start)
            .output()
            .map_err(|e| {
                TelosError::new(
                    ErrorCode::TelosGitError,
                    format!("git is required and was not found on PATH: {e}"),
                )
            })?;

        if !output.status.success() {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "`git rev-parse --show-toplevel` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
            .hint("not a git repository; run `git init`"));
        }

        let toplevel = String::from_utf8_lossy(&output.stdout);
        Ok(GitRepo {
            root: PathBuf::from(toplevel.trim()),
        })
    }

    /// The repository's worktree root, as returned by `git rev-parse
    /// --show-toplevel`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verifies that `ws_root` (a [`crate::workspace::Workspace`]'s
    /// `repo_root`) and `self.root` name the same directory.
    ///
    /// Both sides are canonicalized (`std::fs::canonicalize`, resolving
    /// symlinks and `.`/`..`) before comparing, so two paths that reach the
    /// same directory by different routes -- notably a symlinked `/tmp` on
    /// macOS -- still compare equal instead of false-positiving a mismatch.
    ///
    /// A canonicalization I/O failure (either root vanished, or isn't
    /// readable) and a genuine mismatch are both reported as
    /// `TelosGitError`; a real mismatch also carries a hint. See the module
    /// docs for why this lives here but is called from `seal` and
    /// `compute_state`, not from here.
    pub fn ensure_matches_workspace_root(&self, ws_root: &Path) -> Result<(), TelosError> {
        let git_canon = canonicalize(&self.root)?;
        let ws_canon = canonicalize(ws_root)?;
        if git_canon != ws_canon {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "the telos workspace root (`{}`) and the git repository root (`{}`) do not match",
                    ws_root.display(),
                    self.root.display(),
                ),
            )
            .hint(
                "the telos workspace and the git repository must share the same root; run telos from the repository that contains telos/",
            ));
        }
        Ok(())
    }

    /// Hashes `paths` as git blobs, filters applied, in exactly one child
    /// process -- and writes nothing to the object store.
    ///
    /// The read-only half of the pair: what every *comparison* against the
    /// seal uses (`status`, the reconcile gates, `adopt`). See
    /// [`GitRepo::store_blobs`] for the half a seal uses, and the module
    /// docs for why they are two entry points and not a flag.
    pub fn blob_oids(&self, paths: &[RepoPath]) -> Result<BTreeMap<RepoPath, Oid>, TelosError> {
        self.hash_objects(paths, false)
    }

    /// Hashes `paths` as git blobs exactly like [`GitRepo::blob_oids`] --
    /// same filters, same OIDs, same one child process -- and writes each
    /// object to the store (`git hash-object -w`).
    ///
    /// The one to call when the OIDs are about to be *sealed*. A lock is
    /// only a backup if the content its OIDs name exists somewhere, and
    /// with no commit in between, this is the only thing that puts it
    /// there; [`GitRepo::cat_blob`] is the inverse that reads it back. The
    /// objects stay unreachable until a commit names them, which is fine:
    /// git only prunes unreachable objects past `gc.pruneExpire` (two weeks
    /// by default), and a seal that old with no commit behind it is
    /// exactly the case [`MISSING_BLOB_HINT`] is worded for.
    pub fn store_blobs(&self, paths: &[RepoPath]) -> Result<BTreeMap<RepoPath, Oid>, TelosError> {
        self.hash_objects(paths, true)
    }

    /// The one body behind `blob_oids` (`write = false`) and `store_blobs`
    /// (`write = true`).
    ///
    /// Paths that don't exist on disk (checked with `fs::metadata` against
    /// `root`-joined paths, before git is ever invoked) are silently
    /// absent from the result -- this is not an error, since a spec or
    /// binding can reference a path that hasn't been created yet.
    ///
    /// The existing paths are written to `git hash-object --stdin-paths`'s
    /// stdin, one repo-relative, `/`-separated path per line (git accepts
    /// `/` on every OS, including Windows), with `cwd = root` so
    /// `.gitattributes` filters resolve against this worktree. `-w` if and
    /// only if `write`, and never `--no-filters` (clean filters, e.g.
    /// `eol=lf`, MUST run: that is what makes the resulting OID identical
    /// across OSes for the same logical content).
    ///
    /// Stdin is written from a dedicated thread while the calling thread
    /// drains stdout/stderr through `wait_with_output`, so neither pipe can
    /// block the other for large path sets.
    fn hash_objects(
        &self,
        paths: &[RepoPath],
        write: bool,
    ) -> Result<BTreeMap<RepoPath, Oid>, TelosError> {
        let safe_root = RepoFs::open(&self.root)?;
        let mut existing = Vec::new();
        for path in paths {
            path.validate()?;
            let bytes = safe_root.read_optional(path).map_err(|error| {
                TelosError::new(
                    ErrorCode::TelosIntegrityViolation,
                    format!(
                        "repository path `{path}` resolves outside the repository or through a symlink: {}",
                        error.message
                    ),
                )
            })?;
            if let Some(bytes) = bytes {
                existing.push((path, bytes));
            }
        }

        if existing.is_empty() {
            return Ok(BTreeMap::new());
        }

        let command_line = if write {
            "git hash-object -w --stdin-paths"
        } else {
            "git hash-object --stdin-paths"
        };
        let mut command = Command::new("git");
        command.arg("hash-object");
        if write {
            command.arg("-w");
        }
        let mut child = command
            .arg("--stdin-paths")
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                TelosError::new(
                    ErrorCode::TelosGitError,
                    format!("git is required and was not found on PATH: {e}"),
                )
            })?;

        let mut stdin = child.stdin.take().expect("stdin was piped");
        let input_paths = existing
            .iter()
            .map(|(path, _)| path.as_str().to_owned())
            .collect::<Vec<_>>();
        let writer = thread::spawn(move || -> std::io::Result<()> {
            for path in input_paths {
                writeln!(stdin, "{path}")?;
            }
            Ok(())
        });

        let output = child.wait_with_output().map_err(|e| {
            TelosError::new(
                ErrorCode::TelosGitError,
                format!("failed to read `git hash-object` output: {e}"),
            )
        })?;
        writer
            .join()
            .map_err(|_| {
                TelosError::new(
                    ErrorCode::TelosGitError,
                    "the `git hash-object` stdin writer panicked",
                )
            })?
            .map_err(|e| {
                TelosError::new(
                    ErrorCode::TelosGitError,
                    format!("failed to write to `git hash-object` stdin: {e}"),
                )
            })?;

        if !output.status.success() {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "`{command_line}` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        if lines.len() != existing.len() {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "`{command_line}` returned {} OID(s) for {} path(s); stderr: {}",
                    lines.len(),
                    existing.len(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        for (path, before) in &existing {
            let after = safe_root.read_optional(path).map_err(|error| {
                TelosError::new(
                    ErrorCode::TelosIntegrityViolation,
                    format!(
                        "repository path `{path}` changed identity while it was hashed: {}",
                        error.message
                    ),
                )
            })?;
            if after.as_deref() != Some(before.as_slice()) {
                return Err(TelosError::new(
                    ErrorCode::TelosIntegrityViolation,
                    format!("repository path `{path}` changed while it was hashed"),
                ));
            }
        }

        Ok(existing
            .into_iter()
            .zip(lines)
            .map(|((path, _), oid)| (path.clone(), Oid(oid.to_string())))
            .collect())
    }

    /// Reads one blob's bytes back out of the object store: `git cat-file
    /// blob <oid>`, with `cwd = root`.
    ///
    /// The exact inverse of [`GitRepo::store_blobs`] for
    /// [`crate::adopt::revert`]'s purposes -- given an OID the seal records,
    /// answer with the content it names. A seal made with `store_blobs`
    /// always has that content in the store, commit or no commit; a lock
    /// sealed by an older `telos` (which only hashed) or an object git has
    /// since pruned does not. That case is the one refusal here, and it
    /// carries [`MISSING_BLOB_HINT`], which names the remedy.
    ///
    /// Answers with the blob's bytes as stored, i.e. *after* the clean
    /// filter that produced the OID and without the smudge filter a
    /// checkout would apply. That is the right choice for restoring a
    /// sealed path: what comes back hashes to the OID it came from, which is
    /// precisely what makes the project coherent again. On a repository
    /// whose `.gitattributes` rewrites line endings on checkout, a restored
    /// file therefore holds the *canonical* (clean) form rather than the
    /// working-tree form -- the same form `telos` seals, compares and
    /// emits.
    pub fn cat_blob(&self, oid: &Oid) -> Result<Vec<u8>, TelosError> {
        let output = Command::new("git")
            .arg("cat-file")
            .arg("blob")
            .arg(&oid.0)
            .current_dir(&self.root)
            .output()
            .map_err(|e| {
                TelosError::new(
                    ErrorCode::TelosGitError,
                    format!("git is required and was not found on PATH: {e}"),
                )
            })?;

        if !output.status.success() {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "`git cat-file blob {oid}` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
            .hint(MISSING_BLOB_HINT));
        }

        Ok(output.stdout)
    }
}

/// `std::fs::canonicalize`, with I/O failure reported as `TelosGitError`
/// naming the offending path -- the only failure mode
/// [`GitRepo::ensure_matches_workspace_root`] has to translate.
fn canonicalize(path: &Path) -> Result<PathBuf, TelosError> {
    std::fs::canonicalize(path).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosGitError,
            format!("failed to resolve `{}`: {e}", path.display()),
        )
    })
}
