//! `telos.toml`: the workspace configuration file.
//!
//! Every section is optional. A missing file, an empty file, or a file that
//! only sets some sections all yield the same thing for whatever is left
//! unset: empty globs, an empty test command, and the strict TDD policy.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::model::Evidence;

/// Agent integrations selected when the project was initialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentHost {
    Claude,
    Codex,
}

/// Sorts and removes duplicate hosts before configuration is persisted.
pub fn normalize_hosts(hosts: &mut Vec<AgentHost>) {
    hosts.sort();
    hosts.dedup();
}

/// Parsed `telos.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    /// `[code] globs = [...]` -- source file patterns eligible for sealing.
    #[serde(default)]
    pub code: Globs,
    /// `[tests] globs = [...]` -- test file patterns eligible for sealing.
    #[serde(default)]
    pub tests: Globs,
    /// `[test] cmd = "..."` -- how to run the test suite.
    #[serde(default)]
    pub test: TestCfg,
    /// `[policy] tdd = "strict" | "advisory"`.
    #[serde(default)]
    pub policy: Policy,
    /// `[agents]`: normalized host metadata. Host files are managed by init.
    #[serde(default)]
    pub agents: AgentsCfg,
}

/// `[agents]`: the host integrations initially installed for this project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentsCfg {
    #[serde(default)]
    pub hosts: Vec<AgentHost>,
}

impl Config {
    /// Canonicalize configuration values that are represented as sets.
    pub fn normalize(&mut self) {
        normalize_hosts(&mut self.agents.hosts);
        self.code.globs.sort();
        self.code.globs.dedup();
        self.tests.globs.sort();
        self.tests.globs.dedup();
    }

    /// Validates values whose syntax is only meaningful at runtime.
    ///
    /// Deserialization validates the TOML/JSON shape and enum values; glob
    /// compilation lives here so staging, approval, reconcile, and full
    /// resealing all use the exact matching semantics used by the walker.
    /// The `[test]` section is validated here too: the report path and the
    /// `{report}` placeholder rule.
    pub fn validate_self(&self) -> Result<(), TelosError> {
        compile_globs(&self.code.globs)?;
        compile_globs(&self.tests.globs)?;
        validate_test_cfg(&self.test)?;
        Ok(())
    }

    /// Validates an approved configuration transition.
    ///
    /// Agent hosts describe artifacts installed by `telos init`; config
    /// changes may not silently add or remove those artifacts. Both sides are
    /// normalized before comparison so harmless order/duplicates never look
    /// like a host lifecycle change.
    pub fn validate_transition(base: &Config, effective: &Config) -> Result<(), TelosError> {
        effective.validate_self()?;

        let mut base_hosts = base.agents.hosts.clone();
        let mut effective_hosts = effective.agents.hosts.clone();
        normalize_hosts(&mut base_hosts);
        normalize_hosts(&mut effective_hosts);
        if base_hosts != effective_hosts {
            return Err(TelosError::new(
                ErrorCode::TelosIntegrityViolation,
                "agents.hosts is managed by `telos init --agents` and cannot be changed by `telos config`",
            ));
        }
        Ok(())
    }
}

/// Compiles glob patterns with the runtime walker's path-component semantics.
pub(crate) fn compile_globs(patterns: &[String]) -> Result<GlobSet, TelosError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| {
                TelosError::new(
                    ErrorCode::TelosParseError,
                    format!("invalid glob pattern `{pattern}`: {error}"),
                )
            })?;
        builder.add(glob);
    }
    builder.build().map_err(|error| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("invalid glob pattern(s): {error}"),
        )
    })
}

/// Emits canonical TOML for a configuration value.
pub fn emit(config: &Config) -> Result<String, crate::error::TelosError> {
    crate::emit::emit_config(config)
}

/// A named list of glob patterns (`[code]` or `[tests]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Globs {
    #[serde(default)]
    pub globs: Vec<String>,
}

/// `[test]`: the command used to run the test suite, and the JUnit XML
/// report it writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TestCfg {
    #[serde(default)]
    pub cmd: String,
    /// `[test] report = "..."` -- the repository-relative path of the JUnit
    /// XML report the runner writes. Empty means no report: verdicts are
    /// read from the exit status alone.
    #[serde(default)]
    pub report: String,
}

impl TestCfg {
    /// The configured report as a validated repository path, `None` when
    /// unset. Validity is the code-path rule: normalized, `/`-separated,
    /// nothing under `telos/`.
    pub fn report_path(&self) -> Result<Option<RepoPath>, TelosError> {
        if self.report.is_empty() {
            return Ok(None);
        }
        let path = RepoPath::parse(self.report.clone())?;
        if path.first_component() == Some("telos") {
            return Err(TelosError::new(
                ErrorCode::TelosParseError,
                format!(
                    "invalid [test] report: `{}` is under the spec tree",
                    self.report
                ),
            )
            .hint("write the report outside telos/, e.g. `target/telos-report.xml`"));
        }
        Ok(Some(path))
    }

    /// The kind of evidence runs under this configuration produce.
    pub fn evidence(&self) -> Evidence {
        if self.report.is_empty() {
            Evidence::ExitStatus
        } else {
            Evidence::Report
        }
    }
}

/// `[test] report` must be a code path, and `{report}` in `cmd` requires it.
pub(crate) fn validate_test_cfg(test: &TestCfg) -> Result<(), TelosError> {
    if test.report_path()?.is_none() && test.cmd.contains("{report}") {
        return Err(TelosError::new(
            ErrorCode::TelosParseError,
            "invalid [test] cmd: `{report}` is used but `[test] report` is not configured",
        )
        .hint("set [test] report to the repository-relative path the runner writes its JUnit XML report to"));
    }
    Ok(())
}

/// `[policy]`: process-level policy switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Policy {
    #[serde(default)]
    pub tdd: TddPolicy,
}

/// Whether TDD is enforced (`strict`, the default) or merely suggested
/// (`advisory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TddPolicy {
    #[default]
    Strict,
    Advisory,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RepoPath;
    use crate::model::Evidence;

    #[test]
    fn empty_toml_yields_every_default() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.policy.tdd, TddPolicy::Strict);
        assert!(config.code.globs.is_empty());
        assert!(config.tests.globs.is_empty());
        assert_eq!(config.test.cmd, "");
        assert_eq!(config.test.report, "");
    }

    #[test]
    fn full_toml_round_trips_every_field() {
        let src = r#"
            [code]
            globs = ["src/**/*.rs"]

            [tests]
            globs = ["tests/**/*.rs"]

            [test]
            cmd = "cargo test {filter}"
            report = "target/telos-report.xml"

            [policy]
            tdd = "advisory"
        "#;
        let config: Config = toml::from_str(src).unwrap();
        assert_eq!(config.code.globs, vec!["src/**/*.rs".to_string()]);
        assert_eq!(config.tests.globs, vec!["tests/**/*.rs".to_string()]);
        assert_eq!(config.test.cmd, "cargo test {filter}");
        assert_eq!(config.test.report, "target/telos-report.xml");
        assert_eq!(config.policy.tdd, TddPolicy::Advisory);
    }

    #[test]
    fn partial_toml_defaults_the_missing_sections() {
        let config: Config = toml::from_str("[policy]\ntdd = \"advisory\"\n").unwrap();
        assert_eq!(config.policy.tdd, TddPolicy::Advisory);
        assert!(config.code.globs.is_empty());
        assert_eq!(config.test.cmd, "");
    }

    #[test]
    fn validate_self_uses_runtime_glob_semantics_for_both_families() {
        for mut config in [
            Config {
                code: Globs {
                    globs: vec!["[".to_string()],
                },
                ..Config::default()
            },
            Config {
                tests: Globs {
                    globs: vec!["[".to_string()],
                },
                ..Config::default()
            },
        ] {
            config.normalize();
            let error = config.validate_self().unwrap_err();
            assert_eq!(error.code, ErrorCode::TelosParseError);
            assert!(error.message.contains("invalid glob pattern `[`"));
        }
    }

    #[test]
    fn validate_transition_compares_normalized_hosts_and_rejects_real_changes() {
        let base = Config {
            agents: AgentsCfg {
                hosts: vec![AgentHost::Codex, AgentHost::Claude],
            },
            ..Config::default()
        };
        let reordered = Config {
            agents: AgentsCfg {
                hosts: vec![AgentHost::Claude, AgentHost::Codex, AgentHost::Codex],
            },
            ..Config::default()
        };
        Config::validate_transition(&base, &reordered).unwrap();

        let removed = Config {
            agents: AgentsCfg {
                hosts: vec![AgentHost::Claude],
            },
            ..Config::default()
        };
        let error = Config::validate_transition(&base, &removed).unwrap_err();
        assert_eq!(error.code, ErrorCode::TelosIntegrityViolation);
    }

    #[test]
    fn a_report_under_the_spec_tree_is_refused() {
        let config = Config {
            test: TestCfg {
                cmd: "runner {filter}".to_string(),
                report: "telos/report.xml".to_string(),
            },
            ..Config::default()
        };
        let error = config.validate_self().unwrap_err();
        assert_eq!(error.code, ErrorCode::TelosParseError);
        assert_eq!(
            error.message,
            "invalid [test] report: `telos/report.xml` is under the spec tree"
        );
        assert_eq!(
            error.hint.as_deref(),
            Some("write the report outside telos/, e.g. `target/telos-report.xml`")
        );
    }

    #[test]
    fn a_report_placeholder_without_a_report_is_refused() {
        let config = Config {
            test: TestCfg {
                cmd: "runner --junit {report} {filter}".to_string(),
                report: String::new(),
            },
            ..Config::default()
        };
        let error = config.validate_self().unwrap_err();
        assert_eq!(error.code, ErrorCode::TelosParseError);
        assert_eq!(
            error.message,
            "invalid [test] cmd: `{report}` is used but `[test] report` is not configured"
        );
        assert_eq!(
            error.hint.as_deref(),
            Some(
                "set [test] report to the repository-relative path the runner writes its JUnit XML report to"
            )
        );
    }

    #[test]
    fn report_path_and_evidence_follow_the_report_field() {
        let unset = TestCfg::default();
        assert_eq!(unset.report_path().unwrap(), None);
        assert_eq!(unset.evidence(), Evidence::ExitStatus);

        let set = TestCfg {
            cmd: String::new(),
            report: "target/telos-report.xml".to_string(),
        };
        assert_eq!(
            set.report_path().unwrap(),
            Some(RepoPath::new("target/telos-report.xml"))
        );
        assert_eq!(set.evidence(), Evidence::Report);
        assert!(
            TestCfg {
                cmd: String::new(),
                report: "../escape.xml".to_string()
            }
            .report_path()
            .is_err()
        );
    }
}
