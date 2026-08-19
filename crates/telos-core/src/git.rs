//! Git blob OIDs: the bridge between the working tree and content-addressed
//! sealing (spec §5).
//!
//! [`GitRepo::blob_oids`] is deliberately a *single* `git hash-object
//! --stdin-paths` process: no `-w` (it never writes objects -- read only),
//! and crucially no `--no-filters`. Letting clean filters run is the whole
//! point -- a repo's `.gitattributes` (e.g. `* text eol=lf`) then gives the
//! same OID for the same logical content regardless of which OS checked the
//! file out with which line endings, which is what makes a lock file
//! sealed on Linux verifiable, byte for byte, on Windows.
//!
//! Checking that a [`crate::workspace::Workspace`]'s `repo_root` matches a
//! [`GitRepo`]'s `root` is left to the caller (Task 11's `seal`); this
//! module only discovers the repository and hashes paths in it.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;

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

    /// Hashes `paths` as git blobs, filters applied, in exactly one child
    /// process.
    ///
    /// Paths that don't exist on disk (checked with `fs::metadata` against
    /// `root`-joined paths, before git is ever invoked) are silently
    /// absent from the result -- this is not an error, since a spec or
    /// binding can reference a path that hasn't been created yet.
    ///
    /// The existing paths are written to `git hash-object --stdin-paths`'s
    /// stdin, one repo-relative, `/`-separated path per line (git accepts
    /// `/` on every OS, including Windows), with `cwd = root` so
    /// `.gitattributes` filters resolve against this worktree. No `-w`
    /// (objects are never written -- this is read-only) and no
    /// `--no-filters` (clean filters, e.g. `eol=lf`, MUST run: that is what
    /// makes the resulting OID identical across OSes for the same logical
    /// content).
    ///
    /// Implementation note on avoiding a pipe deadlock: all of stdin is
    /// written and then dropped (closing the write end) *before* stdout is
    /// read. A stricter implementation would shuttle stdin writes and
    /// stdout reads concurrently (e.g. from a second thread), which is
    /// necessary once output could grow large enough to fill the OS pipe
    /// buffer while stdin is still being written. Here output is one short
    /// hex line per input path -- on the batch sizes a spec or a code tree
    /// actually has, it never approaches that buffer size, so writing
    /// everything up front and reading the (small) output afterwards is
    /// both simpler and safe. If `blob_oids` ever needs to hash tens of
    /// thousands of paths in one call, revisit this.
    pub fn blob_oids(&self, paths: &[RepoPath]) -> Result<BTreeMap<RepoPath, Oid>, TelosError> {
        let existing: Vec<&RepoPath> = paths
            .iter()
            .filter(|p| std::fs::metadata(self.abs_path(p)).is_ok())
            .collect();

        if existing.is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut child = Command::new("git")
            .arg("hash-object")
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

        {
            // Scoped so `stdin` is dropped (closing the pipe) before
            // `wait_with_output` reads stdout below -- see the doc comment
            // above on why writing everything first is safe here.
            let mut stdin = child.stdin.take().expect("stdin was piped");
            for path in &existing {
                writeln!(stdin, "{}", path.as_str()).map_err(|e| {
                    TelosError::new(
                        ErrorCode::TelosGitError,
                        format!("failed to write to `git hash-object` stdin: {e}"),
                    )
                })?;
            }
        }

        let output = child.wait_with_output().map_err(|e| {
            TelosError::new(
                ErrorCode::TelosGitError,
                format!("failed to read `git hash-object` output: {e}"),
            )
        })?;

        if !output.status.success() {
            return Err(TelosError::new(
                ErrorCode::TelosGitError,
                format!(
                    "`git hash-object --stdin-paths` failed: {}",
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
                    "`git hash-object --stdin-paths` returned {} OID(s) for {} path(s); stderr: {}",
                    lines.len(),
                    existing.len(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        Ok(existing
            .into_iter()
            .zip(lines)
            .map(|(path, oid)| (path.clone(), Oid(oid.to_string())))
            .collect())
    }

    /// Converts a repo-relative, `/`-separated [`RepoPath`] into an
    /// absolute, OS-native path under `root` -- splitting on `/` rather
    /// than handing the raw string to `PathBuf::join` so the conversion is
    /// explicit and correct on every OS (mirrors
    /// `Workspace::abs_path`).
    fn abs_path(&self, repo_path: &RepoPath) -> PathBuf {
        let mut path = self.root.clone();
        for component in repo_path.as_str().split('/') {
            path.push(component);
        }
        path
    }
}
