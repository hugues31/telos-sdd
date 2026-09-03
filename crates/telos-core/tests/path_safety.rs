use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use telos_core::adopt::revert;
use telos_core::config::Config;
use telos_core::git::{GitRepo, Oid};
use telos_core::ids::RepoPath;
use telos_core::lock::{LOCK_VERSION, Lock};
use telos_core::model::Evidence;
use telos_core::state::{DriftEntry, DriftKind};
use telos_core::workspace::Workspace;

fn git_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(tmp.path())
            .status()
            .unwrap()
            .success()
    );
    tmp
}

#[test]
fn git_hashing_rejects_an_in_repo_symlink_to_outside() {
    let repo = git_repo();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::create_dir(repo.path().join("tests")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), repo.path().join("tests/proof.rs")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(outside.path(), repo.path().join("tests/proof.rs")).unwrap();

    let git = GitRepo::discover(repo.path()).unwrap();
    let error = git
        .blob_oids(&[RepoPath::parse("tests/proof.rs").unwrap()])
        .unwrap_err();

    assert!(
        error.message.contains("outside the repository"),
        "{error:?}"
    );
}

#[test]
fn revert_never_changes_an_outside_owner_through_a_parent_symlink() {
    let repo = git_repo();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("owner.rs"), b"outside owner\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), repo.path().join("escape")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(outside.path(), repo.path().join("escape")).unwrap();

    let ws = Workspace {
        repo_root: repo.path().to_path_buf(),
        telos_dir: repo.path().join("telos"),
        config: Config::default(),
    };
    let git = GitRepo::discover(repo.path()).unwrap();
    let path = RepoPath::parse("escape/owner.rs").unwrap();
    let lock = Lock {
        version: LOCK_VERSION,
        tool: "test".into(),
        sealed_by: None,
        spec_digest: Lock::compute_digest(&BTreeMap::new()),
        proof_evidence: Evidence::ExitStatus,
        spec: BTreeMap::new(),
        code: BTreeMap::from([(path.clone(), Oid("0".repeat(40)))]),
    };

    let error = revert(
        &ws,
        &git,
        &lock,
        &[DriftEntry {
            path,
            kind: DriftKind::Untracked,
        }],
    )
    .unwrap_err();

    assert!(error.message.contains("escape/owner.rs"), "{error:?}");
    assert_eq!(
        fs::read(outside.path().join("owner.rs")).unwrap(),
        b"outside owner\n"
    );
}
