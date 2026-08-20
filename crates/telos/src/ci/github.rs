//! The GitHub Actions sealed-state gate.

use std::fs;
use std::path::{Path, PathBuf};

use telos_core::error::{ErrorCode, TelosError};

const WORKFLOW_PATH: &str = ".github/workflows/telos.yml";

const WORKFLOW: &str = concat!(
    "name: Telos\n\n",
    "on:\n",
    "  pull_request:\n",
    "  push:\n",
    "    branches: [main]\n\n",
    "permissions:\n",
    "  contents: read\n\n",
    "jobs:\n",
    "  sealed:\n",
    "    runs-on: ubuntu-latest\n",
    "    steps:\n",
    "      - uses: actions/checkout@v7\n",
    "      - uses: dtolnay/rust-toolchain@stable\n",
    "      - name: Install Telos v",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "        run: cargo install --git https://github.com/hugues31/telos-sdd --tag v",
    env!("CARGO_PKG_VERSION"),
    " --locked telos\n",
    "      - name: Verify sealed Telos state\n",
    "        run: telos check --sealed\n",
);

pub fn preflight(root: &Path) -> Result<(), TelosError> {
    let path = workflow_path(root);
    match fs::symlink_metadata(&path) {
        Ok(_) => Err(collision()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect", &path, error)),
    }
}

pub fn render(root: &Path) -> Result<(), TelosError> {
    let path = workflow_path(root);
    let parent = path.parent().expect("workflow path has a parent");
    fs::create_dir_all(parent).map_err(|error| io_error("create", parent, error))?;
    fs::write(&path, WORKFLOW).map_err(|error| io_error("write", &path, error))
}

fn workflow_path(root: &Path) -> PathBuf {
    root.join(WORKFLOW_PATH)
}

fn collision() -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{WORKFLOW_PATH}` already exists"),
    )
    .hint("preserve or move the existing workflow before retrying")
}

fn io_error(verb: &str, path: &Path, error: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {error}", path.display()),
    )
}
