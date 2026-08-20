//! Capability-anchored writes below a repository root.
//!
//! All traversal starts from one opened [`cap_std::fs::Dir`].  Each directory
//! component is opened with `O_NOFOLLOW`-equivalent semantics, and each final
//! file is opened through the already-held parent directory handle.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
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
    published: bool,
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
        if let Err(error) = write(&mut file, bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = parent.remove_file(&staging);
            return Err(error);
        }
        drop(file);
        Ok(StagedWrite {
            parent,
            parents,
            target,
            staging,
            published: false,
        })
    }

    pub(crate) fn create_new_write_with<H, J>(
        &self,
        relative: &Path,
        bytes: &[u8],
        before_reserve: H,
        after_reserve: J,
    ) -> io::Result<()>
    where
        H: FnOnce() -> io::Result<()>,
        J: FnOnce() -> io::Result<()>,
    {
        self.create_new_write_inner(relative, bytes, before_reserve, after_reserve)
    }

    fn create_new_write_inner<H, J>(
        &self,
        relative: &Path,
        bytes: &[u8],
        before_reserve: H,
        after_reserve: J,
    ) -> io::Result<()>
    where
        H: FnOnce() -> io::Result<()>,
        J: FnOnce() -> io::Result<()>,
    {
        let (parents, name) = split(relative)?;
        let parent = self
            .open_parent(&parents, true)?
            .ok_or_else(|| io::Error::other("missing parent"))?;
        before_reserve()?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = parent.open_with(&name, &options)?;
        after_reserve()?;
        file.write_all(bytes)?;
        self.validate_parent_path(&parents)
    }

    fn validate_parent_path(&self, parents: &[OsString]) -> io::Result<()> {
        self.open_parent(parents, false)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "parent disappeared"))?;
        Ok(())
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
        root.validate_parent_path(&self.parents)
    }

    /// Publishes one complete file and fails atomically if the final name has
    /// acquired an owner since preflight.
    pub(crate) fn publish_create_only(mut self) -> io::Result<()> {
        self.parent
            .hard_link(&self.staging, &self.parent, &self.target)?;
        self.published = true;
        // Both names refer to the same complete inode after `hard_link`.
        // Failure to remove the private name must not turn a successful,
        // non-clobbering publication into a reported transaction failure.
        let _ = self.parent.remove_file(&self.staging);
        Ok(())
    }

    /// Atomically replaces a regular target whose bytes were checked by the
    /// caller immediately before this operation.
    pub(crate) fn publish_replace(mut self) -> io::Result<()> {
        self.parent
            .rename(&self.staging, &self.parent, &self.target)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for StagedWrite {
    fn drop(&mut self) {
        if !self.published {
            let _ = self.parent.remove_file(&self.staging);
        }
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
