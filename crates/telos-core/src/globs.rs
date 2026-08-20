//! Rule 5 (no code without telos): matching `[code]`/`[tests]` globs
//! against the working tree, and finding files no binding covers (D8).
//!
//! Matching is done with `globset` (a single compiled [`GlobSet`] per glob
//! list, so many patterns cost one pass over the tree, not one per
//! pattern); the walk itself is hand-rolled rather than pulled in from a
//! walking crate, since the only structural exclusion this engine ever
//! applies is `.git/` -- everything else a project wants excluded, it
//! excludes by not globbing it in (its own glob choices are the only
//! opt-in mechanism; there is deliberately no second, implicit one for
//! e.g. `target/`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use globset::GlobSet;

use crate::config::compile_globs;
use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::model::{Binding, TelosModel};
use crate::workspace::Workspace;

/// Every repo-relative `'/'`-separated path under `root` that matches at
/// least one of `patterns`, sorted.
///
/// `patterns` empty short-circuits to `Ok(vec![])` *without* walking the
/// tree at all -- an empty `[code]`/`[tests]` section means nothing is
/// eligible, so there is nothing to look for, and a root that happens not
/// to exist (or not to be readable) must not turn that into an error.
///
/// An invalid pattern is reported as `TelosParseError`, naming the pattern
/// that failed to compile. The walk that follows a successful compile skips
/// `.git/` directories entirely, wherever encountered, and nothing else:
/// this is the one structural exclusion the engine imposes (see the module
/// docs).
pub fn glob_matches(root: &Path, patterns: &[String]) -> Result<Vec<RepoPath>, TelosError> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }

    let set = compile_globs(patterns)?;

    let mut matches = Vec::new();
    walk(root, root, &set, &mut matches)?;
    matches.sort();
    Ok(matches)
}

/// Rule 5: files matched by `[code]`/`[tests]` globs that no binding
/// covers, the two families evaluated independently (D8).
///
/// A file matching `[code]` must be covered by at least one
/// `Binding::Implements` on that exact path; a file matching `[tests]` must
/// be covered by at least one `Binding::Proves` whose `test.path` is that
/// exact path. The two checks never interact: a file matched by both glob
/// families must satisfy both bindings, so it is orphaned if it fails
/// *either* one -- even while the other already covers it. The result is
/// sorted and deduplicated (a file failing both families is reported once).
pub fn orphan_code(ws: &Workspace, model: &TelosModel) -> Result<Vec<RepoPath>, TelosError> {
    let implemented: BTreeSet<&RepoPath> = model
        .bindings
        .iter()
        .filter_map(|b| match b {
            Binding::Implements { path, .. } => Some(path),
            Binding::Proves { .. } => None,
        })
        .collect();
    let proven: BTreeSet<&RepoPath> = model
        .bindings
        .iter()
        .filter_map(|b| match b {
            Binding::Proves { test, .. } => Some(&test.path),
            Binding::Implements { .. } => None,
        })
        .collect();

    let code_files = glob_matches(&ws.repo_root, &ws.config.code.globs)?;
    let test_files = glob_matches(&ws.repo_root, &ws.config.tests.globs)?;

    let mut orphans = BTreeSet::new();
    orphans.extend(code_files.into_iter().filter(|p| !implemented.contains(p)));
    orphans.extend(test_files.into_iter().filter(|p| !proven.contains(p)));

    Ok(orphans.into_iter().collect())
}

/// Compiles `patterns` into one [`GlobSet`]. A pattern `globset` rejects is
/// reported as `TelosParseError`, naming it.
///
/// Every pattern is built with `literal_separator(true)`: `globset`'s
/// *default* (`Glob::new`) lets a bare `*` cross a `/`, so `"src/*.rs"`
/// would also match `"src/deeply/nested/file.rs"` -- the opposite of the
/// gitignore-style semantics this engine wants, where `*` stays within one
/// path component and only `**` spans directories.
/// Recursively visits `dir` (an absolute path, initially `root` itself),
/// appending every file matching `set` to `out` as a `root`-relative
/// [`RepoPath`]. `.git` directories are skipped entirely, wherever found.
fn walk(root: &Path, dir: &Path, set: &GlobSet, out: &mut Vec<RepoPath>) -> Result<(), TelosError> {
    let entries = fs::read_dir(dir).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to read {}: {e}", dir.display()),
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {e}", dir.display()),
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {e}", path.display()),
            )
        })?;

        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            walk(root, &path, set, out)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let repo_path = to_repo_path_str(rel);
        if set.is_match(&repo_path) {
            out.push(RepoPath::new(repo_path));
        }
    }
    Ok(())
}

/// Joins a relative path's components with `'/'`, regardless of the host
/// OS's native separator -- the on-disk representation is only ever
/// OS-native at the `Path` boundary; everything this module reports or
/// matches against is the `'/'`-separated form `RepoPath` always is.
fn to_repo_path_str(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
