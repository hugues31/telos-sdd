//! Workspace discovery and model loading: the filesystem entry point that
//! turns a starting directory into a parsed [`TelosModel`].
//!
//! Everything here is I/O -- reading `telos.toml`, walking directories,
//! reading `.tel` files -- and nothing here is git-aware or lock-aware.
//! Checking that `repo_root` matches the git repository root is
//! [`crate::git::GitRepo`]'s job (Task 10); reading `telos.lock` and
//! sealing are [`crate::lock`]'s (Task 11).

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::model::{TelFile, TelosModel};
use crate::semantic::build_model;
use crate::syntax::{
    parse_bindings_file, parse_constraint_file, parse_intent_file, parse_notion_file,
};

/// A discovered telos-sdd project: its root, the `telos/` directory inside
/// it, and the parsed `telos.toml` configuration.
#[derive(Debug)]
pub struct Workspace {
    pub repo_root: PathBuf,
    pub telos_dir: PathBuf,
    pub config: Config,
}

/// One spec subdirectory scanned by [`Workspace::spec_files`], paired with
/// the repo-relative prefix its entries are reported under.
const SPEC_SUBDIRS: [(&str, &str); 3] = [
    ("notions", "telos/notions"),
    ("intents", "telos/intents"),
    ("constraints", "telos/constraints"),
];

impl Workspace {
    /// Walks up from `cwd` toward the filesystem root looking for
    /// `telos/telos.toml`. The directory that contains `telos/` becomes
    /// `repo_root`.
    ///
    /// This does not require, and does not check, that `repo_root` is a git
    /// repository -- see the module docs.
    pub fn discover(cwd: &Path) -> Result<Workspace, TelosError> {
        let mut dir = cwd.to_path_buf();
        loop {
            let telos_dir = dir.join("telos");
            let config_path = telos_dir.join("telos.toml");
            if config_path.is_file() {
                let config = load_config(&config_path)?;
                return Ok(Workspace {
                    repo_root: dir,
                    telos_dir,
                    config,
                });
            }
            dir = match dir.parent() {
                Some(parent) => parent.to_path_buf(),
                None => {
                    return Err(TelosError::new(
                        ErrorCode::TelosNotInitialized,
                        format!("no `telos/telos.toml` found above `{}`", cwd.display()),
                    )
                    .hint("run `telos init` at the repository root"));
                }
            };
        }
    }

    /// The spec files sealed as a unit: `telos/telos.toml`, every `*.tel`
    /// file directly under `telos/notions/`, `telos/intents/` and
    /// `telos/constraints/`, and `telos/bindings.tel` if present.
    ///
    /// Excludes `telos/changes/` and `telos/telos.lock` (never scanned).
    /// A missing subdirectory contributes nothing. The result is sorted
    /// lexicographically by its `RepoPath` string, so it is deterministic
    /// regardless of directory-listing order.
    pub fn spec_files(&self) -> Result<Vec<RepoPath>, TelosError> {
        let mut files = vec![RepoPath::new("telos/telos.toml")];

        for (subdir, prefix) in SPEC_SUBDIRS {
            collect_tel_files(&self.telos_dir.join(subdir), prefix, &mut files)?;
        }

        let bindings_path = self.telos_dir.join("bindings.tel");
        if bindings_path.is_file() {
            files.push(RepoPath::new("telos/bindings.tel"));
        }

        files.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(files)
    }

    /// Reads and parses every spec file, then folds them into a
    /// [`TelosModel`] via [`build_model`].
    ///
    /// Parse diagnostics from every file are collected first (every file is
    /// parsed even if another one already failed); if any exist, they are
    /// returned without running the semantic pass, since a file that failed
    /// to parse cannot be resolved against. Does not read `telos.lock` and
    /// does not touch git.
    pub fn load_model(&self) -> Result<TelosModel, Vec<Diagnostic>> {
        let spec_files = self
            .spec_files()
            .map_err(|e| vec![telos_error_as_diagnostic(e)])?;

        let mut diagnostics = Vec::new();
        let mut parsed = Vec::new();

        for repo_path in spec_files {
            // `telos.toml` is workspace configuration, not a `.tel` source
            // -- it has no `TelFile` representation.
            if repo_path.as_str() == "telos/telos.toml" {
                continue;
            }

            let abs_path = self.abs_path(&repo_path);
            let src = match fs::read_to_string(&abs_path) {
                Ok(src) => src,
                Err(e) => {
                    diagnostics.push(Diagnostic {
                        code: ErrorCode::TelosInternal,
                        message: format!("failed to read {repo_path}: {e}"),
                        hint: None,
                        file: Some(repo_path),
                        line: None,
                        col: None,
                    });
                    continue;
                }
            };

            match parse_spec_file(&repo_path, &src) {
                Ok(file) => parsed.push((repo_path, file)),
                Err(diags) => diagnostics.extend(diags),
            }
        }

        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        build_model(parsed)
    }

    /// `<telos_dir>/telos.lock`.
    pub fn lock_path(&self) -> PathBuf {
        self.telos_dir.join("telos.lock")
    }

    /// Converts a repo-relative `'/'`-separated path into an absolute,
    /// OS-native path under `repo_root`.
    fn abs_path(&self, repo_path: &RepoPath) -> PathBuf {
        let mut path = self.repo_root.clone();
        for component in repo_path.as_str().split('/') {
            path.push(component);
        }
        path
    }
}

/// Routes a spec file to the parser its location calls for (Annex C.4.2):
/// `notions/*.tel` -> `parse_notion_file`, `intents/*.tel` ->
/// `parse_intent_file`, `constraints/*.tel` -> `parse_constraint_file`,
/// `bindings.tel` -> `parse_bindings_file`.
fn parse_spec_file(repo_path: &RepoPath, src: &str) -> Result<TelFile, Vec<Diagnostic>> {
    let path = repo_path.as_str();
    if path.starts_with("telos/notions/") {
        parse_notion_file(repo_path, src).map(TelFile::Notion)
    } else if path.starts_with("telos/intents/") {
        parse_intent_file(repo_path, src).map(TelFile::Intent)
    } else if path.starts_with("telos/constraints/") {
        parse_constraint_file(repo_path, src).map(TelFile::Constraint)
    } else {
        // The only remaining member of `spec_files()`'s output besides
        // `telos.toml` (filtered out by the caller) is `bindings.tel`.
        parse_bindings_file(repo_path, src).map(TelFile::Bindings)
    }
}

/// Reads `telos.toml` at `path` and parses it into a [`Config`].
fn load_config(path: &Path) -> Result<Config, TelosError> {
    let src = fs::read_to_string(path).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to read {}: {e}", path.display()),
        )
    })?;
    toml::from_str(&src)
        .map_err(|e| TelosError::new(ErrorCode::TelosParseError, format!("telos/telos.toml: {e}")))
}

/// Lists the direct `*.tel` entries of `dir`, reporting them as
/// `<prefix>/<file name>`. A missing directory contributes nothing;
/// non-`.tel` entries and subdirectories are ignored.
fn collect_tel_files(dir: &Path, prefix: &str, out: &mut Vec<RepoPath>) -> Result<(), TelosError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {e}", dir.display()),
            ));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {e}", dir.display()),
            )
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".tel") {
            out.push(RepoPath::new(format!("{prefix}/{name}")));
        }
    }
    Ok(())
}

fn telos_error_as_diagnostic(e: TelosError) -> Diagnostic {
    Diagnostic {
        code: e.code,
        message: e.message,
        hint: e.hint,
        file: None,
        line: None,
        col: None,
    }
}
