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

use telos_core::git::GitRepo;
use telos_core::lock::seal;
use telos_core::workspace::Workspace;

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
/// The seal goes through `telos_core` directly (discover, load, `seal`,
/// write) rather than through a CLI command, because no command can seal an
/// existing workspace yet -- only `init` seals, and `init` refuses a project
/// that already has a `telos/telos.toml`. This debt is paid in T11, when
/// `telos change reconcile --full` can seal a preexisting spec tree from the
/// command line (D14) and the fixture is built the way a user would build it.
pub fn with_fixture() -> TempDir {
    with_fixture_mut(|_| {})
}

/// [`with_fixture`], with `mutate` given the copied tree *before* it is
/// sealed.
///
/// The order is the point: whatever `mutate` writes is part of what the seal
/// records, so the fixture it hands back is coherent rather than drifted.
/// That is what lets a test change `telos.toml`'s `[test] cmd` -- the corpus
/// ships it empty (D13), so a reconcile there runs no test at all -- and
/// still start from a `coherent` project.
pub fn with_fixture_mut(mutate: impl FnOnce(&Path)) -> TempDir {
    let tmp = repo();
    copy_dir(&corpus_root(), tmp.path());
    mutate(tmp.path());

    let ws = Workspace::discover(tmp.path()).expect("the corpus is an initialized workspace");
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));
    let git = GitRepo::discover(tmp.path()).expect("the fixture is a git repository");
    let lock = seal(&ws, &model, &git, None).expect("the corpus seals cleanly");
    lock.write(&ws.lock_path())
        .expect("failed to write the lock");

    tmp
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
