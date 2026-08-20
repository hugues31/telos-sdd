//! `telos.toml`: the workspace configuration file.
//!
//! Every section is optional. A missing file, an empty file, or a file that
//! only sets some sections all yield the same thing for whatever is left
//! unset: empty globs, an empty test command, and the strict TDD policy.

use serde::{Deserialize, Serialize};

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
}
