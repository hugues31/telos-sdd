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
    let cwd = absolute_cwd(cwd);
    let root = repo_root(&cwd);

    if matches!(tool.as_str(), "edit" | "write") {
        if input_paths(input)
            .iter()
            .any(|path| is_telos_path(path, &cwd, &root))
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
            .any(|path| is_telos_path(path, &cwd, &root))
        {
            return deny_manual_write();
        }
        return allow("Patch does not target the repository telos/ tree");
    }

    if tool == "bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        let commands = match simple_commands(command) {
            Ok(commands) => commands,
            Err(()) => return deny_ambiguous_shell(),
        };
        if directly_mutates_telos(&commands, &cwd, &root) {
            return deny_manual_write();
        }

        if let Some(action) = human_action(&commands) {
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

fn absolute_cwd(cwd: &Path) -> PathBuf {
    let absolute = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(cwd)
    };
    lexical_normalize(&absolute)
}

fn repo_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

fn is_telos_path(raw: &str, cwd: &Path, root: &Path) -> bool {
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
        lexical_normalize(&cwd.join(path))
    };
    let root = lexical_normalize(root);
    if !joined.starts_with(&root) {
        return false;
    }
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

fn simple_commands(command: &str) -> Result<Vec<Vec<String>>, ()> {
    let tokens = shell_tokens(command)?;
    let mut commands = Vec::new();
    for slice in tokens.split(|token| is_separator(token)) {
        expand_command(slice, &mut commands)?;
    }
    Ok(commands)
}

fn expand_command(tokens: &[String], commands: &mut Vec<Vec<String>>) -> Result<(), ()> {
    if tokens.is_empty() {
        return Ok(());
    }

    let mut command = tokens;
    let mut wrappers = 0;
    loop {
        wrappers += 1;
        if wrappers > 8 {
            return Err(());
        }
        match command.first().map(|word| program_name(word)) {
            Some("rtk") => command = &command[1..],
            Some("command") => {
                command = &command[1..];
                if command.first().map(String::as_str) == Some("--") {
                    command = &command[1..];
                }
            }
            _ => break,
        }
        if command.is_empty() {
            return Err(());
        }
    }

    let program = program_name(command.first().ok_or(())?);
    if matches!(program, "bash" | "sh" | "zsh") {
        let option = command.get(1).map(String::as_str).ok_or(())?;
        if !option.starts_with('-') || !option.chars().skip(1).any(|ch| ch == 'c') {
            return Err(());
        }
        let nested = command.get(2).ok_or(())?;
        commands.extend(simple_commands(nested)?);
        return Ok(());
    }

    if matches!(
        program,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "select"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "function"
    ) {
        return Err(());
    }

    commands.push(command.to_vec());
    Ok(())
}

fn shell_tokens(command: &str) -> Result<Vec<String>, ()> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            } else if active == '"' && matches!(ch, '$' | '`') {
                return Err(());
            } else if active == '"' && ch == '\\' {
                current.push(chars.next().ok_or(())?);
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ' ' | '\t' => push_token(&mut tokens, &mut current),
            '\r' | '\n' => {
                push_token(&mut tokens, &mut current);
                tokens.push(";".into());
            }
            '\\' => current.push(chars.next().ok_or(())?),
            '$' | '`' | '(' | ')' | '{' | '}' => return Err(()),
            '>' | '<' => {
                push_token(&mut tokens, &mut current);
                let mut op = ch.to_string();
                if chars.peek() == Some(&ch) {
                    op.push(chars.next().expect("peeked character exists"));
                }
                if op == "<<" {
                    return Err(());
                }
                tokens.push(op);
            }
            ';' => {
                push_token(&mut tokens, &mut current);
                tokens.push(ch.to_string());
            }
            '|' | '&' => {
                push_token(&mut tokens, &mut current);
                let mut op = ch.to_string();
                if chars.peek() == Some(&ch) {
                    op.push(chars.next().expect("peeked character exists"));
                }
                tokens.push(op);
            }
            _ => current.push(ch),
        }
    }
    if quote.is_some() {
        return Err(());
    }
    push_token(&mut tokens, &mut current);
    Ok(tokens)
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_separator(token: &str) -> bool {
    matches!(token, ";" | "|" | "||" | "&" | "&&")
}

fn program_name(program: &str) -> &str {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
}

fn directly_mutates_telos(commands: &[Vec<String>], cwd: &Path, root: &Path) -> bool {
    commands.iter().any(|command| {
        let Some(program) = command.first().map(String::as_str) else {
            return false;
        };
        let program = program_name(program);
        if program == "telos" {
            return false;
        }

        for pair in command.windows(2) {
            if matches!(pair[0].as_str(), ">" | ">>") && is_telos_path(&pair[1], cwd, root) {
                return true;
            }
        }

        let any_telos = command
            .iter()
            .skip(1)
            .any(|arg| is_telos_path(arg, cwd, root));
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
            _ => any_telos && !is_proven_read_only(program, command),
        }
    })
}

fn is_proven_read_only(program: &str, command: &[String]) -> bool {
    match program {
        "cat" | "head" | "tail" | "less" | "more" | "rg" | "grep" | "find" | "ls" | "stat"
        | "wc" | "file" | "diff" | "cmp" | "echo" | "printf" => true,
        "git" => matches!(
            command.get(1).map(String::as_str),
            Some("diff" | "status" | "show" | "log" | "grep" | "ls-files")
        ),
        _ => false,
    }
}

fn human_action(commands: &[Vec<String>]) -> Option<&'static str> {
    commands.iter().find_map(|command| {
        let words: Vec<&str> = command
            .iter()
            .filter(|word| word.as_str() != "--json")
            .map(|word| program_name(word))
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

fn deny_ambiguous_shell() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Shell syntax is not proven safe by the Telos guard; use a direct, simple command"
            .into(),
    }
}

fn allow(reason: &str) -> GuardDecision {
    GuardDecision {
        decision: Decision::Allow,
        reason: reason.into(),
    }
}
