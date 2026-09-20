//! Recoverable publication of a set of repository files.
//!
//! A durable manifest contains preimages and postimages. Nothing is published
//! before the commit decision. Once committed, recovery rolls forward and
//! refuses to overwrite bytes that match neither image. The journal is local
//! and self-contained; it never depends on unreferenced Git objects.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::repo_fs::RepoFs;
use crate::work::{digest, new_id, validate_id};

const MANIFEST: &str = "telos/.runtime/transaction.json";
const COMMIT: &str = "telos/.runtime/transaction.commit";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    id: String,
    entries: Vec<Entry>,
}

pub struct Writer {
    root: PathBuf,
    fs: RepoFs,
    _lock: Arc<WorktreeLock>,
}

struct WorktreeLock(fs::File);
impl Drop for WorktreeLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

thread_local! {
    static HELD: RefCell<BTreeMap<PathBuf, Weak<WorktreeLock>>> = const { RefCell::new(BTreeMap::new()) };
}

impl Writer {
    pub fn acquire(root: &Path) -> Result<Self, TelosError> {
        let fs = RepoFs::open(root)?;
        let key = root.canonicalize().map_err(|e| invalid(e.to_string()))?;
        let lock = HELD.with(|held| -> Result<Arc<WorktreeLock>, TelosError> {
            let mut held = held.borrow_mut();
            if let Some(existing) = held.get(&key).and_then(Weak::upgrade) {
                return Ok(existing);
            }
            let lock = Arc::new(WorktreeLock(fs.writer_lock()?));
            held.insert(key, Arc::downgrade(&lock));
            Ok(lock)
        })?;
        require_recovered(root)?;
        Ok(Self {
            root: root.to_owned(),
            fs,
            _lock: lock,
        })
    }

    pub fn read(&self, path: &RepoPath) -> Result<Option<Vec<u8>>, TelosError> {
        self.fs.read_optional(path)
    }

    /// Callers validate their optimistic versions while holding this writer.
    pub fn publish(&self, writes: Vec<(RepoPath, Option<Vec<u8>>)>) -> Result<String, TelosError> {
        self.publish_with(writes, |_| Ok(()))
    }

    fn publish_with(
        &self,
        writes: Vec<(RepoPath, Option<Vec<u8>>)>,
        mut boundary: impl FnMut(usize) -> Result<(), TelosError>,
    ) -> Result<String, TelosError> {
        require_recovered(&self.root)?;
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        for (path, after) in writes {
            path.validate()?;
            if path.as_str().starts_with("telos/.runtime/")
                || path.first_component() == Some(".git")
            {
                return Err(invalid(
                    "transaction destinations cannot target runtime or Git metadata",
                ));
            }
            if !seen.insert(path.clone()) {
                return Err(invalid(format!(
                    "duplicate transaction destination `{path}`"
                )));
            }
            self.fs.validate_writable(&path)?;
            let before = self.fs.read_optional(&path)?;
            if before != after {
                entries.push(Entry {
                    path: path.to_string(),
                    before,
                    after,
                });
            }
        }
        let manifest = Manifest {
            format: 1,
            id: new_id("TXN")?,
            entries,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(|e| invalid(e.to_string()))?;
        self.fs.atomic_write(&RepoPath::new(MANIFEST), &bytes)?;
        boundary(0)?;
        self.fs
            .atomic_write(&RepoPath::new(COMMIT), digest(&manifest)?.as_bytes())?;
        boundary(1)?;
        apply(&self.fs, &manifest, |n| boundary(n + 2))?;
        // Deleting the manifest is the completion marker. A surviving commit
        // marker with no manifest means all destination writes were durable.
        self.fs.remove_file(&RepoPath::new(MANIFEST))?;
        self.fs.remove_file(&RepoPath::new(COMMIT))?;
        Ok(manifest.id)
    }
}

pub fn require_recovered(root: &Path) -> Result<(), TelosError> {
    let fs = RepoFs::open(root)?;
    if fs.read_optional(&RepoPath::new(MANIFEST))?.is_some()
        || fs.read_optional(&RepoPath::new(COMMIT))?.is_some()
    {
        return Err(TelosError::new(
            ErrorCode::TelosRecoveryRequired,
            "an interrupted Telos publication requires recovery",
        )
        .hint("run `telos recover` before continuing"));
    }
    Ok(())
}

pub fn recover(root: &Path) -> Result<Option<String>, TelosError> {
    let fs = RepoFs::open(root)?;
    let _lock = WorktreeLock(fs.writer_lock()?);
    let bytes = fs.read_optional(&RepoPath::new(MANIFEST))?;
    let commit = fs.read_optional(&RepoPath::new(COMMIT))?;
    let Some(bytes) = bytes else {
        if commit.is_some() {
            fs.remove_file(&RepoPath::new(COMMIT))?;
        }
        return Ok(None);
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| invalid(format!("invalid recovery manifest: {e}")))?;
    validate_id("TXN", &manifest.id)?;
    if manifest.format != 1 {
        return Err(invalid("unsupported transaction format"));
    }
    if let Some(commit) = commit {
        if commit != digest(&manifest)?.as_bytes() {
            return Err(invalid(
                "recovery manifest does not match its commit decision",
            ));
        }
        // Validate every path and preimage before performing any recovery write.
        let mut paths = BTreeSet::new();
        for entry in &manifest.entries {
            let path = RepoPath::parse(&entry.path)?;
            if path.first_component() == Some(".git")
                || path.as_str().starts_with("telos/.runtime/")
                || !paths.insert(path.clone())
            {
                return Err(invalid("unsafe or duplicate recovery destination"));
            }
            check_image(&fs, &path, entry)?;
        }
        apply(&fs, &manifest, |_| Ok(()))?;
    }
    fs.remove_file(&RepoPath::new(MANIFEST))?;
    fs.remove_file(&RepoPath::new(COMMIT))?;
    Ok(Some(manifest.id))
}

fn check_image(fs: &RepoFs, path: &RepoPath, entry: &Entry) -> Result<(), TelosError> {
    let current = fs.read_optional(path)?;
    if current != entry.before && current != entry.after {
        return Err(TelosError::new(
            ErrorCode::TelosRecoveryConflict,
            format!("`{path}` matches neither the original nor the intended transaction content"),
        )
        .hint("preserve the external edit, resolve the conflict, then retry `telos recover`"));
    }
    Ok(())
}

fn apply(
    fs: &RepoFs,
    manifest: &Manifest,
    mut boundary: impl FnMut(usize) -> Result<(), TelosError>,
) -> Result<(), TelosError> {
    for (n, entry) in manifest.entries.iter().enumerate() {
        let path = RepoPath::parse(&entry.path)?;
        check_image(fs, &path, entry)?;
        if fs.read_optional(&path)? != entry.after {
            match &entry.after {
                Some(bytes) => fs.atomic_write(&path, bytes)?,
                None => fs.remove_file(&path)?,
            }
        }
        boundary(n)?;
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosIntegrityViolation, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interrupted() -> TelosError {
        TelosError::new(ErrorCode::TelosInternal, "injected interruption")
    }

    #[test]
    fn every_publication_boundary_recovers_to_one_complete_state() {
        for failure in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("first"), b"old").unwrap();
            let writer = Writer::acquire(dir.path()).unwrap();
            let writes = vec![
                (RepoPath::new("first"), Some(b"new".to_vec())),
                (RepoPath::new("second"), Some(b"second".to_vec())),
            ];
            assert!(
                writer
                    .publish_with(writes, |n| if n == failure {
                        Err(interrupted())
                    } else {
                        Ok(())
                    })
                    .is_err()
            );
            drop(writer);
            assert!(require_recovered(dir.path()).is_err());
            recover(dir.path()).unwrap();
            require_recovered(dir.path()).unwrap();
            assert_eq!(
                fs::read(dir.path().join("first")).unwrap(),
                if failure == 0 { b"old" } else { b"new" }
            );
            assert_eq!(dir.path().join("second").exists(), failure != 0);
            assert!(recover(dir.path()).unwrap().is_none());
        }
    }

    #[test]
    fn recovery_preserves_conflicting_external_edits() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("first"), b"old").unwrap();
        let writer = Writer::acquire(dir.path()).unwrap();
        let _ = writer.publish_with(vec![(RepoPath::new("first"), Some(b"new".to_vec()))], |n| {
            if n == 1 { Err(interrupted()) } else { Ok(()) }
        });
        drop(writer);
        fs::write(dir.path().join("first"), b"external").unwrap();
        assert_eq!(
            recover(dir.path()).unwrap_err().code,
            ErrorCode::TelosRecoveryConflict
        );
        assert_eq!(fs::read(dir.path().join("first")).unwrap(), b"external");
    }

    #[test]
    fn a_second_writer_cannot_overwrite_an_active_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let first = Writer::acquire(dir.path()).unwrap();
        let root = dir.path().to_owned();
        assert!(std::thread::spawn(move || matches!(Writer::acquire(&root), Err(e) if e.code == ErrorCode::TelosWorkspaceBusy)).join().unwrap());
        drop(first);
        Writer::acquire(dir.path()).unwrap();
    }

    #[test]
    fn crash_child() {
        let Ok(root) = std::env::var("TELOS_TEST_CRASH_ROOT") else {
            return;
        };
        let boundary: usize = std::env::var("TELOS_TEST_CRASH_BOUNDARY")
            .unwrap()
            .parse()
            .unwrap();
        let writer = Writer::acquire(Path::new(&root)).unwrap();
        writer
            .publish_with(
                vec![
                    (RepoPath::new("first"), Some(b"new".to_vec())),
                    (RepoPath::new("second"), Some(b"second".to_vec())),
                ],
                |n| {
                    if n == boundary {
                        std::process::exit(86);
                    }
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn actual_process_termination_at_each_publication_boundary_is_recoverable() {
        for boundary in 0..4 {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("first"), b"old").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "transaction::tests::crash_child", "--nocapture"])
                .env("TELOS_TEST_CRASH_ROOT", root.path())
                .env("TELOS_TEST_CRASH_BOUNDARY", boundary.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            assert_eq!(
                require_recovered(root.path()).unwrap_err().code,
                ErrorCode::TelosRecoveryRequired
            );
            recover(root.path()).unwrap();
            assert_eq!(
                fs::read(root.path().join("first")).unwrap(),
                if boundary == 0 { b"old" } else { b"new" }
            );
            assert_eq!(root.path().join("second").exists(), boundary != 0);
        }
    }
}
