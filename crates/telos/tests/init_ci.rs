//! GitHub Actions generation for `telos init --ci github`.

mod common;

use std::fs;

use serde_json::{Value, json};

use common::{repo, telos};

const WORKFLOW: &str = concat!(
    "name: Telos\n\n",
    "on:\n",
    "  pull_request:\n",
    "  push:\n",
    "    branches: [main]\n\n",
    "permissions:\n",
    "  contents: read\n\n",
    "jobs:\n",
    "  sealed:\n",
    "    runs-on: ubuntu-latest\n",
    "    steps:\n",
    "      - uses: actions/checkout@v7\n",
    "      - uses: dtolnay/rust-toolchain@stable\n",
    "      - name: Install Telos v",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "        run: cargo install --git https://github.com/hugues31/telos-sdd --tag v",
    env!("CARGO_PKG_VERSION"),
    " --locked telos\n",
    "      - name: Verify sealed Telos state\n",
    "        run: telos check --sealed\n",
);

#[test]
fn init_without_ci_keeps_the_frozen_result_and_creates_no_workflow() {
    let tmp = repo();

    let out = telos(tmp.path(), &["init", "--json"]).output().unwrap();

    assert!(out.status.success(), "init failed: {out:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({
            "ok": true,
            "command": "init",
            "result": {"root": "telos", "sealed": true},
            "error": null,
            "next_actions": ["telos status"],
        })
    );
    assert!(!tmp.path().join(".github/workflows/telos.yml").exists());
}

#[test]
fn init_ci_github_writes_the_exact_sealed_state_workflow() {
    let tmp = repo();

    telos(tmp.path(), &["init", "--ci", "github"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(tmp.path().join(".github/workflows/telos.yml")).unwrap(),
        WORKFLOW
    );
}

#[test]
fn invalid_ci_provider_is_a_clap_error_before_any_write() {
    let tmp = repo();

    telos(tmp.path(), &["init", "--ci", "wat"])
        .assert()
        .failure()
        .code(2);

    assert_init_artifacts_absent(tmp.path(), true);
}

#[test]
fn init_combines_github_ci_with_both_agent_hosts() {
    let tmp = repo();

    telos(
        tmp.path(),
        &["init", "--agents", "claude,codex", "--ci", "github"],
    )
    .assert()
    .success();

    assert_eq!(
        fs::read_to_string(tmp.path().join(".github/workflows/telos.yml")).unwrap(),
        WORKFLOW
    );
    assert!(tmp.path().join(".claude/settings.json").is_file());
    assert!(tmp.path().join(".codex/hooks.json").is_file());
}

#[test]
fn existing_workflow_is_untouched_and_prevents_every_init_write() {
    let tmp = repo();
    let workflow = tmp.path().join(".github/workflows/telos.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(&workflow, "user workflow\n").unwrap();

    let out = telos(
        tmp.path(),
        &[
            "init",
            "--agents",
            "claude,codex",
            "--ci",
            "github",
            "--json",
        ],
    )
    .output()
    .unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({
            "ok": false,
            "command": "init",
            "result": null,
            "error": {
                "code": "TELOS_CHANGE_STATE_INVALID",
                "message": "`.github/workflows/telos.yml` already exists",
                "hint": "preserve or move the existing workflow before retrying",
            },
            "next_actions": [],
        })
    );
    assert_eq!(fs::read_to_string(&workflow).unwrap(), "user workflow\n");
    assert_init_artifacts_absent(tmp.path(), false);
}

#[cfg(unix)]
#[test]
fn existing_workflow_symlink_is_untouched_and_prevents_every_init_write() {
    use std::os::unix::fs::symlink;

    let tmp = repo();
    let target = tmp.path().join("preserve-me.yml");
    fs::write(&target, "user workflow\n").unwrap();
    let workflow = tmp.path().join(".github/workflows/telos.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    symlink(&target, &workflow).unwrap();

    telos(tmp.path(), &["init", "--ci", "github"])
        .assert()
        .failure()
        .code(1);

    assert!(
        fs::symlink_metadata(&workflow)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "user workflow\n");
    assert_init_artifacts_absent(tmp.path(), false);
}

#[test]
fn malformed_requested_agent_config_and_ci_leave_everything_unwritten() {
    let tmp = repo();
    let settings = tmp.path().join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, "not json\n").unwrap();

    telos(
        tmp.path(),
        &["init", "--agents", "claude", "--ci", "github"],
    )
    .assert()
    .failure()
    .code(1);

    assert_eq!(fs::read_to_string(&settings).unwrap(), "not json\n");
    assert_init_artifacts_absent(tmp.path(), true);
}

fn assert_init_artifacts_absent(root: &std::path::Path, include_workflow: bool) {
    let mut paths = vec![
        "telos",
        ".gitattributes",
        ".claude/skills",
        ".codex/hooks.json",
    ];
    if include_workflow {
        paths.push(".github/workflows/telos.yml");
    }
    for path in paths {
        assert!(
            !root.join(path).exists(),
            "unexpected init artifact `{path}`"
        );
    }
}
