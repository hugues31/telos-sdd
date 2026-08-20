//! The `--json` envelope contract (Annex B): every command, whatever it
//! does and however it fails, answers with the same five keys. Nothing here
//! cares what a particular command puts in `result` -- that is each
//! command's own golden test. These tests care only about the shape, which
//! agent tooling parses blind.

mod common;

use std::collections::BTreeSet;

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

/// M3's three agent-facing verbs are no exception to Annex B. `context`
/// exposes its bounded pack as a successful envelope, while an unowned
/// `test` or `bind` still produces a complete, precisely routable failure.
/// These assertions deliberately use the shared sealed corpus instead of
/// recreating a transaction fixture: this test owns the envelope contract,
/// not the commands' lifecycle fixtures.
#[test]
fn context_test_and_bind_freeze_their_real_json_envelopes() {
    let tmp = with_fixture();

    let context_out = telos(tmp.path(), &["context", "INT-0042", "--json"])
        .output()
        .unwrap();
    assert!(
        context_out.status.success(),
        "context failed: {context_out:?}"
    );
    let context = envelope(&context_out);
    assert_exactly_the_five_keys(&context);
    assert_eq!(context["ok"], json!(true));
    assert_eq!(context["command"], json!("context"));
    assert_eq!(context["error"], Value::Null);
    assert_eq!(context["next_actions"], json!([]));
    let result = context["result"]
        .as_object()
        .expect("context result is an object");
    assert_eq!(
        result.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "bindings",
            "canonical",
            "change",
            "constraints",
            "id",
            "neighbors",
            "notions",
            "scenarios",
        ]
    );
    assert_eq!(result["id"], json!("INT-0042"));
    assert_eq!(result["change"], Value::Null);
    let bindings = result["bindings"]
        .as_object()
        .expect("context bindings is an object");
    assert_eq!(
        bindings.keys().map(String::as_str).collect::<Vec<_>>(),
        ["implements", "proves"]
    );

    for (args, expected) in [
        (
            ["test", "SCN-0107", "--json"].as_slice(),
            json!({
                "ok": false,
                "command": "test",
                "result": null,
                "error": {
                    "code": "TELOS_CHANGE_STATE_INVALID",
                    "message": "no open change is implementing SCN-0107",
                    "hint": "stage it into a change and approve it first",
                },
                "next_actions": [],
            }),
        ),
        (
            ["bind", "src/billing/invoice.rs", "INT-0017", "--json"].as_slice(),
            json!({
                "ok": false,
                "command": "bind",
                "result": null,
                "error": {
                    "code": "TELOS_CHANGE_STATE_INVALID",
                    "message": "no open change is implementing INT-0017",
                    "hint": "stage it into a change and approve it first",
                },
                "next_actions": [],
            }),
        ),
    ] {
        let out = telos(tmp.path(), args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "expected domain error: {args:?}"
        );
        let actual: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            actual,
            expected,
            "unexpected `telos {}` envelope",
            args.join(" ")
        );
    }
}

/// Host integration changes files outside `telos/`, never the frozen `init`
/// result an agent routes on. Test the ordinary and `--agents` forms through
/// the real binary so a future installer cannot accidentally grow a
/// host-specific result shape.
#[test]
fn init_and_init_agents_keep_the_same_exact_envelope() {
    let expected = json!({
        "ok": true,
        "command": "init",
        "result": {"root": "telos", "sealed": true},
        "error": null,
        "next_actions": ["telos status"],
    });

    for args in [
        ["init", "--json"].as_slice(),
        ["init", "--agents", "codex", "--json"].as_slice(),
    ] {
        let tmp = repo();
        let out = telos(tmp.path(), args).output().unwrap();
        assert!(
            out.status.success(),
            "`telos {}` failed: {out:?}",
            args.join(" ")
        );
        let actual: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            actual,
            expected,
            "`telos {}` changed init's contract",
            args.join(" ")
        );
    }
}

/// Extracts the one canonical, one-row-per-code table from the published
/// Error codes section. The detailed cases below it intentionally repeat
/// codes; this table is the machine-readable enumeration an agent routes on.
fn published_error_codes(contracts: &str) -> Vec<&str> {
    let canonical = contracts
        .split_once("### Canonical error-code set")
        .expect("docs/contracts.md has a canonical Error codes table")
        .1;
    let table = canonical
        .split_once("### Detailed emission cases")
        .expect("canonical Error codes table ends before detailed cases")
        .0;

    let mut lines = table.lines().filter(|line| !line.is_empty());
    assert_eq!(lines.next(), Some("| Code |"));
    assert_eq!(lines.next(), Some("|---|"));

    lines
        .map(|line| {
            assert!(
                line.starts_with('|') && line.ends_with('|'),
                "bad code row: {line}"
            );
            let cells: Vec<&str> = line.split('|').collect();
            assert_eq!(cells.len(), 3, "code row has one column: {line}");
            cells[1]
                .trim()
                .strip_prefix('`')
                .and_then(|code| code.strip_suffix('`'))
                .expect("canonical code is backtick-quoted")
        })
        .collect()
}

/// The canonical table is an exact set, not a sampling of prose. This catches
/// a removed code, an accidental eighteenth code, and a duplicate row.
#[test]
fn published_error_code_table_is_exact_and_unique() {
    let contracts = include_str!("../../../docs/contracts.md");
    let documented = published_error_codes(contracts);
    let documented_set: BTreeSet<&str> = documented.iter().copied().collect();
    let live: BTreeSet<&str> = [
        "TELOS_DRIFT_DETECTED",
        "TELOS_APPROVAL_STALE",
        "TELOS_REFERENCE_UNKNOWN",
        "TELOS_SCENARIO_RED_EXPECTED",
        "TELOS_TEST_SEALED",
        "TELOS_ORPHAN_CODE",
        "TELOS_CONSTRAINT_FAILED",
        "TELOS_CHANGE_STATE_INVALID",
        "TELOS_FILE_CLAIMED",
        "TELOS_NOT_INITIALIZED",
        "TELOS_ALREADY_INITIALIZED",
        "TELOS_PARSE_ERROR",
        "TELOS_INTEGRITY_VIOLATION",
        "TELOS_CYCLE_DETECTED",
        "TELOS_GIT_ERROR",
        "TELOS_INTERNAL",
        "TELOS_TEST_NOT_FOUND",
    ]
    .into_iter()
    .collect();

    assert_eq!(live.len(), 17, "the executable ErrorCode set is complete");
    assert_eq!(
        documented.len(),
        documented_set.len(),
        "the canonical Error codes table must not repeat a code: {documented:?}"
    );
    assert_eq!(
        documented_set, live,
        "published codes differ from ErrorCode"
    );
}

#[test]
fn published_error_code_parser_accepts_crlf_contracts() {
    let contracts = include_str!("../../../docs/contracts.md")
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    assert!(!contracts.contains("\r\r\n"));
    let documented = published_error_codes(&contracts);

    assert_eq!(documented.len(), 17);
    assert!(documented.contains(&"TELOS_TEST_NOT_FOUND"));
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
        "`open → drafted → approved → implementing → reconciled`",
        "journal records are digest-inert",
        "The ten gates, frozen order",
        "| 7 | sealed code coverage: every path in the previous lock's `code` table remains bound in the folded post-model, unless this delta stages `telos/bindings.tel` | `TELOS_INTEGRITY_VIOLATION`",
        "strict versus advisory",
        "Structurally skips gates 1–4, 7, and 8",
        "the file passed with --file does not exist: `<path>`",
        "no file matched by the [tests] globs contains `scn_NNNN`",
        "name the test after the scenario id (`scn_NNNN_…`) in a file the [tests] globs cover, or pass `--file <path>`",
        "`scn_NNNN` appears in more than one test file: `<path>`, `<path>`",
        "The exact JSON result is identical with or without `--agents`",
        r#"`result`: `{"root": "telos", "sealed": true}`"#,
        r#"`next_actions`: `["telos status"]`"#,
    ] {
        assert!(
            contracts.contains(required),
            "docs/contracts.md must freeze the M3 contract phrase: {required}"
        );
    }
}

#[test]
fn published_contract_pins_reapproval_and_codex_activation() {
    let contracts = include_str!("../../../docs/contracts.md");
    let normalized = contracts.split_whitespace().collect::<Vec<_>>().join(" ");

    for required in [
        "Re-approval accepts both `approved` and `implementing` changes",
        "preserves the entering status in `result.status` (`approved` or `implementing`)",
        "refreshes `approved_digest` from the current ops digest, and makes the next `change diff` report `stale: false`",
        "open `/hooks`, review and trust the repository `.codex` layer",
        "verify the exact `telos agent-guard --host codex` hook",
        "Until that review and trust is complete, `.codex/hooks.json` and `.codex/rules/telos.rules` must be treated as inactive",
    ] {
        assert!(
            normalized.contains(required),
            "docs/contracts.md must freeze: {required}"
        );
    }
}

#[test]
fn readme_and_acceptance_comments_match_the_completed_m3_suite() {
    let readme = include_str!("../../../README.md");
    let normalized = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(normalized.contains("all three run in the ordinary test suite"));
    assert!(normalized.contains("The ignored-test list is expected to be empty"));
    assert!(!readme.contains("un-ignored one at a time"));
    assert!(!readme.contains("what is still ignored"));

    let acceptance = include_str!("acceptance_loops.rs");
    assert!(!acceptance.contains("`#[ignore]`d loops"));
}
