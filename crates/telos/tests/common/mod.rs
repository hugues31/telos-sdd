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
/// Domain fixtures explicitly observe a synthetic baseline through the core
/// initializer. Native governance and reconstruction tests use the public CLI.
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

    seal_fixture(tmp.path());
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

    seal_fixture(tmp.path());

    tmp
}

/// Establish a fresh observed baseline for a synthetic test corpus. Tests of
/// initialization itself use the public CLI; this builder owns every byte.
fn seal_fixture(root: &Path) {
    let ledger = root.join(telos_core::plans::ledger::PATH);
    if ledger.exists() {
        fs::remove_file(ledger).unwrap();
    }
    let plans = root.join("telos/plans");
    if plans.exists() {
        fs::remove_dir_all(plans).unwrap();
    }
    let ws = telos_core::workspace::Workspace::discover(root).unwrap();
    let git = telos_core::git::GitRepo::discover(root).unwrap();
    telos_core::reconcile::reconcile_full(&ws, &git).unwrap();
    telos_core::plans::ledger::bootstrap(root).unwrap();
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

/// Domain command tests share explicit, broad fixture plans. Governance tests
/// use `raw_telos` to exercise missing approval and narrow path contracts.
pub fn fixture_plan(
    root: &Path,
    title: &str,
    id: telos_core::ids::ChangeId,
    recovery: bool,
) -> Result<String, telos_core::error::TelosError> {
    use telos_core::plans::{actions, model::*, store};
    let opened = store::open(root, title, &telos_core::work::new_id("REQ")?)?;
    let plan = opened["plan"].as_str().unwrap().to_owned();
    let definition = Definition {
        title: title.into(),
        request: title.into(),
        goal: "Exercise the domain command contract".into(),
        success_criteria: vec!["Command assertions hold".into()],
        scope: vec!["**".into()],
        brief: Brief {
            summary: "Synthetic domain test fixture".into(),
            brainstormed: true,
            ..Default::default()
        },
        tasks: vec![Task {
            id: "TSK-001".into(),
            change_id: Some(id.to_string()),
            title: title.into(),
            kind: if recovery {
                TaskKind::Recovery
            } else {
                TaskKind::Behavior
            },
            allowed_paths: vec!["**".into()],
            acceptance: vec!["Command assertions hold".into()],
            validation: vec![Validation::Review {
                name: "fixture-review".into(),
            }],
            ..Default::default()
        }],
        validation: vec![Validation::Review {
            name: "final-review".into(),
        }],
    };
    actions::revise(
        root,
        &plan,
        definition,
        &telos_core::work::new_id("REQ")?,
        None,
    )?;
    Ok(plan)
}

fn next_fixture_change(root: &Path) -> telos_core::ids::ChangeId {
    use telos_core::ids::ChangeId;
    let ws = telos_core::workspace::Workspace::discover(root).unwrap();
    let mut next = telos_core::counters::read_counters(&ws)
        .unwrap_or_default()
        .change as u128;
    if let Ok(plans) = telos_core::plans::store::list(root) {
        for plan in plans {
            for task in plan.view().unwrap().tasks {
                if let Some(id) = task.change.and_then(|id| id.parse::<ChangeId>().ok())
                    && id.0 < 100000
                {
                    next = next.max(id.0);
                }
            }
        }
    }
    for id in telos_core::changes::list_change_ids(&ws).unwrap_or_default() {
        if id.0 < 100000 {
            next = next.max(id.0);
        }
    }
    ChangeId(next + 1)
}

fn approve_fixture_change(root: &Path, id: &str) -> Result<(), telos_core::error::TelosError> {
    use telos_core::plans::{actions, execution, store};
    let (plan, task) = store::for_change(root, id)?;
    if plan.view()?.approved {
        return Ok(());
    }
    actions::import_change(
        root,
        &plan.id,
        &task.definition.id,
        &telos_core::work::new_id("REQ")?,
        None,
    )?;
    let plan = store::read(root, &plan.id)?;
    actions::approve(
        root,
        &plan.id,
        &plan.definition_digest()?,
        &telos_core::work::new_id("REQ")?,
        None,
    )?;
    execution::start(
        root,
        &plan.id,
        &task.definition.id,
        &telos_core::work::new_id("REQ")?,
        None,
    )?;
    Ok(())
}

pub fn finish_fixture_task(root: &Path) {
    use telos_core::plans::{execution, ledger, store};
    if let Ok(Some((plan, task))) = store::active(root)
        && ledger::receipts(root)
            .is_ok_and(|rs| rs.iter().any(|r| Some(&r.id) == task.change.as_ref()))
    {
        let req = || telos_core::work::new_id("REQ").unwrap();
        let _ = execution::verify(
            root,
            &plan.id,
            Some(&task.definition.id),
            "fixture-review",
            Some("Previous domain assertions passed"),
            false,
            &req(),
            None,
        );
        let _ = execution::finish(root, &plan.id, &task.definition.id, &req(), None);
    }
}

fn recovery_fixture(
    root: &Path,
    into: Option<&str>,
    start: bool,
) -> Result<String, telos_core::error::TelosError> {
    use telos_core::plans::{actions, execution, ledger, model::*, store};
    if !root.join(ledger::PATH).exists() {
        ledger::bootstrap(root)?;
    }
    finish_fixture_task(root);
    let req = || telos_core::work::new_id("REQ").unwrap();
    let (plan, change) = if let Some(id) = into {
        let (plan, task) = store::for_change(root, id)?;
        let mut definition = plan.revision().definition.clone();
        definition
            .tasks
            .iter_mut()
            .find(|t| t.id == task.definition.id)
            .unwrap()
            .kind = TaskKind::Recovery;
        actions::revise(root, &plan.id, definition, &req(), None)?;
        (plan.id, id.to_owned())
    } else {
        let plan = fixture_plan(
            root,
            "Recover the test fixture",
            next_fixture_change(root),
            true,
        )?;
        let prepared = actions::prepare(root, &plan, "TSK-001", &req(), None)?;
        (
            plan,
            prepared["result"]["change"].as_str().unwrap().to_owned(),
        )
    };
    if start {
        let p = store::read(root, &plan)?;
        actions::approve(root, &plan, &p.definition_digest()?, &req(), None)?;
        execution::start(root, &plan, "TSK-001", &req(), None)?;
    }
    Ok(change)
}

pub fn raw_telos(dir: &Path, args: &[&str]) -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("telos").unwrap();
    cmd.current_dir(dir).args(args);
    cmd
}

/// The `telos` binary under test, ready to run in `dir`.
pub fn telos(dir: &Path, args: &[&str]) -> assert_cmd::Command {
    let mut owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    if args.starts_with(&["change", "reconcile"])
        && args.contains(&"--full")
        && !matches!(telos_core::plans::store::active(dir), Ok(Some(_)))
    {
        let _ = recovery_fixture(dir, None, true);
    }
    if dir.join(telos_core::plans::ledger::PATH).exists() {
        if args.first() == Some(&"adopt") {
            let into = args
                .iter()
                .position(|a| *a == "--into")
                .and_then(|i| args.get(i + 1))
                .copied();
            if let Ok(change) = recovery_fixture(dir, into, false)
                && into.is_none()
            {
                owned.extend(["--into".into(), change]);
            }
        } else if args.first() == Some(&"revert") {
            let _ = recovery_fixture(dir, None, true);
        } else if args.starts_with(&["change", "open"]) && args.len() > 2 {
            finish_fixture_task(dir);
            if let Ok(plan) = fixture_plan(dir, args[2], next_fixture_change(dir), false) {
                owned.extend(["--plan".into(), plan, "--task".into(), "TSK-001".into()]);
            }
        } else if args.starts_with(&["change", "approve"]) && args.len() > 2 {
            let ws = telos_core::workspace::Workspace::discover(dir).unwrap();
            if let Ok(id) = args[2].parse()
                && let Ok(change) = telos_core::changes::read_change(&ws, id)
            {
                let expected = args
                    .iter()
                    .position(|a| *a == "--expected-digest")
                    .and_then(|i| args.get(i + 1));
                if !change.ops.is_empty() && expected.is_none_or(|d| **d == change.ops_digest()) {
                    finish_fixture_task(dir);
                    let _ = approve_fixture_change(dir, args[2]);
                }
            }
        }
    }
    let refs: Vec<_> = owned.iter().map(String::as_str).collect();
    raw_telos(dir, &refs)
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

pub fn git(cwd: &Path, args: &[&str]) {
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
/// Every real invocation -- `telos test`, the reconcile gates, and the
/// sealing `reconcile --full` a report fixture goes through -- runs through
/// `telos_core::exec::run_proof`, which always substitutes `{report}`, so
/// the first argument is never empty here; an unsubstituted `{report}`
/// placeholder reaching the runner as a literal argument is exactly the bug
/// this script must not mask.
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

/// Installs the fake runner and points `[test]` at it and at [`REPORT`],
/// with `[policy] tdd = <policy>`; the report proving both sealed scenarios
/// is written too. Call before the fixture seals.
pub fn configure_report(root: &Path, policy: &str) {
    install_fake_runner(root);
    fs::write(
        root.join(".gitignore"),
        "telos-report.xml\n.report-fixture.xml\n.report-silent\n",
    )
    .unwrap();
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
}

/// [`with_fixture`] with the fake runner installed, `[test] report` set to
/// [`REPORT`], `[policy] tdd` set to `policy`, and a report proving both
/// sealed scenarios in place before the seal.
pub fn with_report_fixture(policy: &str) -> TempDir {
    with_fixture_mut(|root| configure_report(root, policy))
}
