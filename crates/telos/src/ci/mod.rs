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

/// Validates every requested CI integration before init writes anything.
pub fn preflight(root: &Path, provider: Option<CiProvider>) -> Result<(), TelosError> {
    match provider {
        Some(CiProvider::Github) => github::preflight(root),
        None => Ok(()),
    }
}

/// Installs every requested CI integration after Telos has been sealed.
pub fn render(root: &Path, provider: Option<CiProvider>) -> Result<(), TelosError> {
    match provider {
        Some(CiProvider::Github) => github::render(root),
        None => Ok(()),
    }
}
