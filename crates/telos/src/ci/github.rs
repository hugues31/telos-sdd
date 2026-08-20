//! The GitHub Actions sealed-state gate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use telos_core::error::{ErrorCode, TelosError};

use crate::view::export::publish_no_replace;

const WORKFLOW_PATH: &str = ".github/workflows/telos.yml";
const GITHUB_DIR: &str = ".github";
const WORKFLOWS_DIR: &str = ".github/workflows";

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

/// The fully validated immutable GitHub installation.
pub struct InstallPlan {
    root: PathBuf,
    workflow: PathBuf,
}

pub fn preflight(root: &Path) -> Result<InstallPlan, TelosError> {
    validate_ancestors(root)?;
    let workflow = workflow_path(root);
    match fs::symlink_metadata(&workflow) {
        Ok(_) => Err(collision(&workflow)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(InstallPlan {
            root: root.to_path_buf(),
            workflow,
        }),
        Err(error) => Err(io_error("inspect", &workflow, error)),
    }
}

impl InstallPlan {
    pub fn render(&self) -> Result<(), TelosError> {
        self.render_with_before_publish(|| Ok(()))
    }

    fn render_with_before_publish<H>(&self, before_publish: H) -> Result<(), TelosError>
    where
        H: FnOnce() -> io::Result<()>,
    {
        ensure_ancestors(&self.root)?;
        let staging = staging_path(&self.workflow)?;
        let result = (|| {
            fs::write(&staging, WORKFLOW).map_err(|error| io_error("write", &staging, error))?;
            before_publish()
                .map_err(|error| io_error("prepare publication for", &self.workflow, error))?;
            publish_no_replace(&staging, &self.workflow, collision)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    }
}

fn validate_ancestors(root: &Path) -> Result<(), TelosError> {
    for relative in [GITHUB_DIR, WORKFLOWS_DIR] {
        validate_directory(&root.join(relative), relative)?;
    }
    Ok(())
}

fn ensure_ancestors(root: &Path) -> Result<(), TelosError> {
    for relative in [GITHUB_DIR, WORKFLOWS_DIR] {
        let path = root.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(directory_collision(relative)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        directory_collision(relative)
                    } else {
                        io_error("create", &path, error)
                    }
                })?;
            }
            Err(error) => return Err(io_error("inspect", &path, error)),
        }
        let canonical_root =
            fs::canonicalize(root).map_err(|error| io_error("resolve", root, error))?;
        let canonical =
            fs::canonicalize(&path).map_err(|error| io_error("resolve", &path, error))?;
        if !canonical.starts_with(canonical_root) {
            return Err(directory_collision(relative));
        }
    }
    Ok(())
}

fn validate_directory(path: &Path, relative: &str) -> Result<(), TelosError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(directory_collision(relative)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect", path, error)),
    }
}

fn staging_path(workflow: &Path) -> Result<PathBuf, TelosError> {
    let parent = workflow.parent().expect("workflow path has a parent");
    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".telos.yml.staging-{}-{sequence}",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create", &candidate, error)),
        }
    }
    Err(TelosError::new(
        ErrorCode::TelosInternal,
        format!(
            "failed to create unique workflow staging file beside {}",
            workflow.display()
        ),
    ))
}

fn workflow_path(root: &Path) -> PathBuf {
    root.join(WORKFLOW_PATH)
}

fn collision(_path: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{WORKFLOW_PATH}` already exists"),
    )
    .hint("preserve or move the existing workflow before retrying")
}

fn directory_collision(relative: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{relative}` must be a real directory"),
    )
    .hint("replace the existing path with a real directory before retrying")
}

fn io_error(verb: &str, path: &Path, error: io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Barrier};

    use super::{collision, preflight};

    #[test]
    fn publish_race_preserves_the_late_workflow_owner_and_cleans_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow = tmp.path().join(".github/workflows/telos.yml");
        let plan = preflight(tmp.path()).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let actor_barrier = Arc::clone(&barrier);
        let actor_workflow = workflow.clone();
        let actor = std::thread::spawn(move || {
            actor_barrier.wait();
            fs::create_dir_all(actor_workflow.parent().unwrap())?;
            fs::write(&actor_workflow, "late owner\n")
        });

        let error = plan
            .render_with_before_publish(|| {
                barrier.wait();
                actor
                    .join()
                    .map_err(|_| std::io::Error::other("publication actor panicked"))?
            })
            .unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(error.message, collision(&workflow).message);
        assert_eq!(fs::read_to_string(&workflow).unwrap(), "late owner\n");
        assert!(
            fs::read_dir(workflow.parent().unwrap())
                .unwrap()
                .all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".telos.yml.staging-")
                })
        );
    }

    #[cfg(unix)]
    #[test]
    fn publish_race_preserves_a_late_workflow_symlink_owner() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let workflow = tmp.path().join(".github/workflows/telos.yml");
        let owner = tmp.path().join("outside-owner.yml");
        fs::write(&owner, "outside owner\n").unwrap();
        let plan = preflight(tmp.path()).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let actor_barrier = Arc::clone(&barrier);
        let actor_workflow = workflow.clone();
        let actor_owner = owner.clone();
        let actor = std::thread::spawn(move || {
            actor_barrier.wait();
            fs::create_dir_all(actor_workflow.parent().unwrap())?;
            symlink(actor_owner, actor_workflow)
        });

        let error = plan
            .render_with_before_publish(|| {
                barrier.wait();
                actor
                    .join()
                    .map_err(|_| std::io::Error::other("publication actor panicked"))?
            })
            .unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(error.message, collision(&workflow).message);
        assert!(
            fs::symlink_metadata(&workflow)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(owner).unwrap(), "outside owner\n");
    }
}
