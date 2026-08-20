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
//! loop's `#[ignore]` comes off and the test starts asserting for real.
//! `loop_merge` came off at M2, which landed `change open`/`edit`/`approve`/
//! `reconcile [--full]` -- it now runs in the ordinary suite and is the
//! milestone's executable done-criterion. `loop_feature` and `loop_drift`
//! stay `#[ignore]`d for M3, but for two different reasons, and
//! `cargo test --workspace -- --ignored` shows the difference today:
//!
//! - `loop_feature` **fails**, on the *first* command M2 doesn't have
//!   (`test`/`bind`, and the red/green witness protocol) -- not on a compile
//!   error or a panic unrelated to the missing surface. That failure is this
//!   file doing its job: it is the roadmap's done-criterion, not a bug to
//!   fix here.
//! - `loop_drift` **passes** as written: every command it calls landed at
//!   M2. It stays `#[ignore]`d for milestone sequencing -- the Implement
//!   phase it currently skips, going straight from `approve` to `reconcile`,
//!   is the part M3 fills in, and its `#[ignore]` comes off then, alongside
//!   `loop_feature`'s. See the note on the test itself.
//!
//! Payload JSON shapes fed to `add`/`edit` on stdin follow Annex D, frozen
//! by T13 into `docs/contracts.md` -- `loop_feature` uses the real,
//! agent-facing shape rather than an invented one.

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

    // --- Configure the test runner, through the protocol ----------------
    // M1 has no real cargo project in this throwaway sandbox repo to
    // compile a failing test against, so `telos.toml`'s `[test] cmd` is
    // pointed at a tiny shell script this test controls, which reports red
    // or green by reading a marker file the loop flips between the two
    // `telos test` calls below (M1 stand-in only -- M3 exercises a real
    // cargo project end to end).
    //
    // `telos/telos.toml` is itself a sealed spec file (`Workspace::
    // spec_files`), so writing it directly on disk leaves it as *unclaimed*
    // drift, which outranks any later open change (M2: drift refuses
    // `change open`/`approve`/`reconcile` before a change even gets a
    // chance to claim anything else). So this goes through the protocol
    // like every other write, using `adopt` -- the only way to stage an
    // opaque file telos does not model as an entity -- and does so *before*
    // "Invoices can be settled" opens, so that change's own base is
    // coherent again by the time it starts.
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
    let adopted_toml = run_ok(dir, &["adopt", "--json"]);
    let toml_change_id = adopted_toml["result"]["change"]
        .as_str()
        .expect("`adopt` answers with the id of the change capturing the drift (M2)")
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
        .expect("`change open` answers with the new change's id (M2)")
        .to_string();

    // Payload shapes are Annex D's frozen `add` shape (docs/contracts.md):
    // a notion's identity is its `name`, an `add` never carries an id of
    // its own, and `given`/`when` steps carry their state under `fields`.
    // These mirror the spec's canonical example (§4.5): the `Invoice`
    // notion, the `PaymentReceived` event it reacts to, and the intent +
    // scenario that ties them together.
    run_ok_stdin(
        dir,
        &["add", "notion", "--change", &change_id, "--json"],
        r#"{
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
            "name": "PaymentReceived",
            "kind": "event",
            "def": "A payment arrived for an invoice."
        }"#,
    );
    let added_intent = run_ok_stdin(
        dir,
        &["add", "intent", "--change", &change_id, "--json"],
        r#"{
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
    // `add` never carries an id (Annex D): the intent's id and its
    // scenario's id are both allocated by the CLI and captured from
    // `result` here, never hardcoded.
    let intent_id = added_intent["result"]["id"]
        .as_str()
        .expect("`add intent` answers with the allocated intent id (M2)")
        .to_string();
    let scenario_id = added_intent["result"]["scenario_ids"][0]
        .as_str()
        .expect("`add intent` reports the allocated scenario id(s) (M2)")
        .to_string();

    // --- Approve -------------------------------------------------------
    // `diff` renders the staged delta for human review; `approve` locks the
    // approval to that delta's digest -- editing it afterward invalidates
    // the approval (M2, `TELOS_APPROVAL_STALE`), but this loop never edits
    // after approving.
    run_ok(dir, &["change", "diff", &change_id, "--json"]);
    run_ok(dir, &["change", "approve", &change_id, "--json"]);

    // --- Implement -------------------------------------------------------
    // Red witness before green, per §7.2.4.
    let red = run_ok(dir, &["test", &scenario_id, "--json"]);
    assert_eq!(red["result"]["witness"], json!("red"));

    fs::write(dir.join(".fake-test-green"), "").expect("failed to flip the marker to green");

    let green = run_ok(dir, &["test", &scenario_id, "--json"]);
    assert_eq!(green["result"]["witness"], json!("green"));

    // Minimal domain code, bound to the intent it implements.
    fs::create_dir_all(dir.join("src")).expect("failed to create src/");
    fs::write(
        dir.join("src/billing.rs"),
        "// Minimal domain code, named after the notions it implements.\n",
    )
    .expect("failed to write src/billing.rs");
    run_ok(dir, &["bind", "src/billing.rs", &intent_id, "--json"]);

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
/// Every command it calls landed at M2, and it passes as written today
/// (verified at T13 and again at T14 with `cargo test -p telos --test
/// acceptance_loops -- --ignored`). It stays `#[ignore]`d anyway: the
/// milestone plan schedules its un-ignoring for M3, alongside
/// `loop_feature`, and this loop's Implement phase -- the one it currently
/// skips entirely, going straight from `approve` to `reconcile` -- is the
/// part M3's `test`/`bind` fill in.
#[test]
#[ignore = "un-ignored at M3 (passes today; its Implement phase is still M3's)"]
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
/// merged. The spec files themselves merge cleanly -- §5's promise that
/// conflicts stay "rare and localized" -- so `check` is green the moment the
/// merge stops; but `check --sealed` stays red, because the *seal* is what
/// conflicted, until `telos change reconcile --full` re-validates every §3.3
/// rule, re-runs the impacted proof obligations, and re-seals. Proof, not a
/// bypass.
///
/// Un-ignored at M2, which landed `change open`/`edit`/`approve`/`reconcile
/// [--full]` -- the loop's last missing commands. It is M2's executable
/// done-criterion.
///
/// # Amendments made when this loop was un-ignored (T14)
///
/// The loop was committed at M1 as a prediction of M2's behaviour. Its spine
/// -- diverge, merge, conflict on the lock alone, `--full` as the way out --
/// held exactly as predicted when it was first run for real. Three of its
/// *comments*, though, described steps that never happen; the amendments
/// below correct those and write down the assertions the loop had so far
/// only implied. No assertion was weakened.
///
/// 1. **`#[ignore = "un-ignored at M2"]` removed.** M2 landed every command
///    this loop calls.
/// 2. **The two `git checkout --ours/--theirs` calls and their `git add`
///    are gone.** They claimed to "resolve the two `.tel` conflicts by
///    hand"; there are none. Each branch edits a *different* intent file, so
///    git auto-merges both edits -- `git diff --name-only --diff-filter=U`
///    after the merge lists exactly `telos/telos.lock`, and the working tree
///    holds branch A's `INT-0017` wording *and* branch B's `INT-0042`
///    wording. `git checkout --ours <path>` needs a stage-2 entry to pick
///    from; with none, git 2.55 (the version this was verified against)
///    makes it a silent no-op -- "0 paths updated from the index" -- and
///    nothing promises it stays silent. Either way the call asserted a
///    hand-resolution that never happens. The two assertions that replace it
///    (the conflict set is exactly the lock; both wordings survive) are what
///    §5's promise actually means.
/// 3. **Branch B's comment corrected.** It said the two locks conflict
///    because each carries a "new sealing change id". They do not: change
///    ids are allocated per branch from that branch's own counters (D4), so
///    both branches seal their own `CHG-0001` and the `sealed_by` line
///    merges cleanly. What conflicts is `spec_digest` plus the two
///    `[spec]` OID lines around the touched intents.
/// 4. **State assertions added at every phase** (`status --json`): coherent
///    fixture -> `changing` while each branch's transaction is open ->
///    coherent after each branch reconciles -> coherent after the merge is
///    resolved by `--full`. Plus the `--full` envelope itself
///    (`id: null`, `ops_applied: 0`, `checks_run: 1` -- D11's *total*
///    constraint revalidation, not the impacted subset) and a green
///    `check --sealed` at the end, which is the only real proof the re-seal
///    happened.
/// 5. **No `status` assertion in the conflicted phase, deliberately.**
///    `status` reports the tree *against its seal*, and here the seal is the
///    thing carrying git's conflict markers: it answers `TELOS_PARSE_ERROR`
///    (contracts.md's `TELOS_PARSE_ERROR` row covers `telos.lock`). That
///    phase is therefore asserted through `check`/`check --sealed`, which is
///    also exactly the pair §7.4 talks about.
/// 6. **"reruns the whole test suite" softened to the truth in the doc
///    comment.** The `billing` corpus ships `[test] cmd = ""` (D13), so D10
///    skips the run and `tests_run` is `0`. What `--full` re-proves *here*
///    is the whole §3.3 rule set and every constraint; asserting a suite the
///    corpus does not configure would be asserting a fiction.
/// 7. **The merge is now committed at the end.** The loop used to stop with
///    `telos.lock` still unmerged in git's index; a merge nobody can commit
///    is not a merge resolved. `git commit` after `--full` is the last step
///    of §7.4's story, and it is what proves the re-derived lock is a real
///    resolution and not just a file on disk.
///
/// # The one M3 amendment (T5), and what this loop does *not* prove
///
/// The pinned `--full` result gained a `"witness_warnings": []` line, and
/// nothing else in this loop moved. The field is M3's, additive, and always
/// present (D7): a `--full` reseal belongs to no change, hence to no
/// journal, hence to no witness verdict -- `[]` is the only value it can
/// take here, and pinning it is one more thing asserted rather than one
/// fewer.
///
/// This loop is **inert on the witness gate**, and deliberately left that
/// way. The `billing` corpus ships `[test] cmd = ""` (D13), and a project
/// with no runner owes no witness -- `check_witnesses` returns at that
/// carve-out before it ever asks who owes one. So the two branch reconciles
/// here would pass whatever D7's scope rule said, and nothing about that
/// rule may be inferred from this loop being green. Wiring a runner in just
/// to make it say something would move the `--full` envelope this loop
/// pins, for a property it is not the right place to assert.
///
/// The scope rule -- an intent edited without touching its scenarios owes no
/// new witness, because the emitted fragments are identical -- is pinned
/// where it can actually fail: `crates/telos/tests/reconcile.rs`, by
/// `rebinding_a_pair_the_sealed_file_already_holds_leaves_it_unchanged` (a
/// no-op `edit intent`, under `strict`, *with* a runner configured) and by
/// the strict-mode family around it, which prove the gate refuses when a
/// witness really is owed. `required_witnesses`' own unit tests
/// (`telos-core/src/witness.rs`) pin the fragment comparison itself.
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
    // point. Change ids are per-branch (D4), so branch B seals its own
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

    // §5, checked rather than asserted in prose: the spec files are ordinary
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
    let int_0017 = fs::read_to_string(dir.join("telos/intents/INT-0017.tel"))
        .expect("failed to read INT-0017.tel");
    assert!(
        int_0017.contains("branch A wording"),
        "branch A's INT-0017 edit must survive the merge, got: {int_0017}"
    );
    let int_0042 = fs::read_to_string(dir.join("telos/intents/INT-0042.tel"))
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
    // §7.4: total integrity revalidation, every constraint re-checked, every
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
            "tests_run": 0,
            "witness_warnings": [],
        }),
        "--full applies no ops, belongs to no change, and re-checks every \
         constraint (the corpus has one, and ships no test runner -- D13)"
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
