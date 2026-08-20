//! Shared preventive policy for Claude Code and Codex hooks.

use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleCommand {
    argv: Vec<String>,
    native_rule_covered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HumanAction {
    name: &'static str,
    native_rule_covered: bool,
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
            .any(|path| path_requires_denial(path, &cwd, &root))
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
            .any(|path| path_requires_denial(path, &cwd, &root))
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
                    "Human approval required for `{}`; review the current Telos diff and digest",
                    action.name
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
            if !action.native_rule_covered {
                return GuardDecision {
                    decision: Decision::Deny,
                    reason: format!(
                        "Codex native rules cannot prompt for this wrapped or noncanonical action; retry direct canonical command `{}`",
                        action.name
                    ),
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
    fs::canonicalize(&absolute).unwrap_or_else(|_| lexical_normalize(&absolute))
}

fn repo_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// Returns true when a prospective path is not proven safe for a write.
///
/// The hook supplies paths relative to its own cwd.  Resolve the nearest
/// existing ancestor so directory symlinks cannot disguise telos/, then put
/// back the not-yet-created suffix.  A write target outside the repository or
/// a shell-expanded target is also unsafe: the guard must not guess where the
/// shell will send it.
fn path_requires_denial(raw: &str, cwd: &Path, root: &Path) -> bool {
    let cleaned = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | ',' | ';' | ':' | '(' | ')'))
        .replace('\\', "/");
    if cleaned.is_empty() {
        return false;
    }
    if cleaned
        .chars()
        .any(|character| matches!(character, '~' | '?' | '[' | ']' | '$' | '*'))
    {
        return true;
    }
    let path = Path::new(&cleaned);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };

    let Ok(root) = fs::canonicalize(root) else {
        return true;
    };
    let Ok(candidate) = resolve_nearest_existing_parent(&candidate) else {
        return true;
    };
    if !candidate.starts_with(&root) {
        return true;
    }

    let telos_candidate = root.join("telos");
    let telos = resolve_nearest_existing_parent(&telos_candidate)
        .unwrap_or_else(|_| lexical_normalize(&telos_candidate));
    candidate == telos || candidate.starts_with(&telos)
}

fn resolve_nearest_existing_parent(path: &Path) -> Result<PathBuf, ()> {
    let mut parent = path.to_path_buf();
    let mut suffix = Vec::<OsString>::new();
    loop {
        match fs::symlink_metadata(&parent) {
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let name = parent.file_name().ok_or(())?.to_os_string();
                suffix.push(name);
                parent = parent.parent().ok_or(())?.to_path_buf();
            }
            Err(_) => return Err(()),
        }
    }

    let mut resolved = fs::canonicalize(parent).map_err(|_| ())?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(lexical_normalize(&resolved))
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

fn simple_commands(command: &str) -> Result<Vec<SimpleCommand>, ()> {
    let tokens = shell_tokens(command)?;
    let has_separator = tokens.iter().any(|token| is_separator(token));
    let mut commands = Vec::new();
    for slice in tokens.split(|token| is_separator(token)) {
        expand_command(slice, &mut commands, false)?;
    }

    if commands.len() > 1
        && commands.iter().any(|command| {
            command
                .argv
                .first()
                .is_some_and(|word| matches!(program_name(word), "cd" | "pushd" | "popd"))
        })
    {
        return Err(());
    }
    if has_separator || commands.len() != 1 {
        for command in &mut commands {
            command.native_rule_covered = false;
        }
    }
    Ok(commands)
}

fn expand_command(
    tokens: &[String],
    commands: &mut Vec<SimpleCommand>,
    inherited_wrapper: bool,
) -> Result<(), ()> {
    if tokens.is_empty() {
        return Ok(());
    }

    let mut command = tokens;
    let mut wrappers = 0;
    let mut wrapped = inherited_wrapper;
    loop {
        wrappers += 1;
        if wrappers > 8 {
            return Err(());
        }
        match command.first().map(|word| program_name(word)) {
            Some("rtk") => {
                wrapped = true;
                command = &command[1..];
            }
            Some("command") => {
                wrapped = true;
                command = &command[1..];
                if command.first().map(String::as_str) == Some("--") {
                    command = &command[1..];
                } else if command.first().is_some_and(|word| word.starts_with('-')) {
                    return Err(());
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
        if !matches!(option, "-c" | "-lc" | "-cl") || command.len() != 3 {
            return Err(());
        }
        let nested = command.get(2).ok_or(())?;
        let mut nested_commands = simple_commands(nested)?;
        for nested_command in &mut nested_commands {
            nested_command.native_rule_covered = false;
        }
        commands.extend(nested_commands);
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

    commands.push(SimpleCommand {
        argv: command.to_vec(),
        native_rule_covered: !wrapped,
    });
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
            '>' => {
                push_token(&mut tokens, &mut current);
                let op = match chars.peek() {
                    Some('>') => {
                        chars.next();
                        ">>"
                    }
                    Some('|') => {
                        chars.next();
                        ">|"
                    }
                    Some('&') => return Err(()),
                    _ => ">",
                };
                tokens.push(op.into());
            }
            '<' => {
                push_token(&mut tokens, &mut current);
                if matches!(chars.peek(), Some('<' | '>' | '&')) {
                    return Err(());
                }
                tokens.push("<".into());
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

fn directly_mutates_telos(commands: &[SimpleCommand], cwd: &Path, root: &Path) -> bool {
    commands.iter().any(|command| {
        let Some(program) = command.argv.first().map(String::as_str) else {
            return false;
        };
        let program = program_name(program);

        for pair in command.argv.windows(2) {
            if matches!(pair[0].as_str(), ">" | ">>" | ">|")
                && path_requires_denial(&pair[1], cwd, root)
            {
                return true;
            }
        }
        if program == "telos" {
            return false;
        }

        let any_unsafe_path = command
            .argv
            .iter()
            .skip(1)
            .any(|arg| argument_requires_denial(arg, cwd, root));
        match program {
            "touch" | "mkdir" | "rm" | "rmdir" | "unlink" | "truncate" | "mv" | "cp"
            | "install" | "tee" | "chmod" | "chown" => any_unsafe_path,
            "sed" => {
                command
                    .argv
                    .iter()
                    .any(|arg| arg == "-i" || arg.starts_with("-i"))
                    && any_unsafe_path
            }
            "perl" => command.argv.iter().any(|arg| arg.contains('i')) && any_unsafe_path,
            "git" => match git_subcommand(&command.argv) {
                Some(subcommand) if is_read_only_git_subcommand(subcommand) => false,
                Some(_) => any_unsafe_path,
                None => true,
            },
            _ => any_unsafe_path && !is_proven_read_only(program, &command.argv),
        }
    })
}

fn argument_requires_denial(argument: &str, cwd: &Path, root: &Path) -> bool {
    if path_requires_denial(argument, cwd, root) {
        return true;
    }
    if let Some((_, value)) = argument.split_once('=')
        && !value.is_empty()
        && path_requires_denial(value, cwd, root)
    {
        return true;
    }
    if let Some(value) = argument
        .strip_prefix("-C")
        .filter(|value| !value.is_empty())
    {
        return path_requires_denial(value, cwd, root);
    }
    false
}

fn is_proven_read_only(program: &str, command: &[String]) -> bool {
    match program {
        "cat" | "head" | "tail" | "less" | "more" | "rg" | "grep" | "ls" | "stat" | "wc"
        | "file" | "diff" | "cmp" | "echo" | "printf" => true,
        "find" => !command.iter().any(|argument| {
            matches!(
                argument.as_str(),
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
                    | "-fls"
            )
        }),
        "git" => git_subcommand(command).is_some_and(is_read_only_git_subcommand),
        _ => false,
    }
}

fn git_subcommand(command: &[String]) -> Option<&str> {
    let mut index = 1;
    while let Some(argument) = command.get(index).map(String::as_str) {
        if argument == "--" {
            return command.get(index + 1).map(String::as_str);
        }
        if matches!(
            argument,
            "-C" | "-c"
                | "--git-dir"
                | "--work-tree"
                | "--namespace"
                | "--exec-path"
                | "--config-env"
        ) {
            command.get(index + 1)?;
            index += 2;
            continue;
        }
        if argument.starts_with("-C") && argument.len() > 2 {
            index += 1;
            continue;
        }
        if [
            "--git-dir=",
            "--work-tree=",
            "--namespace=",
            "--exec-path=",
            "--config-env=",
        ]
        .iter()
        .any(|prefix| argument.starts_with(prefix))
        {
            index += 1;
            continue;
        }
        if matches!(
            argument,
            "-p" | "-P"
                | "--paginate"
                | "--no-pager"
                | "--bare"
                | "--literal-pathspecs"
                | "--glob-pathspecs"
                | "--noglob-pathspecs"
                | "--icase-pathspecs"
        ) {
            index += 1;
            continue;
        }
        if argument.starts_with('-') {
            return None;
        }
        return Some(argument);
    }
    None
}

fn is_read_only_git_subcommand(subcommand: &str) -> bool {
    matches!(
        subcommand,
        "diff" | "status" | "show" | "log" | "grep" | "ls-files"
    )
}

fn human_action(commands: &[SimpleCommand]) -> Option<HumanAction> {
    commands.iter().find_map(|command| {
        let words: Vec<&str> = command
            .argv
            .iter()
            .filter(|word| word.as_str() != "--json")
            .map(|word| program_name(word))
            .collect();
        let name = match words.as_slice() {
            ["telos", "change", "approve", ..] => "telos change approve",
            ["telos", "adopt", ..] => "telos adopt",
            ["telos", "revert", ..] => "telos revert",
            _ => return None,
        };
        let literal_prefix_covered = match name {
            "telos change approve" => {
                command
                    .argv
                    .starts_with(&["telos".into(), "change".into(), "approve".into()])
            }
            "telos adopt" => command.argv.starts_with(&["telos".into(), "adopt".into()]),
            "telos revert" => command.argv.starts_with(&["telos".into(), "revert".into()]),
            _ => false,
        };
        Some(HumanAction {
            name,
            native_rule_covered: command.native_rule_covered && literal_prefix_covered,
        })
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
