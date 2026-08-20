//! Atomic static export of a rendered [`ViewSnapshot`].
//!
//! The exporter prepares every page in memory, writes only into a freshly
//! created sibling staging directory, and publishes it with one rename.  A
//! destination is therefore all-or-nothing and is never overwritten.
//!
//! A capability and `(device, inode)` identity close deterministic or
//! accidental staging-name substitution. There is no portable source-side
//! compare-and-swap rename, so a same-UID adversary that can continuously race
//! the final identity check remains outside this guarantee.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::IntentId;

use super::html::{LinkMode, Page, render};
use super::model::ViewSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EntryIdentity {
    device: u64,
    inode: u64,
}

struct StagingDirectory {
    parent: Dir,
    parent_path: PathBuf,
    target: OsString,
    name: OsString,
    path: PathBuf,
    dir: Option<Dir>,
    identity: EntryIdentity,
    published: bool,
}

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
        |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
        |_staging| Ok(()),
    )
}

#[cfg(test)]
fn export_with_writer<F>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    write_file: F,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &Dir, &Path, &[u8]) -> io::Result<()>,
{
    export_with_writer_and_before_publish(snapshot, destination, write_file, |_staging| Ok(()))
}

fn export_with_writer_and_before_publish<F, H>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    mut write_file: F,
    before_publish: H,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &Dir, &Path, &[u8]) -> io::Result<()>,
    H: FnOnce(&Path) -> io::Result<()>,
{
    refuse_existing(destination)?;
    let rendered = rendered_files(snapshot)?;
    let mut staging = StagingDirectory::reserve(destination)?;

    (|| {
        for (relative, bytes) in &rendered {
            let path = staging.path().join(relative);
            write_file(staging.path(), staging.dir(), relative, bytes)
                .map_err(|error| io_error("write", &path, error))?;
        }

        // This makes the common already-existing case cheap.  Publication
        // still uses a no-replace primitive below: another actor can create
        // the destination between this check and that syscall.
        staging.refuse_existing_destination(destination)?;
        before_publish(staging.path())
            .map_err(|error| io_error("prepare publication for", destination, error))?;
        staging.refuse_existing_destination(destination)?;
        staging.verify_entry()?;
        publish_no_replace(&staging, destination, existing_destination)?;
        staging.published = true;
        Ok(rendered.into_iter().map(|(path, _)| path).collect())
    })()
}

fn write_relative(staging: &Dir, relative: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent_path = relative
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no name"))?;
    let parent = open_or_create_directory(staging, parent_path)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = parent.open_with(name, &options)?;
    file.write_all(bytes)
}

fn open_or_create_directory(root: &Dir, relative: &Path) -> io::Result<Dir> {
    let mut current = root.open_dir(".")?;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe export path",
            ));
        };
        match current.open_dir_nofollow(component) {
            Ok(next) => current = next,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                current.create_dir(component)?;
                current = current.open_dir_nofollow(component)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(current)
}

impl StagingDirectory {
    fn reserve(destination: &Path) -> Result<Self, TelosError> {
        let parent_path = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let target = destination.file_name().ok_or_else(|| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!(
                    "export destination has no final path component: {}",
                    destination.display()
                ),
            )
        })?;
        let parent = Dir::open_ambient_dir(parent_path, ambient_authority())
            .map_err(|error| io_error("open parent of", destination, error))?;

        for _ in 0..128 {
            let name = random_staging_name(destination)?;
            match parent.create_dir(&name) {
                Ok(()) => {
                    let dir = parent
                        .open_dir_nofollow(&name)
                        .map_err(|error| io_error("open staging beside", destination, error))?;
                    let identity =
                        identity(&dir.metadata(".").map_err(|error| {
                            io_error("inspect staging beside", destination, error)
                        })?);
                    let staging = Self {
                        parent,
                        parent_path: parent_path.to_path_buf(),
                        target: target.to_os_string(),
                        path: parent_path.join(&name),
                        name,
                        dir: Some(dir),
                        identity,
                        published: false,
                    };
                    staging.verify_entry()?;
                    staging.refuse_existing_destination(destination)?;
                    return Ok(staging);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create staging beside", destination, error)),
            }
        }

        Err(TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "failed to reserve a unique export staging directory beside {}",
                destination.display()
            ),
        ))
    }

    fn path(&self) -> &Path {
        // Constructed once by `reserve`; retaining it is only for diagnostics,
        // test barriers, and Windows' path-only MoveFileW API. All writes and
        // Unix publication are relative to the held handles.
        &self.path
    }

    fn dir(&self) -> &Dir {
        self.dir.as_ref().expect("staging handle remains open")
    }

    fn verify_entry(&self) -> Result<(), TelosError> {
        let entry = self.parent.open_dir_nofollow(&self.name).map_err(|error| {
            stale_staging(
                &self.parent_path.join(&self.name),
                format!("cannot reopen the reserved entry: {error}"),
            )
        })?;
        let entry_identity = identity(&entry.metadata(".").map_err(|error| {
            stale_staging(
                &self.path,
                format!("cannot inspect the directory entry: {error}"),
            )
        })?);
        let handle_identity = identity(&self.dir().metadata(".").map_err(|error| {
            stale_staging(
                &self.path,
                format!("cannot inspect the held directory: {error}"),
            )
        })?);
        if entry_identity != self.identity || handle_identity != self.identity {
            return Err(stale_staging(
                &self.parent_path.join(&self.name),
                "the directory entry no longer names the reserved staging directory",
            ));
        }
        Ok(())
    }

    fn refuse_existing_destination(&self, destination: &Path) -> Result<(), TelosError> {
        match self.parent.symlink_metadata(&self.target) {
            Ok(_) => Err(existing_destination(destination)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("inspect", destination, error)),
        }
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.published || self.verify_entry().is_err() {
            return;
        }
        if let Some(dir) = self.dir.take() {
            let _ = dir.remove_open_dir_all();
        }
    }
}

fn identity(metadata: &cap_std::fs::Metadata) -> EntryIdentity {
    EntryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn random_staging_name(destination: &Path) -> Result<OsString, TelosError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to generate an export staging name: {error}"),
        )
    })?;
    let mut suffix = String::with_capacity(entropy.len() * 2);
    for byte in entropy {
        write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(OsString::from(format!(
        "{}{}",
        staging_prefix(destination),
        suffix
    )))
}

fn stale_staging(path: &Path, reason: impl std::fmt::Display) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!(
            "refusing stale export staging entry {}: {reason}",
            path.display()
        ),
    )
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
fn publish_no_replace(
    staging: &StagingDirectory,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let staging_name = CString::new(staging.name.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export staging path contains a NUL byte: {}",
                staging.path().display()
            ),
        )
    })?;
    let destination_name = CString::new(staging.target.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export destination path contains a NUL byte: {}",
                destination.display()
            ),
        )
    })?;

    // SAFETY: both `CString`s are NUL-terminated and remain alive for this
    // call. Both relative names are resolved beneath the already-held parent
    // directory capability.
    let result = unsafe {
        libc::renameat2(
            staging.parent.as_raw_fd(),
            staging_name.as_ptr(),
            staging.parent.as_raw_fd(),
            destination_name.as_ptr(),
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
fn publish_no_replace(
    staging: &StagingDirectory,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let staging_name = CString::new(staging.name.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export staging path contains a NUL byte: {}",
                staging.path().display()
            ),
        )
    })?;
    let destination_name = CString::new(staging.target.as_os_str().as_bytes()).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "export destination path contains a NUL byte: {}",
                destination.display()
            ),
        )
    })?;

    // SAFETY: both path buffers are valid C strings and remain alive for the
    // duration of the Darwin no-replace rename syscall. Both names are
    // resolved beneath the already-held parent directory capability.
    let result = unsafe {
        libc::renameatx_np(
            staging.parent.as_raw_fd(),
            staging_name.as_ptr(),
            staging.parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
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

/// Windows' `MoveFileW` has no replacement flag: it fails when the target
/// exists, so the staging directory cannot overwrite a concurrent owner.
#[cfg(windows)]
fn publish_no_replace(
    staging: &StagingDirectory,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let staging_path = staging.path();
    let staging = wide_path(staging_path)?;
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
fn publish_no_replace(
    _staging: &StagingDirectory,
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
        staging_prefix, write_relative,
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
        let error = export_with_writer(
            &fixture_snapshot(),
            &destination,
            |_staging_path, _staging, _relative, _bytes| {
                Err(io::Error::other("forced staging write failure"))
            },
        )
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
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_staging| {
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
    fn a_substituted_staging_entry_is_never_published() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("displaced-staging");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_before_publish(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |staging| {
                let staging = staging.to_path_buf();
                fs::rename(&staging, &displaced)?;
                fs::create_dir(&staging)?;
                fs::write(staging.join("hostile.txt"), "not the export")?;
                *replacement.lock().unwrap() = Some(staging);
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert!(!destination.exists());
        assert!(displaced.join("index.html").is_file());
        let replacement = replacement.into_inner().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(replacement.join("hostile.txt")).unwrap(),
            "not the export"
        );
    }

    #[test]
    fn failed_publication_never_cleans_up_a_substituted_staging_entry() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("displaced-staging");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_before_publish(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |staging| {
                let staging = staging.to_path_buf();
                fs::rename(&staging, &displaced)?;
                fs::create_dir(&staging)?;
                fs::write(staging.join("hostile.txt"), "replacement owner")?;
                *replacement.lock().unwrap() = Some(staging);
                fs::create_dir(&destination)?;
                fs::write(destination.join("owner.txt"), "destination owner")
            },
        )
        .unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(
            fs::read_to_string(destination.join("owner.txt")).unwrap(),
            "destination owner"
        );
        let replacement = replacement.into_inner().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(replacement.join("hostile.txt")).unwrap(),
            "replacement owner"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_staging_symlink_substituted_before_writes_never_receives_export_bytes() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("displaced-staging");
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let substituted = std::sync::Mutex::new(false);

        let error = export_with_writer(
            &fixture_snapshot(),
            &destination,
            |staging_path, staging, relative, bytes| {
                let mut substituted = substituted.lock().unwrap();
                if !*substituted {
                    fs::rename(staging_path, &displaced)?;
                    symlink(&outside, staging_path)?;
                    *substituted = true;
                }
                write_relative(staging, relative, bytes)
            },
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert!(!destination.exists());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        assert!(displaced.join("index.html").is_file());
    }

    #[test]
    fn utf16_paths_reject_an_embedded_nul_before_the_terminator() {
        let path = PathBuf::from("site");
        let error = nul_terminated_wide([b's' as u16, 0, b't' as u16], &path).unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert_eq!(error.message, "export path contains a NUL code unit: site");
    }
}
