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

use serde_json::Value;
use tempfile::TempDir;

// `webbrowser` honors BROWSER on Unix desktops other than macOS. macOS uses
// Launch Services directly, so this process-level fake is not observable there.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn fake_browser() -> (TempDir, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("failed to create a fake-browser directory");
    let browser = tmp.path().join("browser");
    let target_log = tmp.path().join("target");
    fs::write(
        &browser,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$TELOS_TEST_BROWSER_TARGET\"\n",
    )
    .expect("failed to write the fake browser");
    let mut permissions = fs::metadata(&browser).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&browser, permissions).unwrap();
    (tmp, browser, target_log)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn wait_for_browser_target(target_log: &Path) -> String {
    for _ in 0..500 {
        if let Ok(target) = fs::read_to_string(target_log) {
            return target.trim_end().to_string();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!(
        "fake browser did not record a target at {}",
        target_log.display()
    );
}

/// Supplies the canonical owner for shared test payload builders that focus
/// on another part of the mutation contract.
pub fn canonical_payload(args: &[&str], payload: &str) -> String {
    let Some(entity) = args
        .first()
        .zip(args.get(1))
        .and_then(|(verb, entity)| (*verb == "add").then_some(*entity))
    else {
        return payload.to_string();
    };
    let owner = match entity {
        "notion" | "constraint" => "billing",
        "intent" => "billing/settlement",
        _ => return payload.to_string(),
    };
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return payload.to_string();
    };
    if let Some(object) = value.as_object_mut() {
        object
            .entry("owner".to_string())
            .or_insert_with(|| Value::String(owner.to_string()));
    }
    value.to_string()
}

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

/// An initialized and sealed project with a minimal Billing strategic model
/// but no tactical entities.
pub fn with_empty_billing_domain() -> TempDir {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    for (relative, bytes) in [
        (
            "telos/contexts/billing/context.tel",
            "context billing core \"Billing\" {\n  def \"Owns invoice rules.\"\n}\n",
        ),
        (
            "telos/contexts/billing/capabilities/invoicing/capability.tel",
            "capability billing/invoicing \"Invoicing\" {\n  def \"Issues invoices.\"\n}\n",
        ),
        (
            "telos/contexts/billing/capabilities/settlement/capability.tel",
            "capability billing/settlement \"Settlement\" {\n  def \"Settles invoices.\"\n}\n",
        ),
    ] {
        let target = tmp.path().join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }

    telos(tmp.path(), &["change", "reconcile", "--full", "--json"])
        .assert()
        .success();
    tmp
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
    let bindings_path = root.join("telos/contexts/billing/bindings.tel");
    let bindings = fs::read_to_string(&bindings_path).unwrap();
    let invoice_intent = fs::read_to_string(
        root.join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"),
    )
    .unwrap();
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
    for relative in ["telos/contexts", "telos/constraints", "telos/changes"] {
        fs::create_dir_all(tmp.path().join(relative))
            .unwrap_or_else(|e| panic!("mkdir {relative}: {e}"));
    }
    tmp
}

/// Breaks the settlement-owned `INT-0042.tel` in two independent ways:
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
    let path = root.join("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel");
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

/// The report path every report-backed fixture configures, at the repo root
/// so no `[code]`/`[tests]` glob ever matches it.
pub const REPORT: &str = "telos-report.xml";
/// The file the fake runner copies to `{report}`; tests rewrite it between
/// runs to script what "the runner" reports.
pub const REPORT_FIXTURE: &str = ".report-fixture.xml";
/// Marker: while it exists the fake runner exits 0 without writing a report.
pub const REPORT_SILENT: &str = ".report-silent";
/// The `[test] cmd` the report fixtures install.
pub const FAKE_RUNNER_TEMPLATE: &str = if cfg!(windows) {
    "./fake-runner.bat {report} {filter}"
} else {
    "./fake-runner {report} {filter}"
};

/// Installs a runner that copies [`REPORT_FIXTURE`] to its first argument
/// and exits 0, exits 0 without writing when [`REPORT_SILENT`] exists, and
/// exits 101 without writing (a compile error, a network failure) when the
/// fixture is absent. A shell script on Unix, a batch file on Windows.
///
/// A missing (empty) first argument also exits 0 without writing: today's
/// `reconcile --full` -- the seal every fixture goes through -- runs the
/// full-suite gate through the pre-`{report}` code path and so never passes
/// one. Every real `telos test` invocation always supplies its report path,
/// so this only ever fires for that one sealing run.
pub fn install_fake_runner(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("fake-runner");
        fs::write(
            &script,
            concat!(
                "#!/bin/sh\n",
                "# telos fake runner: $1 is the report path telos asked for.\n",
                "if test -f .report-silent; then exit 0; fi\n",
                "if test -z \"$1\"; then exit 0; fi\n",
                "if test -f .report-fixture.xml; then cp .report-fixture.xml \"$1\" && exit 0; fi\n",
                "exit 101\n",
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(windows)]
    {
        fs::write(
            root.join("fake-runner.bat"),
            concat!(
                "@echo off\r\n",
                "if exist .report-silent exit /b 0\r\n",
                "if \"%~1\"==\"\" exit /b 0\r\n",
                "if exist .report-fixture.xml (\r\n",
                "  copy /Y .report-fixture.xml \"%~1\" >nul\r\n",
                "  exit /b 0\r\n",
                ")\r\n",
                "exit /b 101\r\n",
            ),
        )
        .unwrap();
    }
}

/// Scripts the next runner report.
pub fn write_report_fixture(root: &Path, xml: &str) {
    fs::write(root.join(REPORT_FIXTURE), xml).unwrap();
}

/// A JUnit report with one `testcase` per `(name, status)`, `status` being
/// one of `passed`, `failed`, `error`, `skipped`.
pub fn junit_report(cases: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n  <testsuite name=\"billing\">\n",
    );
    for (name, status) in cases {
        let body = match *status {
            "passed" => "",
            "failed" => "<failure message=\"assertion failed\">left != right</failure>",
            "error" => "<error message=\"panicked\">boom</error>",
            "skipped" => "<skipped/>",
            other => panic!("unknown testcase status `{other}`"),
        };
        xml.push_str(&format!(
            "    <testcase name=\"{name}\" classname=\"billing\" time=\"0.01\">{body}</testcase>\n"
        ));
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    xml
}

/// The corpus' two sealed scenarios, both passed: what the sealing
/// `reconcile --full` of a report fixture must find in the report.
pub fn sealed_scenarios_passed() -> String {
    junit_report(&[
        ("scn_0091_issued_invoice_is_open", "passed"),
        ("scn_0107_full_payment_settles_the_invoice", "passed"),
    ])
}

/// [`with_fixture`] with the fake runner installed, `[test] report` set to
/// [`REPORT`], `[policy] tdd` set to `policy`, and a report proving both
/// sealed scenarios in place before the seal.
pub fn with_report_fixture(policy: &str) -> TempDir {
    with_fixture_mut(|root| {
        install_fake_runner(root);
        write_report_fixture(root, &sealed_scenarios_passed());
        // The corpus test file is a placeholder; give the sealed SCN-0107
        // target a real function so `rebuild status` can resolve it.
        fs::write(
            root.join("tests/billing.rs"),
            "fn scn_0107_full_payment_settles_the_invoice() {}\n",
        )
        .unwrap();
        let config = root.join("telos/telos.toml");
        let src = fs::read_to_string(&config).unwrap();
        assert!(
            src.contains("cmd = \"\""),
            "the corpus no longer ships an empty `[test] cmd`: {src}"
        );
        let src = src
            .replace(
                "cmd = \"\"",
                &format!("cmd = \"{FAKE_RUNNER_TEMPLATE}\"\nreport = \"{REPORT}\""),
            )
            .replace("tdd = \"strict\"", &format!("tdd = \"{policy}\""));
        fs::write(&config, src).unwrap();
    })
}
