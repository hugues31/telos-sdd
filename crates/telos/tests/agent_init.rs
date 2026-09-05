//! Host integration for `telos init --agents` and the generated guard.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use common::{repo, telos, with_fixture};

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

fn stage_drafted_config_change(root: &Path, hosts: &[&str]) {
    telos(root, &["change", "open", "configuration update"])
        .assert()
        .success();
    telos(root, &["config", "--change", "CHG-0001", "--json"])
        .write_stdin(
            json!({
                "code": {"globs": ["src/**/*.rs"]},
                "tests": {"globs": ["tests/**/*.rs"]},
                "test": {"cmd": "cargo test {filter}"},
                "policy": {"tdd": "advisory"},
                "agents": {"hosts": hosts},
            })
            .to_string(),
        )
        .assert()
        .success();
}

fn skill_body(document: &str) -> (&str, &str) {
    let rest = document
        .strip_prefix("---\n")
        .or_else(|| document.strip_prefix("---\r\n"))
        .expect("skill starts with YAML frontmatter");
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .or_else(|| rest.split_once("\r\n---\r\n"))
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
fn skill_frontmatter_parser_accepts_crlf_checkouts() {
    let document = "---\r\nname: telos\r\ndescription: Route Telos requests.\r\n---\r\nBody\r\n";

    let (frontmatter, body) = skill_body(document);

    assert!(frontmatter.contains("name: telos"));
    assert_eq!(body, "Body\r\n");
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
fn init_persists_normalized_agent_hosts_in_project_configuration() {
    let tmp = repo();
    telos(
        tmp.path(),
        &["init", "--agents", "codex,claude,codex,claude"],
    )
    .assert()
    .success();

    assert_eq!(
        read(tmp.path(), "telos/telos.toml"),
        "[code]\nglobs = []\n\n[tests]\nglobs = []\n\n[test]\ncmd = \"\"\nreport = \"\"\n\n[policy]\ntdd = \"strict\"\n\n[agents]\nhosts = [\"claude\", \"codex\"]\n"
    );
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
            "telos pack",
            "Domain-language review",
            "While a material ambiguity exists, stage nothing",
            "Ask exactly one question",
            "stop immediately",
            "Domain review",
            "Perform the final request classification",
            "telos add",
            "telos change diff",
            "Show `result.digest`",
            "telos change approve",
            "Do not answer the native prompt",
        ],
    );
    assert!(challenger.contains("Language delta"));
    assert!(challenger.contains("newly introduced domain terms"));
    assert!(challenger.contains("owning context and capability"));
    assert!(challenger.contains("actor, entity, value, event, or state"));
    assert!(challenger.contains("observable business outcome"));
    assert!(challenger.contains("business trigger"));
    assert!(challenger.contains("affected invariants"));
    assert!(challenger.contains("one nominal case"));
    assert!(challenger.contains("synonyms"));
    assert!(challenger.contains("overloaded terms"));
    assert!(challenger.contains("technical terms presented as domain concepts"));
    assert!(challenger.contains("command, event, state, and entity"));
    assert!(challenger.contains("at least one relevant edge, negative, or failure case"));
    assert!(challenger.contains("correct context and capability"));
    assert!(challenger.contains("the question that reduces uncertainty the most"));
    assert!(challenger.contains("Remaining material questions: none"));
    assert!(challenger.contains("repeat the Domain-language review"));
    assert!(challenger.contains("turn an assumption into a decision"));
    assert!(challenger.contains("behavioral contract, not an engine-enforced guarantee"));
    assert!(challenger.contains("Never edit application code"));
    assert!(challenger.contains("Never approve a change yourself"));
    assert!(challenger.contains(
        "immediately invoke `telos change approve <CHG-id> --expected-digest <result.digest>`"
    ));
    assert!(challenger.contains("fails closed if it is missing or stale"));
    assert!(challenger.contains("ends only after triggering the native approval prompt"));
    assert!(challenger.contains("opens the prompt; it does not grant approval"));
    assert!(challenger.contains("Do not continue until the human answers"));
    assert!(challenger.contains("Expression fields are a grammar, not prose"));
    assert!(challenger.contains("`Notion.attr == literal`"));
    assert!(challenger.contains("Identifiers are ASCII"));
    assert!(challenger.contains("payload.scenarios[0].then[1]"));

    let implementer = read(tmp.path(), ".agents/skills/telos-implementer/SKILL.md");
    ordered(
        &implementer,
        &[
            "telos pack",
            "telos test SCN-",
            "same test bytes",
            "telos bind",
            "telos change reconcile",
        ],
    );
    assert!(implementer.contains("Never alter the approved delta"));
    assert!(implementer.contains("Do not edit the test after the sealed red"));
    assert!(implementer.contains(
        "A compile error, a missing dependency, or a runner that executed zero tests is not a red"
    ));
    assert!(
        implementer.contains(
            "`TELOS_TEST_NOT_EXECUTED`: stop; make the runner execute the scenario's test"
        )
    );
    assert!(router.contains("`TELOS_TEST_NOT_EXECUTED`: route to the implementer"));
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
        (
            "Edit",
            json!({"file_path": "telos/contexts/billing/capabilities/invoicing/intents/INT-0001.tel"}),
        ),
        (
            "Write",
            json!({"file_path": "./telos/contexts/billing/bindings.tel"}),
        ),
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
        (
            "Edit",
            json!({"file_path": "../telos/contexts/billing/bindings.tel"}),
        ),
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
        bash_decision_at(
            tmp.path(),
            &cwd,
            "claude",
            "touch ../telos/contexts/billing/bindings.tel",
        ),
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
        "echo ok\ntouch telos/contexts/billing/bindings.tel",
        "echo ok & touch telos/contexts/billing/bindings.tel",
        "bash -c \"touch telos/contexts/billing/bindings.tel\"",
        "sh -c \"rm telos/contexts/billing/bindings.tel\"",
        "command touch telos/contexts/billing/bindings.tel",
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
fn guard_denies_human_actions_after_separators_and_wrappers() {
    let tmp = repo();
    for command in [
        "bash -c \"telos revert\"",
        "command telos adopt",
        "echo ok\ntelos change approve CHG-0001",
        "echo ok & telos revert",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
}

#[test]
fn guard_fails_closed_on_ambiguous_shell_syntax() {
    let tmp = repo();
    for command in [
        "bash -c \"$TELOS_COMMAND\"",
        "touch $(printf telos/contexts/billing/bindings.tel)",
        "touch `printf telos/contexts/billing/bindings.tel`",
        "touch \"telos/contexts/billing/bindings.tel",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }
}

#[test]
fn guard_denies_opaque_inline_interpreter_evaluation() {
    let tmp = repo();
    for command in [
        r#"python3 -c "open('telos/contexts/billing/bindings.tel','w').write('x')""#,
        r#"python3 -W ignore -c "open('telos/contexts/billing/bindings.tel','w').write('x')""#,
        r#"python -c "print('no visible path')""#,
        r#"ruby -e "File.write('telos/contexts/billing/bindings.tel', 'x')""#,
        r#"ruby -I lib -e "File.write('telos/contexts/billing/bindings.tel', 'x')""#,
        r#"perl -e "open(F, '>', 'telos/contexts/billing/bindings.tel')""#,
        r#"perl -I lib -e "open(F, '>', 'telos/contexts/billing/bindings.tel')""#,
        r#"node -e "require('fs').writeFileSync('telos/contexts/billing/bindings.tel','x')""#,
        r#"node --require preload.js -e "require('fs').writeFileSync('telos/contexts/billing/bindings.tel','x')""#,
        r#"php -r "file_put_contents('telos/contexts/billing/bindings.tel', 'x');""#,
        r#"php -d display_errors=1 -r "file_put_contents('telos/contexts/billing/bindings.tel', 'x');""#,
        r#"lua -e "io.open('telos/contexts/billing/bindings.tel', 'w')""#,
        r#"lua -l helper -e "io.open('telos/contexts/billing/bindings.tel', 'w')""#,
        r#"awk 'BEGIN { print "x" > "telos/contexts/billing/bindings.tel" }'"#,
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "deny",
            "{command}"
        );
    }

    let out = hook(
        tmp.path(),
        "claude",
        json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "python -c \"print('opaque')\""},
        }),
    );
    assert!(
        out["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("run a reviewed script file")
    );
}

#[test]
fn guard_allows_safe_interpreter_script_files() {
    let tmp = repo();
    for command in [
        "python3 scripts/check.py",
        "python3 scripts/check.py -c src/config.toml",
        "python3 -m scripts.check -c src/config.toml",
        "python3 -- scripts/check.py -c src/config.toml",
        "ruby scripts/check.rb",
        "ruby scripts/check.rb -e src/config.toml",
        "ruby -S check.rb -e src/config.toml",
        "perl scripts/check.pl",
        "perl scripts/check.pl -e src/config.toml",
        "node scripts/check.js",
        "node scripts/check.js -e src/config.toml",
        "php scripts/check.php",
        "php scripts/check.php -r src/config.toml",
        "php -f scripts/check.php -r src/config.toml",
        "lua scripts/check.lua",
        "lua scripts/check.lua -e src/config.toml",
        "awk -f scripts/check.awk src/input.txt",
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "claude", command),
            "allow",
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
            "bash --norc -c \"touch telos/contexts/billing/bindings.tel\"",
        ),
        "deny"
    );
}

#[test]
fn guard_round_two_denies_clobber_redirect_operator() {
    let tmp = repo();
    for command in [
        "echo x >| telos/contexts/billing/bindings.tel",
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
        "touch ~+/telos/contexts/billing/bindings.tel",
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
            "dd if=/dev/null of=telos/contexts/billing/bindings.tel",
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
                .contains(
                    if command.starts_with("rtk ") || command == "telos --json adopt" {
                        "current decision context"
                    } else {
                        "native prompt rules"
                    }
                ),
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
    stage_drafted_config_change(tmp.path(), &["codex"]);
    fs::write(
        tmp.path().join("telos/constraints/CON-0900.tel"),
        "constraint CON-0900 in project quality \"Prompt-time drift\" {\n  rule  \"Prompt-time drift.\"\n}\n",
    )
    .unwrap();
    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    let digest = current_change_digest(tmp.path());
    let token = current_drift_token(tmp.path());

    for command in [
        format!("telos change approve CHG-0001 --expected-digest {digest}"),
        format!("telos adopt --expected-state {token}"),
        format!("telos revert --expected-state {token}"),
    ] {
        let out = hook(
            tmp.path(),
            "codex",
            json!({
                "cwd": tmp.path(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": &command},
            }),
        );
        assert!(
            out["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        assert_eq!(
            rendered_rule_decision_for_shell(&rules, &command),
            Some("prompt"),
            "{command}"
        );
    }
}

#[test]
fn guard_round_two_fails_closed_on_combined_directory_changes() {
    let tmp = repo();
    for command in [
        "cd crates && touch ../telos/contexts/billing/bindings.tel",
        "bash -c \"cd crates; touch ../telos/contexts/billing/bindings.tel\"",
        "pushd crates; rm ../telos/contexts/billing/bindings.tel",
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
        "touch telos/contexts/billing/capabilities/invoicing/intents/new.tel",
        "rm telos/contexts/billing/bindings.tel",
        "mv draft.tel telos/contexts/billing/capabilities/invoicing/intents/INT-0001.tel",
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
        "telos pack INT-0001 --json",
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

fn current_change_digest(root: &Path) -> String {
    let output = telos(root, &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "diff failed: {} / {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"]["digest"]
        .as_str()
        .unwrap()
        .to_string()
}

fn current_drift_token(root: &Path) -> String {
    let output = telos(root, &["status", "--json"]).output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"]["drift"]["token"]
        .as_str()
        .expect("fixture must be drifted")
        .to_string()
}

#[test]
fn guard_surfaces_repository_derived_decision_context() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    stage_drafted_config_change(tmp.path(), &[]);

    let diff = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .expect("run change diff");
    assert!(diff.status.success());
    let digest =
        serde_json::from_slice::<Value>(&diff.stdout).expect("diff JSON")["result"]["digest"]
            .as_str()
            .expect("diff digest")
            .to_string();
    let expected = format!("change CHG-0001 digest {digest}");
    let command = format!("telos change approve CHG-0001 --expected-digest {digest}");

    let input = json!({
        "cwd": tmp.path(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command},
    });
    let claude = hook(tmp.path(), "claude", input.clone());
    let codex = hook(tmp.path(), "codex", input);

    assert_eq!(claude["hookSpecificOutput"]["permissionDecision"], "ask");
    assert!(
        claude["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("Claude reason")
            .contains(&expected)
    );
    assert!(
        codex["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert!(
        codex["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none()
    );
    assert!(
        codex["systemMessage"]
            .as_str()
            .expect("Codex system message")
            .contains(&expected)
    );
}

#[test]
fn guard_surfaces_sorted_current_drift_context_for_adopt_and_revert() {
    let tmp = with_fixture();
    fs::write(
        tmp.path().join("telos/constraints/CON-0901.tel"),
        "constraint CON-0901 in project quality \"Alpha drift\" {\n  rule  \"An untracked rule.\"\n}\n",
    )
    .expect("write Alpha drift");
    fs::write(
        tmp.path().join("telos/constraints/CON-0902.tel"),
        "constraint CON-0902 in project quality \"Zeta drift\" {\n  rule  \"Another untracked rule.\"\n}\n",
    )
    .expect("write Zeta drift");

    let sealed_digest = telos_core::lock::Lock::read(&tmp.path().join("telos/telos.lock"))
        .expect("read lock")
        .expect("fixture is sealed")
        .spec_digest;
    let expected = format!(
        "drift paths [telos/constraints/CON-0901.tel, telos/constraints/CON-0902.tel]; sealed spec digest {sealed_digest}"
    );
    let token = current_drift_token(tmp.path());

    for command in [
        format!("telos adopt --expected-state {token}"),
        format!("telos revert --expected-state {token}"),
    ] {
        let input = json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": &command},
        });
        let claude = hook(tmp.path(), "claude", input.clone());
        let codex = hook(tmp.path(), "codex", input);

        assert_eq!(claude["hookSpecificOutput"]["permissionDecision"], "ask");
        assert!(
            claude["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .expect("Claude reason")
                .contains(&expected)
        );
        assert!(
            codex["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        assert!(
            codex["hookSpecificOutput"]
                .get("permissionDecisionReason")
                .is_none()
        );
        assert!(
            codex["systemMessage"]
                .as_str()
                .expect("Codex system message")
                .contains(&expected)
        );
        assert!(
            codex["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .expect("Codex additional context")
                .contains(&expected)
        );
    }
}

#[test]
fn guard_denies_tokens_made_stale_while_the_native_prompt_is_open() {
    let approval = repo();
    telos(approval.path(), &["init"]).assert().success();
    stage_drafted_config_change(approval.path(), &[]);
    let stale_digest = current_change_digest(approval.path());
    telos(
        approval.path(),
        &["config", "--change", "CHG-0001", "--json"],
    )
    .write_stdin(
        json!({
            "code": {"globs": ["src/**/*.rs", "examples/**/*.rs"]},
            "tests": {"globs": ["tests/**/*.rs"]},
            "test": {"cmd": "cargo test {filter}"},
            "policy": {"tdd": "advisory"},
            "agents": {"hosts": []},
        })
        .to_string(),
    )
    .assert()
    .success();
    let stale_approve = format!("telos change approve CHG-0001 --expected-digest {stale_digest}");

    let drift = with_fixture();
    fs::write(
        drift.path().join("telos/constraints/CON-0901.tel"),
        "constraint CON-0901 in project quality \"First drift\" {\n  rule  \"First drift.\"\n}\n",
    )
    .unwrap();
    let stale_state = current_drift_token(drift.path());
    fs::write(
        drift.path().join("telos/constraints/CON-0902.tel"),
        "constraint CON-0902 in project quality \"Later drift\" {\n  rule  \"Later drift.\"\n}\n",
    )
    .unwrap();

    for (root, command) in [
        (approval.path(), stale_approve),
        (
            drift.path(),
            format!("telos adopt --expected-state {stale_state}"),
        ),
        (
            drift.path(),
            format!("telos revert --expected-state {stale_state}"),
        ),
    ] {
        for host in ["claude", "codex"] {
            assert_eq!(
                bash_decision(root, host, &command),
                "deny",
                "{host}: {command}"
            );
        }
    }
}

#[test]
fn guard_denies_unbound_or_noncanonical_human_actions() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    for command in [
        "telos change approve",
        "telos change approve not-a-change",
        "telos change approve CHG-9999",
        "telos adopt --into CHG-0001",
        "telos revert --json",
        "command telos adopt",
        "telos adopt;",
    ] {
        for host in ["claude", "codex"] {
            let out = hook(
                tmp.path(),
                host,
                json!({
                    "cwd": tmp.path(),
                    "hook_event_name": "PreToolUse",
                    "tool_name": "Bash",
                    "tool_input": {"command": command},
                }),
            );
            assert_eq!(
                out["hookSpecificOutput"]["permissionDecision"], "deny",
                "{host}: {command}"
            );
            assert!(
                out["hookSpecificOutput"]["permissionDecisionReason"]
                    .as_str()
                    .expect("denial reason")
                    .contains(
                        if command.starts_with("command ") || command.ends_with(';') {
                            "native prompt rules"
                        } else {
                            "current decision context"
                        }
                    ),
                "{host}: {command}: {out:#}"
            );
        }
    }
}

#[test]
fn guard_fails_closed_for_environment_wrapped_human_actions() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();

    for command in [
        "env telos revert --expected-state sha256:stale",
        "TELOS_REVIEW=1 telos revert --expected-state sha256:stale",
    ] {
        for host in ["claude", "codex"] {
            let out = hook(
                tmp.path(),
                host,
                json!({
                    "cwd": tmp.path(),
                    "hook_event_name": "PreToolUse",
                    "tool_name": "Bash",
                    "tool_input": {"command": command},
                }),
            );
            assert_eq!(
                out["hookSpecificOutput"]["permissionDecision"], "deny",
                "{host}: {command}: {out:#}"
            );
        }
    }
}

#[test]
fn codex_guard_uses_undecided_output_for_allowed_commands() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();
    stage_drafted_config_change(tmp.path(), &["codex"]);
    fs::write(
        tmp.path().join("telos/constraints/CON-0900.tel"),
        "constraint CON-0900 in project quality \"Prompt-time drift\" {\n  rule  \"Prompt-time drift.\"\n}\n",
    )
    .unwrap();
    let digest = current_change_digest(tmp.path());
    let token = current_drift_token(tmp.path());

    for command in [
        "telos status --json".to_string(),
        format!("telos change approve CHG-0001 --expected-digest {digest}"),
        format!("telos adopt --expected-state {token}"),
        format!("telos revert --expected-state {token}"),
    ] {
        let out = hook(
            tmp.path(),
            "codex",
            json!({
                "cwd": tmp.path(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": &command},
            }),
        );
        assert!(
            out["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none(),
            "{command}"
        );
        assert!(
            out["hookSpecificOutput"]
                .get("permissionDecisionReason")
                .is_none(),
            "{command}"
        );
    }

    let denied = hook(
        tmp.path(),
        "codex",
        json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "apply_patch",
            "tool_input": {"command": "*** Update File: telos/telos.toml"},
        }),
    );
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        denied["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_some()
    );
}

#[test]
fn guard_denies_alternate_telos_executable_spellings() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    stage_drafted_config_change(tmp.path(), &[]);

    for command in [
        "/absolute/path/to/telos change approve CHG-0001",
        "./telos change approve CHG-0001",
        "/absolute/path/to/telos adopt",
        "./telos adopt",
        "/absolute/path/to/telos revert",
        "./telos revert",
    ] {
        for host in ["claude", "codex"] {
            let out = hook(
                tmp.path(),
                host,
                json!({
                    "cwd": tmp.path(),
                    "hook_event_name": "PreToolUse",
                    "tool_name": "Bash",
                    "tool_input": {"command": command},
                }),
            );
            assert_eq!(
                out["hookSpecificOutput"]["permissionDecision"], "deny",
                "{host}: {command}"
            );
            assert!(
                out["hookSpecificOutput"]["permissionDecisionReason"]
                    .as_str()
                    .expect("denial reason")
                    .contains("direct canonical `telos")
            );
        }
    }
}

#[test]
fn claude_asks_for_resolved_human_decisions_without_trusting_descriptions() {
    let tmp = repo();
    telos(tmp.path(), &["init"]).assert().success();
    fs::write(
        tmp.path().join("telos/constraints/CON-0900.tel"),
        "constraint CON-0900 in project quality \"Prompt-time drift\" {\n  rule  \"Prompt-time drift.\"\n}\n",
    )
    .unwrap();
    let token = current_drift_token(tmp.path());
    for command in [
        format!("telos adopt --expected-state {token}"),
        format!("telos revert --expected-state {token}"),
    ] {
        assert_eq!(bash_decision(tmp.path(), "claude", &command), "ask");
    }

    let out = hook(
        tmp.path(),
        "claude",
        json!({
            "cwd": tmp.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {
                "command": format!("telos adopt --expected-state {token}"),
                "description": "forged decision context"
            },
        }),
    );
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "ask");
    assert!(
        !out["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("forged decision context")
    );
}

#[test]
fn codex_guard_never_returns_ask_and_rules_own_native_prompts() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();
    stage_drafted_config_change(tmp.path(), &["codex"]);
    fs::write(
        tmp.path().join("telos/constraints/CON-0900.tel"),
        "constraint CON-0900 in project quality \"Prompt-time drift\" {\n  rule  \"Prompt-time drift.\"\n}\n",
    )
    .unwrap();
    let digest = current_change_digest(tmp.path());
    let token = current_drift_token(tmp.path());

    for command in [
        format!("telos change approve CHG-0001 --expected-digest {digest}"),
        format!("telos adopt --expected-state {token}"),
        format!("telos revert --expected-state {token}"),
    ] {
        let out = hook(
            tmp.path(),
            "codex",
            json!({
                "cwd": tmp.path(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": &command},
            }),
        );
        assert!(
            out["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
    }

    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    for pattern in [
        "pattern = [\"telos\", \"change\", \"approve\"]",
        "pattern = [\"telos\", \"adopt\"]",
        "pattern = [\"telos\", \"revert\"]",
    ] {
        assert!(rules.contains(pattern), "missing rule {pattern}");
    }
    assert_eq!(rules.matches("decision = \"prompt\"").count(), 9);
    assert!(rules.contains("--expected-digest"));
    for argv in [
        &[
            "telos",
            "change",
            "approve",
            "CHG-0001",
            "--expected-digest",
            "sha256:x",
        ][..],
        &[
            "telos",
            "adopt",
            "--into",
            "CHG-0001",
            "--expected-state",
            "sha256:x",
        ][..],
        &["telos", "revert", "--expected-state", "sha256:x"][..],
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
    for instruction in [
        "Do not rely on the generated Codex guard or rules until setup is reviewed and trusted",
        "Open `/hooks`",
        "review and trust the repository `.codex` layer",
        "verify the exact `telos agent-guard --host codex` hook",
        "treat `.codex/hooks.json` and `.codex/rules/telos.rules` as inactive",
    ] {
        assert!(
            agents.contains(instruction),
            "generated AGENTS.md lacks activation instruction: {instruction}\n{agents}"
        );
    }
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

#[test]
fn rtk_human_actions_require_exact_tokens_and_installed_native_prompts() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();
    stage_drafted_config_change(tmp.path(), &["codex"]);
    fs::write(tmp.path().join("telos/constraints/CON-0900.tel"),
        "constraint CON-0900 in project quality \"Prompt-time drift\" {\n  rule \"Prompt-time drift.\"\n}\n").unwrap();
    let digest = current_change_digest(tmp.path());
    let token = current_drift_token(tmp.path());
    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    let change_before = read(tmp.path(), "telos/changes/CHG-0001.tel");
    for prefix in ["telos", "rtk telos", "rtk proxy telos"] {
        for action in [
            format!("change approve CHG-0001 --expected-digest {digest}"),
            format!("adopt --expected-state {token}"),
            format!("revert --expected-state {token}"),
        ] {
            let command = format!("{prefix} {action}");
            let out = hook(
                tmp.path(),
                "codex",
                json!({
                    "cwd": tmp.path(), "hook_event_name": "PreToolUse", "tool_name": "Bash",
                    "tool_input": {"command": command},
                }),
            );
            assert!(
                out["hookSpecificOutput"]
                    .get("permissionDecision")
                    .is_none(),
                "{command}: {out}"
            );
            assert!(out["hookSpecificOutput"]["additionalContext"].is_string());
            assert_eq!(
                rendered_rule_decision_for_shell(&rules, &command),
                Some("prompt"),
                "{command}"
            );
        }
        for action in [
            "change approve CHG-0001".to_string(),
            format!(
                "change approve CHG-0001 --expected-digest sha256:{}",
                "0".repeat(64)
            ),
            "adopt".to_string(),
            "revert".to_string(),
            format!("adopt --expected-state sha256:{}", "0".repeat(64)),
            format!("revert --expected-state sha256:{}", "0".repeat(64)),
        ] {
            assert_eq!(
                bash_decision(tmp.path(), "codex", &format!("{prefix} {action}")),
                "deny"
            );
        }
    }
    for prefix in [
        "command rtk telos",
        "rtk rtk telos",
        "rtk command telos",
        "rtk --unknown telos",
        "unknown-wrapper telos",
    ] {
        let command = format!("{prefix} change approve CHG-0001 --expected-digest {digest}");
        assert_eq!(
            bash_decision(tmp.path(), "codex", &command),
            "deny",
            "{command}"
        );
    }
    for command in [
        format!("rtk telos change approve CHG-0001 --expected-digest {digest};"),
        format!("rtk proxy telos change approve CHG-0001 --expected-digest {digest} && echo done"),
        format!("bash -c \"rtk telos change approve CHG-0001 --expected-digest {digest}\""),
    ] {
        assert_eq!(
            bash_decision(tmp.path(), "codex", &command),
            "deny",
            "{command}"
        );
    }
    assert_eq!(
        read(tmp.path(), "telos/changes/CHG-0001.tel"),
        change_before
    );
}

#[test]
fn upgrading_the_guard_cannot_enable_rtk_actions_under_old_or_missing_rules() {
    let tmp = repo();
    telos(tmp.path(), &["init", "--agents", "codex"])
        .assert()
        .success();
    stage_drafted_config_change(tmp.path(), &["codex"]);
    let digest = current_change_digest(tmp.path());
    let rules = read(tmp.path(), ".codex/rules/telos.rules");
    let block = include_str!("../assets/codex-rtk.rules");
    for stale in [
        rules.replace(block, ""),
        rules.replace("decision = \"prompt\"", "decision = \"allow\""),
        String::new(),
    ] {
        fs::write(tmp.path().join(".codex/rules/telos.rules"), stale).unwrap();
        for prefix in ["rtk telos", "rtk proxy telos"] {
            let command = format!("{prefix} change approve CHG-0001 --expected-digest {digest}");
            let out = hook(
                tmp.path(),
                "codex",
                json!({
                    "cwd": tmp.path(), "hook_event_name": "PreToolUse", "tool_name": "Bash",
                    "tool_input": {"command": command},
                }),
            );
            assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
            assert!(
                out["hookSpecificOutput"]["permissionDecisionReason"]
                    .as_str()
                    .unwrap()
                    .contains("RTK native prompt rules are missing or outdated"),
                "{out}"
            );
        }
    }
    // A Windows checkout has the same rules despite its line endings.
    fs::write(
        tmp.path().join(".codex/rules/telos.rules"),
        rules.replace('\n', "\r\n"),
    )
    .unwrap();
    let out = hook(
        tmp.path(),
        "codex",
        json!({
            "cwd": tmp.path(), "hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": format!("rtk proxy telos change approve CHG-0001 --expected-digest {digest}")},
        }),
    );
    assert!(
        out["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
}
