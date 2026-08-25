//! `telos.toml`: the workspace configuration file.
//!
//! Every section is optional. A missing file, an empty file, or a file that
//! only sets some sections all yield the same thing for whatever is left
//! unset: empty globs, an empty test command, and the strict TDD policy.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, TelosError};

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
    /// `[gherkin] enabled = true` -- generate sealed `.feature` files.
    #[serde(default)]
    pub gherkin: GherkinCfg,
    /// `[agents]`: normalized host metadata. Host files are managed by init.
    #[serde(default)]
    pub agents: AgentsCfg,
}

/// `[gherkin]`: whether reconcile generates sealed Cucumber `.feature` files
/// under `telos/features/`.
///
/// Off unless asked for. The directory is deliberately not configurable: the
/// write phase reads the *effective* config, so that enabling generation
/// takes effect in the reconcile that approves it -- and a `dir` a change
/// could also move would then mean pruning one tree while writing another in
/// the same transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GherkinCfg {
    #[serde(default)]
    pub enabled: bool,
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
    pub fn validate_self(&self) -> Result<(), TelosError> {
        compile_globs(&self.code.globs)?;
        compile_globs(&self.tests.globs)?;
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

/// `[test]`: the command used to run the test suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TestCfg {
    #[serde(default)]
    pub cmd: String,
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

    #[test]
    fn empty_toml_yields_every_default() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.policy.tdd, TddPolicy::Strict);
        assert!(config.code.globs.is_empty());
        assert!(config.tests.globs.is_empty());
        assert_eq!(config.test.cmd, "");
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

            [policy]
            tdd = "advisory"
        "#;
        let config: Config = toml::from_str(src).unwrap();
        assert_eq!(config.code.globs, vec!["src/**/*.rs".to_string()]);
        assert_eq!(config.tests.globs, vec!["tests/**/*.rs".to_string()]);
        assert_eq!(config.test.cmd, "cargo test {filter}");
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
    fn gherkin_generation_is_off_unless_asked_for() {
        let config: Config = toml::from_str("").unwrap();
        assert!(
            !config.gherkin.enabled,
            "a project that never mentions gherkin must not generate features"
        );

        let config: Config = toml::from_str("[gherkin]\nenabled = true\n").unwrap();
        assert!(config.gherkin.enabled);
    }

    #[test]
    fn gherkin_round_trips_through_the_canonical_emitter() {
        let config: Config = toml::from_str("[gherkin]\nenabled = true\n").unwrap();
        let emitted = emit(&config).unwrap();
        assert!(
            emitted.contains("[gherkin]\nenabled = true\n"),
            "emitted config: {emitted}"
        );
        assert_eq!(toml::from_str::<Config>(&emitted).unwrap(), config);
    }
}
