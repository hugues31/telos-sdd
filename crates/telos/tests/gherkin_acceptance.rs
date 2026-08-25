//! Proves that what `render_features` emits is Gherkin a real Cucumber can
//! parse and run, that `{features}` delivers it, and that the red/green loop
//! works with Cucumber as the runner.
//!
//! `#[ignore]`d: it writes a throwaway Cargo project depending on `cucumber`,
//! so it needs network and compiles a large dependency tree. That fixture is
//! generated at run time, which is what keeps `cucumber` out of this
//! repository's `Cargo.toml` and `Cargo.lock`. CI runs it on Linux with
//! `-- --ignored`.

mod common;

use std::path::Path;
use std::process::Command;

use common::telos;

const CUCUMBER_TOML: &str = r#"[package]
name = "cucumber-acceptance"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
cucumber = "0.23"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }

[[test]]
name = "features"
harness = false
"#;

/// The domain under test, before it satisfies the scenario. `receive_payment`
/// does nothing, so the `Then` step fails and Cucumber exits non-zero.
const LIB_RED: &str = r#"#[derive(Debug, Default)]
pub struct Invoice {
    pub state: String,
}

impl Invoice {
    pub fn receive_payment(&mut self, _amount: &str) {}
}
"#;

/// The minimum change that satisfies the scenario.
const LIB_GREEN: &str = r#"#[derive(Debug, Default)]
pub struct Invoice {
    pub state: String,
}

impl Invoice {
    pub fn receive_payment(&mut self, _amount: &str) {
        self.state = "settled".to_string();
    }
}
"#;

/// The step definitions: the human-written half, and the file `telos test`
/// seals. It must not change between red and green -- editing it after a red
/// is `TELOS_TEST_SEALED`.
///
/// The step text is transcribed from what `render_features` produces for this
/// spec. If the renderer's prose changed, these regexes would stop matching
/// and the run would go red on an undefined step.
const STEPS: &str = r#"use cucumber::{World, given, then, when};

use cucumber_acceptance::Invoice;

#[derive(Debug, Default, World)]
struct BillingWorld {
    invoice: Invoice,
}

#[given(regex = r"^the invoice with state (.+)$")]
fn given_invoice_state(world: &mut BillingWorld, state: String) {
    world.invoice.state = state;
}

#[when(regex = r"^the payment is received with amount (.+)$")]
fn when_payment_received(world: &mut BillingWorld, amount: String) {
    world.invoice.receive_payment(&amount);
}

#[then(regex = r"^the invoice state is (.+)$")]
fn then_invoice_state(world: &mut BillingWorld, expected: String) {
    assert_eq!(world.invoice.state, expected);
}

/// Telos hands the staged directory as `--features-dir <path>`. Pulled out of
/// argv here, with everything else forwarded to Cucumber's parser; declaring
/// it as a custom CLI struct would pull in `clap` for one argument.
fn features_dir_from_argv() -> (std::path::PathBuf, Vec<String>) {
    let mut args: Vec<String> = std::env::args().collect();
    let position = args
        .iter()
        .position(|arg| arg == "--features-dir")
        .expect("telos passes --features-dir; an empty {features} means generation was off");
    let dir = args.remove(position + 1);
    args.remove(position);
    (dir.into(), args)
}

#[tokio::main]
async fn main() {
    use cucumber::cli::Parser as _;

    let (dir, args) = features_dir_from_argv();
    let opts = cucumber::cli::Opts::<_, _, _>::parse_from(args);
    // `fail_on_skipped` is Cucumber's strict mode: an undefined step becomes
    // a failure instead of a silent skip with a zero exit status.
    BillingWorld::cucumber()
        .with_cli(opts)
        .fail_on_skipped()
        .run_and_exit(dir)
        .await;
}
"#;

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn run(root: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {program}: {e}"));
    assert!(
        out.status.success(),
        "{program} {args:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs a `telos` subcommand that must succeed, returning its `result`.
fn telos_ok(root: &Path, args: &[&str], stdin: &str) -> serde_json::Value {
    let mut cmd = telos(root, args);
    // One shared target directory, so the Cucumber tree compiles once per
    // machine. The runner inherits it from here, which is the only route
    // available: `[test] cmd` never sees a shell.
    cmd.env(
        "CARGO_TARGET_DIR",
        std::env::temp_dir().join("telos-cucumber-acceptance-target"),
    );
    let out = if stdin.is_empty() {
        cmd.output().unwrap()
    } else {
        cmd.write_stdin(stdin.to_string()).output().unwrap()
    };
    let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    assert_eq!(
        envelope["ok"],
        serde_json::json!(true),
        "{args:?}: {envelope}"
    );
    envelope["result"].clone()
}

#[test]
#[ignore = "writes a throwaway project depending on cucumber; CI runs it with --ignored"]
fn generated_gherkin_runs_under_a_real_cucumber() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "Cargo.toml", CUCUMBER_TOML);
    write(root, "src/lib.rs", LIB_RED);
    write(root, "tests/features.rs", STEPS);

    run(root, "git", &["init", "-q", "."]);
    run(
        root,
        "git",
        &["config", "user.email", "acceptance@example.com"],
    );
    run(root, "git", &["config", "user.name", "acceptance"]);

    telos_ok(root, &["init", "--json"], "");
    run(root, "git", &["add", "-A"]);
    run(root, "git", &["commit", "-qm", "fixture"]);

    telos_ok(root, &["change", "open", "settlement", "--json"], "");
    let stage = |args: &[&str], payload: &str| telos_ok(root, args, payload);

    stage(
        &["add", "context", "--change", "CHG-0001", "--json"],
        r#"{"id":"billing","kind":"core","title":"Billing","def":"Owns billing."}"#,
    );
    stage(
        &["add", "capability", "--change", "CHG-0001", "--json"],
        r#"{"owner":"billing","id":"settlement","title":"Settlement","def":"Settles invoices."}"#,
    );
    stage(
        &["add", "notion", "--change", "CHG-0001", "--json"],
        r#"{"owner":"billing","name":"Invoice","kind":"entity","def":"A bill.",
            "attrs":[{"name":"state","type":"enum","values":["open","settled"]}]}"#,
    );
    stage(
        &["add", "notion", "--change", "CHG-0001", "--json"],
        r#"{"owner":"billing/settlement","name":"PaymentReceived","kind":"event",
            "phrase":"payment is received","def":"Money arrived.",
            "attrs":[{"name":"amount","type":"money"}]}"#,
    );
    stage(
        &["add", "intent", "--change", "CHG-0001", "--json"],
        r#"{"owner":"billing/settlement","title":"Invoice payment marks it settled",
            "status":"draft","telos":"Customers must see their debt cleared.",
            "statement":{"template":"event-driven","when":"PaymentReceived","on":"Invoice",
                         "action":"set Invoice.state = settled"},
            "refines":[],"requires":[],"excludes":[],
            "scenarios":[{"title":"full payment settles the invoice",
              "given":[{"notion":"Invoice","fields":{"state":"open"}}],
              "when":{"notion":"PaymentReceived","fields":{"amount":"120.00 EUR"}},
              "then":["Invoice.state == settled"]}]}"#,
    );
    stage(
        &["config", "--change", "CHG-0001", "--json"],
        r#"{"code":{"globs":[]},"tests":{"globs":[]},
            "test":{"cmd":"cargo test --test features -- --features-dir {features}"},
            "policy":{"tdd":"strict"},"gherkin":{"enabled":true},"agents":{"hosts":[]}}"#,
    );
    telos_ok(root, &["change", "approve", "CHG-0001", "--json"], "");

    // Red: the domain does nothing, so Cucumber's `Then` assertion fails.
    // Reaching an assertion at all proves it parsed and ran the generated
    // feature from the directory `{features}` supplied.
    let red = telos_ok(
        root,
        &["test", "SCN-0001", "--file", "tests/features.rs", "--json"],
        "",
    );
    assert_eq!(red["witness"], serde_json::json!("red"), "{red}");

    // Green: the minimum application change. The step definitions, which are
    // the sealed bytes, are untouched.
    write(root, "src/lib.rs", LIB_GREEN);
    let green = telos_ok(
        root,
        &["test", "SCN-0001", "--file", "tests/features.rs", "--json"],
        "",
    );
    assert_eq!(green["witness"], serde_json::json!("green"), "{green}");
}
