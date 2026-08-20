//! The `--json` envelope contract (Annex B): every command, whatever it
//! does and however it fails, answers with the same five keys. Nothing here
//! cares what a particular command puts in `result` -- that is each
//! command's own golden test. These tests care only about the shape, which
//! agent tooling parses blind.

mod common;

use serde_json::{Value, json};

use common::{break_int_0042_in_two_ways, repo, telos, with_fixture};

/// The five envelope keys, in the order Annex B freezes them.
const ENVELOPE_KEYS: [&str; 5] = ["ok", "command", "result", "error", "next_actions"];

fn envelope(out: &std::process::Output) -> serde_json::Map<String, Value> {
    let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other}"),
    }
}

fn assert_exactly_the_five_keys(map: &serde_json::Map<String, Value>) {
    assert_eq!(
        map.len(),
        5,
        "the envelope carries exactly five keys, got {:?}",
        map.keys().collect::<Vec<_>>()
    );
    for key in ENVELOPE_KEYS {
        assert!(map.contains_key(key), "missing envelope key `{key}`");
    }
}

#[test]
fn a_successful_command_answers_with_the_five_keys_and_a_null_error() {
    let tmp = repo();
    let out = telos(tmp.path(), &["version", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let map = envelope(&out);
    assert_exactly_the_five_keys(&map);
    assert_eq!(map["ok"], json!(true));
    assert_eq!(map["error"], Value::Null, "success carries no error body");
    assert!(!map["result"].is_null(), "success carries a result");
}

#[test]
fn a_failing_command_answers_with_the_five_keys_and_a_null_result() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let map = envelope(&out);
    assert_exactly_the_five_keys(&map);
    assert_eq!(map["ok"], json!(false));
    assert_eq!(map["result"], Value::Null, "an error carries no result");
    assert_eq!(map["next_actions"], json!([]));
}

/// The error body itself is a frozen triple: `code`, `message`, `hint` --
/// with `hint` present-and-null rather than absent when there is none, so a
/// consumer can read it unconditionally.
#[test]
fn the_error_body_always_carries_code_message_and_hint() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    let map = envelope(&out);
    let error = match &map["error"] {
        Value::Object(error) => error,
        other => panic!("expected an error object, got {other}"),
    };
    assert_eq!(
        error.len(),
        3,
        "the error body carries exactly three keys, got {:?}",
        error.keys().collect::<Vec<_>>()
    );
    for key in ["code", "message", "hint"] {
        assert!(error.contains_key(key), "missing error key `{key}`");
    }
    assert!(error["code"].is_string());
    assert!(error["message"].is_string());
}

/// `status` and `check` have richer `result` shapes than `version` or
/// `init` (a nested `coverage` object, a `diagnostics` array) -- the
/// generic envelope contract holds for them too: still exactly the five
/// keys, still a null `error` on success.
#[test]
fn status_and_check_success_envelopes_still_carry_exactly_the_five_keys() {
    let tmp = with_fixture();

    for args in [
        ["status", "--json"].as_slice(),
        ["check", "--json"].as_slice(),
    ] {
        let out = telos(tmp.path(), args).output().unwrap();
        assert!(
            out.status.success(),
            "`telos {}`: expected exit 0, got {:?}",
            args.join(" "),
            out.status
        );
        let map = envelope(&out);
        assert_exactly_the_five_keys(&map);
        assert_eq!(map["ok"], json!(true));
        assert_eq!(map["error"], Value::Null);
    }
}

/// `check`'s failure can collapse several diagnostics into one error body
/// (Annex B: the envelope carries one error, `check` can find several) --
/// even then, the error body stays the frozen `{code, message, hint}`
/// triple, never growing a fourth key to hold the rest. The fixture here
/// breaks the spec in two independent, unrelated ways
/// ([`break_int_0042_in_two_ways`]) specifically so this exercises the
/// multi-diagnostic path in `diagnostics_to_error`, not just the
/// single-diagnostic case: `code`/`hint` must come from the *first*
/// diagnostic only, while `message` carries both, newline-joined, so a
/// consumer that inspects it (or a human reading stderr, see
/// `status_check.rs`) still sees everything `check` found.
#[test]
fn checks_error_body_stays_the_frozen_triple_even_with_several_diagnostics() {
    let tmp = with_fixture();
    break_int_0042_in_two_ways(tmp.path());

    let out = telos(tmp.path(), &["check", "--json"]).output().unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let map = envelope(&out);
    assert_exactly_the_five_keys(&map);
    let error = match &map["error"] {
        Value::Object(error) => error,
        other => panic!("expected an error object, got {other}"),
    };
    assert_eq!(
        error.len(),
        3,
        "the error body carries exactly three keys, got {:?}",
        error.keys().collect::<Vec<_>>()
    );

    // `code` and `hint` are the first diagnostic's alone -- the unknown-
    // notion one (the statement is checked before `requires`), which never
    // attaches a hint.
    assert_eq!(error["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(error["hint"], Value::Null);

    // `message` is both diagnostics, newline-joined, first diagnostic
    // first -- proving neither is silently dropped to fit the frozen
    // triple.
    assert_eq!(
        error["message"],
        json!(
            "telos/intents/INT-0042.tel: unknown notion `Invoce`; closest is `Invoice`\n\
             telos/intents/INT-0042.tel: unknown intent `INT-9999`"
        )
    );
}

/// M3 extends the public surface with bounded context, red/green witnesses,
/// journalled bindings, and the two reconciliation gates that enforce them.
/// Keep the published contract at least as explicit as the executable one:
/// this is deliberately a literal, representative freeze rather than prose
/// that a caller must infer.
#[test]
fn published_contract_freezes_the_m3_surface() {
    let contracts = include_str!("../../../docs/contracts.md");

    for required in [
        "`context <INT-id|SCN-id>`",
        "`test <SCN-id|--all> [--file <path>]`",
        "`bind <path> <INT-id>`",
        "`TELOS_TEST_NOT_FOUND`",
        "`TELOS_SCENARIO_RED_EXPECTED`",
        "`TELOS_TEST_SEALED`",
        "`open → drafted → approved → implementing → reconciled`",
        "journal records are digest-inert",
        "The ten gates, frozen order",
        "strict versus advisory",
        "Structurally skips gates 1–4, 7, and 8",
        "`TELOS_DRIFT_DETECTED`",
        "`TELOS_APPROVAL_STALE`",
        "`TELOS_REFERENCE_UNKNOWN`",
        "`TELOS_ORPHAN_CODE`",
        "`TELOS_CONSTRAINT_FAILED`",
        "`TELOS_CHANGE_STATE_INVALID`",
        "`TELOS_FILE_CLAIMED`",
        "`TELOS_NOT_INITIALIZED`",
        "`TELOS_ALREADY_INITIALIZED`",
        "`TELOS_PARSE_ERROR`",
        "`TELOS_INTEGRITY_VIOLATION`",
        "`TELOS_CYCLE_DETECTED`",
        "`TELOS_GIT_ERROR`",
        "`TELOS_INTERNAL`",
    ] {
        assert!(
            contracts.contains(required),
            "docs/contracts.md must freeze the M3 contract phrase: {required}"
        );
    }
}
