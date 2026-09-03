//! Cross-OS command execution: the platform shell `check` runs through, and
//! the proof cycle `[test] cmd` runs under -- `{filter}`/`{report}`
//! substitution, stale-report removal, and the report read-back.

use std::path::Path;
use std::process::Command;

use crate::config::TestCfg;
use crate::error::{ErrorCode, TelosError};
use crate::ids::{RepoPath, ScenarioId};
use crate::model::Evidence;
use crate::repo_fs::RepoFs;
use crate::report::{NotExecuted, Report, ReportVerdict};

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
    filter: &str,
    report: &str,
) -> Result<Vec<String>, TelosError> {
    const FILTER: &str = "{filter}";
    const REPORT: &str = "{report}";
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

        if rest.starts_with(FILTER) {
            if !filter.is_empty() {
                word.push_str(filter);
                in_word = true;
            }
            index += FILTER.len();
            continue;
        }

        if rest.starts_with(REPORT) {
            if !report.is_empty() {
                word.push_str(report);
                in_word = true;
            }
            index += REPORT.len();
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

/// Replaces every `{filter}` and `{report}` in `cmd` with `filter` and
/// `report` respectively, then `trim_end`s the whole result.
///
/// The `trim_end` runs after substitution and over the *whole* string, not
/// just around the placeholders, so an empty filter (the `--full` case) or
/// an unconfigured report leaves no trailing whitespace where the
/// placeholder used to sit: `"cargo test {filter}"` with an empty filter
/// becomes `"cargo test"`, not `"cargo test "`.
pub fn substitute_placeholders(cmd: &str, filter: &str, report: &str) -> String {
    cmd.replace("{filter}", filter)
        .replace("{report}", report)
        .trim_end()
        .to_string()
}

/// One execution of `[test] cmd` for one filter: the frozen display
/// command, the raw exit status, and the evidence the configuration asked
/// for. Built by [`run_proof`]; read through [`ProofRun::verdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRun {
    /// The template with `{filter}` and `{report}` literally substituted --
    /// diagnostic display, not a shell-replay contract.
    pub command: String,
    /// The runner's exit status (`-1` when killed by a signal).
    pub status: i32,
    pub evidence: ProofEvidence,
}

/// What a run left behind to judge it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofEvidence {
    /// No `[test] report`: the exit status is all there is.
    ExitStatus,
    /// `[test] report` is configured: the report read after the run, or the
    /// reason it could not be.
    Report {
        path: RepoPath,
        parsed: Result<Report, NotExecuted>,
    },
}

/// A run's verdict for one scenario. `executed` is the number of testcases
/// named after the scenario that ran (passed plus failed) under report
/// evidence, `None` under exit-status evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofVerdict {
    Green { executed: Option<u32> },
    Red { executed: Option<u32> },
    NotExecuted(NotExecuted),
}

impl ProofRun {
    /// Which kind of evidence this run carries.
    pub fn kind(&self) -> Evidence {
        match self.evidence {
            ProofEvidence::ExitStatus => Evidence::ExitStatus,
            ProofEvidence::Report { .. } => Evidence::Report,
        }
    }

    /// The configured report path, when there is one.
    pub fn report_path(&self) -> Option<&RepoPath> {
        match &self.evidence {
            ProofEvidence::ExitStatus => None,
            ProofEvidence::Report { path, .. } => Some(path),
        }
    }

    /// The verdict for `scenario`. With a report the exit status is
    /// diagnostic only: a runner that exits 1 over an unrelated failure is
    /// still green when the scenario's testcase passed, and one that exits
    /// 0 having run nothing is not executed.
    pub fn verdict(&self, scenario: ScenarioId) -> ProofVerdict {
        match &self.evidence {
            ProofEvidence::ExitStatus if self.status == 0 => ProofVerdict::Green { executed: None },
            ProofEvidence::ExitStatus => ProofVerdict::Red { executed: None },
            ProofEvidence::Report {
                parsed: Err(reason),
                ..
            } => ProofVerdict::NotExecuted(reason.clone()),
            ProofEvidence::Report {
                parsed: Ok(report), ..
            } => match report.verdict(scenario) {
                ReportVerdict::Passed { passed } => ProofVerdict::Green {
                    executed: Some(passed),
                },
                ReportVerdict::Failed { passed, failed } => ProofVerdict::Red {
                    executed: Some(passed + failed),
                },
                ReportVerdict::NotExecuted(reason) => ProofVerdict::NotExecuted(reason),
            },
        }
    }
}

/// Runs `[test] cmd` once for `filter` under the delete-run-read cycle:
/// a stale report is removed first so nothing can be read from a previous
/// run, the template runs as a direct argv, and the report (when one is
/// configured) is read back. The runner not writing it is evidence in its
/// own right (`NotExecuted::ReportMissing`), never an error here.
///
/// The template is parsed with `cmd` as configured; callers have already
/// refused an empty one. A report path that fails validation or a stale
/// report that cannot be removed are `TELOS_PARSE_ERROR` / `TELOS_INTERNAL`
/// respectively, before anything runs.
pub fn run_proof(test: &TestCfg, filter: &str, repo_root: &Path) -> Result<ProofRun, TelosError> {
    let report = test.report_path()?;
    let report_str = report.as_ref().map(RepoPath::as_str).unwrap_or("");
    let command = substitute_placeholders(&test.cmd, filter, report_str);
    validate_filter_data(filter)?;
    let argv = parse_runner_template(&test.cmd, filter, report_str)?;

    let report_file = report.as_ref().map(|path| absolute(repo_root, path));
    if let Some(path) = &report {
        remove_stale_report(repo_root, path)?;
    }

    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(repo_root)
        .output()
        .map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to spawn the test runner displayed as `{command}`: {e}"),
            )
        })?;
    let status = run_result(output).status;

    let evidence = match (report, report_file) {
        (Some(path), Some(file)) => ProofEvidence::Report {
            path,
            parsed: read_report(&file),
        },
        _ => ProofEvidence::ExitStatus,
    };
    Ok(ProofRun {
        command,
        status,
        evidence,
    })
}

/// `repo_root` joined with a validated repository path, component by
/// component, so the OS separator is never guessed from the `/` form.
fn absolute(repo_root: &Path, path: &RepoPath) -> std::path::PathBuf {
    let mut absolute = repo_root.to_path_buf();
    for component in path.as_str().split('/') {
        absolute.push(component);
    }
    absolute
}

/// Removes a stale report through [`RepoFs`], the same capability-anchored,
/// NotFound-tolerant path every other repository write uses -- it refuses to
/// walk through a symlinked parent. `RepoFs::remove_file` reports a refusal
/// as `TELOS_INTEGRITY_VIOLATION`; here that refusal is folded into the
/// spec's own `TELOS_INTERNAL` naming the report path, since a report the
/// runner cannot write to is an environment problem, not an integrity one.
fn remove_stale_report(repo_root: &Path, path: &RepoPath) -> Result<(), TelosError> {
    RepoFs::open(repo_root)?.remove_file(path).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to remove the stale report {path}: {}", e.message),
        )
    })
}

fn read_report(file: &Path) -> Result<Report, NotExecuted> {
    match std::fs::read_to_string(file) {
        Ok(xml) => Report::parse(&xml).map_err(NotExecuted::ReportInvalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(NotExecuted::ReportMissing),
        Err(e) => Err(NotExecuted::ReportInvalid(e.to_string())),
    }
}

#[cfg(test)]
mod filter_rewrite_tests {
    use super::{parse_runner_template, substitute_placeholders, validate_filter_data};

    #[test]
    fn direct_runner_preserves_composition_in_every_quote_context() {
        assert_eq!(
            parse_runner_template(
                "runner module::{filter}_case \"double::{filter}\" 'single::{filter}'",
                "proof;still-data",
                "",
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
            parse_runner_template("git hash-object {filter}", "", "").unwrap(),
            ["git", "hash-object"]
        );
        assert_eq!(
            parse_runner_template("runner prefix-{filter}-suffix", "", "").unwrap(),
            ["runner", "prefix--suffix"]
        );
        assert_eq!(
            parse_runner_template("runner \"{filter}\"", "", "").unwrap(),
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
                parse_runner_template(template, "proof", "").is_err(),
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
            parse_runner_template("runner {filter}", "proof\"'literal", "").unwrap(),
            ["runner", "proof\"'literal"]
        );
    }

    #[test]
    fn the_report_placeholder_is_data_in_every_quote_context() {
        assert_eq!(
            parse_runner_template(
                "runner --junit={report} \"{report}\" {filter}",
                "scn_0108_x",
                "target/telos report.xml",
            )
            .unwrap(),
            [
                "runner",
                "--junit=target/telos report.xml",
                "target/telos report.xml",
                "scn_0108_x",
            ]
        );
    }

    #[test]
    fn an_empty_report_creates_no_empty_argument() {
        assert_eq!(
            parse_runner_template("runner {report} {filter}", "scn_0108_x", "").unwrap(),
            ["runner", "scn_0108_x"]
        );
    }

    #[test]
    fn substitution_displays_both_placeholders_literally() {
        assert_eq!(
            substitute_placeholders("runner {report} {filter}", "", "out.xml"),
            "runner out.xml"
        );
    }
}

#[cfg(test)]
mod run_proof_tests {
    use super::*;

    fn report_run(parsed: Result<Report, NotExecuted>, status: i32) -> ProofRun {
        ProofRun {
            command: "runner".to_string(),
            status,
            evidence: ProofEvidence::Report {
                path: RepoPath::new("telos-report.xml"),
                parsed,
            },
        }
    }

    #[test]
    fn exit_status_evidence_reads_zero_as_green_and_anything_else_as_red() {
        let green = ProofRun {
            command: "runner".to_string(),
            status: 0,
            evidence: ProofEvidence::ExitStatus,
        };
        let red = ProofRun {
            status: 3,
            ..green.clone()
        };
        assert_eq!(green.kind(), Evidence::ExitStatus);
        assert_eq!(
            green.verdict(ScenarioId(1)),
            ProofVerdict::Green { executed: None }
        );
        assert_eq!(
            red.verdict(ScenarioId(1)),
            ProofVerdict::Red { executed: None }
        );
    }

    #[test]
    fn report_evidence_outranks_the_exit_status() {
        let passed =
            Report::parse(r#"<testsuite><testcase name="scn_0001_x"/></testsuite>"#).unwrap();
        let failed = Report::parse(
            r#"<testsuite><testcase name="scn_0001_x"><failure/></testcase><testcase name="scn_0001_y"/></testsuite>"#,
        )
        .unwrap();
        assert_eq!(
            report_run(Ok(passed), 1).verdict(ScenarioId(1)),
            ProofVerdict::Green { executed: Some(1) }
        );
        assert_eq!(
            report_run(Ok(failed), 0).verdict(ScenarioId(1)),
            ProofVerdict::Red { executed: Some(2) }
        );
        assert_eq!(
            report_run(Err(NotExecuted::ReportMissing), 0).verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportMissing)
        );
        assert_eq!(
            report_run(Err(NotExecuted::ReportMissing), 0).kind(),
            Evidence::Report
        );
    }

    #[test]
    fn without_a_report_run_proof_reads_the_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let test = TestCfg {
            cmd: "git --version".to_string(),
            report: String::new(),
        };
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert_eq!(run.command, "git --version");
        assert_eq!(run.status, 0);
        assert_eq!(run.evidence, ProofEvidence::ExitStatus);
    }

    #[test]
    fn a_stale_report_is_deleted_before_the_run_and_its_absence_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("telos-report.xml"), "<testsuite/>").unwrap();
        let test = TestCfg {
            cmd: "git --version".to_string(),
            report: "telos-report.xml".to_string(),
        };
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert!(!tmp.path().join("telos-report.xml").exists());
        assert_eq!(
            run.verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportMissing)
        );
        assert_eq!(run.report_path(), Some(&RepoPath::new("telos-report.xml")));
    }

    #[cfg(unix)]
    fn writer(dir: &std::path::Path, body: &str) -> TestCfg {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("writer");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s' '{body}' > \"$1\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        TestCfg {
            cmd: "./writer {report} {filter}".to_string(),
            report: "out/report.xml".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_report_the_runner_writes_is_read_after_the_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("out")).unwrap();
        let test = writer(
            tmp.path(),
            r#"<testsuite><testcase name="scn_0001_x"/></testsuite>"#,
        );
        let run = run_proof(&test, "scn_0001_x", tmp.path()).unwrap();
        assert_eq!(run.command, "./writer out/report.xml scn_0001_x");
        assert_eq!(
            run.verdict(ScenarioId(1)),
            ProofVerdict::Green { executed: Some(1) }
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unparseable_report_is_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("out")).unwrap();
        let test = writer(tmp.path(), "<testsuite><testcase");
        let run = run_proof(&test, "", tmp.path()).unwrap();
        assert!(matches!(
            run.verdict(ScenarioId(1)),
            ProofVerdict::NotExecuted(NotExecuted::ReportInvalid(_))
        ));
    }
}
