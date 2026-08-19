//! `telos init`: turn a git repository into a telos project.
//!
//! Everything is created at the *git* root, not at the current directory: a
//! lock seals repo-relative paths (`telos/notions/Invoice.tel`), and those
//! paths are only meaningful -- and only hashable by `git hash-object` --
//! relative to the worktree root. A `telos/` nested somewhere below it would
//! seal paths git cannot resolve.
//!
//! The order below matters: `.gitattributes` is written *before* sealing, so
//! that the very first seal already runs the `eol=lf` clean filter and the
//! OIDs it records are the cross-OS ones.

use std::fs;
use std::path::Path;

use serde_json::json;

use telos_core::counters::{Counters, write_counters};
use telos_core::error::{Diagnostic, ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::lock::seal;
use telos_core::workspace::Workspace;

use crate::commands::Ctx;
use crate::envelope::{CmdResult, Outcome};

/// The `telos.toml` a fresh project starts with: every section present and
/// empty, so a reader sees the shape without consulting the documentation.
const DEFAULT_CONFIG: &str = "\
[code]
globs = []

[tests]
globs = []

[test]
cmd = \"\"

[policy]
tdd = \"strict\"
";

/// Pins the byte identity of the spec across operating systems: whatever a
/// checkout does to line endings, the blob git hashes for a `telos/` file is
/// the LF one, so a lock sealed on Linux verifies on Windows.
const GITATTRIBUTES_LINE: &str = "telos/** text eol=lf";

/// The spec subdirectories a project always has, created empty.
const SUBDIRS: [&str; 4] = ["notions", "intents", "constraints", "changes"];

pub fn run(ctx: &Ctx) -> CmdResult {
    let git = GitRepo::discover(&ctx.cwd)?;
    let root = git.root().to_path_buf();
    let telos_dir = root.join("telos");
    let config_path = telos_dir.join("telos.toml");

    if config_path.exists() {
        return Err(TelosError::new(
            ErrorCode::TelosAlreadyInitialized,
            format!("`{}` already exists", display_path(&config_path)),
        )
        .hint("project already initialized; see `telos status`"));
    }

    for subdir in SUBDIRS {
        let path = telos_dir.join(subdir);
        fs::create_dir_all(&path).map_err(|e| io_error("create", &path, e))?;
    }
    write(&config_path, DEFAULT_CONFIG)?;
    // Empty to the byte: `bindings.tel` must seal as git's empty blob,
    // `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`, on every OS.
    write(&telos_dir.join("bindings.tel"), "")?;
    ensure_gitattributes(&root)?;

    let ws = Workspace::discover(&root)?;
    // Seeded before sealing (D4): `telos/changes/` is excluded from
    // `Workspace::spec_files`, so this write never enters the seal.
    write_counters(&ws, &Counters::default())?;
    let model = ws.load_model().map_err(first_error)?;
    let lock = seal(&ws, &model, &git, None)?;
    lock.write(&ws.lock_path())?;

    Ok(Outcome {
        result: json!({ "root": "telos", "sealed": true }),
        human: "initialized telos/ and sealed telos/telos.lock".to_string(),
        next_actions: vec!["telos status".to_string()],
    })
}

/// Makes sure `.gitattributes` at the repository root carries
/// [`GITATTRIBUTES_LINE`], creating the file if it is absent and appending to
/// it otherwise -- never rewriting what is already there. An existing file
/// that does not end in a newline gets one first, so the append cannot glue
/// itself onto somebody else's rule. Already present (on a line of its own,
/// whitespace aside) means there is nothing to do.
fn ensure_gitattributes(root: &Path) -> Result<(), TelosError> {
    let path = root.join(".gitattributes");

    let mut content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_error("read", &path, e)),
    };

    // `lines()` strips the `\r` of a CRLF file too, so a checkout with
    // Windows endings is recognized as already carrying the rule.
    if content
        .lines()
        .any(|line| line.trim() == GITATTRIBUTES_LINE)
    {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(GITATTRIBUTES_LINE);
    content.push('\n');

    write(&path, &content)
}

fn write(path: &Path, content: &str) -> Result<(), TelosError> {
    fs::write(path, content).map_err(|e| io_error("write", path, e))
}

fn io_error(verb: &str, path: &Path, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {e}", display_path(path)),
    )
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// A fresh project cannot fail to load -- but `load_model` reports
/// diagnostics, and the envelope surfaces one error, so the first diagnostic
/// is what a caller sees.
fn first_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    diagnostics
        .into_iter()
        .next()
        .map(TelosError::from)
        .unwrap_or_else(|| {
            TelosError::new(
                ErrorCode::TelosInternal,
                "the workspace failed to load without reporting a diagnostic",
            )
        })
}
