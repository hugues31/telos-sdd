//! Workspace discovery and model loading: the filesystem entry point that
//! turns a starting directory into a parsed [`TelosModel`].
//!
//! Everything here is I/O -- reading `telos.toml`, walking directories,
//! reading `.tel` files -- and nothing here is git-aware or lock-aware.
//! Checking that `repo_root` matches the git repository root is
//! [`crate::git::GitRepo`]'s job; reading `telos.lock` and sealing are
//! [`crate::lock`]'s.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::model::{TelFile, TelosModel};
use crate::repo_fs::RepoFs;
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

/// The one spec file no entity owns: the bindings table, sealed with the
/// rest of the tree and *derived* at reconcile from the folded journal,
/// never claimed by a change.
pub const BINDINGS_PATH: &str = "telos/bindings.tel";

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
            let safe_root = RepoFs::open(&dir)?;
            let config_path = RepoPath::new("telos/telos.toml");
            if let Some(bytes) = safe_root.read_optional(&config_path)? {
                let config = load_config_bytes(&bytes)?;
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
            files.push(RepoPath::new(BINDINGS_PATH));
        }

        files.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(files)
    }

    /// Reads and parses every spec file, in [`Workspace::spec_files`] order.
    ///
    /// Parse diagnostics from every file are collected (every file is parsed
    /// even if another one already failed); if any exist, they are returned
    /// instead of a partial list, since a file that failed to parse cannot
    /// be resolved against.
    ///
    /// This is the half of [`load_model`](Workspace::load_model) that stops
    /// short of the semantic pass. The overlay ([`crate::overlay`]) needs
    /// exactly that: the parsed base, so a
    /// change's staged ops can be applied to it *before* a model is built
    /// from the result.
    pub fn parse_spec_files(&self) -> Result<Vec<(RepoPath, TelFile)>, Vec<Diagnostic>> {
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

            let src = match self.read_to_string(&repo_path) {
                Ok(src) => src,
                Err(e) => {
                    diagnostics.push(Diagnostic {
                        code: ErrorCode::TelosInternal,
                        message: format!("failed to read {repo_path}: {}", e.message),
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

        if diagnostics.is_empty() {
            Ok(parsed)
        } else {
            Err(diagnostics)
        }
    }

    /// Reads and parses every spec file, then folds them into a
    /// [`TelosModel`] via [`build_model`].
    ///
    /// Does not read `telos.lock` and does not touch git.
    pub fn load_model(&self) -> Result<TelosModel, Vec<Diagnostic>> {
        build_model(self.parse_spec_files()?)
    }

    /// `<telos_dir>/telos.lock`.
    pub fn lock_path(&self) -> PathBuf {
        self.telos_dir.join("telos.lock")
    }

    /// Converts a repo-relative `'/'`-separated path into an absolute,
    /// OS-native path under `repo_root`.
    ///
    /// Public because [`crate::reconcile`] writes the spec files a change's
    /// ops name, and an op names its target as a [`RepoPath`] -- resolving
    /// it anywhere else would be a second, independently-maintained answer
    /// to "where does this path live on this OS".
    pub fn abs_path(&self, repo_path: &RepoPath) -> Result<PathBuf, TelosError> {
        repo_path.validate()?;
        let mut path = self.repo_root.clone();
        for component in repo_path.as_str().split('/') {
            path.push(component);
        }
        Ok(path)
    }

    /// Reads a repository file through a root capability and refuses every
    /// symlink encountered along the path.
    pub fn read_bytes(&self, repo_path: &RepoPath) -> Result<Vec<u8>, TelosError> {
        RepoFs::open(&self.repo_root)?.read(repo_path)
    }

    pub fn read_optional_bytes(&self, repo_path: &RepoPath) -> Result<Option<Vec<u8>>, TelosError> {
        RepoFs::open(&self.repo_root)?.read_optional(repo_path)
    }

    pub fn read_to_string(&self, repo_path: &RepoPath) -> Result<String, TelosError> {
        let bytes = self.read_bytes(repo_path)?;
        String::from_utf8(bytes).map_err(|error| {
            TelosError::new(
                ErrorCode::TelosParseError,
                format!("repository path `{repo_path}` is not UTF-8: {error}"),
            )
        })
    }
}

/// Routes a spec file to the parser its location calls for:
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
fn load_config_bytes(bytes: &[u8]) -> Result<Config, TelosError> {
    let src = std::str::from_utf8(bytes).map_err(|e| {
        TelosError::new(ErrorCode::TelosParseError, format!("telos/telos.toml: {e}"))
    })?;
    let mut config: Config = toml::from_str(src).map_err(|e| {
        TelosError::new(ErrorCode::TelosParseError, format!("telos/telos.toml: {e}"))
    })?;
    config.normalize();
    Ok(config)
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
        if !entry
            .file_type()
            .map_err(|e| {
                TelosError::new(
                    ErrorCode::TelosInternal,
                    format!("failed to inspect {}: {e}", entry.path().display()),
                )
            })?
            .is_file()
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.ends_with(".tel") {
            out.push(RepoPath::parse(format!("{prefix}/{name}"))?);
        }
    }
    Ok(())
}

/// Lifts a [`TelosError`] into a file-less [`Diagnostic`], for
/// [`Workspace::parse_spec_files`] -- which must answer with a diagnostics
/// list but can fail with a bare error, when listing the spec files is what
/// went wrong rather than parsing one.
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
