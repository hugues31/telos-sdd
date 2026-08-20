//! The GitHub Actions sealed-state gate.

use std::io;
use std::path::Path;

use telos_core::error::{ErrorCode, TelosError};

use crate::safe_fs::SafeRoot;

const WORKFLOW_PATH: &str = ".github/workflows/telos.yml";
const GITHUB_DIR: &str = ".github";
const WORKFLOWS_DIR: &str = ".github/workflows";

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
    root: SafeRoot,
    publish: bool,
}

pub fn preflight(root: &Path) -> Result<InstallPlan, TelosError> {
    preflight_with_resume(root, false)
}

pub fn preflight_resume(root: &Path) -> Result<InstallPlan, TelosError> {
    preflight_with_resume(root, true)
}

fn preflight_with_resume(root: &Path, resume: bool) -> Result<InstallPlan, TelosError> {
    let root = SafeRoot::open(root)
        .map_err(|error| io_error("open", Path::new("repository root"), error))?;
    validate_ancestors(&root)?;
    match root.exists_no_follow(Path::new(WORKFLOW_PATH)) {
        Ok(true) if resume => match root.read_optional(Path::new(WORKFLOW_PATH)) {
            Ok(Some(bytes)) if bytes == WORKFLOW.as_bytes() => Ok(InstallPlan {
                root,
                publish: false,
            }),
            Ok(_) | Err(_) => Err(collision(Path::new(WORKFLOW_PATH))),
        },
        Ok(true) => Err(collision(Path::new(WORKFLOW_PATH))),
        Ok(false) => Ok(InstallPlan {
            root,
            publish: true,
        }),
        Err(error) => Err(io_error("inspect", Path::new(WORKFLOW_PATH), error)),
    }
}

impl InstallPlan {
    pub fn render(&self) -> Result<(), TelosError> {
        self.render_with_writer_and_before_publish(std::io::Write::write_all, || Ok(()))
    }

    #[cfg(test)]
    fn render_with_before_write<H>(&self, before_write: H) -> Result<(), TelosError>
    where
        H: FnOnce() -> io::Result<()>,
    {
        self.render_with_writer_and_before_publish(std::io::Write::write_all, before_write)
    }

    fn render_with_writer_and_before_publish<F, H>(
        &self,
        write: F,
        before_publish: H,
    ) -> Result<(), TelosError>
    where
        F: FnOnce(&mut cap_std::fs::File, &[u8]) -> io::Result<()>,
        H: FnOnce() -> io::Result<()>,
    {
        if !self.publish {
            return match self.root.read_optional(Path::new(WORKFLOW_PATH)) {
                Ok(Some(bytes)) if bytes == WORKFLOW.as_bytes() => Ok(()),
                Ok(_) | Err(_) => Err(collision(Path::new(WORKFLOW_PATH))),
            };
        }
        self.root
            .create_new_write_with(
                Path::new(WORKFLOW_PATH),
                WORKFLOW.as_bytes(),
                write,
                before_publish,
            )
            .map_err(render_error)
    }
}

fn validate_ancestors(root: &SafeRoot) -> Result<(), TelosError> {
    for relative in [GITHUB_DIR, WORKFLOWS_DIR] {
        root.validate_directory(Path::new(relative))
            .map_err(|_| directory_collision(relative))?;
    }
    Ok(())
}

fn collision(_path: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{WORKFLOW_PATH}` already exists"),
    )
    .hint("preserve or move the existing workflow before retrying")
}

fn render_error(error: io::Error) -> TelosError {
    match error.kind() {
        io::ErrorKind::AlreadyExists => collision(Path::new(WORKFLOW_PATH)),
        io::ErrorKind::InvalidInput
        | io::ErrorKind::NotADirectory
        | io::ErrorKind::PermissionDenied => directory_collision(WORKFLOWS_DIR),
        _ => io_error("write", Path::new(WORKFLOW_PATH), error),
    }
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
    use std::io::{self, Write};
    use std::sync::{Arc, Barrier};

    use super::{WORKFLOW, WORKFLOW_PATH, collision, preflight, preflight_resume};

    #[test]
    fn exact_workflow_is_a_noop_only_during_init_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let first = preflight(tmp.path()).unwrap();
        first.render().unwrap();

        let fresh_error = match preflight(tmp.path()) {
            Ok(_) => panic!("fresh init accepted an existing workflow"),
            Err(error) => error,
        };
        assert_eq!(
            fresh_error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );

        let resumed = preflight_resume(tmp.path()).unwrap();
        resumed.render().unwrap();
        assert_eq!(
            fs::read(tmp.path().join(WORKFLOW_PATH)).unwrap(),
            WORKFLOW.as_bytes()
        );
    }

    #[test]
    fn resumed_exact_workflow_changed_after_preflight_is_not_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let first = preflight(tmp.path()).unwrap();
        first.render().unwrap();
        let resumed = preflight_resume(tmp.path()).unwrap();
        let workflow = tmp.path().join(WORKFLOW_PATH);
        fs::write(&workflow, b"late workflow owner\n").unwrap();

        let error = resumed.render().unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(fs::read(workflow).unwrap(), b"late workflow owner\n");
    }

    #[test]
    fn partial_staging_write_never_creates_the_final_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow = tmp.path().join(WORKFLOW_PATH);
        let plan = preflight(tmp.path()).unwrap();

        let error = plan
            .render_with_writer_and_before_publish(
                |file, bytes| {
                    file.write_all(&bytes[..17])?;
                    Err(io::Error::other("forced partial CI staging write"))
                },
                || Ok(()),
            )
            .unwrap_err();

        assert!(error.message.contains("forced partial CI staging write"));
        assert!(!workflow.exists());
        assert!(
            fs::read_dir(workflow.parent().unwrap())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".telos-staging-"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_staging_write_does_not_remove_a_late_staging_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow = tmp.path().join(WORKFLOW_PATH);
        let parent = workflow.parent().unwrap().to_path_buf();
        let plan = preflight(tmp.path()).unwrap();
        let mut late_owner = None;

        plan.render_with_writer_and_before_publish(
            |file, bytes| {
                file.write_all(&bytes[..17])?;
                let staging = fs::read_dir(&parent)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<io::Result<Vec<_>>>()?
                    .into_iter()
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .contains(".telos-staging-")
                    })
                    .unwrap();
                fs::rename(&staging, parent.join("moved-original-staging"))?;
                fs::write(&staging, b"late staging owner\n")?;
                late_owner = Some(staging);
                Err(io::Error::other("forced staging failure after replacement"))
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(!workflow.exists());
        assert_eq!(
            fs::read(late_owner.unwrap()).unwrap(),
            b"late staging owner\n"
        );
    }

    #[test]
    fn reserved_final_write_does_not_follow_a_late_parent_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow = tmp.path().join(".github/workflows/telos.yml");
        let outside = tempfile::tempdir().unwrap();
        let github = tmp.path().join(".github");
        let plan = preflight(tmp.path()).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let actor_barrier = Arc::clone(&barrier);
        let actor_github = github.clone();
        let actor_outside = outside.path().to_path_buf();
        let actor = std::thread::spawn(move || {
            actor_barrier.wait();
            fs::rename(&actor_github, actor_github.with_extension("owned"))?;
            #[cfg(unix)]
            return std::os::unix::fs::symlink(actor_outside, actor_github);
            #[cfg(not(unix))]
            return fs::create_dir(actor_github);
        });

        let error = plan
            .render_with_before_write(|| {
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
        assert!(!outside.path().join("workflows/telos.yml").exists());
        assert!(!workflow.exists());
    }

    #[cfg(unix)]
    #[test]
    fn final_create_new_preserves_a_late_workflow_symlink_owner() {
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
            .render_with_before_write(|| {
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
