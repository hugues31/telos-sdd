//! Shared preventive policy for Claude Code and Codex hooks.

use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use telos_core::changes::{read_change, scan_changes};
use telos_core::git::GitRepo;
use telos_core::ids::ChangeId;
use telos_core::lock::Lock;
use telos_core::state::compute_state;
use telos_core::workspace::Workspace;

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
    pub context: Option<DecisionContext>,
}

/// Current repository facts presented with a human decision. This is kept
/// separate from the decision reason so the Codex adapter can use only its
/// supported model-visible hook fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionContext {
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleCommand {
    argv: Vec<String>,
    native_rule_covered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HumanAction {
    Approve(ChangeId, String),
    Adopt(String),
    Revert(String),
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
            context: None,
        },
    };

    println!("{}", hook_output(host, outcome));
    ExitCode::SUCCESS
}

fn hook_output(host: AgentHost, outcome: GuardDecision) -> Value {
    let mut output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
        }
    });

    match host {
        AgentHost::Claude => {
            let permission = match outcome.decision {
                Decision::Allow => "allow",
                Decision::Deny => "deny",
                Decision::Ask => "ask",
            };
            output["hookSpecificOutput"]["permissionDecision"] = json!(permission);
            output["hookSpecificOutput"]["permissionDecisionReason"] = json!(outcome.reason);
        }
        AgentHost::Codex => {
            // Codex accepts an undecided PreToolUse result, but rejects both
            // `ask` and `allow`. Static `.rules` own approval prompts, while
            // explicit denials still need to prevent unsafe mutations.
            if outcome.decision == Decision::Deny {
                output["hookSpecificOutput"]["permissionDecision"] = json!("deny");
                output["hookSpecificOutput"]["permissionDecisionReason"] = json!(outcome.reason);
            }
            if let Some(context) = outcome.context {
                output["systemMessage"] = json!(context.text);
                output["hookSpecificOutput"]["additionalContext"] = json!(context.text);
            }
        }
    }
    output
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
        if commands.iter().any(|command| {
            command.argv.first().is_some_and(|program| {
                uses_opaque_inline_eval(program_name(program), &command.argv)
            })
        }) {
            return deny_opaque_inline_eval();
        }
        if directly_mutates_telos(&commands, &cwd, &root) {
            return deny_manual_write();
        }

        let action = match human_action(&commands) {
            Ok(action) => action,
            Err(()) => return deny_unbound_action(),
        };
        if let Some(action) = action {
            let Ok(context) = decision_context(&action, &cwd) else {
                return deny_unbound_action();
            };
            if host == AgentHost::Claude {
                return GuardDecision {
                    decision: Decision::Ask,
                    reason: format!(
                        "Human approval required for `{}`: {}",
                        action.name(),
                        context.text
                    ),
                    context: Some(context),
                };
            }
            if commands.iter().any(|command| !command.native_rule_covered) {
                return GuardDecision {
                    decision: Decision::Deny,
                    reason: format!(
                        "Codex native rules cannot prompt for this wrapped or noncanonical action; retry direct canonical command `{}`",
                        action.name()
                    ),
                    context: None,
                };
            }
            return GuardDecision {
                decision: Decision::Allow,
                reason: "Codex native rules own the human approval prompt".into(),
                context: Some(context),
            };
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

/// Inline interpreter programs are opaque to this shell parser: their source
/// can construct a write target without exposing it as an argv path. Refuse
/// the interpreter's eval mode structurally, regardless of source text, while
/// leaving ordinary `interpreter path/to/script` invocations available.
fn uses_opaque_inline_eval(program: &str, command: &[String]) -> bool {
    if matches!(program, "awk" | "gawk" | "mawk" | "nawk") {
        return awk_uses_inline_program(command);
    }

    let options = if versioned_program(program, "python") || versioned_program(program, "pypy") {
        PYTHON_OPTIONS
    } else if versioned_program(program, "ruby") {
        RUBY_OPTIONS
    } else if versioned_program(program, "perl") {
        PERL_OPTIONS
    } else if matches!(program, "node" | "nodejs") {
        NODE_OPTIONS
    } else if versioned_program(program, "php") {
        PHP_OPTIONS
    } else if versioned_program(program, "lua") || versioned_program(program, "luajit") {
        LUA_OPTIONS
    } else {
        return false;
    };
    eval_before_interpreter_operand(command, options)
}

#[derive(Clone, Copy)]
struct InterpreterOptions {
    eval_short: &'static [char],
    eval_long: &'static [&'static str],
    value_short: &'static [char],
    attached_value_short: &'static [char],
    value_long: &'static [&'static str],
    terminal_short: &'static [char],
    terminal_long: &'static [&'static str],
}

const PYTHON_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['c'],
    eval_long: &[],
    value_short: &['W', 'X'],
    attached_value_short: &[],
    value_long: &["check-hash-based-pycs"],
    terminal_short: &['m'],
    terminal_long: &[],
};
const RUBY_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['e'],
    eval_long: &[],
    value_short: &['C', 'E', 'F', 'I', 'K', 'T', 'r'],
    attached_value_short: &[],
    value_long: &[
        "backtrace-limit",
        "disable",
        "dump",
        "enable",
        "encoding",
        "external-encoding",
        "internal-encoding",
    ],
    terminal_short: &['S'],
    terminal_long: &[],
};
const PERL_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['e', 'E'],
    eval_long: &[],
    value_short: &['F', 'I', 'M', 'm'],
    attached_value_short: &['0', 'C', 'D', 'd', 'i', 'l', 'x'],
    value_long: &[],
    terminal_short: &[],
    terminal_long: &[],
};
const NODE_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['e', 'p'],
    eval_long: &["eval", "print"],
    value_short: &['r'],
    attached_value_short: &[],
    value_long: &[
        "conditions",
        "env-file",
        "env-file-if-exists",
        "import",
        "inspect-port",
        "loader",
        "require",
        "title",
    ],
    terminal_short: &[],
    terminal_long: &["run"],
};
const PHP_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['r'],
    eval_long: &["run"],
    value_short: &['c', 'd', 'z'],
    attached_value_short: &[],
    value_long: &["define", "php-ini", "zend-extension"],
    terminal_short: &['f'],
    terminal_long: &["file"],
};
const LUA_OPTIONS: InterpreterOptions = InterpreterOptions {
    eval_short: &['e'],
    eval_long: &[],
    value_short: &['l'],
    attached_value_short: &[],
    value_long: &[],
    terminal_short: &[],
    terminal_long: &[],
};

fn eval_before_interpreter_operand(command: &[String], options: InterpreterOptions) -> bool {
    let mut index = 1;
    while let Some(argument) = command.get(index).map(String::as_str) {
        if argument == "--" || argument == "-" || !argument.starts_with('-') {
            return false;
        }
        if argument.starts_with("--") {
            if options
                .eval_long
                .iter()
                .any(|option| long_option(argument, option))
            {
                return true;
            }
            if options
                .terminal_long
                .iter()
                .any(|option| long_option(argument, option))
            {
                return false;
            }
            index += if options
                .value_long
                .iter()
                .any(|option| argument == format!("--{option}"))
            {
                2
            } else {
                1
            };
            continue;
        }

        let mut consumes_next = false;
        let mut short_options = argument[1..].chars().peekable();
        while let Some(option) = short_options.next() {
            if options.eval_short.contains(&option) {
                return true;
            }
            if options.terminal_short.contains(&option) {
                return false;
            }
            if options.value_short.contains(&option) {
                consumes_next = short_options.peek().is_none();
                break;
            }
            if options.attached_value_short.contains(&option) && short_options.peek().is_some() {
                break;
            }
        }
        index += if consumes_next { 2 } else { 1 };
    }
    false
}

fn versioned_program(program: &str, stem: &str) -> bool {
    program == stem
        || program.strip_prefix(stem).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
        })
}

fn long_option(argument: &str, option: &str) -> bool {
    argument == format!("--{option}") || argument.starts_with(&format!("--{option}="))
}

fn awk_uses_inline_program(command: &[String]) -> bool {
    let mut index = 1;
    let mut reads_program_file = false;
    while let Some(argument) = command.get(index).map(String::as_str) {
        match argument {
            "--" => return !reads_program_file && command.get(index + 1).is_some(),
            "-f" | "--file" => {
                reads_program_file = true;
                index += 2;
            }
            "-F" | "-v" | "--assign" => index += 2,
            _ if argument.starts_with("-f") && argument.len() > 2 => {
                reads_program_file = true;
                index += 1;
            }
            _ if argument.starts_with("--file=") => {
                reads_program_file = true;
                index += 1;
            }
            _ if argument.starts_with('-') => index += 1,
            _ => return !reads_program_file,
        }
    }
    false
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

impl HumanAction {
    fn name(&self) -> &'static str {
        match self {
            Self::Approve(_, _) => "telos change approve",
            Self::Adopt(_) => "telos adopt",
            Self::Revert(_) => "telos revert",
        }
    }
}

/// Recognizes the direct command forms the generated Codex native rules can
/// prompt for. Every other attempted human action is denied: presenting a
/// prompt without a bound current repository context would be unsafe.
fn human_action(commands: &[SimpleCommand]) -> Result<Option<HumanAction>, ()> {
    let Some(command) = commands.first() else {
        return Ok(None);
    };
    if commands.len() != 1 {
        return if commands.iter().any(is_human_action_attempt) {
            Err(())
        } else {
            Ok(None)
        };
    }
    if !command.native_rule_covered {
        return if is_human_action_attempt(command) {
            Err(())
        } else {
            Ok(None)
        };
    }

    match command.argv.as_slice() {
        [program, change, approve, id, flag, digest]
            if program == "telos" && change == "change" && approve == "approve" =>
        {
            if flag != "--expected-digest" {
                return Err(());
            }
            id.parse::<ChangeId>()
                .map(|id| HumanAction::Approve(id, digest.clone()))
                .map(Some)
                .map_err(|_| ())
        }
        [program, action, flag, token]
            if program == "telos" && action == "adopt" && flag == "--expected-state" =>
        {
            Ok(Some(HumanAction::Adopt(token.clone())))
        }
        [program, action, into_flag, id, state_flag, token]
            if program == "telos"
                && action == "adopt"
                && into_flag == "--into"
                && state_flag == "--expected-state" =>
        {
            id.parse::<ChangeId>().map_err(|_| ())?;
            Ok(Some(HumanAction::Adopt(token.clone())))
        }
        [program, action, flag, token]
            if program == "telos" && action == "revert" && flag == "--expected-state" =>
        {
            Ok(Some(HumanAction::Revert(token.clone())))
        }
        _ if is_human_action_attempt(command) => Err(()),
        _ => Ok(None),
    }
}

fn is_human_action_attempt(command: &SimpleCommand) -> bool {
    command
        .argv
        .first()
        .is_some_and(|program| program_name(program) == "telos")
        && (command.argv.windows(2).any(
            |words| matches!(words, [first, second] if first == "change" && second == "approve"),
        ) || command
            .argv
            .iter()
            .skip(1)
            .any(|word| matches!(word.as_str(), "adopt" | "revert")))
}

fn decision_context(action: &HumanAction, cwd: &Path) -> Result<DecisionContext, ()> {
    let workspace = Workspace::discover(cwd).map_err(|_| ())?;
    let text = match action {
        HumanAction::Approve(id, expected) => {
            let change = read_change(&workspace, *id).map_err(|_| ())?;
            let digest = change.ops_digest();
            if digest != expected.as_str() {
                return Err(());
            }
            format!("change {id} digest {digest}; token-bound command confirmed")
        }
        HumanAction::Adopt(expected) | HumanAction::Revert(expected) => {
            let lock = Lock::read(&workspace.lock_path())
                .map_err(|_| ())?
                .ok_or(())?;
            let git = GitRepo::discover(cwd).map_err(|_| ())?;
            let changes = scan_changes(&workspace).map_err(|_| ())?;
            let state = compute_state(&workspace, &lock, &git, &changes.infos).map_err(|_| ())?;
            let token = telos_core::state::drift_token(&lock, &state.drift);
            if token != expected.as_str() {
                return Err(());
            }
            let paths = state
                .drift
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} drift paths [{}]; sealed spec digest {}",
                action.name(),
                paths,
                lock.spec_digest
            )
        }
    };
    Ok(DecisionContext { text })
}

fn deny_manual_write() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Direct writes under repository telos/ are forbidden; use the Telos CLI".into(),
        context: None,
    }
}

fn deny_ambiguous_shell() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Shell syntax is not proven safe by the Telos guard; use a direct, simple command"
            .into(),
        context: None,
    }
}

fn deny_opaque_inline_eval() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Inline interpreter evaluation is not analyzable by the Telos guard; run a reviewed script file instead"
            .into(),
        context: None,
    }
}

fn deny_unbound_action() -> GuardDecision {
    GuardDecision {
        decision: Decision::Deny,
        reason: "Telos guard could not resolve current decision context; retry the direct canonical `telos ...` command spelling from an initialized repository".into(),
        context: None,
    }
}

fn allow(reason: &str) -> GuardDecision {
    GuardDecision {
        decision: Decision::Allow,
        reason: reason.into(),
        context: None,
    }
}
