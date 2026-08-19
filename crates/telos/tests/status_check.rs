//! End-to-end tests for `telos status` and `telos check [--sealed]`: the
//! frozen `status --json` schema (Annex B), drift reporting, integrity
//! checking, and the `--sealed` gate. Every test runs the real binary
//! against the sealed `billing` corpus fixture (or a deliberately broken
//! copy of it) -- these prove the CLI contract, not the engine (that lives
//! in `telos-core`'s own tests).

mod common;

use std::fs;

use serde_json::{Value, json};

use common::{telos, unsealed_fixture, with_fixture};

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The `telos/notions/Invoice.tel` path, repeated across several tests.
const INVOICE_TEL: &str = "telos/notions/Invoice.tel";

/// The exact `TELOS_DRIFT_DETECTED` hint `check --sealed` reports, frozen
/// by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

// --- status: the golden schema ------------------------------------------

/// The whole envelope, on a freshly sealed, untouched fixture, equals the
/// golden JSON from Annex B's «Schéma `status --json` gelé» -- byte for
/// byte as a `Value`, not just field by field.
#[test]
fn status_json_on_the_sealed_fixture_matches_the_golden_envelope() {
    let tmp = with_fixture();
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
                "state": "coherent",
                "changes": [],
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
            "next_actions": []
        })
    );
}

#[test]
fn status_reports_drift_after_a_one_byte_append_to_a_sealed_file() {
    let tmp = with_fixture();
    let invoice_tel = tmp.path().join(INVOICE_TEL);
    let mut content = fs::read_to_string(&invoice_tel).unwrap();
    content.push('\n');
    fs::write(&invoice_tel, content).unwrap();

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "status reports, it does not fail -- expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["state"], json!("drifted"));
    assert_eq!(
        envelope["result"]["drift"],
        json!({ "paths": [INVOICE_TEL], "suggestion": "telos adopt" })
    );
    assert_eq!(
        envelope["next_actions"],
        json!(["telos adopt", "telos revert"])
    );
}

#[test]
fn status_without_a_lock_is_not_initialized() {
    let tmp = unsealed_fixture();

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["command"], json!("status"));
    assert_eq!(envelope["result"], Value::Null);
    assert_eq!(envelope["error"]["code"], json!("TELOS_NOT_INITIALIZED"));
    assert_eq!(envelope["error"]["message"], json!("telos.lock is missing"));
}

/// A spec file that no longer parses is still drift ([`compute_state`]
/// never parses anything), and `status` still answers: the state is
/// `drifted` (the OID no longer matches what was sealed) and coverage
/// falls back to all zeros, since `load_model` cannot be trusted to
/// describe a spec it failed to read.
#[test]
fn status_on_a_corrupted_spec_file_still_reports_drifted_with_zero_coverage() {
    let tmp = with_fixture();
    fs::write(
        tmp.path().join(INVOICE_TEL),
        "@@@ not valid .tel syntax @@@\n",
    )
    .unwrap();

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["state"], json!("drifted"));
    assert_eq!(
        envelope["result"]["coverage"],
        json!({
            "notions": 0,
            "constraints": 0,
            "intents_total": 0,
            "intents_active": 0,
            "scenarios_total": 0,
            "scenarios_proved": 0,
            "intents_implemented": 0
        })
    );
}

// --- check: the happy path -----------------------------------------------

#[test]
fn check_on_the_fixture_passes() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["check", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["result"], json!({ "diagnostics": [] }));
}

#[test]
fn check_sealed_on_an_intact_fixture_passes() {
    let tmp = with_fixture();

    telos(tmp.path(), &["check", "--sealed"]).assert().success();
}

// --- check: integrity failures --------------------------------------------

/// Injecting `on Invoce` (a syntactically valid but unresolvable notion
/// name) into `INT-0042.tel`'s event-driven statement makes the spec fail
/// to load with a `TELOS_REFERENCE_UNKNOWN` diagnostic whose message names
/// the closest known notion. `check` does not consult the lock at all, so
/// this doesn't need the fixture to be resealed -- it's already sealed by
/// `with_fixture`, and `check` (without `--sealed`) ignores that.
#[test]
fn check_json_on_an_unresolvable_reference_reports_telos_reference_unknown() {
    let tmp = with_fixture();
    let int_0042 = tmp.path().join("telos/intents/INT-0042.tel");
    let content = fs::read_to_string(&int_0042).unwrap();
    assert!(
        content.contains("on Invoice"),
        "fixture no longer contains the expected `on Invoice` clause"
    );
    fs::write(&int_0042, content.replace("on Invoice", "on Invoce")).unwrap();

    let out = telos(tmp.path(), &["check", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["result"], Value::Null);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("closest is `Invoice`"),
        "message: {message}"
    );
    // The suggestion lives in the message, not the hint: semantic reference
    // diagnostics never attach one.
    assert_eq!(envelope["error"]["hint"], Value::Null);
}

// --- check --sealed: the drift gate ---------------------------------------

#[test]
fn check_sealed_on_a_drifted_fixture_reports_telos_drift_detected() {
    let tmp = with_fixture();
    let invoice_tel = tmp.path().join(INVOICE_TEL);
    let mut content = fs::read_to_string(&invoice_tel).unwrap();
    content.push('\n');
    fs::write(&invoice_tel, content).unwrap();

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(envelope["error"]["hint"], json!(DRIFT_HINT));
}

/// A corrupted, unparseable spec file is drift *first*: `--sealed` checks
/// state before it ever tries to parse, so a spec that is both drifted and
/// syntactically broken reports `TELOS_DRIFT_DETECTED`, never
/// `TELOS_PARSE_ERROR`.
#[test]
fn check_sealed_on_a_corrupted_spec_reports_drift_not_a_parse_error() {
    let tmp = with_fixture();
    fs::write(
        tmp.path().join(INVOICE_TEL),
        "@@@ not valid .tel syntax @@@\n",
    )
    .unwrap();

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
}
