//! End-to-end tests for the foundational command surface: `version` and `init`, in
//! both human and `--json` mode. Every one of them runs the real binary in a
//! throwaway directory -- these are the tests that prove the CLI contract,
//! not the ones that prove the engine (those live in `telos-core`).

mod common;

use std::fs;

use serde_json::{Value, json};
use telos_core::git::Oid;
use telos_core::ids::RepoPath;
use telos_core::lock::Lock;

use common::{repo, telos, with_fixture};

/// The git OID of an empty blob -- what `telos/bindings.tel` must hash to
/// right after `init`, on every OS.
const EMPTY_BLOB: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

// --- version -----------------------------------------------------------

/// `telos --version` prints the crate version and exits successfully.
#[test]
fn version_flag_prints_telos_version() {
    let tmp = repo();
    telos(tmp.path(), &["--version"])
        .assert()
        .success()
        .stdout("telos 0.7.0\n");
}

#[test]
fn version_subcommand_prints_telos_version() {
    let tmp = repo();
    telos(tmp.path(), &["version"])
        .assert()
        .success()
        .stdout("telos 0.7.0\n");
}

#[test]
fn version_subcommand_json_reports_the_crate_version() {
    let tmp = repo();
    let out = telos(tmp.path(), &["version", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(json_stdout(&out)["result"]["version"], json!("0.7.0"));
}

// --- init: the happy path ----------------------------------------------

#[test]
fn init_seals_an_empty_repository() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    let lock_path = tmp.path().join("telos/telos.lock");
    assert!(
        lock_path.is_file(),
        "expected {} to exist",
        lock_path.display()
    );

    let lock = Lock::read(&lock_path)
        .expect("the lock parses")
        .expect("the lock exists");
    assert_eq!(lock.version, 1);
    assert_eq!(lock.sealed_by, None);
    assert_eq!(
        lock.spec.get(&RepoPath::new("telos/bindings.tel")),
        Some(&Oid(EMPTY_BLOB.to_string())),
        "`telos/bindings.tel` must be sealed as the empty blob"
    );
    assert!(lock.code.is_empty(), "a fresh project has no bindings");
}

#[test]
fn init_creates_the_whole_telos_tree() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    let telos_dir = tmp.path().join("telos");
    for sub in ["notions", "intents", "constraints", "changes"] {
        assert!(
            telos_dir.join(sub).is_dir(),
            "expected telos/{sub}/ to be a directory"
        );
    }
    assert_eq!(
        fs::read_to_string(telos_dir.join("telos.toml")).unwrap(),
        "[code]\nglobs = []\n\n[tests]\nglobs = []\n\n[test]\ncmd = \"\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = []\n"
    );
    assert_eq!(
        fs::read(telos_dir.join("bindings.tel")).unwrap(),
        Vec::<u8>::new(),
        "`bindings.tel` starts empty, to the byte"
    );
}

/// `init` seeds `telos/changes/counters.toml` at zero and, because
/// `changes/` is excluded from `Workspace::spec_files`, this seed never
/// enters the seal.
#[test]
fn init_seeds_a_zeroed_counters_file() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    assert_eq!(
        fs::read_to_string(tmp.path().join("telos/changes/counters.toml")).unwrap(),
        "intent = 0\nscenario = 0\nconstraint = 0\nchange = 0\n"
    );
}

#[test]
fn init_seals_without_the_lock_mentioning_counters_toml() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    let lock = Lock::read(&tmp.path().join("telos/telos.lock"))
        .expect("the lock parses")
        .expect("the lock exists");
    assert!(
        !lock
            .spec
            .contains_key(&RepoPath::new("telos/changes/counters.toml")),
        "the seal must not mention telos/changes/counters.toml: {:?}",
        lock.spec.keys().collect::<Vec<_>>()
    );
}

#[test]
fn init_json_envelope_is_exact() {
    let tmp = repo();
    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "init",
            "result": { "root": "telos", "sealed": true },
            "error": null,
            "next_actions": ["telos status"]
        })
    );
}

// --- init: .gitattributes ----------------------------------------------

#[test]
fn init_creates_gitattributes_when_it_is_absent() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    assert_eq!(
        fs::read_to_string(tmp.path().join(".gitattributes")).unwrap(),
        "telos/** text eol=lf\n"
    );
}

#[test]
fn init_appends_to_gitattributes_that_lacks_a_trailing_newline() {
    let tmp = repo();
    fs::write(tmp.path().join(".gitattributes"), "*.rs text eol=lf").unwrap();

    telos(tmp.path(), &["init"]).assert().success();

    assert_eq!(
        fs::read_to_string(tmp.path().join(".gitattributes")).unwrap(),
        "*.rs text eol=lf\ntelos/** text eol=lf\n"
    );
}

#[test]
fn init_leaves_gitattributes_alone_when_the_line_is_already_there() {
    let tmp = repo();
    fs::write(
        tmp.path().join(".gitattributes"),
        "telos/** text eol=lf\n*.rs text eol=lf\n",
    )
    .unwrap();

    telos(tmp.path(), &["init"]).assert().success();

    assert_eq!(
        fs::read_to_string(tmp.path().join(".gitattributes")).unwrap(),
        "telos/** text eol=lf\n*.rs text eol=lf\n"
    );
}

// --- init: refusals ----------------------------------------------------

#[test]
fn second_init_is_already_initialized() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["command"], json!("init"));
    assert_eq!(envelope["result"], Value::Null);
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_ALREADY_INITIALIZED")
    );
    assert_eq!(
        envelope["error"]["hint"],
        json!("project already initialized; see `telos status`")
    );
}

#[test]
fn init_on_an_initialized_project_writes_the_error_to_stderr_in_human_mode() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["init"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    assert!(
        out.stdout.is_empty(),
        "human-mode errors go to stderr only, got stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[TELOS_ALREADY_INITIALIZED]"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("hint: project already initialized; see `telos status`"),
        "stderr: {stderr}"
    );
}

#[test]
fn init_outside_a_git_repository_is_a_git_error() {
    let tmp = tempfile::tempdir().unwrap();

    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("TELOS_GIT_ERROR"));
    assert!(
        !tmp.path().join("telos").exists(),
        "init must not create anything outside a git repository"
    );
}
