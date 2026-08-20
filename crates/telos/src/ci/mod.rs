//! Continuous-integration integrations installed by `telos init`.

pub mod github;

use std::path::Path;

use clap::ValueEnum;
use telos_core::error::TelosError;

/// The CI providers Telos can install during initialization.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CiProvider {
    Github,
}

/// Every requested CI integration, fully planned before init writes anything.
pub enum InstallPlan {
    None,
    Github(github::InstallPlan),
}

/// Reads and validates every requested CI integration before init writes
/// anything.
pub fn preflight(root: &Path, provider: Option<CiProvider>) -> Result<InstallPlan, TelosError> {
    match provider {
        Some(CiProvider::Github) => github::preflight(root).map(InstallPlan::Github),
        None => Ok(InstallPlan::None),
    }
}

/// Installs every requested CI integration after Telos has been sealed.
pub fn render(plan: &InstallPlan) -> Result<(), TelosError> {
    match plan {
        InstallPlan::None => Ok(()),
        InstallPlan::Github(plan) => plan.render(),
    }
}
