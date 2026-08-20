//! Contract tests for project configuration.

mod common;

use std::fs;

use serde_json::json;

use common::{repo, telos};

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
                "test": {"cmd": "cargo test {filter}"},
                "policy": {"tdd": "strict"},
                "agents": {"hosts": ["claude", "codex"]}
            },
            "error": null,
            "next_actions": []
        })
    );
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
    assert!(change.contains("tdd        advisory"));
    telos(tmp.path(), &["change", "diff", "CHG-0001"])
        .assert()
        .success();
}
