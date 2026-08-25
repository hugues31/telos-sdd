//! `seal` and `compute_state`, end to end against a copy of the `billing`
//! corpus inside a throwaway git repository. `compute_state` compares git
//! blob OIDs, so a real `git` binary (not an in-memory stand-in) is what
//! actually exercises it -- see `telos_core::state`'s module docs for why it
//! never parses a `.tel` file itself.
//!
//! The claim-aware section below drives `compute_state` with real
//! `OpenChangeInfo`s -- built via `write_change` + `open_change_infos`
//! rather than constructed by hand -- so the same store `changes_store.rs`
//! exercises is what state.rs is proved against.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use telos_core::changes::{open_change_infos, write_change};
use telos_core::error::ErrorCode;
use telos_core::git::GitRepo;
use telos_core::ids::{
    CapabilityId, CapabilityRef, ChangeId, ContextId, IntentId, NotionName, Owner, RepoPath,
};
use telos_core::lock::{Lock, seal};
use telos_core::model::{Change, ChangeStatus, Notion, NotionKind, StagedOp};
use telos_core::state::{
    ChangeSummary, DriftEntry, DriftKind, ProjectStateKind, compute_state, coverage, drift_token,
};
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

    assert_eq!(lock.version, 2);
    assert!(
        lock.tool.starts_with("telos "),
        "expected tool to start with `telos `, got: {}",
        lock.tool
    );
    assert_eq!(lock.sealed_by, None);
    assert_eq!(lock.spec_digest, Lock::compute_digest(&lock.spec));

    // telos.toml plus the twelve canonical strategic/tactical `.tel` files.
    assert_eq!(lock.spec.len(), 13);
    assert!(lock.spec.contains_key(&RepoPath::new("telos/telos.toml")));
    assert!(
        lock.spec
            .contains_key(&RepoPath::new("telos/contexts/billing/notions/Invoice.tel"))
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
    let bindings_path = tmp.path().join("telos/contexts/billing/bindings.tel");
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

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

    assert_eq!(report.state, ProjectStateKind::Coherent);
    assert!(report.drift.is_empty());
    assert!(report.open_changes.is_empty());
}

// --- compute_state: drift kinds ------------------------------------------

#[test]
fn compute_state_reports_modified_for_a_one_byte_spec_edit() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    let invoice_path = tmp
        .path()
        .join("telos/contexts/billing/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_path).unwrap();
    content.push('\n');
    fs::write(&invoice_path, content).unwrap();

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/contexts/billing/notions/Invoice.tel"),
            kind: DriftKind::Modified,
        }]
    );
}

#[test]
fn drift_token_changes_when_modified_bytes_change_but_the_scope_does_not() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());
    let invoice_path = tmp
        .path()
        .join("telos/contexts/billing/notions/Invoice.tel");

    fs::write(&invoice_path, "first modified version\n").unwrap();
    let first = compute_state(&ws, &lock, &git, &[]).unwrap();
    let first_token = drift_token(&ws, &git, &lock, &first.drift).unwrap();

    fs::write(&invoice_path, "second modified version\n").unwrap();
    let second = compute_state(&ws, &lock, &git, &[]).unwrap();
    let second_token = drift_token(&ws, &git, &lock, &second.drift).unwrap();

    assert_eq!(first.drift, second.drift, "the displayed scope stays equal");
    assert_ne!(
        first_token, second_token,
        "live bytes must be authenticated"
    );
}

#[test]
fn compute_state_reports_missing_for_a_deleted_spec_file() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::remove_file(
        tmp.path()
            .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"),
    )
    .unwrap();

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new(
                "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"
            ),
            kind: DriftKind::Missing,
        }]
    );
}

#[test]
fn compute_state_reports_untracked_for_a_spec_file_created_outside_the_protocol() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::write(
        tmp.path().join("telos/contexts/billing/notions/Rogue.tel"),
        "notion Rogue value {\n  def \"unsanctioned\"\n}\n",
    )
    .unwrap();

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/contexts/billing/notions/Rogue.tel"),
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

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

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
    // spec file -- unlinked code is permitted while reading project state;
    // coverage is enforced during reconcile.
    fs::write(tmp.path().join("src/other.rs"), "// unrelated\n").unwrap();

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

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
        tmp.path()
            .join("telos/contexts/billing/notions/Invoice.tel"),
        b"\x00\x01\xffnot even valid utf8 or tel syntax{{{".as_slice(),
    )
    .unwrap();

    let report = compute_state(&ws, &lock, &git, &[]).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/contexts/billing/notions/Invoice.tel"),
            kind: DriftKind::Modified,
        }]
    );
}

// --- compute_state: claim-aware -----------------------------------

/// A minimal notion distinct from anything the corpus already declares.
fn ledger_notion() -> Notion {
    Notion {
        name: NotionName::new("Ledger").unwrap(),
        kind: NotionKind::Entity,
        def: "A record of postings.".to_string(),
        attrs: vec![],
        rels: vec![],
    }
}

/// A notion that claims the same path as the corpus's own `Invoice.tel`
/// (`StagedOp::target_path` derives the path from the name alone, so its
/// content need not match the corpus's).
fn invoice_notion() -> Notion {
    Notion {
        name: NotionName::new("Invoice").unwrap(),
        kind: NotionKind::Entity,
        def: "A bill.".to_string(),
        attrs: vec![],
        rels: vec![],
    }
}

fn billing_owner() -> Owner {
    Owner::context(ContextId::new("billing").unwrap())
}

fn invoicing_owner() -> Owner {
    Owner::capability(CapabilityRef::new(
        ContextId::new("billing").unwrap(),
        CapabilityId::new("invoicing").unwrap(),
    ))
}

fn drafted_change(id: u32, ops: Vec<StagedOp>) -> Change {
    Change {
        id: ChangeId(id),
        motivation: "x".to_string(),
        status: ChangeStatus::Drafted,
        approved_digest: None,
        ops,
        journal: vec![],
    }
}

#[test]
fn compute_state_is_changing_when_an_open_change_has_no_drift() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    write_change(
        &ws,
        &drafted_change(
            1,
            vec![StagedOp::AddOwnedNotion {
                owner: billing_owner(),
                notion: ledger_notion(),
            }],
        ),
    )
    .unwrap();
    let open_changes = open_change_infos(&ws).unwrap();

    let report = compute_state(&ws, &lock, &git, &open_changes).unwrap();

    assert_eq!(report.state, ProjectStateKind::Changing);
    assert!(report.drift.is_empty());
    assert_eq!(
        report.open_changes,
        vec![ChangeSummary {
            id: ChangeId(1),
            status: "drafted".to_string(),
            obligations: vec!["approve".to_string(), "reconcile".to_string()],
        }]
    );
}

#[test]
fn compute_state_is_changing_not_drifted_when_the_open_change_claims_the_drifted_path() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    let invoice_path = tmp
        .path()
        .join("telos/contexts/billing/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_path).unwrap();
    content.push('\n');
    fs::write(&invoice_path, content).unwrap();

    write_change(
        &ws,
        &drafted_change(
            1,
            vec![StagedOp::AddOwnedNotion {
                owner: billing_owner(),
                notion: invoice_notion(),
            }],
        ),
    )
    .unwrap();
    let open_changes = open_change_infos(&ws).unwrap();

    let report = compute_state(&ws, &lock, &git, &open_changes).unwrap();

    assert_eq!(report.state, ProjectStateKind::Changing);
    assert!(
        report.drift.is_empty(),
        "the claimed path must not be reported as drift: {:?}",
        report.drift
    );
}

#[test]
fn compute_state_is_drifted_listing_only_the_path_no_open_change_claims() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    // Two drifted paths: Invoice.tel modified, INT-0017.tel deleted.
    let invoice_path = tmp
        .path()
        .join("telos/contexts/billing/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_path).unwrap();
    content.push('\n');
    fs::write(&invoice_path, content).unwrap();
    fs::remove_file(
        tmp.path()
            .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"),
    )
    .unwrap();

    // The open change claims only INT-0017.tel, not Invoice.tel.
    write_change(
        &ws,
        &drafted_change(
            1,
            vec![StagedOp::RemoveOwnedIntent {
                owner: invoicing_owner(),
                id: IntentId(17),
            }],
        ),
    )
    .unwrap();
    let open_changes = open_change_infos(&ws).unwrap();

    let report = compute_state(&ws, &lock, &git, &open_changes).unwrap();

    assert_eq!(report.state, ProjectStateKind::Drifted);
    assert_eq!(
        report.drift,
        vec![DriftEntry {
            path: RepoPath::new("telos/contexts/billing/notions/Invoice.tel"),
            kind: DriftKind::Modified,
        }]
    );
}

#[test]
fn compute_state_reports_an_unparseable_change_file_as_changing_with_a_repair_obligation() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(
        tmp.path().join("telos/changes/CHG-0001.tel"),
        b"\x00not even a change file{{{".as_slice(),
    )
    .unwrap();
    let open_changes = open_change_infos(&ws).unwrap();

    let report = compute_state(&ws, &lock, &git, &open_changes).unwrap();

    assert_eq!(report.state, ProjectStateKind::Changing);
    assert!(report.drift.is_empty());
    assert_eq!(
        report.open_changes,
        vec![ChangeSummary {
            id: ChangeId(1),
            status: "open".to_string(),
            obligations: vec!["repair telos/changes/CHG-0001.tel (unparseable)".to_string()],
        }]
    );
}

/// Same shape as the test above, but the file is not even valid UTF-8 --
/// bytes `parse_change_file` (`&str`-only) can never be offered. This must
/// still reach `compute_state` as a `Changing` report with the repair
/// obligation, never as an `Err`: `open_change_infos` is best-effort per
/// A truncated write or binary corruption is exactly the on-disk
/// damage that guarantee exists to survive.
#[test]
fn compute_state_reports_invalid_utf8_change_bytes_as_changing_never_an_error() {
    let tmp = corpus_repo();
    let (ws, lock, git) = discover_and_seal(tmp.path());

    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(
        tmp.path().join("telos/changes/CHG-0001.tel"),
        b"\xff\xfe garbage".as_slice(),
    )
    .unwrap();
    let open_changes = open_change_infos(&ws).unwrap();

    let report = compute_state(&ws, &lock, &git, &open_changes).unwrap();

    assert_eq!(report.state, ProjectStateKind::Changing);
    assert!(report.drift.is_empty());
    assert_eq!(
        report.open_changes,
        vec![ChangeSummary {
            id: ChangeId(1),
            status: "open".to_string(),
            obligations: vec!["repair telos/changes/CHG-0001.tel (unparseable)".to_string()],
        }]
    );
}

// --- workspace/git root guard -------------------------------------------

#[test]
fn compute_state_reports_git_error_for_a_nested_git_repo_under_a_sealed_workspace() {
    let tmp = corpus_repo();
    let (ws, lock, _git) = discover_and_seal(tmp.path());

    // A second, independent git repository nested a couple of levels below
    // the sealed workspace's root -- e.g. a vendored dependency or an
    // accidentally-`git init`ed scratch directory.
    let nested = tmp.path().join("src/vendor/nested-repo");
    fs::create_dir_all(&nested).unwrap();
    run_git(&nested, &["init", "--quiet"]);

    // `Workspace::discover` walks up from `nested` and still finds the
    // outer `telos/telos.toml` -- `ws.repo_root` is the *outer* root.
    let ws_from_nested = Workspace::discover(&nested).unwrap();
    assert_eq!(ws_from_nested.repo_root, ws.repo_root);

    // `GitRepo::discover` walks up from `nested` too, but stops at the
    // *first* `.git` it finds -- the nested repo, not the outer one. This
    // is exactly the mismatch `status`/`check --sealed` can hit when they
    // discover a `Workspace` and a `GitRepo` independently from `cwd`.
    let git_from_nested = GitRepo::discover(&nested).unwrap();
    assert_ne!(git_from_nested.root(), ws_from_nested.repo_root);

    let err = compute_state(&ws_from_nested, &lock, &git_from_nested, &[]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosGitError);
    assert!(
        err.message
            .contains(&ws_from_nested.repo_root.display().to_string())
            && err
                .message
                .contains(&git_from_nested.root().display().to_string()),
        "expected the message to name both roots, got: {}",
        err.message
    );
    assert_eq!(
        err.hint.as_deref(),
        Some(
            "the telos workspace and the git repository must share the same root; run telos from the repository that contains telos/"
        )
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

#[test]
fn coverage_counts_same_named_notions_in_distinct_contexts() {
    let tmp = corpus_repo();
    let terminal = tmp.path().join("telos/contexts/terminal");
    fs::create_dir_all(terminal.join("notions")).unwrap();
    fs::write(
        terminal.join("context.tel"),
        "context terminal supporting \"Terminal\" {\n  def \"Presents a local projection.\"\n}\n",
    )
    .unwrap();
    fs::write(
        terminal.join("notions/Invoice.tel"),
        "notion terminal/Invoice entity {\n  def  \"The terminal's local invoice projection.\"\n}\n",
    )
    .unwrap();

    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected both contexts to load cleanly, got {diags:?}"));

    assert_eq!(coverage(&model).notions, 5);
}
