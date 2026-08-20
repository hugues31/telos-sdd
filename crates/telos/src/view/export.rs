//! Atomic static export of a rendered [`ViewSnapshot`].
//!
//! The exporter prepares every page in memory, writes only into a freshly
//! created sibling staging directory, and publishes it with one rename.  A
//! destination is therefore all-or-nothing and is never overwritten.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::IntentId;

use super::html::{LinkMode, Page, render};
use super::model::ViewSnapshot;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Renders and atomically publishes every static page below `destination`.
/// The returned paths are relative to `destination` and lexicographically
/// sorted, making both the answer and an exported tree deterministic.
pub(crate) fn export(
    snapshot: &ViewSnapshot,
    destination: &Path,
) -> Result<Vec<PathBuf>, TelosError> {
    export_with_writer(snapshot, destination, |path, bytes| fs::write(path, bytes))
}

fn export_with_writer<F>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    mut write_file: F,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &[u8]) -> io::Result<()>,
{
    refuse_existing(destination)?;
    let rendered = rendered_files(snapshot)?;
    let staging = staging_directory(destination)?;

    let result = (|| {
        for (relative, bytes) in &rendered {
            let path = staging.join(relative);
            let parent = path.parent().expect("export paths always have a parent");
            fs::create_dir_all(parent).map_err(|error| io_error("create", parent, error))?;
            write_file(&path, bytes).map_err(|error| io_error("write", &path, error))?;
        }

        // A second check prevents the ordinary, already-existing case from
        // ever reaching `rename`, whose platform behavior may overwrite.
        refuse_existing(destination)?;
        fs::rename(&staging, destination)
            .map_err(|error| io_error("rename", destination, error))?;
        Ok(rendered.into_iter().map(|(path, _)| path).collect())
    })();

    if result.is_err() {
        // Staging is wholly exporter-owned: it was created with `create_dir`
        // under a unique name, so cleanup cannot touch caller data.
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn rendered_files(snapshot: &ViewSnapshot) -> Result<Vec<(PathBuf, Vec<u8>)>, TelosError> {
    let mut pages = vec![
        (PathBuf::from("index.html"), Page::Dashboard),
        (PathBuf::from("graph.html"), Page::Graph),
        (PathBuf::from("glossary.html"), Page::Glossary),
        (PathBuf::from("coverage.html"), Page::Coverage),
    ];
    for intent in &snapshot.intents {
        let id: IntentId = intent.id.parse().map_err(|_| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("snapshot contains an invalid intent id `{}`", intent.id),
            )
        })?;
        pages.push((
            PathBuf::from(format!("intents/{id}.html")),
            Page::Intent(id),
        ));
    }
    pages.sort_by(|left, right| left.0.cmp(&right.0));

    pages
        .into_iter()
        .map(|(path, page)| {
            let bytes = render(snapshot, page, LinkMode::Export)
                .ok_or_else(|| {
                    TelosError::new(
                        ErrorCode::TelosInternal,
                        format!("snapshot cannot render {}", path.display()),
                    )
                })?
                .into_bytes();
            Ok((path, bytes))
        })
        .collect()
}

fn refuse_existing(destination: &Path) -> Result<(), TelosError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            format!(
                "export destination `{}` already exists",
                destination.display()
            ),
        )
        .hint("choose an empty path that does not exist")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect", destination, error)),
    }
}

fn staging_directory(destination: &Path) -> Result<PathBuf, TelosError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let prefix = staging_prefix(destination);

    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!("{prefix}{}-{sequence}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create", &candidate, error)),
        }
    }

    Err(TelosError::new(
        ErrorCode::TelosInternal,
        format!(
            "failed to create unique export staging directory beside {}",
            destination.display()
        ),
    ))
}

fn staging_prefix(destination: &Path) -> String {
    let name = destination
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("telos-export"));
    format!(".{}.telos-staging-", name.to_string_lossy())
}

fn io_error(verb: &str, path: &Path, error: io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::{export_with_writer, staging_prefix};
    use crate::view::model::ViewSnapshot;

    fn fixture_snapshot() -> ViewSnapshot {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        let workspace = Workspace::discover(&root).unwrap();
        let model = workspace.load_model().unwrap();
        ViewSnapshot::build(
            &StateReport {
                state: ProjectStateKind::Coherent,
                drift: vec![],
                open_changes: vec![],
            },
            &model,
        )
    }

    #[test]
    fn a_staging_write_failure_leaves_no_destination_or_staging_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let error = export_with_writer(&fixture_snapshot(), &destination, |_path, _bytes| {
            Err(io::Error::other("forced staging write failure"))
        })
        .unwrap_err();

        assert!(error.message.contains("forced staging write failure"));
        assert!(!destination.exists());
        let prefix = staging_prefix(&destination);
        assert!(std::fs::read_dir(temporary.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        }));
    }
}
