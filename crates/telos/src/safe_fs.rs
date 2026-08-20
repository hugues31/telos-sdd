//! Capability-anchored writes below a repository root.
//!
//! All traversal starts from one opened [`cap_std::fs::Dir`].  Each directory
//! component is opened with `O_NOFOLLOW`-equivalent semantics, and each final
//! file is opened through the already-held parent directory handle.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};

pub(crate) struct SafeRoot {
    dir: Dir,
}

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A complete sibling file that is not visible at its final name yet.
pub(crate) struct StagedWrite {
    parent: Dir,
    parents: Vec<OsString>,
    target: OsString,
    staging: OsString,
    staging_identity: FileIdentity,
    published: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl SafeRoot {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Dir::open_ambient_dir(path, ambient_authority()).map(|dir| Self { dir })
    }

    pub(crate) fn read_optional(&self, relative: &Path) -> io::Result<Option<Vec<u8>>> {
        let (parents, name) = split(relative)?;
        let Some(parent) = self.open_parent(&parents, false)? else {
            return Ok(None);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        match parent.open_with(&name, &options) {
            Ok(mut file) => {
                if !file.metadata()?.is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "target is not a regular file",
                    ));
                }
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn exists_no_follow(&self, relative: &Path) -> io::Result<bool> {
        let (parents, name) = split(relative)?;
        let Some(parent) = self.open_parent(&parents, false)? else {
            return Ok(false);
        };
        match parent.symlink_metadata(&name) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn validate_directory(&self, relative: &Path) -> io::Result<bool> {
        let components = directories(relative)?;
        Ok(self.open_parent(&components, false)?.is_some())
    }

    /// Creates a directory path below the held root, opening every component
    /// without following symlinks. Existing real directories are a no-op.
    pub(crate) fn create_directory(&self, relative: &Path) -> io::Result<()> {
        let components = directories(relative)?;
        self.open_parent(&components, true)?
            .ok_or_else(|| io::Error::other("failed to create directory"))?;
        Ok(())
    }

    pub(crate) fn stage_with<F>(
        &self,
        relative: &Path,
        bytes: &[u8],
        write: F,
    ) -> io::Result<StagedWrite>
    where
        F: FnOnce(&mut cap_std::fs::File, &[u8]) -> io::Result<()>,
    {
        let (parents, target) = split(relative)?;
        let parent = self
            .open_parent(&parents, true)?
            .ok_or_else(|| io::Error::other("missing parent"))?;
        let (staging, mut file) = reserve_staging(&parent, &target)?;
        let staging_identity = file_identity(&file.metadata()?);
        if let Err(error) = write(&mut file, bytes).and_then(|()| file.sync_all()) {
            drop(file);
            remove_if_owned(&parent, &staging, staging_identity);
            return Err(error);
        }
        drop(file);
        Ok(StagedWrite {
            parent,
            parents,
            target,
            staging,
            staging_identity,
            published: false,
        })
    }

    pub(crate) fn create_new_write_with<F, H>(
        &self,
        relative: &Path,
        bytes: &[u8],
        write: F,
        before_publish: H,
    ) -> io::Result<()>
    where
        F: FnOnce(&mut cap_std::fs::File, &[u8]) -> io::Result<()>,
        H: FnOnce() -> io::Result<()>,
    {
        let staging = self.stage_with(relative, bytes, write)?;
        before_publish()?;
        staging.validate_parent_path(self)?;
        staging.publish_create_only()
    }

    pub(crate) fn remove_file_if_matches(
        &self,
        relative: &Path,
        expected: &[u8],
    ) -> io::Result<()> {
        let (parents, name) = split(relative)?;
        let parent = self
            .open_parent(&parents, false)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "parent disappeared"))?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = parent.open_with(&name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target is not a regular file",
            ));
        }
        let identity = file_identity(&file.metadata()?);
        let mut actual = Vec::new();
        file.read_to_end(&mut actual)?;
        if actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target bytes changed",
            ));
        }
        drop(file);
        match parent.symlink_metadata(&name) {
            Ok(metadata) if file_identity(&metadata) == identity => parent.remove_file(&name),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target identity changed",
            )),
            Err(error) => Err(error),
        }
    }

    fn open_parent(&self, components: &[OsString], create: bool) -> io::Result<Option<Dir>> {
        let mut current = self.dir.open_dir(".")?;
        for component in components {
            match current.open_dir_nofollow(component) {
                Ok(next) => current = next,
                Err(error) if error.kind() == io::ErrorKind::NotFound && !create => {
                    return Ok(None);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    current.create_dir(component)?;
                    current = current.open_dir_nofollow(component)?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Some(current))
    }
}

impl StagedWrite {
    pub(crate) fn read_target(&self) -> io::Result<Option<Vec<u8>>> {
        read_optional_from(&self.parent, &self.target)
    }

    pub(crate) fn validate_parent_path(&self, root: &SafeRoot) -> io::Result<()> {
        let reopened = root
            .open_parent(&self.parents, false)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "parent disappeared"))?;
        let held = self.parent.dir_metadata()?;
        let current = reopened.dir_metadata()?;
        if held.dev() == current.dev() && held.ino() == current.ino() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "parent directory identity changed",
            ))
        }
    }

    /// Publishes one complete file and fails atomically if the final name has
    /// acquired an owner since preflight.
    pub(crate) fn publish_create_only(mut self) -> io::Result<()> {
        self.validate_staging_identity()?;
        self.parent
            .hard_link(&self.staging, &self.parent, &self.target)?;
        self.published = true;
        // Both names refer to the same complete inode after `hard_link`.
        // Failure to remove the private name must not turn a successful,
        // non-clobbering publication into a reported transaction failure.
        remove_if_owned(&self.parent, &self.staging, self.staging_identity);
        Ok(())
    }

    /// Atomically replaces a regular target whose bytes were checked by the
    /// caller immediately before this operation.
    pub(crate) fn publish_replace(mut self) -> io::Result<()> {
        self.validate_staging_identity()?;
        self.parent
            .rename(&self.staging, &self.parent, &self.target)?;
        self.published = true;
        Ok(())
    }

    fn validate_staging_identity(&self) -> io::Result<()> {
        match self.parent.symlink_metadata(&self.staging) {
            Ok(metadata) if file_identity(&metadata) == self.staging_identity => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging file identity changed",
            )),
            Err(error) => Err(error),
        }
    }
}

impl Drop for StagedWrite {
    fn drop(&mut self) {
        if !self.published {
            remove_if_owned(&self.parent, &self.staging, self.staging_identity);
        }
    }
}

fn remove_if_owned(parent: &Dir, staging: &std::ffi::OsStr, expected: FileIdentity) {
    if parent
        .symlink_metadata(staging)
        .is_ok_and(|metadata| file_identity(&metadata) == expected)
    {
        let _ = parent.remove_file(staging);
    }
}

fn file_identity(metadata: &cap_std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

fn reserve_staging(
    parent: &Dir,
    target: &std::ffi::OsStr,
) -> io::Result<(OsString, cap_std::fs::File)> {
    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = OsString::from(format!(
            ".{}.telos-staging-{}-{sequence}",
            target.to_string_lossy(),
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        match parent.open_with(&candidate, &options) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to reserve a unique staging file",
    ))
}

fn read_optional_from(parent: &Dir, name: &std::ffi::OsStr) -> io::Result<Option<Vec<u8>>> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    match parent.open_with(name, &options) {
        Ok(mut file) => {
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "target is not a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn split(relative: &Path) -> io::Result<(Vec<OsString>, OsString)> {
    let mut components = directories(relative)?;
    let name = components
        .pop()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty relative path"))?;
    Ok((components, name))
}

fn directories(relative: &Path) -> io::Result<Vec<OsString>> {
    relative
        .components()
        .map(|component| match component {
            Component::Normal(component) => Ok(component.to_os_string()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe relative path",
            )),
        })
        .collect()
}
