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
use crate::ids::{CapabilityRef, ConstraintId, ContextId, IntentId, NotionName, Owner, RepoPath};
use crate::model::{TelFile, TelosModel};
use crate::repo_fs::RepoFs;
use crate::semantic::build_model;
use crate::syntax::{
    parse_bindings_file, parse_capability_file, parse_context_file, parse_context_map_file,
    parse_owned_constraint_file, parse_owned_intent_file, parse_owned_notion_file,
};

/// A discovered telos-sdd project: its root, the `telos/` directory inside
/// it, and the parsed `telos.toml` configuration.
#[derive(Debug)]
pub struct Workspace {
    pub repo_root: PathBuf,
    pub telos_dir: PathBuf,
    pub config: Config,
}

const LEGACY_PATHS: [&str; 4] = ["notions", "intents", "bindings.tel", "constraints-legacy"];
/// Internal compatibility path for in-memory overlay fixtures. Discovery
/// rejects it; real bindings are context-local in Telos 0.9.
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

        for legacy in LEGACY_PATHS {
            let actual = if legacy == "constraints-legacy" {
                self.telos_dir.join("constraints")
            } else {
                self.telos_dir.join(legacy)
            };
            if actual.exists() && legacy != "constraints-legacy" {
                return Err(TelosError::new(
                    ErrorCode::TelosLayoutViolation,
                    format!("legacy layout `telos/{legacy}` is not supported by Telos 0.9"),
                )
                .hint(
                    "move specifications under telos/contexts/<context>/ and regenerate telos.lock",
                ));
            }
        }

        collect_tel_files_recursive(
            &self.telos_dir.join("contexts"),
            "telos/contexts",
            &mut files,
        )?;
        collect_tel_files_direct(
            &self.telos_dir.join("constraints"),
            "telos/constraints",
            &mut files,
        )?;
        if self.telos_dir.join("context-map.tel").is_file() {
            files.push(RepoPath::new("telos/context-map.tel"));
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

/// Parses one specification using its canonical path as part of the contract.
///
/// Unlike the low-level syntax entry points, this also verifies that the
/// declared identity and owner agree with the context/capability hierarchy in
/// `repo_path`. It is public so tooling and tests can validate an in-memory
/// canonical specification without constructing a temporary workspace.
pub fn parse_spec_file(repo_path: &RepoPath, src: &str) -> Result<TelFile, Vec<Diagnostic>> {
    let route = classify(repo_path).map_err(|error| vec![layout_diagnostic(repo_path, error)])?;
    match route {
        SpecRoute::Context(expected) => parse_context_file(repo_path, src).and_then(|context| {
            ensure_layout(
                repo_path,
                context.id == expected,
                format!("expected context `{expected}`"),
            )?;
            Ok(TelFile::Context(context))
        }),
        SpecRoute::Capability(expected) => {
            parse_capability_file(repo_path, src).and_then(|capability| {
                ensure_layout(
                    repo_path,
                    capability.id == expected,
                    format!("expected capability `{expected}`"),
                )?;
                Ok(TelFile::Capability(capability))
            })
        }
        SpecRoute::Notion(expected_owner, expected_name) => parse_owned_notion_file(repo_path, src)
            .and_then(|(owner, notion)| {
                ensure_layout(
                    repo_path,
                    owner == expected_owner,
                    format!("expected owner `{expected_owner}`"),
                )?;
                ensure_layout(
                    repo_path,
                    notion.name == expected_name,
                    format!("expected notion `{expected_name}`"),
                )?;
                Ok(TelFile::OwnedNotion { owner, notion })
            }),
        SpecRoute::Intent(expected_owner, expected_id) => parse_owned_intent_file(repo_path, src)
            .and_then(|(owner, intent)| {
                ensure_layout(
                    repo_path,
                    owner == expected_owner,
                    format!("expected owner `{expected_owner}`"),
                )?;
                ensure_layout(
                    repo_path,
                    intent.id == expected_id,
                    format!("expected intent `{expected_id}`"),
                )?;
                Ok(TelFile::OwnedIntent { owner, intent })
            }),
        SpecRoute::Constraint(expected_owner, expected_id) => {
            parse_owned_constraint_file(repo_path, src).and_then(|(owner, constraint)| {
                ensure_layout(
                    repo_path,
                    owner == expected_owner,
                    format!(
                        "expected owner `{}`",
                        expected_owner
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "project".to_string())
                    ),
                )?;
                ensure_layout(
                    repo_path,
                    constraint.id == expected_id,
                    format!("expected constraint `{expected_id}`"),
                )?;
                Ok(TelFile::OwnedConstraint { owner, constraint })
            })
        }
        SpecRoute::Bindings(context) => parse_bindings_file(repo_path, src)
            .map(|bindings| TelFile::ContextBindings { context, bindings }),
        SpecRoute::ContextMap => parse_context_map_file(repo_path, src).map(TelFile::ContextMap),
    }
}

enum SpecRoute {
    Context(ContextId),
    Capability(CapabilityRef),
    Notion(Owner, NotionName),
    Intent(Owner, IntentId),
    Constraint(Option<Owner>, ConstraintId),
    Bindings(ContextId),
    ContextMap,
}

fn classify(path: &RepoPath) -> Result<SpecRoute, String> {
    let parts: Vec<&str> = path.as_str().split('/').collect();
    match parts.as_slice() {
        ["telos", "context-map.tel"] => Ok(SpecRoute::ContextMap),
        ["telos", "constraints", file] => Ok(SpecRoute::Constraint(
            None,
            tel_stem(file)?.parse().map_err(|e: TelosError| e.message)?,
        )),
        ["telos", "contexts", context, "context.tel"] => Ok(SpecRoute::Context(
            ContextId::new(*context).map_err(|e| e.message)?,
        )),
        ["telos", "contexts", context, "bindings.tel"] => Ok(SpecRoute::Bindings(
            ContextId::new(*context).map_err(|e| e.message)?,
        )),
        ["telos", "contexts", context, "notions", file] => Ok(SpecRoute::Notion(
            Owner::context(ContextId::new(*context).map_err(|e| e.message)?),
            NotionName::new(tel_stem(file)?).map_err(|e| e.message)?,
        )),
        ["telos", "contexts", context, "constraints", file] => Ok(SpecRoute::Constraint(
            Some(Owner::context(
                ContextId::new(*context).map_err(|e| e.message)?,
            )),
            tel_stem(file)?.parse().map_err(|e: TelosError| e.message)?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "capability.tel",
        ] => Ok(SpecRoute::Capability(CapabilityRef::new(
            ContextId::new(*context).map_err(|e| e.message)?,
            crate::ids::CapabilityId::new(*capability).map_err(|e| e.message)?,
        ))),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "notions",
            file,
        ] => Ok(SpecRoute::Notion(
            Owner::capability(CapabilityRef::new(
                ContextId::new(*context).map_err(|e| e.message)?,
                crate::ids::CapabilityId::new(*capability).map_err(|e| e.message)?,
            )),
            NotionName::new(tel_stem(file)?).map_err(|e| e.message)?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "intents",
            file,
        ] => Ok(SpecRoute::Intent(
            Owner::capability(CapabilityRef::new(
                ContextId::new(*context).map_err(|e| e.message)?,
                crate::ids::CapabilityId::new(*capability).map_err(|e| e.message)?,
            )),
            tel_stem(file)?.parse().map_err(|e: TelosError| e.message)?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "constraints",
            file,
        ] => Ok(SpecRoute::Constraint(
            Some(Owner::capability(CapabilityRef::new(
                ContextId::new(*context).map_err(|e| e.message)?,
                crate::ids::CapabilityId::new(*capability).map_err(|e| e.message)?,
            ))),
            tel_stem(file)?.parse().map_err(|e: TelosError| e.message)?,
        )),
        _ => Err("file is outside the canonical context/capability layout".to_string()),
    }
}

fn tel_stem(value: &str) -> Result<&str, String> {
    value
        .strip_suffix(".tel")
        .ok_or_else(|| "expected a .tel file".to_string())
}

fn ensure_layout(path: &RepoPath, valid: bool, message: String) -> Result<(), Vec<Diagnostic>> {
    if valid {
        Ok(())
    } else {
        Err(vec![layout_diagnostic(path, message)])
    }
}

fn layout_diagnostic(path: &RepoPath, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: ErrorCode::TelosLayoutViolation,
        message: message.into(),
        hint: Some(
            "move the file to the path implied by its declared owner and identity".to_string(),
        ),
        file: Some(path.clone()),
        line: None,
        col: None,
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
fn collect_tel_files_direct(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<RepoPath>,
) -> Result<(), TelosError> {
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

fn collect_tel_files_recursive(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<RepoPath>,
) -> Result<(), TelosError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {error}", dir.display()),
            ));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {error}", dir.display()),
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to inspect {}: {error}", entry.path().display()),
            )
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let child_prefix = format!("{prefix}/{name}");
        if file_type.is_dir() {
            collect_tel_files_recursive(&entry.path(), &child_prefix, out)?;
        } else if file_type.is_file() && name.ends_with(".tel") {
            out.push(RepoPath::parse(child_prefix)?);
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
