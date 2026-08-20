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
    export_with_writer_and_before_publish(
        snapshot,
        destination,
        |path, bytes| fs::write(path, bytes),
        || Ok(()),
    )
}

#[cfg(test)]
fn export_with_writer<F>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    write_file: F,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &[u8]) -> io::Result<()>,
{
    export_with_writer_and_before_publish(snapshot, destination, write_file, || Ok(()))
}

fn export_with_writer_and_before_publish<F, H>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    mut write_file: F,
    before_publish: H,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &[u8]) -> io::Result<()>,
    H: FnOnce() -> io::Result<()>,
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

        // This makes the common already-existing case cheap.  Publication
        // still uses a no-replace primitive below: another actor can create
        // the destination between this check and that syscall.
        refuse_existing(destination)?;
        before_publish()
            .map_err(|error| io_error("prepare publication for", destination, error))?;
        publish_no_replace(&staging, destination, existing_destination)?;
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
        Ok(_) => Err(existing_destination(destination)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect", destination, error)),
    }
}

fn existing_destination(destination: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!(
            "export destination `{}` already exists",
            destination.display()
        ),
    )
    .hint("choose an empty path that does not exist")
}

/// Atomically promotes `staging` only when `destination` does not exist.
///
/// A check followed by `std::fs::rename` is not safe: POSIX rename may
/// replace a directory another process created in between.  Linux's
/// `renameat2(RENAME_NOREPLACE)` gives this operation one kernel boundary.
/// Platforms without an equivalent primitive fail closed rather than falling
/// back to a replacement-capable rename.
#[cfg(target_os = "linux")]
pub(crate) fn publish_no_replace(
    staging: &Path,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let staging = CString::new(staging.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export staging path contains a NUL byte: {}",
                staging.display()
            ),
        )
    })?;
    let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export destination path contains a NUL byte: {}",
                destination.display()
            ),
        )
    })?;

    // SAFETY: both `CString`s are NUL-terminated and remain alive for this
    // call. `AT_FDCWD` asks the kernel to resolve the supplied paths exactly
    // as the surrounding filesystem calls do.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            staging.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        return Err(existing(destination));
    }
    Err(io_error("publish", destination, error))
}

/// Darwin's `renamex_np(RENAME_EXCL)` has the same no-replacement guarantee
/// as Linux's `renameat2(RENAME_NOREPLACE)`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn publish_no_replace(
    staging: &Path,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let staging = CString::new(staging.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export staging path contains a NUL byte: {}",
                staging.display()
            ),
        )
    })?;
    let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export destination path contains a NUL byte: {}",
                destination.display()
            ),
        )
    })?;

    // SAFETY: both path buffers are valid C strings and remain alive for the
    // duration of the Darwin no-replace rename syscall.
    let result =
        unsafe { libc::renamex_np(staging.as_ptr(), destination_c.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        return Err(existing(destination));
    }
    Err(io_error("publish", destination, error))
}

/// Windows' `MoveFileW` has no replacement flag: it fails when the target
/// exists, so the staging directory cannot overwrite a concurrent owner.
#[cfg(windows)]
pub(crate) fn publish_no_replace(
    staging: &Path,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let staging = wide_path(staging)?;
    let destination_wide = wide_path(destination)?;

    // SAFETY: the buffers are NUL-terminated UTF-16 paths and remain alive
    // for the duration of the Win32 call.
    let result = unsafe { MoveFileW(staging.as_ptr(), destination_wide.as_ptr()) };
    if result != 0 {
        return Ok(());
    }

    // SAFETY: `MoveFileW` just failed on this thread, so the Win32 last-error
    // value identifies that call's failure.
    let error = unsafe { GetLastError() };
    if error == ERROR_ALREADY_EXISTS || error == ERROR_FILE_EXISTS {
        return Err(existing(destination));
    }
    Err(io_error(
        "publish",
        destination,
        io::Error::from_raw_os_error(error as i32),
    ))
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Result<Vec<u16>, TelosError> {
    use std::os::windows::ffi::OsStrExt;

    nul_terminated_wide(path.as_os_str().encode_wide(), path)
}

#[cfg(any(windows, test))]
fn nul_terminated_wide(
    units: impl IntoIterator<Item = u16>,
    path: &Path,
) -> Result<Vec<u16>, TelosError> {
    let mut wide: Vec<u16> = units.into_iter().collect();
    if wide.contains(&0) {
        return Err(TelosError::new(
            ErrorCode::TelosInternal,
            format!("export path contains a NUL code unit: {}", path.display()),
        ));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios", windows)))]
pub(crate) fn publish_no_replace(
    _staging: &Path,
    destination: &Path,
    _existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    Err(TelosError::new(
        ErrorCode::TelosInternal,
        format!(
            "atomic no-replace export publication is unsupported on this platform: {}",
            destination.display()
        ),
    ))
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
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::{
        export_with_writer, export_with_writer_and_before_publish, nul_terminated_wide,
        staging_prefix,
    };
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

    #[test]
    fn publication_race_preserves_a_destination_created_after_the_last_check() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let snapshot = fixture_snapshot();
        let barrier = Arc::new(Barrier::new(2));
        let actor_barrier = Arc::clone(&barrier);
        let actor_destination = destination.clone();
        let actor = std::thread::spawn(move || {
            actor_barrier.wait();
            fs::create_dir(&actor_destination)?;
            fs::write(actor_destination.join("owner.txt"), "concurrent owner")
        });

        let error = export_with_writer_and_before_publish(
            &snapshot,
            &destination,
            |path, bytes| fs::write(path, bytes),
            || {
                barrier.wait();
                actor
                    .join()
                    .map_err(|_| io::Error::other("publication actor panicked"))?
            },
        )
        .unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(
            error.message,
            format!(
                "export destination `{}` already exists",
                destination.display()
            )
        );
        assert_eq!(
            fs::read_to_string(destination.join("owner.txt")).unwrap(),
            "concurrent owner"
        );
        let prefix = staging_prefix(&destination);
        assert!(fs::read_dir(temporary.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        }));
    }

    #[test]
    fn utf16_paths_reject_an_embedded_nul_before_the_terminator() {
        let path = PathBuf::from("site");
        let error = nul_terminated_wide([b's' as u16, 0, b't' as u16], &path).unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert_eq!(error.message, "export path contains a NUL code unit: site");
    }
}
