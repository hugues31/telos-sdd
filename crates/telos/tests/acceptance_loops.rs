//! End-to-end acceptance loops for the public product workflows. Each
//! exercises a full loop — `change open`, `add`, `test`,
//! `bind`, `adopt`, `reconcile --full`, and friends — by spawning the `telos`
//! binary. They have no compile-time coupling to the command implementation.
//!
//! All three specification loops run in the ordinary suite. `loop_feature`
//! covers a discoverable scenario test and sealed red/green
//! witnesses; `loop_drift` proves an out-of-protocol edit can be adopted and
//! reconciled; and `loop_merge` proves a lock-only merge conflict is resolved
//! by `reconcile --full`. `loop_projection` covers the shared read-only
//! projection without duplicating the reconstruction owned by `rebuild_demo`.
//!
//! Payload JSON shapes fed to `add`/`edit` on stdin follow the schemas frozen
//! in `docs/contracts.md`. `loop_feature` uses the real,
//! agent-facing shape rather than an invented one.
mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use common::{repo, telos, with_fixture};

const DATA_PREFIX: &str = "window.__TELOS_DATA__ = ";
const DATA_SUFFIX: &str = ";\n";

struct ProjectionServer(Option<Child>);

impl Drop for ProjectionServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn exported_files(root: &Path) -> Vec<String> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(directory).expect("read exported directory") {
            let entry = entry.expect("read exported entry");
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.push(
                    path.strip_prefix(root)
                        .expect("exported file stays below export root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    let mut files = Vec::new();
    visit(root, root, &mut files);
    files.sort();
    files
}

fn data_payload(script: &str) -> Value {
    let json = script
        .strip_prefix(DATA_PREFIX)
        .and_then(|script| script.strip_suffix(DATA_SUFFIX))
        .expect("data.js is one window.__TELOS_DATA__ assignment");
    serde_json::from_str(json).expect("data.js assignment contains valid JSON")
}

fn http_get(url: &str, route: &str, cookie: Option<&str>) -> String {
    let address = url
        .strip_prefix("http://")
        .and_then(|url| url.strip_suffix('/'))
        .expect("view URL has the documented HTTP shape");
    let mut stream = TcpStream::connect(address).expect("connect to live view");
    write!(stream, "GET {route} HTTP/1.1\r\nHost: {address}\r\n").expect("write HTTP request head");
    if let Some(cookie) = cookie {
        write!(stream, "Cookie: {cookie}\r\n").expect("write session cookie");
    }
    write!(stream, "Connection: close\r\n\r\n").expect("finish HTTP request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read HTTP response");
    response
}

fn session_cookie(response: &str) -> String {
    response
        .split_once("\r\n\r\n")
        .expect("HTTP response separates headers and body")
        .0
        .split("\r\n")
        .find_map(|line| {
            line.strip_prefix("set-cookie: ")
                .or_else(|| line.strip_prefix("Set-Cookie: "))
        })
        .expect("the live shell establishes a session")
        .split(';')
        .next()
        .expect("Set-Cookie starts with a cookie pair")
        .to_string()
}

// --- shared harness: run a step, assert its envelope ------------------------
//
// Every `telos` invocation in these loops goes through `run_ok`/`run_err`
// (or the stdin-carrying `run_ok_stdin`) rather than being spawned loose --
// that's what turns "the loop compiles" into "the loop asserts something",
// and it also enforces that none of these loops need to put a hash or a digest
// in a CLI argument,
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

/// Verifies that ordinary workflow arguments never expose lock hashes. No
/// argument passed to `telos` anywhere in these loops may contain the substring
/// `sha` or a run of 40 hex characters (a SHA-1, which is what a Git blob OID
/// looks like).
fn assert_args_never_mention_a_hash(args: &[&str]) {
    for arg in args {
        assert!(
            !arg.to_ascii_lowercase().contains("sha"),
            "a loop's CLI arguments must never mention a hash: {arg:?}"
        );
        assert!(
            !contains_forty_hex_run(arg),
            "a loop's CLI arguments must never mention a hash: {arg:?}"
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
// exported, so this is a small local copy rather than a shared export.

/// Runs `git <args>` in `dir` and asserts it succeeded.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Runs `git <args>` in `dir`, asserts it succeeded, and returns its stdout.
fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}",
        dir.display()
    );
    String::from_utf8(out.stdout).expect("git output is valid UTF-8")
}

/// The paths git left unmerged, sorted. `--name-only` collapses a conflicted
/// path's three index stages into one line, and paths are not localized (the
/// surrounding prose is), so this reads the same under any `LANG`.
fn unmerged_paths(dir: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git_stdout(dir, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    paths.sort();
    paths
}

/// Asserts `telos status --json` reports `state`, and returns the whole
/// envelope so a caller can look at `changes` or `coverage`.
fn assert_state(dir: &Path, state: &str) -> Value {
    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(
        status["result"]["state"],
        json!(state),
        "expected the project to be {state}, got: {status}"
    );
    status
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

/// The **feature loop**, run start to finish: the
/// full path from a bare git repository to a `COHERENT` project with one
/// proved scenario, driven exactly the way an agent following the
/// `telos-challenger`/`telos-implementer` skills would drive it -- CLI
/// commands and JSON payloads, never a hand-edited `.tel` file, never a
/// hash.
///
/// The `test`/`bind` surface and sealed red/green witness protocol make this
/// loop executable end to end.
#[test]
fn loop_feature() {
    let tmp = repo();
    let dir = tmp.path();

    // --- Observe -------------------------------------------------------
    // A fresh repository has nothing yet; `init` seals the empty project.
    run_ok(dir, &["init", "--json"]);
    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));

    // --- Configure the test runner, through the protocol ----------------
    // The fixture configures a direct marker-controlled runner. The generated
    // scenario source below is discoverable under `[tests].globs`; its bytes
    // stay unchanged across the red and green witness calls, while the marker
    // alone selects the runner outcome.
    //
    // `telos/telos.toml` is a sealed spec file, so this setup is adopted
    // before the feature change opens and the repository is coherent again.
    fs::write(
        dir.join("telos/telos.toml"),
        "[code]\nglobs = [\"src/**/*.rs\"]\n\n\
         [tests]\nglobs = [\"tests/**/*.rs\"]\n\n\
         [test]\ncmd = \"git hash-object .fake-test-green\"\n\n\
         [policy]\ntdd = \"strict\"\n",
    )
    .expect("failed to write telos.toml");

    let adopted_toml = run_ok(dir, &["adopt", "--json"]);
    let toml_change_id = adopted_toml["result"]["change"]
        .as_str()
        .expect("`adopt` answers with the id of the change capturing the drift")
        .to_string();
    run_ok(dir, &["change", "diff", &toml_change_id, "--json"]);
    run_ok(dir, &["change", "approve", &toml_change_id, "--json"]);
    run_ok(dir, &["change", "reconcile", &toml_change_id, "--json"]);
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
        .expect("`change open` answers with the new change's id")
        .to_string();

    run_ok_stdin(
        dir,
        &["add", "context", "--change", &change_id, "--json"],
        r#"{"id":"billing","kind":"core","title":"Billing","def":"Owns invoice rules."}"#,
    );
    run_ok_stdin(
        dir,
        &["add", "capability", "--change", &change_id, "--json"],
        r#"{"owner":"billing","id":"settlement","title":"Settlement","def":"Settles invoices."}"#,
    );

    // Payloads use the documented `add` shape from docs/contracts.md:
    // a notion's identity is its `name`, an `add` never carries an id of
    // its own, and `given`/`when` steps carry their state under `fields`.
    // These mirror the spec's canonical example: the `Invoice`
    // notion, the `PaymentReceived` event it reacts to, and the intent +
    // scenario that ties them together.
    run_ok_stdin(
        dir,
        &["add", "notion", "--change", &change_id, "--json"],
        r#"{
            "owner": "billing",
            "name": "Invoice",
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
            "owner": "billing/settlement",
            "name": "PaymentReceived",
            "kind": "event",
            "def": "A payment arrived for an invoice."
        }"#,
    );
    let added_intent = run_ok_stdin(
        dir,
        &["add", "intent", "--change", &change_id, "--json"],
        r#"{
            "owner": "billing/settlement",
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
                    "title": "a full payment settles the invoice",
                    "given": [{"notion": "Invoice", "fields": {"state": "open"}}],
                    "when": {"notion": "PaymentReceived", "fields": {}},
                    "then": ["Invoice.state == settled"]
                }
            ]
        }"#,
    );
    // `add` never carries an id: the intent's id and its
    // scenario's id are both allocated by the CLI and captured from
    // `result` here, never hardcoded.
    let intent_id = added_intent["result"]["id"]
        .as_str()
        .expect("`add intent` answers with the allocated intent id")
        .to_string();
    let scenario_id = added_intent["result"]["scenario_ids"][0]
        .as_str()
        .expect("`add intent` reports the allocated scenario id(s)")
        .to_string();

    // --- Approve -------------------------------------------------------
    // `diff` renders the staged delta for human review; `approve` locks the
    // approval to that delta's digest -- editing it afterward invalidates
    // the approval (`TELOS_APPROVAL_STALE`), but this loop never edits
    // after approving.
    run_ok(dir, &["change", "diff", &change_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_id, "--json"]);

    // Create the discoverable generated-project source before the first
    // `telos test`. These bytes remain unchanged across the red and green
    // calls; the runner marker below is their only varying input.
    fs::create_dir_all(dir.join("tests")).expect("failed to create tests/");
    fs::write(
        dir.join("tests/scn_0001_invoice.rs"),
        "#[test]\nfn scn_0001_invoice_settles() {}\n",
    )
    .expect("failed to write scenario test source");

    // --- Implement -------------------------------------------------------
    // Record a red witness before the green run.
    let red = run_ok(dir, &["test", &scenario_id, "--json"]);
    assert_eq!(red["result"]["witness"], json!("red"));

    fs::write(dir.join(".fake-test-green"), "").expect("failed to flip the marker to green");

    let green = run_ok(dir, &["test", &scenario_id, "--json"]);
    assert_eq!(green["result"]["witness"], json!("green"));

    // The discoverable test source proves the intent, so it is bound before
    // reconciliation just as the domain source below is.
    run_ok(
        dir,
        &["bind", "tests/scn_0001_invoice.rs", &intent_id, "--json"],
    );

    // Minimal domain code, bound to the intent it implements.
    fs::create_dir_all(dir.join("src")).expect("failed to create src/");
    fs::write(
        dir.join("src/billing.rs"),
        "// Minimal domain code, named after the notions it implements.\n",
    )
    .expect("failed to write src/billing.rs");
    run_ok(dir, &["bind", "src/billing.rs", &intent_id, "--json"]);

    // --- Reconcile -------------------------------------------------------
    // Applies the staged delta atomically, revalidates every integrity rule,
    // reruns the impacted scenarios, checks constraints, requires no
    // orphan code, and re-seals -- closing the change.
    run_ok(dir, &["change", "reconcile", &change_id, "--json"]);

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));
    assert_eq!(status["result"]["coverage"]["scenarios_proved"], json!(1));
}

// --- loop_drift: out-of-protocol edit -> DRIFTED -> adopt -> coherent ------

/// The **drift loop**: an edit outside the CLI's protocol -- a
/// hand-edited `.tel` file, the only kind of write this system considers
/// illegitimate -- blocks forward progress (`change open` refused) until
/// `adopt` captures it as a real change, after which the ordinary loop
/// finishes it the same way `loop_feature` does.
///
/// The adoption path restores a coherent repository through the same
/// lifecycle commands used by the feature loop.
#[test]
fn loop_drift() {
    let tmp = with_fixture();
    let dir = tmp.path();

    // --- Observe -------------------------------------------------------
    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("coherent"));

    // Out-of-protocol edit: appending a byte to a sealed spec file directly
    // on disk, bypassing the CLI entirely -- the same drift the
    // `status`/`check` tests (`status_check.rs`) exercise.
    let invoice_tel = dir.join("telos/contexts/billing/notions/Invoice.tel");
    let mut content = fs::read_to_string(&invoice_tel).unwrap();
    content.push('\n');
    fs::write(&invoice_tel, content).unwrap();

    let status = run_ok(dir, &["status", "--json"]);
    assert_eq!(status["result"]["state"], json!("drifted"));

    // --- Challenge, refused -------------------------------------------------------
    // Every forward-progress operation is gated on drift; `open`
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

/// The **merge loop**: two branches, each sealed independently
/// off the same starting point, diverge and conflict on `telos.lock` when
/// merged. The spec files themselves merge cleanly, so `check` is green the
/// moment the
/// merge stops; but `check --sealed` stays red, because the *seal* is what
/// conflicted, until `telos change reconcile --full` re-validates referential
/// integrity, re-runs the impacted proof obligations, and re-seals. Proof, not a
/// bypass.
///
/// This loop is intentionally inert on the witness gate because the Billing
/// corpus has no configured runner. Witness scope is covered separately by
/// the strict-mode reconcile tests and `required_witnesses` unit tests.
#[test]
fn loop_merge() {
    let tmp = with_fixture();
    let dir = tmp.path();
    let base = current_branch(dir);
    assert_state(dir, "coherent");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-m", "seal the billing corpus"]);

    // --- Branch A: touch INT-0017 through the ordinary change flow. --------
    git(dir, &["checkout", "-b", "branch-a"]);
    let change_a = run_ok(dir, &["change", "open", "tighten INT-0017", "--json"]);
    let change_a_id = change_a["result"]["id"].as_str().unwrap().to_string();
    let status = assert_state(dir, "changing");
    assert_eq!(
        status["result"]["changes"],
        json!([{
            "id": change_a_id,
            "status": "open",
            "obligations": ["stage the delta", "approve", "reconcile"],
        }]),
        "an open transaction is the whole of what branch A has going on"
    );
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
    // Reconciled: the change file is gone and the branch is sealed again.
    let status = assert_state(dir, "coherent");
    assert_eq!(status["result"]["changes"], json!([]));
    git(dir, &["add", "-A"]);
    git(
        dir,
        &["commit", "-m", "branch A: reconcile the INT-0017 edit"],
    );

    // --- Branch B: touch INT-0042 the same way, from the same starting
    // point. Change ids are per-branch, so branch B seals its own
    // `CHG-0001` too and `sealed_by` merges cleanly; what makes the two
    // `telos.lock`s conflict is `spec_digest` -- rewritten wholesale by
    // every reconcile -- plus the `[spec]` OID lines of the touched
    // intents, which sit close enough together for git to fold them into
    // one hunk.
    git(dir, &["checkout", &base]);
    git(dir, &["checkout", "-b", "branch-b"]);
    let change_b = run_ok(dir, &["change", "open", "tighten INT-0042", "--json"]);
    let change_b_id = change_b["result"]["id"].as_str().unwrap().to_string();
    assert_state(dir, "changing");
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
    assert_state(dir, "coherent");
    git(dir, &["add", "-A"]);
    git(
        dir,
        &["commit", "-m", "branch B: reconcile the INT-0042 edit"],
    );

    // --- Merge: git conflicts on telos.lock, and on nothing else. ----------
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

    // The spec files are ordinary files, checked rather than asserted in prose:
    // text files that merge like any other, so a conflict on them is rare and
    // local. Here there is none at all -- each branch edited a different
    // intent -- and the only unmerged path is the derived one.
    assert_eq!(
        unmerged_paths(dir),
        vec!["telos/telos.lock".to_string()],
        "only the derived lock may conflict; the spec files merge like any other text"
    );
    // Both branches' edits survived the merge: nothing was picked over
    // anything else.
    let int_0017 = fs::read_to_string(
        dir.join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel"),
    )
    .expect("failed to read INT-0017.tel");
    assert!(
        int_0017.contains("branch A wording"),
        "branch A's INT-0017 edit must survive the merge, got: {int_0017}"
    );
    let int_0042 = fs::read_to_string(
        dir.join("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel"),
    )
    .expect("failed to read INT-0042.tel");
    assert!(
        int_0042.contains("branch B wording"),
        "branch B's INT-0042 edit must survive the merge, got: {int_0042}"
    );

    // `check` only re-parses the spec, which merged cleanly -- green.
    run_ok(dir, &["check", "--json"]);
    // `check --sealed` reads telos.lock -- which still has git's conflict
    // markers in it -- so it stays red. (`status` answers the same way for
    // the same reason, which is why this phase is asserted here and not
    // through `status`.)
    run_err(dir, &["check", "--sealed", "--json"], "TELOS_PARSE_ERROR");

    // --- Reconcile --full -------------------------------------------------------
    // full reconciliation: total integrity revalidation, every constraint re-checked, every
    // impacted proof obligation re-run, then re-seal. Not a bypass -- it
    // demands full proof, and it is the only way out of a conflicted lock.
    let reconciled = run_ok(dir, &["change", "reconcile", "--full", "--json"]);
    assert_eq!(
        reconciled["result"],
        json!({
            "checks_run": 1,
            "full": true,
            "id": null,
            "ops_applied": 0,
            "tests_run": 1,
            "witness_warnings": [],
        }),
        "--full applies no ops, belongs to no change, and re-checks every \
         constraint; this fixture has active proved scenarios, so its \
         configured whole-suite runner executes exactly once"
    );

    let status = assert_state(dir, "coherent");
    assert_eq!(status["result"]["changes"], json!([]));
    // The seal is real, not merely re-written: `check --sealed` is the
    // command that was red one line above the reconcile.
    run_ok(dir, &["check", "--sealed", "--json"]);

    // And the merge is genuinely resolved: the re-derived lock stages, the
    // merge commits, and the merged branch is still coherent afterwards.
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--no-edit"]);
    assert!(
        unmerged_paths(dir).is_empty(),
        "the merge must be fully resolved once `--full` has re-derived the lock"
    );
    assert_state(dir, "coherent");
}

/// Starting from the same coherent project, static export and the live SPA
/// must expose one shared projection without changing project state. The
/// separate `rebuild_demo` acceptance owns the longer spec-only reconstruction
/// lifecycle.
#[test]
fn loop_projection() {
    let tmp = with_fixture();
    assert_state(tmp.path(), "coherent");

    let exported = run_ok(tmp.path(), &["view", "--export", "site", "--json"]);
    assert_eq!(
        exported["result"],
        json!({
            "mode": "export",
            "destination": "site",
            "files": [
                ".nojekyll",
                "assets/app.css",
                "assets/app.js",
                "assets/logo.png",
                "data.js",
                "index.html",
            ]
        })
    );
    let expected_files = exported["result"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(exported_files(&tmp.path().join("site")), expected_files);

    let export_payload = data_payload(
        &fs::read_to_string(tmp.path().join("site/data.js")).expect("read exported data.js"),
    );
    assert_eq!(export_payload["meta"]["mode"], "export");
    assert_eq!(export_payload["snapshot"]["dashboard"]["state"], "coherent");
    assert_eq!(
        export_payload["snapshot"]["intents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|intent| intent["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["INT-0017", "INT-0042"]
    );
    assert!(
        export_payload["snapshot"]["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .any(|scenario| scenario["id"] == "SCN-0107")
    );
    assert!(
        export_payload["snapshot"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notion| notion["name"] == "billing/Invoice")
    );
    assert!(
        export_payload["snapshot"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge
                == &json!({
                    "from": {"kind": "intent", "id": "INT-0042"},
                    "relation": "requires",
                    "to": {"kind": "intent", "id": "INT-0017"}
                }))
    );

    let args = ["view", "--port", "0", "--json"];
    assert_args_never_mention_a_hash(&args);
    let mut server = ProjectionServer(Some(
        Command::new(env!("CARGO_BIN_EXE_telos"))
            .args(args)
            .current_dir(tmp.path())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn live projection"),
    ));
    let stdout = server.0.as_mut().unwrap().stdout.take().unwrap();
    let mut startup = String::new();
    BufReader::new(stdout).read_line(&mut startup).unwrap();
    let envelope: Value = serde_json::from_str(startup.trim_end()).unwrap();
    assert_eq!(
        envelope["ok"],
        json!(true),
        "live projection startup failed: {envelope}"
    );
    let url = envelope["result"]["url"]
        .as_str()
        .expect("a successful live projection has result.url");
    assert!(url.starts_with("http://127.0.0.1:"));
    assert_eq!(envelope["command"], "view");
    assert_eq!(envelope["result"]["mode"], "server");

    let shell = http_get(url, "/", None);
    assert!(shell.starts_with("HTTP/1.1 200 "), "/: {shell}");
    let cookie = session_cookie(&shell);

    let live_data = http_get(url, "/data.js", Some(&cookie));
    assert!(
        live_data.starts_with("HTTP/1.1 200 "),
        "/data.js: {live_data}"
    );
    let live_payload = data_payload(
        live_data
            .split_once("\r\n\r\n")
            .expect("HTTP response separates headers and body")
            .1,
    );
    assert_eq!(live_payload["meta"]["mode"], "live");
    assert_eq!(live_payload["snapshot"], export_payload["snapshot"]);

    let live_status = http_get(url, "/live.json", Some(&cookie));
    assert!(
        live_status.starts_with("HTTP/1.1 200 "),
        "/live.json: {live_status}"
    );
    let status: Value = serde_json::from_str(
        live_status
            .split_once("\r\n\r\n")
            .expect("HTTP response separates headers and body")
            .1,
    )
    .expect("live.json is valid JSON");
    assert_eq!(
        status,
        json!({"generation": 0, "reload_error": null, "watcher_error": null})
    );

    for route in [
        "/intents",
        "/graph",
        "/glossary",
        "/intent/INT-0042",
        "/coverage",
        "/missing",
    ] {
        let response = http_get(url, route, Some(&cookie));
        assert!(response.starts_with("HTTP/1.1 404 "), "{route}: {response}");
    }

    drop(server);
    assert_state(tmp.path(), "coherent");
}
