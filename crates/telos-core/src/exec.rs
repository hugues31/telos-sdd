//! Cross-OS command execution: the platform shell `check` and `[test] cmd`
//! run through, and `{filter}` substitution.

use std::path::Path;
use std::process::Command;

use crate::error::{ErrorCode, TelosError};

/// The result of running a shell command: its exit status and captured
/// output.
///
/// A non-zero `status` is *not* an error at this level -- a failing check
/// or test run is exactly the outcome the caller needs to see and decide
/// what to do with. The caller converts it to `TELOS_INTEGRITY_VIOLATION` or
/// `TELOS_CONSTRAINT_FAILED`; this module only runs the command and reports
/// what happened.
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
/// directory: `sh -c "<cmd>"` on Unix, `cmd /C "<cmd>"` on Windows. The
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

/// What a runner template's placeholders resolve to.
///
/// A struct rather than three positional `&str`, which would be trivially
/// swappable at four call sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct Substitutions<'a> {
    /// `{filter}`: the target a runner should select, its test name or path.
    pub filter: &'a str,
    /// `{features}`: a directory of freshly-rendered `.feature` files, empty
    /// when `[gherkin]` is off.
    pub features: &'a str,
    /// `{scenario}`: the Gherkin tag expression selecting the scenarios this
    /// run proves (`@SCN-0107`, or `@SCN-0091 or @SCN-0107`). Empty for a
    /// whole-suite run.
    pub scenario: &'a str,
}

/// Displays the runner template's exact literal substitution while executing a deliberately
/// restricted runner template as a direct process argv. No shell sees any
/// substituted value; each remains data even in an embedded word such as
/// `module::{filter}` or `{features}/billing/INT-0042.feature`.
///
/// An empty value drops its token from the argument vector entirely.
pub fn run_runner_template(
    template: &str,
    subs: &Substitutions<'_>,
    cwd: &Path,
) -> Result<FilteredRun, TelosError> {
    let command = substitute_placeholders(template, subs);
    validate_filter_data(subs.filter)?;
    validate_filter_data(subs.features)?;
    validate_filter_data(subs.scenario)?;
    let argv = parse_runner_template(template, subs)?;
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to spawn the test runner displayed as `{command}`: {e}"),
            )
        })?;

    Ok(FilteredRun {
        command,
        result: run_result(output),
    })
}

fn validate_filter_data(filter: &str) -> Result<(), TelosError> {
    if filter.chars().any(char::is_control) {
        return Err(TelosError::new(
            ErrorCode::TelosParseError,
            "test filter contains control bytes that cannot be passed safely",
        ));
    }
    Ok(())
}

fn parse_runner_template(
    template: &str,
    subs: &Substitutions<'_>,
) -> Result<Vec<String>, TelosError> {
    const PLACEHOLDERS: [&str; 3] = ["{filter}", "{features}", "{scenario}"];
    let unsafe_template = || {
        TelosError::new(
            ErrorCode::TelosParseError,
            "unsafe [test] cmd: use one direct executable with simple quoted arguments; shell operators, substitutions, eval/call and nested interpreters are not supported",
        )
        .hint("use a dedicated runner script and pass `{filter}` as one direct argument")
    };
    let mut argv = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut index = 0;

    while index < template.len() {
        let rest = &template[index..];

        // Substituted as *data*: the value joins the current word and is
        // never re-scanned for shell syntax. An empty value contributes
        // nothing, so the token vanishes rather than leaving an empty argv
        // entry.
        if let Some((placeholder, value)) = PLACEHOLDERS
            .iter()
            .zip([subs.filter, subs.features, subs.scenario])
            .find(|(placeholder, _)| rest.starts_with(**placeholder))
        {
            if !value.is_empty() {
                word.push_str(value);
                in_word = true;
            }
            index += placeholder.len();
            continue;
        }

        let character = rest
            .chars()
            .next()
            .expect("index is within the command string");
        index += character.len_utf8();

        if character.is_control() && !character.is_whitespace() {
            return Err(unsafe_template());
        }
        if quote.is_none() && character.is_whitespace() {
            if in_word {
                argv.push(std::mem::take(&mut word));
                in_word = false;
            }
            continue;
        }
        if "$`;&|<>()".contains(character) {
            return Err(unsafe_template());
        }
        match quote {
            Some(active) if character == active => {
                quote = None;
                in_word = true;
            }
            None if matches!(character, '\'' | '"') => {
                quote = Some(character);
                in_word = true;
            }
            Some('\'') => {
                word.push(character);
                in_word = true;
            }
            _ if character == '\\' => {
                let Some(escaped) = template[index..].chars().next() else {
                    return Err(unsafe_template());
                };
                if matches!(escaped, '\n' | '\r') {
                    return Err(unsafe_template());
                }
                word.push(escaped);
                in_word = true;
                index += escaped.len_utf8();
            }
            _ => {
                word.push(character);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err(unsafe_template());
    }
    if in_word {
        argv.push(word);
    }
    if argv.is_empty() {
        return Err(unsafe_template());
    }
    const INTERPRETERS: &[&str] = &[
        "sh",
        "bash",
        "dash",
        "zsh",
        "fish",
        "cmd",
        "cmd.exe",
        "powershell",
        "powershell.exe",
        "pwsh",
        "eval",
        "call",
        "env",
    ];
    if argv.iter().any(|arg| {
        let basename = std::path::Path::new(arg)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(arg)
            .to_ascii_lowercase();
        INTERPRETERS.contains(&basename.as_str())
    }) {
        return Err(unsafe_template());
    }
    Ok(argv)
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

/// Builds the unspawned platform-shell [`Command`] for `cmd`.
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

/// Replaces every placeholder in `cmd`, then `trim_end`s the whole result.
///
/// The *display* form: what the envelope reports as the command that ran.
/// Every placeholder must be substituted, or the report would show a literal
/// token where one resolved.
///
/// The `trim_end` runs after substitution and over the *whole* string, not
/// just around the placeholder, so an empty value (the `--full` case, or
/// `[gherkin]` being off) leaves no trailing whitespace where the
/// placeholder used to sit: `"cargo test {filter}"` with an empty filter
/// becomes `"cargo test"`, not `"cargo test "`.
pub fn substitute_placeholders(cmd: &str, subs: &Substitutions<'_>) -> String {
    cmd.replace("{filter}", subs.filter)
        .replace("{features}", subs.features)
        .replace("{scenario}", subs.scenario)
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod filter_rewrite_tests {
    use super::{Substitutions, parse_runner_template, validate_filter_data};

    #[test]
    fn direct_runner_preserves_composition_in_every_quote_context() {
        assert_eq!(
            parse_runner_template(
                "runner module::{filter}_case \"double::{filter}\" 'single::{filter}'",
                &Substitutions {
                    filter: "proof;still-data",
                    ..Default::default()
                }
            )
            .unwrap(),
            [
                "runner",
                "module::proof;still-data_case",
                "double::proof;still-data",
                "single::proof;still-data",
            ]
        );
    }

    #[test]
    fn an_empty_trailing_filter_does_not_create_an_empty_argument() {
        assert_eq!(
            parse_runner_template(
                "git hash-object {filter}",
                &Substitutions {
                    filter: "",
                    ..Default::default()
                }
            )
            .unwrap(),
            ["git", "hash-object"]
        );
        assert_eq!(
            parse_runner_template(
                "runner prefix-{filter}-suffix",
                &Substitutions {
                    filter: "",
                    ..Default::default()
                }
            )
            .unwrap(),
            ["runner", "prefix--suffix"]
        );
        assert_eq!(
            parse_runner_template(
                "runner \"{filter}\"",
                &Substitutions {
                    filter: "",
                    ..Default::default()
                }
            )
            .unwrap(),
            ["runner", ""]
        );
    }

    #[test]
    fn nested_interpretation_is_rejected() {
        for template in [
            "sh -c 'runner {filter}'",
            "cmd /C runner {filter}",
            "eval runner {filter}",
            "runner $(({filter}))",
            "runner $(echo {filter})",
            "runner `echo {filter}`",
            "runner {filter} && second",
        ] {
            assert!(
                parse_runner_template(
                    template,
                    &Substitutions {
                        filter: "proof",
                        ..Default::default()
                    }
                )
                .is_err(),
                "accepted {template}"
            );
        }
    }

    #[test]
    fn filter_controls_are_rejected_but_quotes_are_plain_data() {
        for filter in ["line\nbreak", "line\rbreak", "nul\0byte"] {
            assert!(validate_filter_data(filter).is_err());
        }
        assert_eq!(
            parse_runner_template(
                "runner {filter}",
                &Substitutions {
                    filter: "proof\"'literal",
                    ..Default::default()
                }
            )
            .unwrap(),
            ["runner", "proof\"'literal"]
        );
    }
}
