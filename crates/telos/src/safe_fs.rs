//! Capability-anchored writes below a repository root.
//!
//! All traversal starts from one opened [`cap_std::fs::Dir`].  Each directory
//! component is opened with `O_NOFOLLOW`-equivalent semantics, and each final
//! file is opened through the already-held parent directory handle.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};

pub(crate) struct SafeRoot {
    dir: Dir,
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

    pub(crate) fn validate_target(&self, relative: &Path) -> io::Result<()> {
        let (parents, name) = split(relative)?;
        let Some(parent) = self.open_parent(&parents, false)? else {
            return Ok(());
        };
        match parent.symlink_metadata(&name) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target is not a regular file",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
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

    pub(crate) fn write_cached(&self, relative: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_cached_with(relative, bytes, || Ok(()))
    }

    pub(crate) fn write_cached_with<H>(
        &self,
        relative: &Path,
        bytes: &[u8],
        hook: H,
    ) -> io::Result<()>
    where
        H: FnOnce() -> io::Result<()>,
    {
        self.write_cached_inner(relative, bytes, hook)
    }

    fn write_cached_inner<H>(&self, relative: &Path, bytes: &[u8], hook: H) -> io::Result<()>
    where
        H: FnOnce() -> io::Result<()>,
    {
        let (parents, name) = split(relative)?;
        let parent = self
            .open_parent(&parents, true)?
            .ok_or_else(|| io::Error::other("missing parent"))?;
        hook()?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create(true)
            .truncate(true)
            .follow(FollowSymlinks::No);
        let mut file = parent.open_with(&name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target is not a regular file",
            ));
        }
        file.write_all(bytes)?;
        self.validate_parent_path(&parents)
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
