//! The `--json` envelope contract (Annex B): every command, whatever it
//! does and however it fails, answers with the same five keys. Nothing here
//! cares what a particular command puts in `result` -- that is each
//! command's own golden test. These tests care only about the shape, which
//! agent tooling parses blind.

mod common;

use serde_json::{Value, json};

use common::{repo, telos};

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
