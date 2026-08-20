//! Shared preventive policy for Claude Code and Codex hooks.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use super::AgentHost;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardDecision {
    pub decision: Decision,
    pub reason: String,
}

/// Reads one official hook event from stdin and prints the host's structured
/// `PreToolUse` answer. Invalid input fails closed because a guard that cannot
/// understand a prospective write must not silently permit it.
pub fn run(host: AgentHost) -> ExitCode {
    let mut input = String::new();
    let read = std::io::stdin().read_to_string(&mut input);
    let value = read
        .ok()
        .and_then(|_| serde_json::from_str::<Value>(&input).ok());

    let outcome = match value {
        Some(value) => {
            let tool = value.get("tool_name").and_then(Value::as_str).unwrap_or("");
            let tool_input = value.get("tool_input").unwrap_or(&Value::Null);
            let cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            decide(host, tool, tool_input, &cwd)
        }
        None => GuardDecision {
            decision: Decision::Deny,
            reason: "Telos guard could not parse the hook input; retry with valid JSON".into(),
        },
    };

    let permission = match outcome.decision {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
        Decision::Ask => "ask",
    };
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": permission,
                "permissionDecisionReason": outcome.reason,
            }
        })
    );
    ExitCode::SUCCESS
}

/// Applies the same normalized policy for both hosts. Codex deliberately
/// never receives `Ask`: its current hook runtime rejects that answer, so
/// native prompts are supplied by the generated `.rules` file instead.
pub fn decide(host: AgentHost, tool_name: &str, input: &Value, cwd: &Path) -> GuardDecision {
    let tool = normalize_tool(tool_name);
    let root = repo_root(cwd);

    if matches!(tool.as_str(), "edit" | "write") {
        if input_paths(input)
            .iter()
            .any(|path| is_telos_path(path, &root))
        {
            return deny_manual_write();
        }
        return allow("Source-code edit is outside the repository telos/ tree");
    }

    if tool == "apply_patch" {
        let patch = input
            .get("command")
            .or_else(|| input.get("patch"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if patch_paths(patch)
            .iter()
            .any(|path| is_telos_path(path, &root))
        {
            return deny_manual_write();
        }
        return allow("Patch does not target the repository telos/ tree");
    }

    if tool == "bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        let tokens = shell_tokens(command);
        if directly_mutates_telos(&tokens, &root) {
            return deny_manual_write();
        }

        if let Some(action) = human_action(&tokens) {
            if host == AgentHost::Claude {
                let context = input
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty());
                let mut reason = format!(
                    "Human approval required for `{action}`; review the current Telos diff and digest"
                );
                if let Some(context) = context {
                    reason.push_str(": ");
                    reason.push_str(context);
                }
                return GuardDecision {
                    decision: Decision::Ask,
                    reason,
                };
            }
            return allow("Codex native rules own the human approval prompt");
        }

        return allow("Command does not directly mutate the repository telos/ tree");
    }

    allow("Tool is outside the Telos guard policy")
}

fn normalize_tool(tool: &str) -> String {
    tool.rsplit("::")
        .next()
        .unwrap_or(tool)
        .trim()
        .to_ascii_lowercase()
}

fn input_paths(input: &Value) -> Vec<&str> {
    ["file_path", "path", "target_path"]
        .into_iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .collect()
}

fn patch_paths(patch: &str) -> Vec<&str> {
    const HEADERS: [&str; 4] = [
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    patch
        .lines()
        .filter_map(|line| HEADERS.iter().find_map(|header| line.strip_prefix(header)))
        .collect()
}

fn repo_root(cwd: &Path) -> PathBuf {
    let absolute = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(cwd)
    };
    absolute
        .ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(&absolute)
        .to_path_buf()
}

fn is_telos_path(raw: &str, root: &Path) -> bool {
    let cleaned = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | ',' | ';' | ':' | '(' | ')'))
        .replace('\\', "/");
    if cleaned.is_empty() {
        return false;
    }
    let path = Path::new(&cleaned);
    let joined = if path.is_absolute() {
        lexical_normalize(path)
    } else {
        lexical_normalize(&root.join(path))
    };
    let telos = lexical_normalize(&root.join("telos"));
    joined == telos || joined.starts_with(&telos)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ' ' | '\t' | '\r' | '\n' => push_token(&mut tokens, &mut current),
            '>' | '<' => {
                push_token(&mut tokens, &mut current);
                let mut op = ch.to_string();
                if chars.peek() == Some(&ch) {
                    op.push(chars.next().expect("peeked character exists"));
                }
                tokens.push(op);
            }
            ';' | '|' => {
                push_token(&mut tokens, &mut current);
                tokens.push(ch.to_string());
            }
            '&' if chars.peek() == Some(&'&') => {
                push_token(&mut tokens, &mut current);
                chars.next();
                tokens.push("&&".into());
            }
            _ => current.push(ch),
        }
    }
    push_token(&mut tokens, &mut current);
    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn command_slices(tokens: &[String]) -> impl Iterator<Item = &[String]> {
    tokens.split(|token| matches!(token.as_str(), ";" | "|" | "&&" | "||"))
}

fn strip_rtk(tokens: &[String]) -> &[String] {
    if tokens.first().map(String::as_str) == Some("rtk") {
        &tokens[1..]
    } else {
        tokens
    }
}

fn directly_mutates_telos(tokens: &[String], root: &Path) -> bool {
    command_slices(tokens).any(|slice| {
        let command = strip_rtk(slice);
        let Some(program) = command.first().map(String::as_str) else {
            return false;
        };
        if program == "telos" {
            return false;
        }

        for pair in command.windows(2) {
            if matches!(pair[0].as_str(), ">" | ">>") && is_telos_path(&pair[1], root) {
                return true;
            }
        }

        let any_telos = command.iter().skip(1).any(|arg| is_telos_path(arg, root));
        match program {
            "touch" | "mkdir" | "rm" | "rmdir" | "unlink" | "truncate" | "mv" | "cp"
            | "install" | "tee" | "chmod" | "chown" => any_telos,
            "sed" => {
                command
                    .iter()
                    .any(|arg| arg == "-i" || arg.starts_with("-i"))
                    && any_telos
            }
            "perl" => command.iter().any(|arg| arg.contains('i')) && any_telos,
            "git" => {
                matches!(
                    command.get(1).map(String::as_str),
                    Some("checkout" | "restore" | "clean")
                ) && any_telos
            }
            _ => false,
        }
    })
}

fn human_action(tokens: &[String]) -> Option<&'static str> {
    command_slices(tokens).find_map(|slice| {
        let command = strip_rtk(slice);
        let words: Vec<&str> = command
            .iter()
            .filter(|word| word.as_str() != "--json")
            .map(String::as_str)
            .collect();
        match words.as_slice() {
            ["telos", "change", "approve", ..] => Some("telos change approve"),
            ["telos", "adopt", ..] => Some("telos adopt"),
            ["telos", "revert", ..] => Some("telos revert"),
            _ => None,
        }
    })
}

fn deny_manual_write() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Direct writes under repository telos/ are forbidden; use the Telos CLI".into(),
    }
}

fn allow(reason: &str) -> GuardDecision {
    GuardDecision {
        decision: Decision::Allow,
        reason: reason.into(),
    }
}
