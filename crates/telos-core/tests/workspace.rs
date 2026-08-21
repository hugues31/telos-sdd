//! `Workspace` discovery and model loading, end to end against a copy of
//! the Billing corpus in a tempdir -- so every test exercises real
//! filesystem I/O, not an in-memory stand-in.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use telos_core::config::TddPolicy;
use telos_core::error::ErrorCode;
use telos_core::workspace::Workspace;

// --- fixture plumbing ------------------------------------------------------

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

/// A fresh tempdir holding a copy of the `billing` corpus's `telos/` tree
/// (plus its `src/` and `tests/` companions).
fn copied_corpus() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&corpus_root(), tmp.path());
    tmp
}

const EXPECTED_SPEC_FILES: [&str; 9] = [
    "telos/bindings.tel",
    "telos/constraints/CON-0003.tel",
    "telos/intents/INT-0017.tel",
    "telos/intents/INT-0042.tel",
    "telos/notions/Customer.tel",
    "telos/notions/Invoice.tel",
    "telos/notions/InvoiceIssued.tel",
    "telos/notions/PaymentReceived.tel",
    "telos/telos.toml",
];

// --- discover ---------------------------------------------------------------

#[test]
fn discover_walks_up_from_a_nested_subdirectory() {
    let tmp = copied_corpus();
    let nested = tmp.path().join("src").join("billing");
    fs::create_dir_all(&nested).unwrap();

    let ws = Workspace::discover(&nested).unwrap();

    assert_eq!(ws.repo_root, tmp.path());
    assert_eq!(ws.telos_dir, tmp.path().join("telos"));
}

#[test]
fn discover_finds_the_workspace_from_its_own_root() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    assert_eq!(ws.repo_root, tmp.path());
}

#[test]
fn discover_reports_not_initialized_when_no_telos_dir_exists_anywhere_above() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("a").join("b").join("c");
    fs::create_dir_all(&nested).unwrap();

    let err = Workspace::discover(&nested).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosNotInitialized);
    assert_eq!(
        err.hint.as_deref(),
        Some("run `telos init` at the repository root")
    );
}

#[test]
fn discover_reports_a_parse_error_with_the_file_path_for_invalid_toml() {
    let tmp = copied_corpus();
    fs::write(
        tmp.path().join("telos/telos.toml"),
        "this is not valid toml [[[",
    )
    .unwrap();

    let err = Workspace::discover(tmp.path()).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(
        err.message.contains("telos.toml"),
        "message should name the file, got: {}",
        err.message
    );
}

#[test]
fn discover_defaults_an_empty_telos_toml() {
    let tmp = copied_corpus();
    fs::write(tmp.path().join("telos/telos.toml"), "").unwrap();

    let ws = Workspace::discover(tmp.path()).unwrap();

    assert_eq!(ws.config.policy.tdd, TddPolicy::Strict);
    assert!(ws.config.code.globs.is_empty());
    assert!(ws.config.tests.globs.is_empty());
    assert_eq!(ws.config.test.cmd, "");
}

// --- spec_files --------------------------------------------------------------

#[test]
fn spec_files_matches_the_exact_sorted_corpus_list() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    let files: Vec<String> = ws
        .spec_files()
        .unwrap()
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();

    assert_eq!(files, EXPECTED_SPEC_FILES.to_vec());
}

#[test]
fn spec_files_excludes_the_lock_file_and_the_changes_directory() {
    let tmp = copied_corpus();
    fs::write(tmp.path().join("telos/telos.lock"), "").unwrap();
    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(tmp.path().join("telos/changes/CHG-0001.tel"), "").unwrap();

    let ws = Workspace::discover(tmp.path()).unwrap();
    let files: Vec<String> = ws
        .spec_files()
        .unwrap()
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();

    assert_eq!(
        files,
        EXPECTED_SPEC_FILES.to_vec(),
        "telos.lock and changes/ must never appear in spec_files()"
    );
}

#[test]
fn spec_files_ignores_stray_non_tel_files() {
    let tmp = copied_corpus();
    fs::write(tmp.path().join("telos/notions/README.md"), "not a spec").unwrap();

    let ws = Workspace::discover(tmp.path()).unwrap();
    let files: Vec<String> = ws
        .spec_files()
        .unwrap()
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();

    assert_eq!(files, EXPECTED_SPEC_FILES.to_vec());
}

// --- load_model ----------------------------------------------------------

#[test]
fn load_model_succeeds_on_the_corpus() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));

    assert_eq!(model.notions.len(), 4);
    assert_eq!(model.intents.len(), 2);
    assert_eq!(model.constraints.len(), 1);
    assert_eq!(model.bindings.len(), 2);
}

#[test]
fn load_model_reports_a_diagnostic_naming_an_intent_source_stray_in_notions() {
    let tmp = copied_corpus();
    // Well-formed intent content, just filed under `notions/` -- location
    // contradicts content, so `parse_notion_file` must reject it on its
    // own terms (it never sees a `notion` keyword).
    let stray = tmp.path().join("telos/notions/Stray.tel");
    fs::write(
        &stray,
        r#"intent INT-0099 "misplaced intent" {
  status active
  telos  "should never live in notions/"
  statement ubiquitous {
    system shall "do it"
  }
}
"#,
    )
    .unwrap();

    let diags = ws_load_model_err(tmp.path());

    assert!(
        diags
            .iter()
            .any(|d| d.file.as_ref().map(|f| f.as_str()) == Some("telos/notions/Stray.tel")),
        "expected a diagnostic naming telos/notions/Stray.tel, got {diags:?}"
    );
}

fn ws_load_model_err(root: &Path) -> Vec<telos_core::error::Diagnostic> {
    let ws = Workspace::discover(root).unwrap();
    match ws.load_model() {
        Ok(_) => panic!("expected load_model to fail"),
        Err(diags) => diags,
    }
}

// --- lock_path -------------------------------------------------------------

#[test]
fn lock_path_is_telos_lock_inside_the_telos_dir() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    assert_eq!(ws.lock_path(), tmp.path().join("telos").join("telos.lock"));
}
