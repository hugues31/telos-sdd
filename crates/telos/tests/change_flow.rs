//! End-to-end tests for `telos change open|list|abandon`, the `changing`
//! state `status` now really reports, `show CHG-…`, and the two gates of
//! D17/D15: `change open` refused on unclaimed drift, `check --sealed`
//! refused while a change is open.
//!
//! Every test drives the real binary against the sealed `billing` corpus
//! fixture. What they prove is the CLI contract -- the frozen result shapes
//! of Annex E, the exact bytes of what lands on disk, and the never-reuse
//! rule of D4 -- not the engine, which has its own tests in `telos-core`.

mod common;

use std::fs;

use serde_json::{Value, json};

use common::{telos, with_fixture};

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The motivation every test opens its change with -- Annex C's own.
const MOTIVATION: &str = "Invoices can be settled";

/// The first change's file, relative to the repository root.
const CHG_0001: &str = "telos/changes/CHG-0001.tel";

/// The bytes `telos change open` must leave at [`CHG_0001`]: a change with
/// no op yet, in the canonical form `emit_change` defines (D1).
const CANONICAL_OPEN_CHANGE: &str =
    "change CHG-0001 \"Invoices can be settled\" {\n  status open\n}\n";

/// The `telos/notions/Invoice.tel` path, drifted by several tests.
const INVOICE_TEL: &str = "telos/notions/Invoice.tel";

/// The exact `TELOS_DRIFT_DETECTED` hint, frozen by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

/// The obligations of an `open` change, frozen by Annex E.
fn open_obligations() -> Value {
    json!(["stage the delta", "approve", "reconcile"])
}

/// Runs `telos change open <motivation>` and returns the allocated id.
fn open_change(dir: &std::path::Path, motivation: &str) -> String {
    let out = telos(dir, &["change", "open", motivation, "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "`telos change open`: expected exit 0, got {:?} -- {}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    json_stdout(&out)["result"]["id"]
        .as_str()
        .expect("`change open` reports the allocated id")
        .to_string()
}

/// Appends one byte to a sealed spec file: the minimal unclaimed drift.
fn drift(dir: &std::path::Path) {
    let path = dir.join(INVOICE_TEL);
    let mut content = fs::read_to_string(&path).unwrap();
    content.push('\n');
    fs::write(&path, content).unwrap();
}

// --- change open ---------------------------------------------------------

/// The whole envelope, on a freshly sealed fixture, equals Annex E's
/// golden: the result pair and the single next action, byte for byte as a
/// `Value`.
#[test]
fn change_open_json_matches_the_golden_envelope() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "open", MOTIVATION, "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "change",
            "result": { "id": "CHG-0001", "status": "open" },
            "error": null,
            "next_actions": ["telos add intent --change CHG-0001"]
        })
    );
}

/// The file `change open` writes is canonical to the byte -- it is
/// `emit_change` that wrote it, and D1's round-trip is what makes it
/// readable back.
#[test]
fn change_open_writes_the_canonical_change_file_byte_for_byte() {
    let tmp = with_fixture();

    open_change(tmp.path(), MOTIVATION);

    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        CANONICAL_OPEN_CHANGE
    );
}

/// `counters.toml` is persisted at the corpus' own floors, with `change`
/// alone advanced by the allocation (D4): the billing corpus tops out at
/// INT-0042, SCN-0107 and CON-0003, none of which this command allocated.
#[test]
fn change_open_persists_the_counters_at_the_corpus_floors() {
    let tmp = with_fixture();

    open_change(tmp.path(), MOTIVATION);

    assert_eq!(
        fs::read_to_string(tmp.path().join("telos/changes/counters.toml")).unwrap(),
        "intent = 42\nscenario = 107\nconstraint = 3\nchange = 1\n"
    );
}

#[test]
fn a_second_change_open_allocates_the_next_id() {
    let tmp = with_fixture();

    assert_eq!(open_change(tmp.path(), MOTIVATION), "CHG-0001");
    assert_eq!(open_change(tmp.path(), "another motivation"), "CHG-0002");
}

/// The floor scan is what makes D4 hold when `counters.toml` is not to be
/// trusted -- a bad merge resolution, or the file simply gone -- *and* when
/// the change on disk cannot be parsed to be scanned for its ops: its id is
/// still read off its file name, so the next allocation lands past it
/// instead of reissuing it over a file that already exists.
#[test]
fn an_unparseable_change_still_holds_the_counter_down_without_counters_toml() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    fs::write(tmp.path().join(CHG_0001), "@@@ not a change @@@\n").unwrap();
    fs::remove_file(tmp.path().join("telos/changes/counters.toml")).unwrap();

    assert_eq!(open_change(tmp.path(), "after the damage"), "CHG-0002");
    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        "@@@ not a change @@@\n",
        "the damaged file must not have been overwritten by a reissued id"
    );
}

// --- status: the changing state ------------------------------------------

/// With one change open and nothing drifted, `status` reports `changing`,
/// the real `changes[]` of Annex E, and the one next action D15 names.
#[test]
fn status_json_with_an_open_change_reports_the_changing_state() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "status",
            "result": {
                "state": "changing",
                "changes": [{
                    "id": "CHG-0001",
                    "status": "open",
                    "obligations": ["stage the delta", "approve", "reconcile"]
                }],
                "drift": null,
                "coverage": {
                    "notions": 4,
                    "constraints": 1,
                    "intents_total": 2,
                    "intents_active": 2,
                    "scenarios_total": 2,
                    "scenarios_proved": 1,
                    "intents_implemented": 1
                }
            },
            "error": null,
            "next_actions": ["telos change list"]
        })
    );
}

// --- change abandon ------------------------------------------------------

#[test]
fn change_abandon_reports_the_abandoned_id_and_deletes_the_file() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "abandon", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    assert_eq!(
        envelope["result"],
        json!({ "id": "CHG-0001", "status": "abandoned" })
    );
    assert!(
        !tmp.path().join(CHG_0001).exists(),
        "the change file must be gone"
    );

    // Nothing is left open -- in particular `counters.toml`, which stays
    // behind in `telos/changes/`, is not mistaken for a change file.
    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_stdout(&out)["result"], json!({ "changes": [] }));
    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();
    assert_eq!(json_stdout(&out)["result"]["state"], json!("coherent"));
}

/// A malformed id is a domain error under the same code as an id that does
/// not exist, not a clap usage error: an agent reading the envelope finds
/// both under `TELOS_REFERENCE_UNKNOWN`.
#[test]
fn change_abandon_of_a_malformed_id_is_a_domain_error() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "abandon", "nonsense", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("cannot parse `nonsense` as a change id")
    );
}

/// D4's whole point: an abandoned id is never handed out again, because the
/// counters file remembers it even once its change file is deleted.
#[test]
fn an_abandoned_id_is_never_reused() {
    let tmp = with_fixture();
    assert_eq!(open_change(tmp.path(), MOTIVATION), "CHG-0001");
    assert_eq!(open_change(tmp.path(), "second"), "CHG-0002");

    telos(tmp.path(), &["change", "abandon", "CHG-0001"])
        .assert()
        .success();

    assert_eq!(open_change(tmp.path(), "third"), "CHG-0003");
}

/// Abandoning an id the store does not hold is the change store's own
/// `TELOS_REFERENCE_UNKNOWN`, with its nearest-id hint.
#[test]
fn change_abandon_of_an_unknown_id_reports_the_store_error() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "abandon", "CHG-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("unknown change `CHG-9999`")
    );
    assert_eq!(envelope["error"]["hint"], json!("closest is CHG-0001"));
}

// --- change list ---------------------------------------------------------

#[test]
fn change_list_json_reports_id_status_motivation_and_obligations() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    open_change(tmp.path(), "second motivation");

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    assert_eq!(
        envelope["result"],
        json!({
            "changes": [
                {
                    "id": "CHG-0001",
                    "status": "open",
                    "motivation": MOTIVATION,
                    "obligations": open_obligations()
                },
                {
                    "id": "CHG-0002",
                    "status": "open",
                    "motivation": "second motivation",
                    "obligations": open_obligations()
                }
            ]
        })
    );
}

#[test]
fn change_list_on_a_project_with_no_change_is_empty() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(json_stdout(&out)["result"], json!({ "changes": [] }));
}

/// Human mode: one line per change, naming the id, the status and the
/// motivation.
#[test]
fn change_list_human_mode_prints_one_line_per_change() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "list"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count(), 1, "one line per change: {stdout}");
    let line = stdout.lines().next().unwrap();
    assert!(line.contains("CHG-0001"), "line: {line}");
    assert!(line.contains("open"), "line: {line}");
    assert!(line.contains(MOTIVATION), "line: {line}");
}

/// An unparseable change file does not take `change list` down with it
/// (D15): the entry is still reported, with an empty motivation (there is
/// nothing trustworthy to read) and the repair obligation.
#[test]
fn change_list_is_best_effort_on_an_unparseable_change_file() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    fs::write(tmp.path().join(CHG_0001), "@@@ not a change @@@\n").unwrap();

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out)["result"],
        json!({
            "changes": [{
                "id": "CHG-0001",
                "status": "open",
                "motivation": "",
                "obligations": ["repair telos/changes/CHG-0001.tel (unparseable)"]
            }]
        })
    );
}

// --- show CHG-… ----------------------------------------------------------

#[test]
fn show_of_a_change_matches_the_annex_e_shape() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("show"));
    assert_eq!(
        envelope["result"],
        json!({
            "entity": {
                "id": "CHG-0001",
                "status": "open",
                "motivation": MOTIVATION,
                "ops": []
            },
            "canonical": CANONICAL_OPEN_CHANGE,
            "relations": { "out": [], "in": [] }
        })
    );
}

#[test]
fn show_of_an_unknown_change_reports_the_store_error() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("unknown change `CHG-9999`")
    );
    assert_eq!(envelope["error"]["hint"], json!("closest is CHG-0001"));
}

/// Human mode prints the change's canonical text, then the (always empty)
/// relations section a change has -- a change is not a graph node.
#[test]
fn show_of_a_change_human_mode_prints_the_canonical_text_and_no_relation() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-0001"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{CANONICAL_OPEN_CHANGE}\nrelations:\n")
    );
}

// --- D17: the drift gate --------------------------------------------------

#[test]
fn change_open_on_a_drifted_project_is_refused() {
    let tmp = with_fixture();
    drift(tmp.path());

    let out = telos(tmp.path(), &["change", "open", MOTIVATION, "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(envelope["error"]["hint"], json!(DRIFT_HINT));
    assert!(
        !tmp.path().join(CHG_0001).exists(),
        "a refused `change open` allocates nothing"
    );
}

/// D17 permits `list` and `abandon` while drifted: they are the two ways
/// out of a mess, not more mutation of the spec.
#[test]
fn change_list_and_abandon_are_allowed_while_drifted() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    drift(tmp.path());

    telos(tmp.path(), &["change", "list"]).assert().success();
    telos(tmp.path(), &["change", "abandon", "CHG-0001"])
        .assert()
        .success();
    assert!(!tmp.path().join(CHG_0001).exists());
}

// --- D15: check --sealed while changing -----------------------------------

#[test]
fn check_sealed_while_a_change_is_open_reports_change_state_invalid() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_CHANGE_STATE_INVALID")
    );
    assert_eq!(
        envelope["error"]["message"],
        json!("open changes; reconcile or abandon them")
    );
    assert_eq!(envelope["error"]["hint"], json!("run `telos change list`"));
}

/// Drift still wins over an open change: the state priority of D15 is
/// unclaimed drift first, and `check --sealed` reports what it finds.
#[test]
fn check_sealed_reports_drift_before_open_changes() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    drift(tmp.path());

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    assert_eq!(
        json_stdout(&out)["error"]["code"],
        json!("TELOS_DRIFT_DETECTED")
    );
}

/// `check` without `--sealed` never looks at state at all: an open change
/// changes nothing about whether the spec parses and holds together.
#[test]
fn check_without_sealed_passes_while_a_change_is_open() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    telos(tmp.path(), &["check"]).assert().success();
}
