//! Complete Git-aware repository inventory, independent of behavior bindings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, TelosError};
use crate::git::GitRepo;
use crate::ids::RepoPath;

pub type Snapshot = BTreeMap<String, FileState>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileState {
    pub oid: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChange {
    pub path: String,
    pub before: Option<FileState>,
    pub after: Option<FileState>,
}

/// Work records have their own integrity rules and cannot recursively seal
/// themselves. Tracked build outputs and ignored-but-tracked files are managed.
pub fn is_managed(path: &str) -> bool {
    if matches!(
        path,
        "telos/telos.lock"
            | "telos/ledger.tel"
            | "telos/changes/counters.toml"
            | ".telos-init.json"
            | ".git"
    ) || path.starts_with("telos/.runtime/")
        || path.starts_with(".git/")
    {
        return false;
    }
    for (directory, prefix) in [
        ("telos/plans/", "PLN"),
        ("telos/changes/", "CHG"),
        ("telos/history/", "CHG"),
    ] {
        if let Some(id) = path
            .strip_prefix(directory)
            .and_then(|n| n.strip_suffix(".tel"))
            && crate::work::validate_id(prefix, id).is_ok()
        {
            return false;
        }
    }
    true
}

/// Runtime recovery material is local infrastructure, never versioned content.
/// Refuse tracked occupants rather than creating an ungoverned path namespace.
fn validate_versioned_path(path: &str) -> Result<(), TelosError> {
    if path == ".telos-init.json" || path == "telos/.runtime" || path.starts_with("telos/.runtime/")
    {
        return Err(TelosError::new(
            ErrorCode::TelosUnplannedChange,
            format!("local protocol state `{path}` must not be tracked by Git"),
        )
        .hint("remove this path from the Git index; keep recovery data local"));
    }
    Ok(())
}

pub fn head(root: &Path) -> Result<Option<String>, TelosError> {
    let out = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|e| git_error(e.to_string()))?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

pub fn capture(root: &Path) -> Result<Snapshot, TelosError> {
    let git = GitRepo::discover(root)?;
    git.ensure_matches_workspace_root(root)?;
    let index = index_entries(root)?;
    for path in index.keys() {
        validate_versioned_path(path)?;
    }
    let names = git_output(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut regular = Vec::new();
    let mut snapshot = Snapshot::new();
    let mut seen = BTreeSet::new();
    let fs = crate::repo_fs::RepoFs::open(root)?;
    for raw in names.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = path_text(raw)?;
        if !is_managed(&path) || !seen.insert(path.clone()) {
            continue;
        }
        let safe = RepoPath::parse(&path)?;
        fs.validate_parents(&safe)?;
        let metadata = match fs::symlink_metadata(root.join(&path)) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(git_error(format!("cannot inspect `{path}`: {e}"))),
        };
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(root.join(&path)).map_err(|e| git_error(e.to_string()))?;
            let target = target
                .to_str()
                .ok_or_else(|| git_error("non-UTF-8 symlink targets are not supported"))?;
            let oid = hash_bytes(root, None, target.as_bytes(), false)?;
            snapshot.insert(
                path,
                FileState {
                    oid,
                    mode: "120000".into(),
                },
            );
        } else if metadata.is_dir() {
            if index.get(&path).is_some_and(|s| s.mode == "160000") {
                let oid = head(&root.join(&path))?
                    .ok_or_else(|| git_error(format!("submodule `{path}` has no HEAD")))?;
                snapshot.insert(
                    path,
                    FileState {
                        oid,
                        mode: "160000".into(),
                    },
                );
            } else {
                return Err(git_error(format!("tracked path `{path}` is not a file")));
            }
        } else if metadata.is_file() {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    "100755"
                } else {
                    "100644"
                }
            };
            #[cfg(not(unix))]
            let mode = index.get(&path).map_or("100644", |s| s.mode.as_str());
            snapshot.insert(
                path,
                FileState {
                    oid: String::new(),
                    mode: mode.to_owned(),
                },
            );
            regular.push(safe);
        } else {
            return Err(git_error(format!("unsupported file type at `{path}`")));
        }
    }
    for (path, oid) in git.blob_oids(&regular)? {
        snapshot
            .get_mut(path.as_str())
            .expect("inventoried file")
            .oid = oid.0;
    }
    if snapshot.values().any(|s| s.oid.is_empty()) {
        return Err(git_error(
            "repository changed while its inventory was being captured",
        ));
    }
    Ok(snapshot)
}

pub fn at_commit(root: &Path, commit: &str) -> Result<Snapshot, TelosError> {
    if commit.starts_with('-') || commit.contains(':') || commit.chars().any(char::is_control) {
        return Err(git_error("invalid base commit"));
    }
    let resolved = git_output(
        root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )?;
    let oid = String::from_utf8_lossy(&resolved).trim().to_owned();
    let entries = git_output(root, &["ls-tree", "-r", "-z", &oid])?;
    let mut snapshot = Snapshot::new();
    for entry in entries.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let (header, path) = split_entry(entry)?;
        let words: Vec<_> = header.split_whitespace().collect();
        if words.len() != 3 {
            return Err(git_error("invalid Git tree entry"));
        }
        validate_versioned_path(&path)?;
        if is_managed(&path) {
            snapshot.insert(
                path,
                FileState {
                    mode: words[0].into(),
                    oid: words[2].into(),
                },
            );
        }
    }
    Ok(snapshot)
}

pub fn changes(before: &Snapshot, after: &Snapshot) -> Vec<FileChange> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .map(|path| FileChange {
            path: path.clone(),
            before: before.get(path).cloned(),
            after: after.get(path).cloned(),
        })
        .collect()
}

pub fn store(root: &Path, snapshot: &Snapshot) -> Result<(), TelosError> {
    let paths = snapshot
        .iter()
        .filter(|(_, state)| state.mode.starts_with("100"))
        .map(|(path, _)| RepoPath::parse(path))
        .collect::<Result<Vec<_>, _>>()?;
    let git = GitRepo::discover(root)?;
    git.store_blobs(&paths)?;
    for (path, oid) in git.blob_oids(&paths)? {
        if snapshot
            .get(path.as_str())
            .is_none_or(|state| state.oid != oid.0)
        {
            return Err(git_error("repository changed while storing its inventory"));
        }
    }
    Ok(())
}

pub fn restore(root: &Path, baseline: &Snapshot) -> Result<Vec<FileChange>, TelosError> {
    let writer = crate::transaction::Writer::acquire(root)?;
    let delta = changes(&capture(root)?, baseline);
    let git = GitRepo::discover(root)?;
    let mut writes = Vec::new();
    for change in &delta {
        if change
            .before
            .iter()
            .chain(&change.after)
            .any(|s| !s.mode.starts_with("100"))
        {
            return Err(git_error(format!(
                "restore symlink or submodule `{}` explicitly within the approved recovery task",
                change.path
            )));
        }
        if change
            .before
            .as_ref()
            .zip(change.after.as_ref())
            .is_some_and(|(before, after)| before.mode != after.mode)
        {
            return Err(git_error(format!(
                "restore the executable mode of `{}` explicitly within the approved recovery task",
                change.path
            )));
        }
        let bytes = change
            .after
            .as_ref()
            .map(|s| git.cat_blob(&crate::git::Oid(s.oid.clone())))
            .transpose()?;
        writes.push((RepoPath::parse(&change.path)?, bytes));
    }
    writer.publish(writes)?;
    Ok(delta)
}

pub fn hash_bytes(
    root: &Path,
    path: Option<&str>,
    bytes: &[u8],
    store: bool,
) -> Result<String, TelosError> {
    let mut cmd = Command::new("git");
    cmd.arg("hash-object").arg("--stdin").current_dir(root);
    if store {
        cmd.arg("-w");
    }
    if let Some(path) = path {
        RepoPath::parse(path)?;
        cmd.arg(format!("--path={path}"));
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| git_error(e.to_string()))?;
    let mut input = child.stdin.take().expect("piped input");
    let bytes = bytes.to_vec();
    let writer = std::thread::spawn(move || input.write_all(&bytes));
    let output = child
        .wait_with_output()
        .map_err(|e| git_error(e.to_string()))?;
    writer
        .join()
        .map_err(|_| git_error("Git input writer panicked"))?
        .map_err(|e| git_error(e.to_string()))?;
    if !output.status.success() {
        return Err(git_error(String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, TelosError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| git_error(e.to_string()))?;
    if !out.status.success() {
        return Err(git_error(String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn index_entries(root: &Path) -> Result<Snapshot, TelosError> {
    let bytes = git_output(root, &["ls-files", "--stage", "-z"])?;
    let mut result = Snapshot::new();
    for entry in bytes.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let (header, path) = split_entry(entry)?;
        let words: Vec<_> = header.split_whitespace().collect();
        if words.len() != 3 || words[2] != "0" {
            return Err(git_error("resolve Git index conflicts before continuing"));
        }
        result.insert(
            path,
            FileState {
                mode: words[0].into(),
                oid: words[1].into(),
            },
        );
    }
    Ok(result)
}

fn split_entry(entry: &[u8]) -> Result<(&str, String), TelosError> {
    let separator = entry
        .iter()
        .position(|&b| b == b'\t')
        .ok_or_else(|| git_error("invalid Git entry"))?;
    let header = std::str::from_utf8(&entry[..separator]).map_err(|e| git_error(e.to_string()))?;
    Ok((header, path_text(&entry[separator + 1..])?))
}

fn path_text(bytes: &[u8]) -> Result<String, TelosError> {
    let path = std::str::from_utf8(bytes)
        .map_err(|_| git_error("non-UTF-8 repository paths are not supported"))?;
    RepoPath::parse(path)?;
    Ok(path.to_owned())
}

fn git_error(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosGitError, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_ignored_docs_and_file_deletions_remain_managed() {
        let dir = tempfile::tempdir().unwrap();
        git_output(dir.path(), &["init", "--quiet"]).unwrap();
        fs::write(dir.path().join("README.md"), "first").unwrap();
        git_output(dir.path(), &["add", "README.md"]).unwrap();
        fs::write(dir.path().join(".gitignore"), "README.md\ncache/\n").unwrap();
        fs::create_dir(dir.path().join("cache")).unwrap();
        fs::write(dir.path().join("cache/output"), "ignored").unwrap();
        let before = capture(dir.path()).unwrap();
        assert!(before.contains_key("README.md"));
        assert!(!before.contains_key("cache/output"));
        fs::remove_file(dir.path().join("README.md")).unwrap();
        let delta = changes(&before, &capture(dir.path()).unwrap());
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].path, "README.md");
        assert!(delta[0].after.is_none());
    }
}
