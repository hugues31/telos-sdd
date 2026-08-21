//! Shared plumbing for the `telos` end-to-end tests: throwaway git
//! repositories, the sealed `billing` corpus fixture, and the builder every
//! test drives the real binary through.
//!
//! Included by several test binaries, each of which uses only part of it --
//! hence the crate-wide `dead_code` allowance.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A fresh, empty git repository in a throwaway directory, with the `user.*`
/// config a real checkout would have.
pub fn repo() -> TempDir {
    let tmp = tempfile::tempdir().expect("failed to create a temporary directory");
    git(tmp.path(), &["init", "--quiet"]);
    git(tmp.path(), &["config", "user.email", "test@example.com"]);
    git(tmp.path(), &["config", "user.name", "Test"]);
    tmp
}

/// A [`repo`] holding a copy of the `billing` corpus, already sealed: the
/// starting point for every command that needs an initialized, coherent
/// project.
///
/// The seal is the real one, produced by running the real binary: `telos
/// change reconcile --full` is exactly the command a user reaches for to
/// seal a spec tree that exists but has no lock. The fixture is therefore
/// built through the public command rather than by calling
/// `telos_core::lock::seal` behind the CLI's back. The full flow (`init`,
/// `change open`, `add`, `test`, `bind`, and `reconcile`) is covered by the
/// end-to-end tests that drive it through the public CLI.
pub fn with_fixture() -> TempDir {
    with_fixture_mut(|_| {})
}

/// [`with_fixture`], with `mutate` given the copied tree *before* it is
/// sealed.
///
/// The order is the point: whatever `mutate` writes is part of what the seal
/// records, so the fixture it hands back is coherent rather than drifted.
/// That is what lets a test change `telos.toml`'s `[test] cmd` -- the corpus
/// ships it empty, so a reconcile there runs no test at all -- and
/// still start from a `coherent` project. Note that the sealing reconcile is
/// itself subject to whatever `mutate` did: a `[test] cmd` it installs runs
/// once, with an empty `{filter}`, before this returns.
pub fn with_fixture_mut(mutate: impl FnOnce(&Path)) -> TempDir {
    let tmp = unsealed_fixture();
    mutate(tmp.path());
    complete_fixture_for_sealing(tmp.path());

    let out = telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .output()
        .expect("failed to run `telos change reconcile --full`");
    // Loudly: a harness that hands back an unsealed fixture would make every
    // test built on it fail somewhere else, for reasons that look nothing
    // like “the fixture never got sealed”.
    let ok = serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .map(|envelope| envelope["ok"] == serde_json::Value::Bool(true))
        .unwrap_or(false);
    assert!(
        ok,
        "sealing the fixture with `telos change reconcile --full` failed:\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    tmp
}

/// Upgrades the intentionally partial spec-only corpus to a sealable tree.
///
/// `unsealed_fixture` stays at 0/2-capable reconstruction input. Consumers
/// that ask for `with_fixture*`, however, ask for a coherent sealed project,
/// so every active scenario needs a proof and the project needs a runner.
fn complete_fixture_for_sealing(root: &Path) {
    let bindings_path = root.join("telos/bindings.tel");
    let bindings = fs::read_to_string(&bindings_path).unwrap();
    let invoice_intent = fs::read_to_string(root.join("telos/intents/INT-0017.tel")).unwrap();
    if invoice_intent.contains("status active") && !bindings.contains("-> SCN-0091") {
        let (implements, rest) = bindings
            .split_once('\n')
            .expect("the billing corpus starts with its implements binding");
        fs::write(
            &bindings_path,
            format!("{implements}\nproves     \"tests/billing.rs\" -> SCN-0091\n{rest}"),
        )
        .unwrap();
    }

    let config_path = root.join("telos/telos.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    if config.contains("cmd = \"\"") {
        fs::write(
            &config_path,
            config.replace("cmd = \"\"", "cmd = \"git --version\""),
        )
        .unwrap();
    }
}

/// A [`repo`] holding a copy of the `billing` corpus, *without* sealing it:
/// `telos/telos.toml` and every `.tel` file are on disk, but there is no
/// `telos.lock` -- the abnormal state a project ends up in if its lock is
/// deleted or never committed. Distinct from an uninitialized repository
/// (no `telos/` at all), which `Workspace::discover` itself rejects.
pub fn unsealed_fixture() -> TempDir {
    let tmp = repo();
    copy_dir(&corpus_root(), tmp.path());
    tmp
}

/// Breaks `telos/intents/INT-0042.tel` in two independent, unrelated ways:
/// its `on Invoice` clause becomes an unresolvable `on Invoce`, and its
/// `requires INT-0017` becomes an unresolvable `requires INT-9999`.
///
/// `telos_core::semantic::build_model` collects diagnostics for the whole
/// spec in one pass and, within one intent, checks its statement before its
/// `refines`/`requires`/`excludes` relations (`Checker::check_intent`) --
/// so this reliably produces exactly two diagnostics, in this order: the
/// unknown-notion one from the statement's `on` clause, then the
/// unknown-intent one from `requires`. Used by tests that need to prove
/// `check` handles more than one diagnostic correctly, not just the
/// single-diagnostic case a single edit produces.
pub fn break_int_0042_in_two_ways(root: &Path) {
    let path = root.join("telos/intents/INT-0042.tel");
    let content =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        content.contains("on Invoice"),
        "fixture no longer contains the expected `on Invoice` clause"
    );
    assert!(
        content.contains("requires INT-0017"),
        "fixture no longer contains the expected `requires INT-0017` clause"
    );
    let content = content
        .replace("on Invoice", "on Invoce")
        .replace("requires INT-0017", "requires INT-9999");
    fs::write(&path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// The `telos` binary under test, ready to run in `dir`.
pub fn telos(dir: &Path, args: &[&str]) -> assert_cmd::Command {
    let mut cmd =
        assert_cmd::Command::cargo_bin("telos").expect("`cargo test` builds the `telos` binary");
    cmd.current_dir(dir);
    cmd.args(args);
    cmd
}

/// The `billing` corpus, which lives in `telos-core`'s test tree.
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing")
}

/// Recursively copies every file and subdirectory of `src` into `dst`,
/// creating `dst` (and any nested directory) as needed.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {}: {e}", dst.display()));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display())) {
        let entry = entry.expect("failed to read a directory entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("failed to stat an entry").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("copy {}: {e}", entry.path().display()));
        }
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}
