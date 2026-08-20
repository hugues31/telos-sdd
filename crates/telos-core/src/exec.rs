//! Cross-OS command execution: the platform shell `check` and `[test] cmd`
//! run through (D9), and `{filter}` substitution (D10).

use std::path::Path;
use std::process::Command;

use crate::error::{ErrorCode, TelosError};

/// The result of running a shell command: its exit status and captured
/// output.
///
/// A non-zero `status` is *not* an error at this level -- a failing check
/// or test run is exactly the outcome the caller needs to see and decide
/// what to do with (D10/D11 turn it into `TELOS_INTEGRITY_VIOLATION` /
/// `TELOS_CONSTRAINT_FAILED` one layer up); this module only runs the
/// command and reports what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A filtered shell run: the frozen literal-substitution command exposed to
/// callers, and the outcome of executing the filter as one data argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredRun {
    pub command: String,
    pub result: RunResult,
}

/// Runs `cmd` through the platform shell, with `cwd` as the working
/// directory (D9): `sh -c "<cmd>"` on Unix, `cmd /C "<cmd>"` on Windows. The
/// spec calls `check`/`[test] cmd` a shell command, so this is the one
/// place that distinction is made -- everything above it just has a string.
///
/// Output is decoded with `String::from_utf8_lossy` rather than rejected on
/// invalid UTF-8: a test runner's output is still worth reporting even if a
/// stray byte in it isn't valid UTF-8.
///
/// Failing to spawn the shell itself -- `sh`/`cmd` missing from `PATH`,
/// pathological since both ship with their OS -- is `TelosInternal`,
/// carrying the `io` error in the message. It is not folded into
/// `RunResult`: a process that never started has no exit status to carry.
pub fn run_shell(cmd: &str, cwd: &Path) -> Result<RunResult, TelosError> {
    let output = shell_command(cmd).current_dir(cwd).output().map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to spawn the platform shell to run `{cmd}`: {e}"),
        )
    })?;

    Ok(run_result(output))
}

/// Displays D10's exact literal substitution while executing a non-empty
/// filter through one quoted environment expansion.
///
/// Shell syntax inside a spec-owned test locator must stay data. The command
/// returned in [`FilteredRun::command`] remains byte-for-byte
/// [`substitute_filter`], preserving the public contract. The process itself
/// receives the filter through a private environment variable referenced
/// inside shell quotes, so spaces and metacharacters cannot split it into
/// more commands. Empty filters and templates without `{filter}` retain the
/// historical exact execution path.
pub fn run_shell_with_filter(
    template: &str,
    filter: &str,
    cwd: &Path,
) -> Result<FilteredRun, TelosError> {
    let command = substitute_filter(template, filter);
    if filter.is_empty() || !template.contains("{filter}") {
        return Ok(FilteredRun {
            result: run_shell(&command, cwd)?,
            command,
        });
    }

    const FILTER_ENV: &str = "TELOS_INTERNAL_TEST_FILTER";
    let reference = if cfg!(windows) {
        "\"%TELOS_INTERNAL_TEST_FILTER%\""
    } else {
        "\"$TELOS_INTERNAL_TEST_FILTER\""
    };
    let executable = safe_filter_command(template, reference)?;
    let output = shell_command(&executable)
        .env(FILTER_ENV, filter)
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to spawn the platform shell to run `{command}`: {e}"),
            )
        })?;

    Ok(FilteredRun {
        command,
        result: run_result(output),
    })
}

/// Rewrites each shell-active `{filter}` as a quoted environment reference.
///
/// A placeholder may be unquoted or be the complete contents of one quoted
/// argument. In the latter case the source quotes are consumed because the
/// environment reference already supplies its own quotes. Embedding the
/// placeholder in a larger quoted fragment is rejected: composing shell
/// syntax around untrusted data that way cannot be made portable without
/// changing the command's argument contract.
fn safe_filter_command(template: &str, reference: &str) -> Result<String, TelosError> {
    const FILTER: &str = "{filter}";
    const DOUBLE_QUOTED_FILTER: &str = "\"{filter}\"";
    const SINGLE_QUOTED_FILTER: &str = "'{filter}'";

    let mut executable = String::with_capacity(template.len() + reference.len());
    let mut quote = None;
    let mut index = 0;

    while index < template.len() {
        let rest = &template[index..];

        if quote.is_none() {
            if rest.starts_with(DOUBLE_QUOTED_FILTER)
                || (cfg!(unix) && rest.starts_with(SINGLE_QUOTED_FILTER))
            {
                executable.push_str(reference);
                index += DOUBLE_QUOTED_FILTER.len();
                continue;
            }
            if rest.starts_with(FILTER) {
                executable.push_str(reference);
                index += FILTER.len();
                continue;
            }
        } else if rest.starts_with(FILTER) {
            return Err(TelosError::new(
                ErrorCode::TelosParseError,
                "unsafe [test] cmd: {filter} must be unquoted or the whole quoted argument",
            ));
        }

        let character = rest
            .chars()
            .next()
            .expect("index is within the command string");
        executable.push(character);
        index += character.len_utf8();

        if is_shell_escape(character, quote) {
            if template[index..].starts_with(FILTER) {
                return Err(TelosError::new(
                    ErrorCode::TelosParseError,
                    "unsafe [test] cmd: {filter} cannot be shell-escaped",
                ));
            }
            if let Some(escaped) = template[index..].chars().next() {
                executable.push(escaped);
                index += escaped.len_utf8();
            }
            continue;
        }

        match quote {
            Some(active) if character == active => quote = None,
            None if character == '"' || (cfg!(unix) && character == '\'') => {
                quote = Some(character)
            }
            _ => {}
        }
    }

    Ok(executable.trim_end().to_string())
}

fn is_shell_escape(character: char, quote: Option<char>) -> bool {
    if cfg!(windows) {
        character == '^'
    } else {
        character == '\\' && quote != Some('\'')
    }
}

fn run_result(output: std::process::Output) -> RunResult {
    RunResult {
        // A `None` exit code (the process was killed by a signal, Unix
        // only) has no faithful `i32` -- `-1` is not a real exit code a
        // process can produce, so it cannot be confused with one.
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Builds the unspawned platform-shell [`Command`] for `cmd` (D9).
fn shell_command(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(cmd);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg("-c").arg(cmd);
        command
    }
}

/// Replaces every occurrence of the literal `{filter}` in `cmd` with
/// `filter`, then `trim_end`s the whole result (D10).
///
/// The `trim_end` runs after substitution and over the *whole* string, not
/// just around the placeholder, so an empty filter (the `--full` case)
/// leaves no trailing whitespace where the placeholder used to sit:
/// `"cargo test {filter}"` with an empty filter becomes `"cargo test"`, not
/// `"cargo test "`.
pub fn substitute_filter(cmd: &str, filter: &str) -> String {
    cmd.replace("{filter}", filter).trim_end().to_string()
}
