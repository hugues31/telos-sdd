//! Contract tests for project configuration.

mod common;

use std::fs;

use serde_json::json;

use telos_core::changes::{read_change, write_change};
use telos_core::config::AgentHost;
use telos_core::ids::ChangeId;
use telos_core::model::{ChangeStatus, StagedOp};
use telos_core::workspace::Workspace;

use common::{repo, telos};

const PAYLOAD: &str = r#"{"code":{"globs":["src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}"},"policy":{"tdd":"advisory"},"agents":{"hosts":["claude","codex"]}}"#;

fn configured_change() -> tempfile::TempDir {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "claude,codex"])
        .assert()
        .success();
    telos(tmp.path(), &["change", "open", "configuration update"])
        .assert()
        .success();
    tmp
}

fn bytes(root: &std::path::Path) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        fs::read(root.join("telos/telos.toml")).unwrap(),
        fs::read(root.join("telos/changes/CHG-0001.tel")).unwrap(),
        fs::read(root.join("telos/changes/counters.toml")).unwrap(),
    )
}

#[test]
fn reads_the_complete_config() {
    let tmp = repo();
    fs::create_dir_all(tmp.path().join("telos")).unwrap();
    fs::write(
        tmp.path().join("telos/telos.toml"),
        "[code]\nglobs = [\"src/**/*.rs\"]\n\n[tests]\nglobs = [\"tests/**/*.rs\"]\n\n[test]\ncmd = \"cargo test {filter}\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = [\"claude\", \"codex\"]\n",
    ).unwrap();

    let output = telos(tmp.path(), &["config", "--json"])
        .output()
        .expect("run telos config");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("config JSON"),
        json!({
            "ok": true,
            "command": "config",
            "result": {
                "code": {"globs": ["src/**/*.rs"]},
                "tests": {"globs": ["tests/**/*.rs"]},
                "test": {"cmd": "cargo test {filter}", "report": ""},
                "policy": {"tdd": "strict"},
                "agents": {"hosts": ["claude", "codex"]}
            },
            "error": null,
            "next_actions": []
        })
    );
}

#[test]
fn human_config_is_canonical_toml_with_one_trailing_newline() {
    let tmp = repo();
    fs::create_dir_all(tmp.path().join("telos")).unwrap();
    fs::write(
        tmp.path().join("telos/telos.toml"),
        "[code]\nglobs = []\n\n[tests]\nglobs = []\n\n[test]\ncmd = \"\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = []\n",
    )
    .unwrap();

    let output = telos(tmp.path(), &["config"]).output().expect("run config");

    assert!(output.status.success());
    assert!(output.stdout.ends_with(b"\n"));
    assert!(!output.stdout.ends_with(b"\n\n"));
}

#[test]
fn stages_config_without_touching_the_base() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "claude,codex"])
        .assert()
        .success();
    let before = fs::read(tmp.path().join("telos/telos.toml")).unwrap();
    telos(tmp.path(), &["change", "open", "configuration update"])
        .assert()
        .success();

    let payload = r#"{"code":{"globs":["src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}"},"policy":{"tdd":"advisory"},"agents":{"hosts":["claude","codex"]}}"#;
    let output = telos(tmp.path(), &["config", "--change", "CHG-0001", "--json"])
        .write_stdin(payload)
        .output()
        .expect("stage config");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/telos.toml")).unwrap(),
        before
    );
    let change = fs::read_to_string(tmp.path().join("telos/changes/CHG-0001.tel")).unwrap();
    assert!(change.contains("op edit config"));
    assert!(change.contains("tdd         advisory"));
    telos(tmp.path(), &["change", "diff", "CHG-0001"])
        .assert()
        .success();
}

#[test]
fn staged_config_has_a_typed_diff_digest_and_reconciles_only_after_approval() {
    let tmp = configured_change();
    let base = fs::read(tmp.path().join("telos/telos.toml")).unwrap();

    let staged = telos(tmp.path(), &["config", "--change", "CHG-0001", "--json"])
        .write_stdin(PAYLOAD)
        .output()
        .unwrap();
    assert!(
        staged.status.success(),
        "{}",
        String::from_utf8_lossy(&staged.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&staged.stdout).unwrap(),
        json!({"ok":true,"command":"config","result":{"change":"CHG-0001","path":"telos/telos.toml","config":{"code":{"globs":["src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}","report":""},"policy":{"tdd":"advisory"},"agents":{"hosts":["claude","codex"]}}},"error":null,"next_actions":["telos change diff CHG-0001"]})
    );
    assert_eq!(fs::read(tmp.path().join("telos/telos.toml")).unwrap(), base);
    let changing = telos(tmp.path(), &["status", "--json"]).output().unwrap();
    let changing: serde_json::Value = serde_json::from_slice(&changing.stdout).unwrap();
    assert_eq!(changing["result"]["state"], "changing");
    assert_eq!(changing["result"]["drift"], serde_json::Value::Null);
    let change_path = tmp.path().join("telos/changes/CHG-0001.tel");
    let change = fs::read_to_string(&change_path).unwrap();
    assert!(change.contains("op edit config {\n    code_glob   \"src/**/*.rs\"\n    test_glob   \"tests/**/*.rs\"\n    test_cmd    \"cargo test {filter}\"\n    test_report \"\"\n    tdd         advisory\n    agent_host  claude\n    agent_host  codex\n  }"));

    let diff = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(diff.status.success());
    let diff: serde_json::Value = serde_json::from_slice(&diff.stdout).unwrap();
    assert_eq!(diff["result"]["status"], "drafted");
    assert_eq!(
        diff["result"]["ops"][0]["before"],
        "[code]\nglobs = []\n\n[tests]\nglobs = []\n\n[test]\ncmd = \"\"\nreport = \"\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = [\"claude\", \"codex\"]\n"
    );
    assert_eq!(
        diff["result"]["ops"][0]["after"],
        "[code]\nglobs = [\"src/**/*.rs\"]\n\n[tests]\nglobs = [\"tests/**/*.rs\"]\n\n[test]\ncmd = \"cargo test {filter}\"\nreport = \"\"\n\n[policy]\ntdd = \"advisory\"\n\n[agents]\nhosts = [\"claude\", \"codex\"]\n"
    );
    let first_digest = diff["result"]["digest"].as_str().unwrap().to_string();
    let changed = PAYLOAD.replace("advisory", "strict");
    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(changed)
        .assert()
        .success();
    let second = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_ne!(second["result"]["digest"], first_digest);

    telos(tmp.path(), &["change", "approve", "CHG-0001"])
        .assert()
        .success();
    telos(tmp.path(), &["change", "reconcile", "CHG-0001"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(tmp.path().join("telos/telos.toml")).unwrap(),
        "[code]\nglobs = [\"src/**/*.rs\"]\n\n[tests]\nglobs = [\"tests/**/*.rs\"]\n\n[test]\ncmd = \"cargo test {filter}\"\nreport = \"\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = [\"claude\", \"codex\"]\n"
    );
    let status = telos(tmp.path(), &["status", "--json"]).output().unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["result"]["state"], "coherent");
}

#[test]
fn rejected_config_payloads_leave_transaction_bytes_unchanged() {
    for payload in [
        "{",  // malformed
        "{}", // partial
        r#"{"code":{"globs":[]},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"strict"},"agents":{"hosts":["claude","codex"]},"extra":true}"#,
        r#"{"code":{"globs":[],"extra":true},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"strict"},"agents":{"hosts":["claude","codex"]}}"#,
        r#"{"code":{"globs":[]},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"invalid"},"agents":{"hosts":["claude","codex"]}}"#,
        r#"{"code":{"globs":["["]},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"strict"},"agents":{"hosts":["claude","codex"]}}"#,
        r#"{"code":{"globs":[]},"tests":{"globs":[]},"test":{"cmd":""},"policy":{"tdd":"strict"},"agents":{"hosts":["claude"]}}"#,
    ] {
        let tmp = configured_change();
        let before = bytes(tmp.path());
        telos(tmp.path(), &["config", "--change", "CHG-0001"])
            .write_stdin(payload)
            .assert()
            .failure();
        assert_eq!(bytes(tmp.path()), before, "payload {payload}");
    }
}

#[test]
fn rejected_config_changes_leave_transaction_bytes_unchanged() {
    let tmp = configured_change();
    let before = bytes(tmp.path());
    telos(tmp.path(), &["config", "--change", "CHG-9999"])
        .write_stdin(PAYLOAD)
        .assert()
        .failure();
    assert_eq!(bytes(tmp.path()), before);

    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .success();
    telos(tmp.path(), &["change", "approve", "CHG-0001"])
        .assert()
        .success();
    let before_approved = bytes(tmp.path());
    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .failure();
    assert_eq!(bytes(tmp.path()), before_approved);

    let foreign = configured_change();
    telos(foreign.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .success();
    telos(foreign.path(), &["change", "open", "second"])
        .assert()
        .success();
    let before_foreign = bytes(foreign.path());
    telos(foreign.path(), &["config", "--change", "CHG-0002"])
        .write_stdin(PAYLOAD)
        .assert()
        .failure();
    assert_eq!(bytes(foreign.path()), before_foreign);

    let drifted = configured_change();
    fs::write(drifted.path().join("telos/telos.toml"), "drift\n").unwrap();
    let before_drift = bytes(drifted.path());
    telos(drifted.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .failure();
    assert_eq!(bytes(drifted.path()), before_drift);
}

#[test]
fn approve_rejects_a_hand_edited_invalid_config_without_freezing_a_digest() {
    let tmp = configured_change();
    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .success();
    let change_path = tmp.path().join("telos/changes/CHG-0001.tel");
    let source = fs::read_to_string(&change_path).unwrap();
    fs::write(
        &change_path,
        source.replace("code_glob   \"src/**/*.rs\"", "code_glob   \"[\""),
    )
    .unwrap();
    let config_before = fs::read(tmp.path().join("telos/telos.toml")).unwrap();
    let counters_before = fs::read(tmp.path().join("telos/changes/counters.toml")).unwrap();

    let output = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(!output.status.success(), "got {envelope}");
    assert_eq!(envelope["error"]["code"], json!("TELOS_PARSE_ERROR"));
    let after = fs::read_to_string(&change_path).unwrap();
    assert!(after.contains("status drafted"), "{after}");
    assert!(!after.contains("approved_digest"), "{after}");
    assert_eq!(
        fs::read(tmp.path().join("telos/telos.toml")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/changes/counters.toml")).unwrap(),
        counters_before
    );
}

#[test]
fn reconcile_rejects_a_freshly_approved_host_change_without_writing_anything() {
    let tmp = configured_change();
    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .success();

    // Simulate a hand-edited change whose attacker also refreshed the digest.
    // Reconcile must enforce the transition independently of staging/approve.
    let ws = Workspace::discover(tmp.path()).unwrap();
    let id = ChangeId(1);
    let mut change = read_change(&ws, id).unwrap();
    let StagedOp::EditConfig(config) = change
        .ops
        .iter_mut()
        .find(|op| matches!(op, StagedOp::EditConfig(_)))
        .unwrap()
    else {
        unreachable!()
    };
    config.agents.hosts = vec![AgentHost::Claude];
    change.status = ChangeStatus::Approved;
    change.approved_digest = Some(change.ops_digest());
    write_change(&ws, &change).unwrap();

    let config_before = fs::read(tmp.path().join("telos/telos.toml")).unwrap();
    let change_before = fs::read(tmp.path().join("telos/changes/CHG-0001.tel")).unwrap();
    let lock_before = fs::read(tmp.path().join("telos/telos.lock")).unwrap();
    let output = telos(tmp.path(), &["change", "reconcile", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(!output.status.success(), "got {envelope}");
    assert_eq!(
        envelope["error"],
        json!({
            "code": "TELOS_INTEGRITY_VIOLATION",
            "message": "agents.hosts is managed by `telos init --agents` and cannot be changed by `telos config`",
            "hint": null
        })
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/telos.toml")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/changes/CHG-0001.tel")).unwrap(),
        change_before
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/telos.lock")).unwrap(),
        lock_before
    );
}

#[test]
fn reapprove_rejects_a_hand_edited_host_change_and_preserves_the_frozen_digest() {
    let tmp = configured_change();
    telos(tmp.path(), &["config", "--change", "CHG-0001"])
        .write_stdin(PAYLOAD)
        .assert()
        .success();
    telos(tmp.path(), &["change", "approve", "CHG-0001"])
        .assert()
        .success();

    let ws = Workspace::discover(tmp.path()).unwrap();
    let id = ChangeId(1);
    let mut change = read_change(&ws, id).unwrap();
    let original_digest = change.approved_digest.clone().unwrap();
    let StagedOp::EditConfig(config) = change
        .ops
        .iter_mut()
        .find(|op| matches!(op, StagedOp::EditConfig(_)))
        .unwrap()
    else {
        unreachable!()
    };
    config.agents.hosts = vec![AgentHost::Claude];
    write_change(&ws, &change).unwrap();
    let before = fs::read(tmp.path().join("telos/changes/CHG-0001.tel")).unwrap();

    let output = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(!output.status.success(), "got {envelope}");
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_INTEGRITY_VIOLATION")
    );
    assert_eq!(
        fs::read(tmp.path().join("telos/changes/CHG-0001.tel")).unwrap(),
        before
    );
    assert_eq!(
        read_change(&ws, id).unwrap().approved_digest.as_deref(),
        Some(original_digest.as_str())
    );
}
