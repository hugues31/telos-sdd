//! `GitRepo::blob_oids` and `Lock` against a real `git` binary -- this is
//! the whole point of Task 10: prove OIDs are filter-aware (same logical
//! content hashes the same regardless of the checkout's line endings) and
//! that hashing a batch of paths costs exactly one `git` process.
//!
//! Every test spins up its own throwaway repository under a `TempDir`; none
//! of them need a commit, since `git hash-object` reads the working tree
//! and `.gitattributes` directly.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use tempfile::TempDir;

use telos_core::error::ErrorCode;
use telos_core::git::{GitRepo, Oid};
use telos_core::ids::{ChangeId, RepoPath};
use telos_core::lock::Lock;

// --- fixture plumbing ------------------------------------------------------

/// A fresh `git init`ed repository, with just enough `user.*` config set
/// that git never balks (`hash-object` doesn't need it, but staying
/// consistent with a real checkout costs nothing).
fn init_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    run_git(tmp.path(), &["init", "--quiet"]);
    run_git(tmp.path(), &["config", "user.email", "test@example.com"]);
    run_git(tmp.path(), &["config", "user.name", "Test"]);
    tmp
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}

fn repo_paths(names: &[&str]) -> Vec<RepoPath> {
    names.iter().map(|n| RepoPath::new(*n)).collect()
}

// --- blob_oids: well-known OIDs --------------------------------------------

#[test]
fn blob_oids_of_hello_matches_the_well_known_git_oid() {
    let tmp = init_repo();
    fs::write(tmp.path().join("f.txt"), "hello\n").unwrap();

    let repo = GitRepo::discover(tmp.path()).unwrap();
    let oids = repo.blob_oids(&repo_paths(&["f.txt"])).unwrap();

    assert_eq!(
        oids.get(&RepoPath::new("f.txt")),
        Some(&Oid("ce013625030ba8dba906f756967f9e9ca394464a".to_string()))
    );
}

#[test]
fn blob_oids_of_an_empty_file_matches_the_well_known_git_oid() {
    let tmp = init_repo();
    fs::write(tmp.path().join("empty.txt"), "").unwrap();

    let repo = GitRepo::discover(tmp.path()).unwrap();
    let oids = repo.blob_oids(&repo_paths(&["empty.txt"])).unwrap();

    assert_eq!(
        oids.get(&RepoPath::new("empty.txt")),
        Some(&Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()))
    );
}

// --- blob_oids: one process per call, batched ------------------------------

#[test]
fn blob_oids_hashes_a_batch_in_one_call() {
    let tmp = init_repo();
    fs::write(tmp.path().join("a.txt"), "aaa\n").unwrap();
    fs::write(tmp.path().join("b.txt"), "bbb\n").unwrap();
    fs::write(tmp.path().join("c.txt"), "ccc\n").unwrap();

    let repo = GitRepo::discover(tmp.path()).unwrap();
    let oids = repo
        .blob_oids(&repo_paths(&["a.txt", "b.txt", "c.txt"]))
        .unwrap();

    // Each expected value is `git hash-object --stdin` computed
    // independently of `blob_oids` (a separate, unbatched invocation per
    // file) -- this pins each path to *its own* OID, not merely to three
    // mutually distinct ones, which is what actually proves
    // `--stdin-paths` output lines are paired back up with the paths in
    // the order they were fed, not just that hashing happened three times.
    assert_eq!(
        oids.get(&RepoPath::new("a.txt")),
        Some(&Oid("72943a16fb2c8f38f9dde202b7a70ccc19c52f34".to_string()))
    );
    assert_eq!(
        oids.get(&RepoPath::new("b.txt")),
        Some(&Oid("f761ec192d9f0dca3329044b96ebdb12839dbff6".to_string()))
    );
    assert_eq!(
        oids.get(&RepoPath::new("c.txt")),
        Some(&Oid("b2a7546679fdf79ca0eb7bfbee1e1bb342487380".to_string()))
    );
    assert_eq!(oids.len(), 3);
}

/// More stdout than a conventional 64 KiB pipe can hold. The writer and
/// reader must make progress concurrently; writing every stdin path before
/// draining stdout deadlocks this exact real-Git batch.
#[test]
fn blob_oids_drains_a_large_real_git_batch_without_pipe_deadlock() {
    const PATHS: usize = 4_096;
    let tmp = init_repo();
    let mut paths = Vec::with_capacity(PATHS);
    for index in 0..PATHS {
        let name = format!("many/f{index:04}.txt");
        let absolute = tmp.path().join(&name);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(absolute, format!("{index}\n")).unwrap();
        paths.push(RepoPath::parse(&name).unwrap());
    }
    let repo = GitRepo::discover(tmp.path()).unwrap();
    let (sent, received) = mpsc::sync_channel(1);

    std::thread::spawn(move || {
        let result = repo.blob_oids(&paths).map(|oids| oids.len());
        let _keep_repository_alive = tmp;
        sent.send(result).unwrap();
    });

    let count = received
        .recv_timeout(Duration::from_secs(10))
        .expect("git hash-object batch exceeded its bounded completion time")
        .unwrap();
    assert_eq!(count, PATHS);
}

// --- blob_oids: filters apply (the hard Windows point) ---------------------

#[test]
fn blob_oids_applies_the_eol_lf_clean_filter() {
    // Repo A: `.gitattributes` forcing LF, file written with raw CRLF bytes.
    let repo_a_dir = init_repo();
    fs::write(repo_a_dir.path().join(".gitattributes"), "* text eol=lf\n").unwrap();
    fs::write(repo_a_dir.path().join("f.txt"), b"a\r\nb\r\n".as_slice()).unwrap();
    let repo_a = GitRepo::discover(repo_a_dir.path()).unwrap();
    let oids_a = repo_a.blob_oids(&repo_paths(&["f.txt"])).unwrap();

    // Repo B: no attributes at all, file written with plain LF bytes --
    // this is what the eol=lf filter should normalize repo A's file to
    // before hashing.
    let repo_b_dir = init_repo();
    fs::write(repo_b_dir.path().join("f.txt"), b"a\nb\n".as_slice()).unwrap();
    let repo_b = GitRepo::discover(repo_b_dir.path()).unwrap();
    let oids_b = repo_b.blob_oids(&repo_paths(&["f.txt"])).unwrap();

    assert_eq!(
        oids_a.get(&RepoPath::new("f.txt")),
        oids_b.get(&RepoPath::new("f.txt")),
        "eol=lf must normalize CRLF to LF before hashing, giving the same \
         OID as a file that was already LF"
    );
}

// --- blob_oids: missing paths are absent, not an error ---------------------

#[test]
fn blob_oids_omits_a_missing_path_without_erroring() {
    let tmp = init_repo();
    fs::write(tmp.path().join("present.txt"), "here\n").unwrap();

    let repo = GitRepo::discover(tmp.path()).unwrap();
    let oids = repo
        .blob_oids(&repo_paths(&["present.txt", "missing.txt"]))
        .unwrap();

    assert_eq!(oids.len(), 1);
    assert!(oids.contains_key(&RepoPath::new("present.txt")));
    assert!(!oids.contains_key(&RepoPath::new("missing.txt")));
}

#[test]
fn blob_oids_of_only_missing_paths_is_an_empty_map_not_an_error() {
    let tmp = init_repo();

    let repo = GitRepo::discover(tmp.path()).unwrap();
    let oids = repo.blob_oids(&repo_paths(&["nope.txt"])).unwrap();

    assert!(oids.is_empty());
}

// --- discover: not a git repository -----------------------------------------

#[test]
fn discover_reports_telos_git_error_outside_a_repository() {
    let tmp = tempfile::tempdir().unwrap();

    let err = GitRepo::discover(tmp.path()).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosGitError);
    assert_eq!(
        err.hint.as_deref(),
        Some("not a git repository; run `git init`")
    );
}

// --- Lock: round-trip, byte-identity, digest --------------------------------

fn sample_lock() -> Lock {
    let mut spec = BTreeMap::new();
    spec.insert(
        RepoPath::new("telos/notions/Invoice.tel"),
        Oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
    );
    spec.insert(
        RepoPath::new("telos/telos.toml"),
        Oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
    );

    let mut code = BTreeMap::new();
    code.insert(
        RepoPath::new("src/billing/invoice.rs"),
        Oid("cccccccccccccccccccccccccccccccccccccccc".to_string()),
    );

    Lock {
        version: 1,
        tool: "telos 0.7.0".to_string(),
        sealed_by: Some(ChangeId(7)),
        spec_digest: Lock::compute_digest(&spec),
        spec,
        code,
    }
}

#[test]
fn lock_write_then_read_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let lock = sample_lock();

    lock.write(&path).unwrap();
    let read_back = Lock::read(&path).unwrap();

    assert_eq!(read_back, Some(lock));
}

#[test]
fn lock_write_then_read_round_trips_with_no_sealed_by() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let mut lock = sample_lock();
    lock.sealed_by = None;

    lock.write(&path).unwrap();
    let read_back = Lock::read(&path).unwrap();

    assert_eq!(read_back, Some(lock));
}

#[test]
fn lock_write_omits_sealed_by_key_entirely_when_none() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let mut lock = sample_lock();
    lock.sealed_by = None;

    lock.write(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();

    assert!(
        !text.contains("sealed_by"),
        "sealed_by must be omitted entirely when None, got:\n{text}"
    );
}

#[test]
fn lock_write_renders_sealed_by_as_a_change_id_string() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let lock = sample_lock();

    lock.write(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();

    assert!(
        text.contains("sealed_by = \"CHG-0007\""),
        "expected sealed_by = \"CHG-0007\", got:\n{text}"
    );
}

#[test]
fn lock_two_successive_writes_are_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let lock = sample_lock();

    lock.write(&path).unwrap();
    let first = fs::read(&path).unwrap();
    lock.write(&path).unwrap();
    let second = fs::read(&path).unwrap();

    assert_eq!(first, second);
}

#[test]
fn lock_write_uses_lf_line_endings_and_one_trailing_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let lock = sample_lock();

    lock.write(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();

    assert!(!text.contains('\r'), "must not contain CR, got:\n{text}");
    assert!(text.ends_with('\n'), "must end with a newline");
    assert!(
        !text.ends_with("\n\n"),
        "must have exactly one trailing newline, got:\n{text:?}"
    );
}

#[test]
fn lock_write_quotes_paths_as_toml_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    let lock = sample_lock();

    lock.write(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();

    assert!(
        text.contains(
            "\"telos/notions/Invoice.tel\" = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""
        ),
        "got:\n{text}"
    );
}

#[test]
fn lock_read_of_a_missing_file_is_ok_none() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");

    assert_eq!(Lock::read(&path).unwrap(), None);
}

#[test]
fn lock_read_of_invalid_toml_is_a_parse_error_naming_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    fs::write(&path, "this is not valid toml [[[").unwrap();

    let err = Lock::read(&path).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(
        err.message.contains("telos.lock"),
        "message should name the file, got: {}",
        err.message
    );
}

#[test]
fn lock_read_tolerates_reformatted_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("telos.lock");
    // Hand-written, differently spaced/ordered from what `write` produces --
    // `read` must not care, only `write` is canonical.
    fs::write(
        &path,
        r#"
            spec_digest = "sha256:deadbeef"
            version     = 1
            tool = "telos 0.7.0"

            [code]
            "src/billing/invoice.rs" = "cccccccccccccccccccccccccccccccccccccccc"

            [spec]
            "telos/telos.toml" = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        "#,
    )
    .unwrap();

    let lock = Lock::read(&path).unwrap().unwrap();

    assert_eq!(lock.version, 1);
    assert_eq!(lock.tool, "telos 0.7.0");
    assert_eq!(lock.sealed_by, None);
    assert_eq!(lock.spec_digest, "sha256:deadbeef");
    assert_eq!(
        lock.spec.get(&RepoPath::new("telos/telos.toml")),
        Some(&Oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()))
    );
}

// --- Lock::compute_digest ----------------------------------------------------

#[test]
fn compute_digest_is_stable_under_insertion_order_permutation() {
    let mut a: BTreeMap<RepoPath, Oid> = BTreeMap::new();
    a.insert(
        RepoPath::new("telos/notions/Invoice.tel"),
        Oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
    );
    a.insert(
        RepoPath::new("telos/telos.toml"),
        Oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
    );

    let mut b: BTreeMap<RepoPath, Oid> = BTreeMap::new();
    b.insert(
        RepoPath::new("telos/telos.toml"),
        Oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
    );
    b.insert(
        RepoPath::new("telos/notions/Invoice.tel"),
        Oid("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
    );

    assert_eq!(Lock::compute_digest(&a), Lock::compute_digest(&b));
}

#[test]
fn compute_digest_has_the_sha256_prefix_and_is_deterministic() {
    let mut spec: BTreeMap<RepoPath, Oid> = BTreeMap::new();
    spec.insert(
        RepoPath::new("telos/telos.toml"),
        Oid("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
    );

    let digest = Lock::compute_digest(&spec);

    assert!(digest.starts_with("sha256:"));
    assert_eq!(digest.len(), "sha256:".len() + 64);
    assert_eq!(digest, Lock::compute_digest(&spec));
}
