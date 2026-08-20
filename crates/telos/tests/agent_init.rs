//! Host integration for `telos init --agents` and the generated guard.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use common::{repo, telos};

const SKILLS: [&str; 3] = ["telos", "telos-challenger", "telos-implementer"];

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn hook(root: &Path, host: &str, input: Value) -> Value {
    let mut cmd = telos(root, &["agent-guard", "--host", host]);
    let out = cmd.write_stdin(input.to_string()).output().unwrap();
    assert!(
        out.status.success(),
        "guard failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("guard output is JSON")
}

fn skill_body(document: &str) -> (&str, &str) {
    let rest = document
        .strip_prefix("---\n")
        .expect("skill starts with YAML frontmatter");
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .expect("skill closes YAML frontmatter");
    assert!(frontmatter.lines().any(|line| line.starts_with("name: ")));
    assert!(
        frontmatter
            .lines()
            .any(|line| line.starts_with("description: "))
    );
    (frontmatter, body)
}

#[test]
fn init_without_agents_creates_no_host_artifacts() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    for path in [".claude", ".agents", ".codex", "AGENTS.md"] {
        assert!(!tmp.path().join(path).exists(), "unexpected {path}");
    }
}

#[test]
fn init_creates_exactly_the_requested_hosts() {
    for (arg, claude, codex) in [
        ("claude", true, false),
        ("codex", false, true),
        ("claude,codex", true, true),
    ] {
        let tmp = repo();
        telos(tmp.path(), &["init", "--agents", arg])
            .assert()
            .success();

        assert_eq!(tmp.path().join(".claude/settings.json").exists(), claude);
        assert_eq!(tmp.path().join(".codex/hooks.json").exists(), codex);
        assert_eq!(tmp.path().join("AGENTS.md").exists(), codex);
        for skill in SKILLS {
            assert_eq!(
                tmp.path()
                    .join(format!(".claude/skills/{skill}/SKILL.md"))
                    .exists(),
                claude
            );
            assert_eq!(
                tmp.path()
                    .join(format!(".agents/skills/{skill}/SKILL.md"))
                    .exists(),
                codex
            );
        }
    }
}

#[test]
fn duplicate_hosts_normalize_deterministically() {
    let tmp = repo();
    telos(
        tmp.path(),
        &["init", "--agents", "codex,claude,codex,claude"],
    )
    .assert()
    .success();

    let claude = read(tmp.path(), ".claude/settings.json");
    let codex = read(tmp.path(), ".codex/hooks.json");
    assert_eq!(claude.matches("telos agent-guard --host claude").count(), 1);
    assert_eq!(codex.matches("telos agent-guard --host codex").count(), 1);
}

#[test]
fn unknown_host_is_a_clap_error_before_any_project_write() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "claude,wat"])
        .assert()
        .failure()
        .code(2);

    assert!(!tmp.path().join("telos").exists());
    assert!(!tmp.path().join(".claude").exists());
    assert!(!tmp.path().join(".gitattributes").exists());
}

#[test]
fn skills_have_valid_frontmatter_and_identical_host_bytes() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "claude,codex"])
        .assert()
        .success();

    for skill in SKILLS {
        let claude = read(tmp.path(), &format!(".claude/skills/{skill}/SKILL.md"));
        let codex = read(tmp.path(), &format!(".agents/skills/{skill}/SKILL.md"));
        let (frontmatter, body) = skill_body(&claude);
        assert!(frontmatter.contains(&format!("name: {skill}")));
        assert!(!body.trim().is_empty());
        assert_eq!(claude.as_bytes(), codex.as_bytes());
    }
}

#[test]
fn skill_pressure_rules_pin_order_and_stop_conditions() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();

    let router = read(tmp.path(), ".agents/skills/telos/SKILL.md");
    ordered(
        &router,
        &[
            "telos status --json",
            "result.state",
            "telos adopt",
            "telos revert",
        ],
    );
    assert!(router.contains("Never edit any path under `telos/` manually"));
    assert!(router.contains("Stop and ask the human"));
    assert!(router.contains("Routing is a mandatory handoff"));
    assert!(router.contains("load and invoke the routed skill before any action in that phase"));
    assert!(router.contains("Never execute Challenge or Implement steps yourself"));

    let challenger = read(tmp.path(), ".agents/skills/telos-challenger/SKILL.md");
    ordered(
        &challenger,
        &[
            "telos change open",
            "telos impact",
            "telos context",
            "telos change diff",
            "telos change approve",
        ],
    );
    assert!(challenger.contains("Never edit application code"));
    assert!(challenger.contains("Never approve a change yourself"));
    assert!(challenger.contains("immediately invoke `telos change approve <CHG-id>`"));
    assert!(challenger.contains("Do not answer the native prompt"));
    assert!(challenger.contains("ends only after triggering the native approval prompt"));
    assert!(challenger.contains("opens the prompt; it does not grant approval"));
    assert!(challenger.contains("Do not continue until the human answers"));

    let implementer = read(tmp.path(), ".agents/skills/telos-implementer/SKILL.md");
    ordered(
        &implementer,
        &[
            "telos context",
            "telos test SCN-",
            "same test bytes",
            "telos bind",
            "telos change reconcile",
        ],
    );
    assert!(implementer.contains("Never alter the approved delta"));
    assert!(implementer.contains("Do not edit the test after the sealed red"));
}

fn ordered(haystack: &str, needles: &[&str]) {
    let mut offset = 0;
    for needle in needles {
        let found = haystack[offset..]
            .find(needle)
            .unwrap_or_else(|| panic!("missing `{needle}` after byte {offset}"));
        offset += found + needle.len();
    }
}

#[test]
fn guard_denies_direct_file_writes_under_telos() {
    let tmp = repo();
    for (tool_name, tool_input) in [
        ("Edit", json!({"file_path": "telos/intents/INT-0001.tel"})),
        ("Write", json!({"file_path": "./telos/bindings.tel"})),
        (
            "apply_patch",
            json!({"command": "*** Update File: telos/telos.toml"}),
        ),
    ] {
        let out = hook(
            tmp.path(),
            "claude",
            json!({
                "cwd": tmp.path(),
                "hook_event_name": "PreToolUse",
                "tool_name": tool_name,
                "tool_input": tool_input,
            }),
        );
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    }
}

#[test]
fn guard_resolves_file_tool_paths_from_hook_cwd_not_repo_root() {
    let tmp = repo();
    let cwd = tmp.path().join("crates");
    fs::create_dir_all(&cwd).unwrap();

    for (tool_name, tool_input) in [
        ("Edit", json!({"file_path": "../telos/bindings.tel"})),
        (
            "apply_patch",
            json!({"command": "*** Update File: ../telos/telos.toml"}),
        ),
    ] {
        assert_eq!(
            tool_decision(tmp.path(), &cwd, "claude", tool_name, tool_input),
            "deny"
        );
    }

    assert_eq!(
        tool_decision(
            tmp.path(),
            &cwd,
            "claude",
            "Edit",
            json!({"file_path": "telos/not-the-spec"}),
        ),
        "allow"
    );
}

#[test]
fn guard_resolves_bash_paths_from_hook_cwd_not_repo_root() {
    let tmp = repo();
    let cwd = tmp.path().join("crates");
    fs::create_dir_all(&cwd).unwrap();

    assert_eq!(
        bash_decision_at(tmp.path(), &cwd, "claude", "touch ../telos/bindings.tel",),
        "deny"
    );
    assert_eq!(
        bash_decision_at(tmp.path(), &cwd, "claude", "touch telos/not-the-spec",),
        "allow"
    );
}

#[test]
fn guard_checks_newline_background_and_supported_shell_wrappers() {
    let tmp = repo();
    for command in [
        "echo ok\ntouch telos/bindings.tel",
        "echo ok & touch telos/bindings.tel",
        "bash -c \"touch telos/bindings.tel\"",
        "sh -c \"rm telos/bindings.tel\"",
        "command touch telos/bindings.tel",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }

    for command in [
        "cat telos/telos.toml",
        "bash -c \"cat telos/telos.toml\"",
        "command cat telos/telos.toml",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "allow",
            "{command}"
        );
    }
}

#[test]
fn guard_finds_human_actions_after_separators_and_wrappers() {
    let tmp = repo();
    for command in [
        "bash -c \"telos revert\"",
        "command telos adopt",
        "echo ok\ntelos change approve CHG-0001",
        "echo ok & telos revert",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "ask",
            "{command}"
        );
    }
}

#[test]
fn guard_fails_closed_on_ambiguous_shell_syntax() {
    let tmp = repo();
    for command in [
        "bash -c \"$TELOS_COMMAND\"",
        "touch $(printf telos/bindings.tel)",
        "touch `printf telos/bindings.tel`",
        "touch \"telos/bindings.tel",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_denies_shell_wrapper_options_before_c() {
    let tmp = repo();
    assert_eq!(
        bash_decision(
            tmp.path(),
            "claude",
            "bash --norc -c \"touch telos/bindings.tel\"",
        ),
        "deny"
    );
}

#[test]
fn guard_round_two_denies_clobber_redirect_operator() {
    let tmp = repo();
    for command in [
        "echo x >| telos/bindings.tel",
        "telos status --json >| telos/status.json",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_denies_unproven_shell_path_expansions() {
    let tmp = repo();
    for command in [
        "touch ~+/telos/bindings.tel",
        "rm -rf telo?",
        "rm -rf telo[s]",
        "rm -rf telo*",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_qualifies_find_read_only_flags() {
    let tmp = repo();
    assert_eq!(
        bash_decision(tmp.path(), "claude", "find telos -delete"),
        "deny"
    );
}

#[test]
fn guard_round_two_finds_git_subcommand_after_global_options() {
    let tmp = repo();
    assert_eq!(
        bash_decision(tmp.path(), "claude", "git -C telos clean -fd"),
        "deny"
    );
}

#[test]
fn guard_round_two_extracts_key_value_paths() {
    let tmp = repo();
    assert_eq!(
        bash_decision(
            tmp.path(),
            "claude",
            "dd if=/dev/null of=telos/bindings.tel",
        ),
        "deny"
    );
}

#[test]
fn guard_round_two_extracts_long_option_paths() {
    let tmp = repo();
    assert_eq!(
        bash_decision(
            tmp.path(),
            "claude",
            "cp Cargo.toml --target-directory=telos",
        ),
        "deny"
    );
}

#[test]
fn guard_round_two_codex_denies_human_actions_not_covered_by_native_rules() {
    let tmp = repo();
    for command in [
        "bash -c \"telos revert\"",
        "command telos adopt",
        "rtk telos change approve CHG-0001",
        "telos --json adopt",
        "telos adopt;",
    ] {
        let out = hook(
            tmp.path(),
            "codex",
            json!({
                "cwd": tmp.path(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": command},
            }),
        );
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(
            out["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("retry direct canonical command"),
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_codex_allows_only_direct_actions_matched_by_rendered_rules() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();
    let rules = read(tmp.path(), ".codex/rules/telos.rules");

    for command in [
        "telos change approve CHG-0001",
        "telos adopt",
        "telos revert",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "codex", command),
            "allow",
            "{command}"
        );
        assert_eq!(
            rendered_rule_decision_for_shell(&rules, command),
            Some("prompt"),
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_fails_closed_on_combined_directory_changes() {
    let tmp = repo();
    for command in [
        "cd crates && touch ../telos/bindings.tel",
        "bash -c \"cd crates; touch ../telos/bindings.tel\"",
        "pushd crates; rm ../telos/bindings.tel",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
    assert_eq!(bash_decision(tmp.path(), "claude", "cd crates"), "allow");
}

#[cfg(unix)]
#[test]
fn guard_round_two_resolves_existing_symlink_parents_for_new_paths() {
    use std::os::unix::fs::symlink;

    let tmp = repo();
    fs::create_dir_all(tmp.path().join("telos")).unwrap();
    symlink("telos", tmp.path().join("spec-link")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), tmp.path().join("outside-link")).unwrap();

    for (tool_name, tool_input) in [
        ("Edit", json!({"file_path": "spec-link/bindings.tel"})),
        (
            "apply_patch",
            json!({"command": "*** Add File: spec-link/new-intent.tel"}),
        ),
    ] {
        assert_eq!(
            tool_decision(tmp.path(), tmp.path(), "claude", tool_name, tool_input,),
            "deny"
        );
    }
    assert_eq!(
        bash_decision(tmp.path(), "claude", "touch spec-link/new-binding.tel"),
        "deny"
    );
    assert_eq!(
        tool_decision(
            tmp.path(),
            tmp.path(),
            "claude",
            "Edit",
            json!({"file_path": "outside-link/new-source.rs"}),
        ),
        "deny"
    );
}

fn tool_decision(
    root: &Path,
    cwd: &Path,
    host: &str,
    tool_name: &str,
    tool_input: Value,
) -> String {
    hook(
        root,
        host,
        json!({
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "tool_name": tool_name,
            "tool_input": tool_input,
        }),
    )["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn guard_denies_direct_shell_mutations_but_allows_inspection_and_source_edits() {
    let tmp = repo();
    for command in [
        "touch telos/intents/new.tel",
        "rm telos/bindings.tel",
        "mv draft.tel telos/intents/INT-0001.tel",
        "echo changed > telos/telos.toml",
        "sed -i s/old/new/ telos/telos.toml",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }

    for command in [
        "telos status --json",
        "telos show INT-0001 --json",
        "telos context INT-0001 --json",
        "telos change diff CHG-0001 --json",
        "cat telos/telos.toml",
        "echo telosophy",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "allow",
            "{command}"
        );
    }

    let source = hook(
        tmp.path(),
        "claude",
        json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/telos_adapter.rs"},
        }),
    );
    assert_eq!(source["hookSpecificOutput"]["permissionDecision"], "allow");
}

fn bash_decision(root: &Path, host: &str, command: &str) -> String {
    bash_decision_at(root, root, host, command)
}

fn bash_decision_at(root: &Path, cwd: &Path, host: &str, command: &str) -> String {
    hook(
        root,
        host,
        json!({
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
        }),
    )["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn claude_asks_for_human_decisions_and_surfaces_available_digest_context() {
    let tmp = repo();
    for command in ["telos adopt", "telos revert"] {
        assert_eq!(bash_decision(tmp.path(), "claude", command), "ask");
    }

    let out = hook(
        tmp.path(),
        "claude",
        json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {
                "command": "telos change approve CHG-0001",
                "description": "Approve reviewed digest 8a81d1f9"
            },
        }),
    );
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "ask");
    assert!(
        out["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("8a81d1f9")
    );
}

#[test]
fn codex_guard_never_returns_ask_and_rules_own_native_prompts() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();

    for command in [
        "telos change approve CHG-0001",
        "telos adopt",
        "telos revert",
    ] {
        assert_eq!(bash_decision(tmp.path(), "codex", command), "allow");
    }

    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    for pattern in [
        "pattern = [\"telos\", \"change\", \"approve\"]",
        "pattern = [\"telos\", \"adopt\"]",
        "pattern = [\"telos\", \"revert\"]",
    ] {
        assert!(rules.contains(pattern), "missing rule {pattern}");
    }
    assert_eq!(rules.matches("decision = \"prompt\"").count(), 3);
    assert!(
        !rules.contains("digest ="),
        "there is no public digest flag"
    );
    for argv in [
        &["telos", "change", "approve", "CHG-0001"][..],
        &["telos", "adopt", "--into", "CHG-0001"][..],
        &["telos", "revert"][..],
    ] {
        assert_eq!(rendered_rule_decision(&rules, argv), Some("prompt"));
    }
    assert_eq!(
        rendered_rule_decision(&rules, &["telos", "change", "diff", "CHG-0001"]),
        None
    );
    assert_eq!(rendered_rule_decision(&rules, &["mytelos", "adopt"]), None);
}

fn rendered_rule_decision<'a>(rules: &'a str, argv: &[&str]) -> Option<&'a str> {
    rules.split("prefix_rule(").skip(1).find_map(|block| {
        let block = block.split_once(')')?.0;
        let pattern_line = block
            .lines()
            .find(|line| line.trim().starts_with("pattern ="))?;
        let prefix: Vec<&str> = pattern_line
            .split('"')
            .enumerate()
            .filter_map(|(index, value)| (index % 2 == 1).then_some(value))
            .collect();
        if !argv.starts_with(&prefix) {
            return None;
        }
        let decision_line = block
            .lines()
            .find(|line| line.trim().starts_with("decision ="))?;
        decision_line.split('"').nth(1)
    })
}

fn rendered_rule_decision_for_shell<'a>(rules: &'a str, command: &str) -> Option<&'a str> {
    let argv: Vec<&str> = command.split_ascii_whitespace().collect();
    rendered_rule_decision(rules, &argv)
}

#[test]
fn claude_settings_merge_is_idempotent_and_preserves_user_configuration() {
    let tmp = repo();
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    fs::write(
        tmp.path().join(".claude/settings.json"),
        serde_json::to_vec_pretty(&json!({
            "env": {"KEEP": "yes"},
            "hooks": {"PreToolUse": [{
                "matcher": "Read",
                "hooks": [{"type": "command", "command": "user-check"}]
            }]}
        }))
        .unwrap(),
    )
    .unwrap();

    telos(tmp.path(), &["init", "--agents", "claude"])
        .assert()
        .success();
    let settings: Value = serde_json::from_str(&read(tmp.path(), ".claude/settings.json")).unwrap();
    assert_eq!(settings["env"]["KEEP"], "yes");
    let encoded = settings.to_string();
    assert_eq!(encoded.matches("user-check").count(), 1);
    assert_eq!(
        encoded.matches("telos agent-guard --host claude").count(),
        1
    );
}

#[test]
fn codex_configuration_merge_preserves_unrelated_content_and_owned_blocks_once() {
    let tmp = repo();
    fs::create_dir_all(tmp.path().join(".codex")).unwrap();
    fs::write(
        tmp.path().join(".codex/hooks.json"),
        serde_json::to_vec_pretty(&json!({
            "description": "keep me",
            "hooks": {"PostToolUse": [{
                "matcher": "Bash",
                "hooks": [{"type": "command", "command": "user-post"}]
            }]}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        tmp.path().join("AGENTS.md"),
        "# User instructions\n\nKeep this.\n",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join(".codex/rules")).unwrap();
    fs::write(
        tmp.path().join(".codex/rules/telos.rules"),
        "# user rule\nprefix_rule(pattern = [\"cargo\"], decision = \"allow\")\n",
    )
    .unwrap();

    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();

    let hooks: Value = serde_json::from_str(&read(tmp.path(), ".codex/hooks.json")).unwrap();
    assert_eq!(hooks["description"], "keep me");
    let encoded = hooks.to_string();
    assert_eq!(encoded.matches("user-post").count(), 1);
    assert_eq!(encoded.matches("telos agent-guard --host codex").count(), 1);

    let agents = read(tmp.path(), "AGENTS.md");
    assert!(agents.starts_with("# User instructions\n\nKeep this.\n"));
    assert_eq!(agents.matches("<!-- telos-sdd:start -->").count(), 1);
    assert_eq!(agents.matches("<!-- telos-sdd:end -->").count(), 1);
    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    assert!(rules.starts_with("# user rule\n"));
    assert_eq!(rules.matches("# telos-sdd:start").count(), 1);
}

#[test]
fn malformed_existing_host_json_aborts_before_partial_initialization() {
    for (host, path) in [
        ("claude", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
    ] {
        let tmp = repo();
        fs::create_dir_all(tmp.path().join(Path::new(path).parent().unwrap())).unwrap();
        fs::write(tmp.path().join(path), "{ not json").unwrap();

        let out = telos(tmp.path(), &["init", "--agents", host, "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(envelope["error"]["code"], "TELOS_PARSE_ERROR");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains(path)
        );
        assert!(!tmp.path().join("telos").exists());
        assert!(!tmp.path().join(".gitattributes").exists());
    }
}

#[test]
fn structurally_invalid_host_hooks_abort_before_partial_initialization() {
    for (host, path) in [
        ("claude", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
    ] {
        let tmp = repo();
        fs::create_dir_all(tmp.path().join(Path::new(path).parent().unwrap())).unwrap();
        fs::write(tmp.path().join(path), r#"{"hooks": []}"#).unwrap();

        let out = telos(tmp.path(), &["init", "--agents", host, "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(envelope["error"]["code"], "TELOS_PARSE_ERROR");
        assert!(!tmp.path().join("telos").exists());
        assert!(!tmp.path().join(".gitattributes").exists());
    }
}
