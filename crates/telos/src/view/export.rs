//! Atomic static export of a rendered [`ViewSnapshot`].
//!
//! Every page is rendered in memory, written through a held capability into a
//! high-entropy sibling staging directory, and validated before one atomic,
//! no-replace publication. The final destination is therefore absent on
//! failure and never exposes a partially written tree.
//!
//! `mkdir` does not return the directory it creates, and Unix/Darwin rename
//! directories by a parent capability plus source name rather than by source
//! handle. Telos records the new entry's identity immediately after creation,
//! reopens it no-follow, and checks it again after the publication test hook.
//! Together with an unpredictable name this covers the negligence/concurrency
//! model; a same-UID adversary observing and swapping the name inside an
//! unhooked syscall interval is explicitly outside that model.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
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
    parent_identity: EntryIdentity,
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
    export_with_writer_and_hooks(
        snapshot,
        destination,
        |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
        |_destination| Ok(()),
        |_destination| Ok(()),
        |_destination| Ok(()),
        |_destination| Ok(()),
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
    export_with_writer_and_hooks(
        snapshot,
        destination,
        write_file,
        |_destination| Ok(()),
        |_destination| Ok(()),
        |_destination| Ok(()),
        |_destination| Ok(()),
    )
}

#[cfg(test)]
fn export_with_writer_and_before_publish<F, H>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    write_file: F,
    before_publish: H,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &Dir, &Path, &[u8]) -> io::Result<()>,
    H: FnOnce(&Path) -> io::Result<()>,
{
    export_with_writer_and_hooks(
        snapshot,
        destination,
        write_file,
        |_destination| Ok(()),
        |_destination| Ok(()),
        before_publish,
        |_destination| Ok(()),
    )
}

fn export_with_writer_and_hooks<F, B, R, H, J>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    mut write_file: F,
    before_reserve: B,
    after_staging_create: R,
    before_publish: H,
    after_identity_check: J,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &Dir, &Path, &[u8]) -> io::Result<()>,
    B: FnOnce(&Path) -> io::Result<()>,
    R: FnOnce(&Path) -> io::Result<()>,
    H: FnOnce(&Path) -> io::Result<()>,
    J: FnOnce(&Path) -> io::Result<()>,
{
    before_reserve(destination)
        .map_err(|error| io_error("prepare destination reservation for", destination, error))?;
    refuse_existing(destination)?;
    let rendered = rendered_files(snapshot)?;
    let mut staging = StagingDirectory::reserve_with_hook(destination, after_staging_create)?;

    (|| {
        for (relative, bytes) in &rendered {
            let path = staging.path().join(relative);
            write_file(staging.path(), staging.dir(), relative, bytes)
                .map_err(|error| io_error("write", &path, error))?;
        }

        staging.validate_contents(&rendered)?;
        before_publish(staging.path())
            .map_err(|error| io_error("finalize staging for", destination, error))?;
        staging.refuse_existing_destination(destination)?;
        staging.verify_entry()?;
        after_identity_check(staging.path())
            .map_err(|error| io_error("finish staging verification for", destination, error))?;
        staging.refuse_existing_destination(destination)?;
        staging.verify_entry()?;
        staging.verify_announced_parent(destination)?;
        publish_no_replace(&mut staging, destination, existing_destination)?;
        staging.published = true;
        Ok(rendered.into_iter().map(|(path, _)| path).collect())
    })()
}

fn write_relative(destination: &Dir, relative: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new_relative(destination, relative)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn create_new_relative(destination: &Dir, relative: &Path) -> io::Result<cap_std::fs::File> {
    let parent_path = relative
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no name"))?;
    let parent = open_or_create_directory(destination, parent_path)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    parent.open_with(name, &options)
}

fn open_or_create_directory(root: &Dir, relative: &Path) -> io::Result<Dir> {
    open_directory(root, relative, true)
}

fn open_directory(root: &Dir, relative: &Path, create: bool) -> io::Result<Dir> {
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
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                current.create_dir(component)?;
                current = current.open_dir_nofollow(component)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(current)
}

fn read_relative(root: &Dir, relative: &Path) -> io::Result<Vec<u8>> {
    let parent_path = relative
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no name"))?;
    let parent = open_directory(root, parent_path, false)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export entry is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl StagingDirectory {
    fn reserve_with_hook<H>(destination: &Path, after_create: H) -> Result<Self, TelosError>
    where
        H: FnOnce(&Path) -> io::Result<()>,
    {
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
        let parent = open_ambient_directory_nofollow(parent_path)
            .map_err(|error| io_error("open parent of", destination, error))?;
        let parent_identity = identity(
            &parent
                .dir_metadata()
                .map_err(|error| io_error("inspect parent of", destination, error))?,
        );

        for _ in 0..128 {
            let name = random_staging_name(destination)?;
            match parent.create_dir(&name) {
                Ok(()) => {
                    let path = parent_path.join(&name);
                    let created_metadata = parent.symlink_metadata(&name).map_err(|error| {
                        io_error("inspect created staging beside", destination, error)
                    })?;
                    if !created_metadata.is_dir() || created_metadata.file_type().is_symlink() {
                        return Err(stale_staging(
                            &path,
                            "the newly created entry is not a real directory",
                        ));
                    }
                    let created_identity = identity(&created_metadata);

                    after_create(&path).map_err(|error| {
                        io_error("finish staging reservation for", destination, error)
                    })?;
                    let dir = parent.open_dir_nofollow(&name).map_err(|error| {
                        stale_staging(&path, format!("cannot open it no-follow: {error}"))
                    })?;
                    let opened_identity = identity(
                        &dir.dir_metadata()
                            .map_err(|error| stale_staging(&path, error))?,
                    );
                    if opened_identity != created_identity {
                        return Err(stale_staging(
                            &path,
                            "the entry changed between creation and no-follow open",
                        ));
                    }

                    let staging = Self {
                        parent,
                        parent_path: parent_path.to_path_buf(),
                        parent_identity,
                        target: target.to_os_string(),
                        name,
                        path,
                        dir: Some(dir),
                        identity: created_identity,
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
        &self.path
    }

    fn dir(&self) -> &Dir {
        self.dir.as_ref().expect("staging capability remains open")
    }

    fn verify_entry(&self) -> Result<(), TelosError> {
        let entry = self.parent.open_dir_nofollow(&self.name).map_err(|error| {
            stale_staging(
                &self.parent_path.join(&self.name),
                format!("cannot reopen the reserved entry: {error}"),
            )
        })?;
        let entry_identity = identity(&entry.dir_metadata().map_err(|error| {
            stale_staging(
                &self.path,
                format!("cannot inspect the directory entry: {error}"),
            )
        })?);
        let handle_identity = identity(&self.dir().dir_metadata().map_err(|error| {
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

    fn validate_contents(&self, rendered: &[(PathBuf, Vec<u8>)]) -> Result<(), TelosError> {
        let mut actual = Vec::new();
        collect_files(self.dir(), Path::new(""), &mut actual)
            .map_err(|error| io_error("inspect staged export", &self.path, error))?;
        actual.sort();
        let mut expected: Vec<_> = rendered.iter().map(|(path, _)| path.clone()).collect();
        expected.sort();
        if actual != expected {
            return Err(stale_staging(
                &self.path,
                "its contents changed before publication",
            ));
        }
        for (relative, bytes) in rendered {
            let actual = read_relative(self.dir(), relative)
                .map_err(|error| io_error("verify staging", &self.path.join(relative), error))?;
            if actual != *bytes {
                return Err(stale_staging(
                    &self.path.join(relative),
                    "its bytes changed before publication",
                ));
            }
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

    fn verify_announced_parent(&self, destination: &Path) -> Result<(), TelosError> {
        let reopened = open_ambient_directory_nofollow(&self.parent_path)
            .map_err(|error| stale_parent(destination, error))?;
        let current = identity(
            &reopened
                .dir_metadata()
                .map_err(|error| stale_parent(destination, error))?,
        );
        let held = identity(
            &self
                .parent
                .dir_metadata()
                .map_err(|error| stale_parent(destination, error))?,
        );
        if current == self.parent_identity && held == self.parent_identity {
            Ok(())
        } else {
            Err(stale_parent(
                destination,
                "the announced destination parent changed identity",
            ))
        }
    }
}

fn open_ambient_directory_nofollow(path: &Path) -> io::Result<Dir> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "destination parent escapes its filesystem root",
                    ));
                }
            }
            std::path::Component::Normal(name) => normalized.push(name),
        }
    }

    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for component in normalized.components() {
        match component {
            std::path::Component::Prefix(prefix) => anchor.push(prefix.as_os_str()),
            std::path::Component::RootDir => anchor.push(component.as_os_str()),
            std::path::Component::Normal(name) => names.push(name.to_os_string()),
            std::path::Component::CurDir | std::path::Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination parent is not normalized",
                ));
            }
        }
    }
    let mut current = Dir::open_ambient_dir(&anchor, ambient_authority())?;
    for name in names {
        current = current.open_dir_nofollow(name)?;
    }
    Ok(current)
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.published || self.dir.is_none() || self.verify_entry().is_err() {
            return;
        }
        let Some(dir) = self.dir.take() else {
            return;
        };
        #[cfg(windows)]
        {
            let _ = cleanup_windows_owned_directory(dir, &self.path, self.identity);
        }
        #[cfg(not(windows))]
        {
            let _ = dir.remove_open_dir_all();
        }
    }
}

fn collect_files(directory: &Dir, prefix: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in directory.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let relative = prefix.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let child = directory.open_dir_nofollow(&name)?;
            collect_files(&child, &relative, paths)?;
        } else if file_type.is_file() {
            paths.push(relative);
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "export contains a non-file, non-directory entry",
            ));
        }
    }
    Ok(())
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

fn staging_prefix(destination: &Path) -> String {
    let name = destination
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("telos-export"));
    format!(".{}.telos-staging-", name.to_string_lossy())
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

fn stale_parent(destination: &Path, reason: impl std::fmt::Display) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!(
            "export destination parent changed before publication for `{}`: {reason}",
            destination.display()
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

/// Atomically promotes `staging` only when `destination` does not exist.
///
/// Linux exposes no directory-rename-by-handle primitive. Both names are
/// resolved below the retained parent capability, and the caller verifies the
/// recorded staging identity immediately before this no-replace syscall.
#[cfg(target_os = "linux")]
fn publish_no_replace(
    staging: &mut StagingDirectory,
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

    // SAFETY: both C strings remain alive for the call, and both names are
    // resolved relative to the retained destination-parent capability.
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

/// Darwin's exclusive rename has the same destination no-replace contract.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn publish_no_replace(
    staging: &mut StagingDirectory,
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

    // SAFETY: both C strings remain alive for the call, and both names are
    // resolved relative to the retained destination-parent capability.
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

/// Windows renames the staging directory object referenced by a DELETE-capable
/// handle. The ordinary capability is closed first because cap-std opens
/// directories without `FILE_SHARE_DELETE`; the replacement-resistant handle
/// is then opened no-follow and checked against the recorded identity.
#[cfg(windows)]
fn publish_no_replace(
    staging: &mut StagingDirectory,
    destination: &Path,
    existing: fn(&Path) -> TelosError,
) -> Result<(), TelosError> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};

    let dir = staging
        .dir
        .take()
        .expect("staging capability remains open before publication");
    drop(dir);

    let handle =
        open_windows_owned_directory(&staging.path, staging.identity).map_err(|error| {
            stale_staging(
                &staging.path,
                format!("cannot acquire its source-bound publication handle: {error}"),
            )
        })?;
    let result = rename_windows_directory_handle(&handle, &staging.parent, &staging.target);
    drop(handle);
    if result.is_ok() {
        return Ok(());
    }

    restore_windows_capability(staging);
    let error = result.expect_err("checked error above");
    if error
        .raw_os_error()
        .is_some_and(|code| code as u32 == ERROR_ALREADY_EXISTS || code as u32 == ERROR_FILE_EXISTS)
    {
        return Err(existing(destination));
    }
    Err(io_error("publish", destination, error))
}

#[cfg(windows)]
fn restore_windows_capability(staging: &mut StagingDirectory) {
    let Ok(dir) = staging.parent.open_dir_nofollow(&staging.name) else {
        return;
    };
    let Ok(metadata) = dir.dir_metadata() else {
        return;
    };
    if identity(&metadata) == staging.identity {
        staging.dir = Some(dir);
    }
}

#[cfg(windows)]
fn open_windows_owned_directory(path: &Path, expected: EntryIdentity) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = cap_std::fs::File::from_std(file.try_clone()?).metadata()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || identity(&metadata) != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory handle does not identify the reserved staging directory",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn rename_windows_directory_handle(
    source: &std::fs::File,
    parent: &Dir,
    target: &std::ffi::OsStr,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_RENAME_INFO, FileRenameInfo, SetFileInformationByHandle,
    };

    let name: Vec<u16> = target.encode_wide().collect();
    if name.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export destination name contains a NUL code unit",
        ));
    }
    let name_bytes = name
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export name is too long"))?;
    let buffer_len = std::mem::offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(name_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export name is too long"))?;
    let word = std::mem::size_of::<usize>();
    let mut buffer = vec![0_usize; buffer_len.div_ceil(word)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();

    // SAFETY: `buffer` is pointer-aligned and large enough for the fixed
    // header plus every UTF-16 code unit. The source and parent handles remain
    // open for the duration of the call, and replacement is explicitly false.
    let result = unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = parent.as_raw_handle();
        (*info).FileNameLength = u32::try_from(name_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "export name is too long"))?;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            u32::try_from(buffer_len).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "export name is too long")
            })?,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn cleanup_windows_owned_directory(
    dir: Dir,
    path: &Path,
    expected: EntryIdentity,
) -> io::Result<()> {
    clear_windows_directory(&dir, path)?;
    drop(dir);
    let handle = open_windows_owned_directory(path, expected)?;
    delete_windows_directory_handle(&handle)
}

#[cfg(windows)]
fn clear_windows_directory(directory: &Dir, path: &Path) -> io::Result<()> {
    let entries = directory
        .entries()?
        .map(|entry| {
            let entry = entry?;
            Ok((entry.file_name(), entry.file_type()?))
        })
        .collect::<io::Result<Vec<_>>>()?;

    for (name, file_type) in entries {
        if file_type.is_dir() && !file_type.is_symlink() {
            let child = directory.open_dir_nofollow(&name)?;
            let child_identity = identity(&child.dir_metadata()?);
            let child_path = path.join(&name);
            clear_windows_directory(&child, &child_path)?;
            drop(child);
            let handle = open_windows_owned_directory(&child_path, child_identity)?;
            delete_windows_directory_handle(&handle)?;
        } else {
            directory.remove_file_or_symlink(&name)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn delete_windows_directory_handle(directory: &std::fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
    };

    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the structure has the exact Windows ABI and stays alive for the
    // call; `directory` was opened with DELETE and FILE_SHARE_DELETE.
    let result = unsafe {
        SetFileInformationByHandle(
            directory.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::addr_of!(disposition).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>())
                .expect("Windows disposition structure size fits u32"),
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios", windows)))]
fn publish_no_replace(
    _staging: &mut StagingDirectory,
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
        export_with_writer, export_with_writer_and_before_publish, export_with_writer_and_hooks,
        write_relative,
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
        assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
    }

    #[test]
    fn a_finalization_failure_leaves_no_destination_or_staging_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");

        let error = export_with_writer_and_before_publish(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_staging| Err(io::Error::other("forced finalization failure")),
        )
        .unwrap_err();

        assert!(error.message.contains("forced finalization failure"));
        assert!(!destination.exists());
        assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
    }

    #[test]
    fn reservation_race_preserves_a_destination_created_before_create_dir() {
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

        let error = export_with_writer_and_hooks(
            &snapshot,
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_destination| {
                barrier.wait();
                actor
                    .join()
                    .map_err(|_| io::Error::other("publication actor panicked"))?
            },
            |_destination| Ok(()),
            |_destination| Ok(()),
            |_destination| Ok(()),
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

    #[test]
    fn a_staging_entry_substituted_between_creation_and_open_is_not_adopted() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("created-staging");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_hooks(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_destination| Ok(()),
            |staging| {
                let staging = staging.to_path_buf();
                fs::rename(&staging, &displaced)?;
                fs::create_dir(&staging)?;
                *replacement.lock().unwrap() = Some(staging);
                Ok(())
            },
            |_destination| Ok(()),
            |_destination| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        let replacement = replacement.into_inner().unwrap().unwrap();
        assert!(!destination.exists());
        assert!(fs::read_dir(&replacement).unwrap().next().is_none());
        assert!(displaced.is_dir());
    }

    #[test]
    fn a_staging_entry_substituted_after_identity_check_is_not_published() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("verified-staging");

        let error = export_with_writer_and_hooks(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_destination| Ok(()),
            |_destination| Ok(()),
            |_destination| Ok(()),
            |staging| {
                let staging = staging.to_path_buf();
                fs::rename(&staging, &displaced)?;
                fs::create_dir(&staging)?;
                fs::write(staging.join("hostile.txt"), "post-check replacement")
            },
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert!(!destination.exists());
        assert!(displaced.join("graph.html").is_file());
        let replacement = fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.join("hostile.txt").is_file())
            .expect("replacement staging remains owned by the concurrent actor");
        assert_eq!(
            fs::read_to_string(replacement.join("hostile.txt")).unwrap(),
            "post-check replacement"
        );
    }

    #[test]
    fn a_rotated_destination_parent_is_refused_and_its_new_owner_is_preserved() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("publish");
        let rotated = temporary.path().join("rotated-publish");
        fs::create_dir(&parent).unwrap();
        let destination = parent.join("site");

        let error = export_with_writer_and_before_publish(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |_staging| {
                fs::rename(&parent, &rotated)?;
                fs::create_dir(&parent)?;
                fs::write(parent.join("owner.txt"), "replacement parent owner")
            },
        )
        .unwrap_err();

        assert_eq!(
            error.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert!(!destination.exists());
        assert!(!rotated.join("site").exists());
        assert_eq!(
            fs::read_to_string(parent.join("owner.txt")).unwrap(),
            "replacement parent owner"
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
        let replacement = fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
            })
            .expect("replacement staging symlink remains owned by the concurrent actor");
        assert_eq!(fs::read_link(replacement).unwrap(), outside);
    }
}
