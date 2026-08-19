//! `seal` and `compute_state`, end to end against a copy of the `billing`
//! corpus inside a throwaway git repository. `compute_state` compares git
//! blob OIDs, so a real `git` binary (not an in-memory stand-in) is what
//! actually exercises it -- see `telos_core::state`'s module docs for why it
//! never parses a `.tel` file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use telos_core::error::ErrorCode;
use telos_core::git::GitRepo;
use telos_core::ids::RepoPath;
use telos_core::lock::{Lock, seal};
use telos_core::state::{DriftEntry, DriftKind, ProjectStateKind, compute_state, coverage};
use telos_core::workspace::Workspace;

// --- fixture plumbing --------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

/// Recursively copies every file and subdirectory of `src` into `dst`,
/// creating `dst` (and any nested directory) as needed.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {}: {e}", dst.display()));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display())) {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("copy {}: {e}", entry.path().display()));
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}

/// A fresh tempdir holding a copy of the `billing` corpus, initialized as a
/// git repository (`blob_oids` requires a real repo).
fn corpus_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&corpus_root(), tmp.path());
    run_git(tmp.path(), &["init", "--quiet"]);
    run_git(tmp.path(), &["config", "user.email", "test@example.com"]);
    run_git(tmp.path(), &["config", "user.name", "Test"]);
    tmp
}

/// Discovers the workspace and model, seals it, and writes the lock -- the
/// common setup every scenario below starts from.
fn discover_and_seal(root: &Path) -> (Workspace, Lock, GitRepo) {
    let ws = Workspace::discover(root).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));
    let git = GitRepo::discover(root).unwrap();
    let lock = seal(&ws, &model, &git, None).unwrap();
    lock.write(&ws.lock_path()).unwrap();
    (ws, lock, git)
}

// --- seal --------------------------------------------------------------

#[test]
fn seal_hashes_spec_files_and_every_bindings_code_path() {
    let tmp = corpus_repo();
    let (_ws, lock, _git) = discover_and_seal(tmp.path());

    assert_eq!(lock.version, 1);
    assert!(
        lock.tool.starts_with("telos "),
        "expected tool to start with `telos `, got: {}",
        lock.tool
    );
    assert_eq!(lock.sealed_by, None);
    assert_eq!(lock.spec_digest, Lock::compute_digest(&lock.spec));

    // Corpus spec_files(): telos.toml + 4 notions + 2 intents + 1 constraint
    // + bindings.tel = 9.
    assert_eq!(lock.spec.len(), 9);
    assert!(lock.spec.contains_key(&RepoPath::new("telos/telos.toml")));
    assert!(
        lock.spec
            .contains_key(&RepoPath::new("telos/notions/Invoice.tel"))
    );

    // Corpus bindings.tel: one `implements`, one `proves`, distinct files.
    assert_eq!(lock.code.len(), 2);
    assert!(
        lock.code
            .contains_key(&RepoPath::new("src/billing/invoice.rs"))
    );
    assert!(lock.code.contains_key(&RepoPath::new("tests/billing.rs")));
}

#[test]
fn seal_fails_with_integrity_violation_naming_a_binding_to_a_missing_code_file() {
    let tmp = corpus_repo();
    let bindings_path = tmp.path().join("telos/bindings.tel");
    let mut content = fs::read_to_string(&bindings_path).unwrap();
    content.push_str("implements \"src/billing/ghost.rs\" -> INT-0042\n");
    fs::write(&bindings_path, content).unwrap();

    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));
    let git = GitRepo::discover(tmp.path()).unwrap();

    let err = seal(&ws, &model, &git, None).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosIntegrityViolation);
    assert!(
        err.message.contains("src/billing/ghost.rs"),
        "expected the message to name the missing file, got: {}",
        err.message
    );
}

// --- compute_state: happy path ------------------------------------------

#[test]
fn compute_state_is_coherent_right_after_seal() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Coherent);
    assert!(report.drift.is_empty());
    assert!(report.open_changes.is_empty());
}

// --- compute_state: drift kinds ------------------------------------------

#[test]
fn compute_state_reports_modified_for_a_one_byte_spec_edit() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    let invoice_path = tmp.path().join("telos/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_path).unwrap();
    content.push('\n');
    fs::write(&invoice_path, content).unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/notions/Invoice.tel"),
            kind: DriftKind::Modified,
        }]
    );
}

#[test]
fn compute_state_reports_missing_for_a_deleted_spec_file() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::remove_file(tmp.path().join("telos/intents/INT-0017.tel")).unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/intents/INT-0017.tel"),
            kind: DriftKind::Missing,
        }]
    );
}

#[test]
fn compute_state_reports_untracked_for_a_spec_file_created_outside_the_protocol() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::write(
        tmp.path().join("telos/notions/Rogue.tel"),
        "notion Rogue value {\n  def \"unsanctioned\"\n}\n",
    )
    .unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/notions/Rogue.tel"),
            kind: DriftKind::Untracked,
        }]
    );
}

#[test]
fn compute_state_reports_modified_for_edited_bound_code() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    let invoice_rs = tmp.path().join("src/billing/invoice.rs");
    let mut content = fs::read_to_string(&invoice_rs).unwrap();
    content.push_str("// edited\n");
    fs::write(&invoice_rs, content).unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("src/billing/invoice.rs"),
            kind: DriftKind::Modified,
        }]
    );
}

#[test]
fn compute_state_ignores_a_new_unbound_code_file() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    // Created after the seal, never referenced by any binding, and not a
    // spec file -- unlinked code is free in M1 (rule 5 is M2 reconcile-time).
    fs::write(tmp.path().join("src/other.rs"), "// unrelated\n").unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Coherent);
    assert!(report.drift.is_empty());
}

#[test]
fn compute_state_answers_drifted_on_a_corrupted_spec_file_without_parsing() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    // Not valid UTF-8, not valid `.tel` syntax -- `compute_state` must not
    // even attempt to parse it, only hash it.
    fs::write(
        tmp.path().join("telos/notions/Invoice.tel"),
        b"\x00\x01\xffnot even valid utf8 or tel syntax{{{".as_slice(),
    )
    .unwrap();

    let report = compute_state(&ws, &lock, &git).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/notions/Invoice.tel"),
            kind: DriftKind::Modified,
        }]
    );
}

// --- coverage --------------------------------------------------------------

#[test]
fn coverage_matches_the_corpus_exactly() {
    let tmp = corpus_repo();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));

    let cov = coverage(&model);

    assert_eq!(cov.notions, 4);
    assert_eq!(cov.constraints, 1);
    assert_eq!(cov.intents_total, 2);
    assert_eq!(cov.intents_active, 2);
    assert_eq!(cov.scenarios_total, 2);
    assert_eq!(cov.scenarios_proved, 1);
    assert_eq!(cov.intents_implemented, 1);
}
