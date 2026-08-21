//! GitHub Actions generation for `telos init --ci github`.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    "      - name: Install Telos v",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "        run: |\n",
    "          version=",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "          asset=\"telos_${version}_linux_amd64.tar.gz\"\n",
    "          base=\"https://github.com/hugues31/telos-sdd/releases/download/v${version}\"\n",
    "          cd \"$RUNNER_TEMP\"\n",
    "          curl -fsSLO \"${base}/${asset}\"\n",
    "          curl -fsSLO \"${base}/checksums.txt\"\n",
    "          sha256sum --check --ignore-missing checksums.txt\n",
    "          tar -xzf \"${asset}\"\n",
    "          install -D -m 0755 telos \"$HOME/.local/bin/telos\"\n",
    "          echo \"$HOME/.local/bin\" >> \"$GITHUB_PATH\"\n",
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

#[test]
fn workflow_occupants_return_the_frozen_collision_and_leave_the_full_tree_unchanged() {
    for occupant in ["file", "directory"] {
        let tmp = repo();
        let workflow = tmp.path().join(".github/workflows/telos.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        match occupant {
            "file" => fs::write(&workflow, "user workflow\n").unwrap(),
            "directory" => fs::create_dir(&workflow).unwrap(),
            _ => unreachable!(),
        }
        let before = non_git_tree(tmp.path());

        assert_workflow_collision(tmp.path());

        assert_eq!(non_git_tree(tmp.path()), before, "{occupant}");
    }
}

#[cfg(unix)]
#[test]
fn workflow_symlink_occupants_return_the_frozen_collision_and_leave_the_full_tree_unchanged() {
    use std::os::unix::fs::symlink;

    for dangling in [false, true] {
        let tmp = repo();
        let workflow = tmp.path().join(".github/workflows/telos.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        let target = tmp.path().join("workflow-owner.yml");
        if !dangling {
            fs::write(&target, "user workflow\n").unwrap();
        }
        symlink(&target, &workflow).unwrap();
        let before = non_git_tree(tmp.path());

        assert_workflow_collision(tmp.path());

        assert_eq!(non_git_tree(tmp.path()), before, "dangling={dangling}");
    }
}

#[test]
fn ci_parent_file_collisions_leave_the_full_tree_unchanged() {
    for path in [".github", ".github/workflows"] {
        let tmp = repo();
        let target = tmp.path().join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&target, "not a directory\n").unwrap();
        let before = non_git_tree(tmp.path());

        assert_ci_parent_collision(tmp.path(), path);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn ci_parent_symlinks_never_escape_the_repository() {
    use std::os::unix::fs::symlink;

    for path in [".github", ".github/workflows"] {
        let tmp = repo();
        let outside = tempfile::tempdir().unwrap();
        let target = tmp.path().join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        symlink(outside.path(), &target).unwrap();
        let before = non_git_tree(tmp.path());
        let outside_before = non_git_tree(outside.path());

        assert_ci_parent_collision(tmp.path(), path);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
        assert_eq!(non_git_tree(outside.path()), outside_before, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn dangling_ci_parent_symlinks_leave_the_full_tree_unchanged() {
    use std::os::unix::fs::symlink;

    for path in [".github", ".github/workflows"] {
        let tmp = repo();
        let target = tmp.path().join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        symlink(tmp.path().join("missing-parent"), &target).unwrap();
        let before = non_git_tree(tmp.path());

        assert_ci_parent_collision(tmp.path(), path);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
    }
}

#[test]
fn invalid_agent_text_and_ci_leave_the_full_tree_unchanged() {
    for path in ["AGENTS.md", ".codex/rules/telos.rules"] {
        let tmp = repo();
        let target = tmp.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, [0xff]).unwrap();
        let before = non_git_tree(tmp.path());

        telos(
            tmp.path(),
            &["init", "--agents", "claude,codex", "--ci", "github"],
        )
        .assert()
        .failure()
        .code(1);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn agent_target_and_parent_symlinks_with_ci_leave_the_full_tree_unchanged() {
    use std::os::unix::fs::symlink;

    for path in ["AGENTS.md", ".codex"] {
        let tmp = repo();
        let outside = tempfile::tempdir().unwrap();
        let target = tmp.path().join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        symlink(outside.path(), &target).unwrap();
        let before = non_git_tree(tmp.path());
        let outside_before = non_git_tree(outside.path());

        telos(
            tmp.path(),
            &["init", "--agents", "claude,codex", "--ci", "github"],
        )
        .assert()
        .failure()
        .code(1);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
        assert_eq!(non_git_tree(outside.path()), outside_before, "{path}");
    }
}

#[test]
fn agent_file_and_directory_collisions_with_ci_leave_the_full_tree_unchanged() {
    for path in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".codex/rules/telos.rules",
        ".agents/skills/telos/SKILL.md",
    ] {
        let tmp = repo();
        let target = tmp.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir(&target).unwrap();
        let before = non_git_tree(tmp.path());

        telos(
            tmp.path(),
            &["init", "--agents", "claude,codex", "--ci", "github"],
        )
        .assert()
        .failure()
        .code(1);

        assert_eq!(non_git_tree(tmp.path()), before, "{path}");
    }
}

#[test]
fn agent_preflight_failure_leaves_a_clean_retry_path() {
    let tmp = repo();
    let agents = tmp.path().join("AGENTS.md");
    fs::write(&agents, [0xff]).unwrap();

    telos(tmp.path(), &["init", "--agents", "codex", "--ci", "github"])
        .assert()
        .failure()
        .code(1);
    assert_eq!(non_git_tree(tmp.path()).len(), 1);

    fs::remove_file(&agents).unwrap();
    telos(tmp.path(), &["init", "--agents", "codex", "--ci", "github"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(tmp.path().join(".github/workflows/telos.yml")).unwrap(),
        WORKFLOW
    );
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

fn assert_workflow_collision(root: &Path) {
    let out = telos(
        root,
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
}

fn assert_ci_parent_collision(root: &Path, parent: &str) {
    let out = telos(root, &["init", "--ci", "github", "--json"])
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
                "message": format!("`{parent}` must be a real directory"),
                "hint": "replace the existing path with a real directory before retrying",
            },
            "next_actions": [],
        })
    );
}

#[derive(Debug, Eq, PartialEq)]
enum TreeEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn non_git_tree(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    let mut entries = BTreeMap::new();
    snapshot_dir(root, root, &mut entries);
    entries
}

fn snapshot_dir(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, TreeEntry>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if relative == Path::new(".git") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.file_type().is_symlink() {
            entries.insert(relative, TreeEntry::Symlink(fs::read_link(path).unwrap()));
        } else if metadata.is_dir() {
            entries.insert(relative.clone(), TreeEntry::Directory);
            snapshot_dir(root, &path, entries);
        } else {
            entries.insert(relative, TreeEntry::File(fs::read(path).unwrap()));
        }
    }
}
