//! The three **acceptance loops** from spec §14: the product's executable
//! roadmap. Each one scripts a full product loop -- `change open`, `add`,
//! `test`, `bind`, `adopt`, `reconcile --full`, and friends -- through
//! commands that do not exist yet (M2: the change/transaction surface; M3:
//! `test`/`bind`/`context`). They compile today because every command is
//! just a string argument handed to the spawned `telos` binary: there is no
//! compile-time coupling to a Rust API that doesn't exist.
//!
//! They are committed `#[ignore]`d and stay that way until the milestone
//! that implements their last missing command lands, at which point that
//! loop's `#[ignore]` comes off and the test starts asserting for real. Run
//! with `cargo test --workspace -- --ignored` today, every loop is expected
//! to fail -- on the *first* command M1 doesn't have, not on a compile
//! error or a panic unrelated to the missing surface. That failure is this
//! file doing its job: it is the roadmap's done-criterion, not a bug to fix
//! here.
//!
//! Payload JSON shapes fed to `add`/`edit` on stdin are invented for this
//! test -- M2 freezes their real shape. They're marked below wherever they
//! appear.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use common::{repo, telos, with_fixture};

// --- shared harness: run a step, assert its envelope ------------------------
//
// Every `telos` invocation in these loops goes through `run_ok`/`run_err`
// (or the stdin-carrying `run_ok_stdin`) rather than being spawned loose --
// that's what turns "the loop compiles" into "the loop asserts something"
// once its commands exist, and it's also where the §14 anti-goal lives: none
// of these loops may ever need to put a hash or a digest in a CLI argument,
// so every argument list passed through here is checked for one on the way
// in, loop_feature included.

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Spec §14's anti-goal, checked mechanically: "if daily usage requires
/// thinking about hashes or the lock, the abstraction has failed and a test
/// must show it." No argument passed to `telos` anywhere in these loops may
/// contain the substring `sha` or a run of 40 hex characters (a sha1, which
/// is what a git blob OID -- the only kind of hash this system has -- looks
/// like).
fn assert_args_never_mention_a_hash(args: &[&str]) {
    for arg in args {
        assert!(
            !arg.to_ascii_lowercase().contains("sha"),
            "a loop's CLI arguments must never mention a hash (spec §14 anti-goal): {arg:?}"
        );
        assert!(
            !contains_forty_hex_run(arg),
            "a loop's CLI arguments must never mention a hash (spec §14 anti-goal): {arg:?}"
        );
    }
}

/// Whether `s` contains 40 consecutive hex digits -- the shape of a sha1 hex
/// string, whatever surrounds it.
fn contains_forty_hex_run(s: &str) -> bool {
    let mut run = 0;
    for b in s.bytes() {
        if b.is_ascii_hexdigit() {
            run += 1;
            if run >= 40 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Runs `telos <args>` in `dir`, feeding `stdin` if given, and parses stdout
/// as a JSON envelope. Private: every loop goes through `run_ok`/`run_err`/
/// `run_ok_stdin` instead, which additionally assert the envelope's `ok`.
fn run(dir: &Path, args: &[&str], stdin: Option<&str>) -> Value {
    assert_args_never_mention_a_hash(args);
    let mut cmd = telos(dir, args);
    if let Some(input) = stdin {
        cmd.write_stdin(input);
    }
    let out = cmd.output().expect("failed to spawn `telos`");
    json_stdout(&out)
}

/// Runs a step expected to succeed; asserts `ok: true` and returns the whole
/// envelope.
fn run_ok(dir: &Path, args: &[&str]) -> Value {
    let envelope = run(dir, args, None);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected `telos {}` to succeed, got: {envelope}",
        args.join(" ")
    );
    envelope
}

/// Like [`run_ok`], but writes `stdin` to the child first -- the shape every
/// `add`/`edit` call needs for its JSON payload.
fn run_ok_stdin(dir: &Path, args: &[&str], stdin: &str) -> Value {
    let envelope = run(dir, args, Some(stdin));
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected `telos {}` to succeed, got: {envelope}",
        args.join(" ")
    );
    envelope
}

/// Runs a step expected to be *refused*; asserts `ok: false` and
/// `error.code == code`, and returns the whole envelope.
fn run_err(dir: &Path, args: &[&str], code: &str) -> Value {
    let envelope = run(dir, args, None);
    assert_eq!(
        envelope["ok"],
        json!(false),
        "expected `telos {}` to fail with {code}, got: {envelope}",
        args.join(" ")
    );
    assert_eq!(
        envelope["error"]["code"],
        json!(code),
        "expected `telos {}` to fail with {code}, got: {envelope}",
        args.join(" ")
    );
    envelope
}

// --- git plumbing, local to this file ---------------------------------------
//
// Only `loop_merge` needs to script git directly (creating two diverging
// branches and resolving their merge conflict). `common/mod.rs` already has
// a private helper exactly like this one for its own fixtures -- it isn't
// exported, and per the task brief `common/mod.rs` stays untouched for these
// `#[ignore]`d loops, so this is a small local copy rather than a shared
// export.

/// Runs `git <args>` in `dir` and asserts it succeeded.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// The branch `HEAD` currently points to. Used instead of a hardcoded
/// `main`/`master` so the loop doesn't depend on `git`'s
/// `init.defaultBranch` -- it works whether that config is unset (old-style
/// `master`) or set to `main`.
fn current_branch(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("failed to run git symbolic-ref");
    assert!(
        out.status.success(),
        "git symbolic-ref --short HEAD failed in {}",
        dir.display()
    );
    String::from_utf8(out.stdout)
        .expect("branch name is valid UTF-8")
        .trim()
        .to_string()
}

// --- loop_feature: open -> challenge -> approve -> red/green -> reconcile --

/// The **feature loop** (spec §7.2's five phases, run start to finish): the
/// full path from a bare git repository to a `COHERENT` project with one
/// proved scenario, driven exactly the way an agent following the
/// `telos-challenger`/`telos-implementer` skills would drive it -- CLI
/// commands and JSON payloads, never a hand-edited `.tel` file, never a
/// hash.
///
/// Un-ignored once M3 lands `test`/`bind` (M2's `change`/`add` surface is a
/// prerequisite but M3's red/green witness protocol is the last piece this
/// loop needs end to end).
#[test]
#[ignore = "un-ignored at M3 (test/bind)"]
fn loop_feature() {
    let tmp = repo();
    let dir = tmp.path();

    // --- Observe -------------------------------------------------------
    // A fresh repository has nothing yet; `init` seals the empty project.
    run_ok(dir, &["init", "--json"]);
    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));

    // --- Challenge -------------------------------------------------------
    // `telos change open "<motivation>"` starts a transaction; the delta is
    // staged into it with `add`, never written to `telos/` directly.
    let opened = run_ok(
        dir,
        &["change", "open", "Invoices can be settled", "--json"],
    );
    let change_id = opened["result"]["id"]
        .as_str()
        .expect("`change open` answers with the new change's id (M2)")
        .to_string();

    // Payload shapes are invented for this test -- M2 freezes the `add`
    // family's real JSON. These mirror the spec's canonical example (§4.5):
    // the `Invoice` notion, the `PaymentReceived` event it reacts to, and
    // the intent + scenario that ties them together.
    run_ok_stdin(
        dir,
        &["add", "notion", "--change", &change_id, "--json"],
        r#"{
            "id": "Invoice",
            "kind": "entity",
            "def": "A bill issued to a Customer for delivered work.",
            "attrs": [
                {"name": "state", "type": "enum", "values": ["open", "settled"]}
            ]
        }"#,
    );
    run_ok_stdin(
        dir,
        &["add", "notion", "--change", &change_id, "--json"],
        r#"{
            "id": "PaymentReceived",
            "kind": "event",
            "def": "A payment arrived for an invoice."
        }"#,
    );
    run_ok_stdin(
        dir,
        &["add", "intent", "--change", &change_id, "--json"],
        r#"{
            "id": "INT-0001",
            "title": "Invoices can be settled",
            "status": "active",
            "telos": "Customers must see immediately that their debt is cleared.",
            "statement": {
                "template": "event-driven",
                "when": "PaymentReceived",
                "on": "Invoice",
                "action": "set Invoice.state = settled"
            },
            "scenarios": [
                {
                    "id": "SCN-0001",
                    "title": "a full payment settles the invoice",
                    "given": [{"notion": "Invoice", "state": {"state": "open"}}],
                    "when": {"notion": "PaymentReceived", "payload": {}},
                    "then": ["Invoice.state == settled"]
                }
            ]
        }"#,
    );

    // --- Approve -------------------------------------------------------
    // `diff` renders the staged delta for human review; `approve` locks the
    // approval to that delta's digest -- editing it afterward invalidates
    // the approval (M2, `TELOS_APPROVAL_STALE`), but this loop never edits
    // after approving.
    run_ok(dir, &["change", "diff", &change_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_id, "--json"]);

    // --- Implement -------------------------------------------------------
    // Red witness before green, per §7.2.4. M1 has no real cargo project in
    // this throwaway sandbox repo to compile a failing test against, so
    // (per this task's brief) `telos.toml`'s `[test] cmd` is pointed at a
    // tiny shell script this test controls, which reports red or green by
    // reading a marker file the loop flips between the two `telos test`
    // calls below. This is an M1 stand-in only -- M3 exercises a real cargo
    // project end to end.
    fs::write(
        dir.join("telos/telos.toml"),
        "[code]\nglobs = [\"src/**/*.rs\"]\n\n\
         [tests]\nglobs = [\"tests/**/*.rs\"]\n\n\
         [test]\ncmd = \"sh scripts/fake-test.sh {filter}\"\n\n\
         [policy]\ntdd = \"strict\"\n",
    )
    .expect("failed to write telos.toml");
    fs::create_dir_all(dir.join("scripts")).expect("failed to create scripts/");
    fs::write(
        dir.join("scripts/fake-test.sh"),
        "#!/bin/sh\n\
         # M1 stand-in for a real test runner (task 16 brief): red/green is\n\
         # decided by a marker file this test flips, not by compiling anything.\n\
         # M3 replaces this with cargo test against a real fixture project.\n\
         dir=$(dirname \"$0\")\n\
         if [ -f \"$dir/../.fake-test-green\" ]; then exit 0; else exit 1; fi\n",
    )
    .expect("failed to write scripts/fake-test.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("scripts/fake-test.sh");
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }

    let red = run_ok(dir, &["test", "SCN-0001", "--json"]);
    assert_eq!(red["result"]["witness"], json!("red"));

    fs::write(dir.join(".fake-test-green"), "").expect("failed to flip the marker to green");

    let green = run_ok(dir, &["test", "SCN-0001", "--json"]);
    assert_eq!(green["result"]["witness"], json!("green"));

    // Minimal domain code, bound to the intent it implements.
    fs::create_dir_all(dir.join("src")).expect("failed to create src/");
    fs::write(
        dir.join("src/billing.rs"),
        "// Minimal domain code for INT-0001, named after the notions it implements.\n",
    )
    .expect("failed to write src/billing.rs");
    run_ok(dir, &["bind", "src/billing.rs", "INT-0001", "--json"]);

    // --- Reconcile -------------------------------------------------------
    // Applies the staged delta atomically, revalidates every §3.3 rule,
    // reruns the impacted scenarios, checks constraints, requires no
    // orphan code, and re-seals -- closing the change.
    run_ok(dir, &["change", "reconcile", &change_id, "--json"]);

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
    assert_eq!(status["result"]["coverage"]["scenarios_proved"], json!(1));
}

// --- loop_drift: out-of-protocol edit -> DRIFTED -> adopt -> coherent ------

/// The **drift loop** (spec §6): an edit outside the CLI's protocol -- a
/// hand-edited `.tel` file, the only kind of write this system considers
/// illegitimate -- blocks forward progress (`change open` refused) until
/// `adopt` captures it as a real change, after which the ordinary loop
/// finishes it the same way `loop_feature` does.
///
/// Un-ignored once M3 lands (`adopt` needs the M2 change machinery to
/// capture drift *into*, and this loop's final phase is the same
/// implement/reconcile path `loop_feature` exercises).
#[test]
#[ignore = "un-ignored at M3"]
fn loop_drift() {
    let tmp = with_fixture();
    let dir = tmp.path();

    // --- Observe -------------------------------------------------------
    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));

    // Out-of-protocol edit: appending a byte to a sealed spec file directly
    // on disk, bypassing the CLI entirely -- the same drift M1's own
    // `status`/`check` tests (`status_check.rs`) exercise.
    let invoice_tel = dir.join("telos/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_tel).unwrap();
    content.push('\n');
    fs::write(&invoice_tel, content).unwrap();

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("drifted"));

    // --- Challenge, refused -------------------------------------------------------
    // §6: every forward-progress operation is gated on drift, `open`
    // included.
    run_err(
        dir,
        &["change", "open", "x", "--json"],
        "TELOS_DRIFT_DETECTED",
    );

    // `adopt`: the out-of-protocol edit is captured as a legitimate change
    // -- nothing is ever lost to a refusal, only routed back through the
    // protocol.
    run_ok(dir, &["adopt", "--json"]);

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("changing"));
    let changes = status["result"]["changes"]
        .as_array()
        .expect("`status.changes` is an array");
    assert_eq!(
        changes.len(),
        1,
        "adopt captures the drift as exactly one open change"
    );
    let change_id = changes[0]["id"]
        .as_str()
        .expect("the adopting change has an id")
        .to_string();

    // --- Approve, Implement, Reconcile -------------------------------------------------------
    // The same loop `loop_feature` runs from Approve onward: the adopted
    // edit is now an ordinary staged delta.
    run_ok(dir, &["change", "diff", &change_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_id, "--json"]);
    run_ok(dir, &["change", "reconcile", &change_id, "--json"]);

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
}

// --- loop_merge: two sealed branches -> lock conflict -> reconcile --full -

/// The **merge loop** (spec §7.4): two branches, each sealed independently
/// off the same starting point, diverge and conflict on `telos.lock` when
/// merged. `check` alone is green once the `.tel` conflicts are resolved by
/// hand (the spec parses and resolves cleanly), but `check --sealed` stays
/// red until `telos change reconcile --full` re-validates every §3.3 rule,
/// reruns the whole test suite, and re-seals -- proof, not a bypass.
///
/// Un-ignored once M2 lands `change open/add/edit/reconcile --full`.
#[test]
#[ignore = "un-ignored at M2"]
fn loop_merge() {
    let tmp = with_fixture();
    let dir = tmp.path();
    let base = current_branch(dir);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-m", "seal the billing corpus"]);

    // --- Branch A: touch INT-0017 through the ordinary change flow. --------
    git(dir, &["checkout", "-b", "branch-a"]);
    let change_a = run_ok(dir, &["change", "open", "tighten INT-0017", "--json"]);
    let change_a_id = change_a["result"]["id"].as_str().unwrap().to_string();
    run_ok_stdin(
        dir,
        &[
            "edit",
            "intent",
            "INT-0017",
            "--change",
            &change_a_id,
            "--json",
        ],
        r#"{"telos": "An invoice must start its life open and unpaid -- branch A wording."}"#,
    );
    run_ok(dir, &["change", "diff", &change_a_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_a_id, "--json"]);
    run_ok(dir, &["change", "reconcile", &change_a_id, "--json"]);
    git(dir, &["add", "-A"]);
    git(
        dir,
        &["commit", "-m", "branch A: reconcile the INT-0017 edit"],
    );

    // --- Branch B: touch INT-0042 the same way, from the same starting
    // point -- each branch's `reconcile` rewrites the whole lock (new
    // digest, new sealing change id), so the two branches' `telos.lock`
    // conflict line-for-line on merge, not just around the touched intent.
    git(dir, &["checkout", &base]);
    git(dir, &["checkout", "-b", "branch-b"]);
    let change_b = run_ok(dir, &["change", "open", "tighten INT-0042", "--json"]);
    let change_b_id = change_b["result"]["id"].as_str().unwrap().to_string();
    run_ok_stdin(
        dir,
        &[
            "edit",
            "intent",
            "INT-0042",
            "--change",
            &change_b_id,
            "--json",
        ],
        r#"{"telos": "Customers must see immediately that their debt is cleared -- branch B wording."}"#,
    );
    run_ok(dir, &["change", "diff", &change_b_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_b_id, "--json"]);
    run_ok(dir, &["change", "reconcile", &change_b_id, "--json"]);
    git(dir, &["add", "-A"]);
    git(
        dir,
        &["commit", "-m", "branch B: reconcile the INT-0042 edit"],
    );

    // --- Merge: git conflicts on telos.lock. --------------------------------
    git(dir, &["checkout", "branch-a"]);
    let merge_status = Command::new("git")
        .args(["merge", "branch-b"])
        .current_dir(dir)
        .status()
        .expect("failed to run git merge");
    assert!(
        !merge_status.success(),
        "expected `git merge branch-b` to conflict on telos.lock"
    );

    // Resolve the two `.tel` conflicts by hand -- each branch touched a
    // different intent file, so this is a clean pick from each side.
    // `telos.lock` itself is deliberately left conflicted: it is not a file
    // a human resolves by hand, `reconcile --full` re-derives it.
    git(dir, &["checkout", "--ours", "telos/intents/INT-0017.tel"]);
    git(dir, &["checkout", "--theirs", "telos/intents/INT-0042.tel"]);
    git(
        dir,
        &[
            "add",
            "telos/intents/INT-0017.tel",
            "telos/intents/INT-0042.tel",
        ],
    );

    // `check` only re-parses the (now-resolved) spec -- green.
    run_ok(dir, &["check", "--json"]);
    // `check --sealed` still reads telos.lock -- which still has git's
    // conflict markers in it -- so it stays red.
    run_err(dir, &["check", "--sealed", "--json"], "TELOS_PARSE_ERROR");

    // --- Reconcile --full -------------------------------------------------------
    // §7.4: total integrity revalidation + the whole test suite green +
    // re-seal. Not a bypass -- it demands full proof.
    run_ok(dir, &["change", "reconcile", "--full", "--json"]);

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
}
