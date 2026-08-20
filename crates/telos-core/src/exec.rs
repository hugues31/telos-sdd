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

    Ok(RunResult {
        // A `None` exit code (the process was killed by a signal, Unix
        // only) has no faithful `i32` -- `-1` is not a real exit code a
        // process can produce, so it cannot be confused with one.
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
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
