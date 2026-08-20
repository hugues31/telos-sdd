//! Capability-anchored mutation below one repository root.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;

pub(crate) struct RepoFs {
    root: Dir,
}

impl RepoFs {
    pub(crate) fn open(root: &Path) -> Result<Self, TelosError> {
        Dir::open_ambient_dir(root, ambient_authority())
            .map(|root| Self { root })
            .map_err(|error| io_error("open repository root", None, error))
    }

    pub(crate) fn read_optional(&self, path: &RepoPath) -> Result<Option<Vec<u8>>, TelosError> {
        path.validate()?;
        let (parents, name) = split(path)?;
        let Some(parent) = self.open_parent(&parents, false, path)? else {
            return Ok(None);
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = match parent.open_with(&name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(unsafe_io_path("read", path, error)),
        };
        if !file
            .metadata()
            .map_err(|error| unsafe_io_path("inspect", path, error))?
            .is_file()
        {
            return Err(unsafe_path(path));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| io_error("read", Some(path), error))?;
        Ok(Some(bytes))
    }

    pub(crate) fn read(&self, path: &RepoPath) -> Result<Vec<u8>, TelosError> {
        self.read_optional(path)?.ok_or_else(|| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read `{path}`: file does not exist"),
            )
        })
    }

    pub(crate) fn write(&self, path: &RepoPath, bytes: &[u8]) -> Result<(), TelosError> {
        path.validate()?;
        let (parents, name) = split(path)?;
        let parent = self
            .open_parent(&parents, true, path)?
            .expect("create=true always returns a parent");
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create(true)
            .truncate(true)
            .follow(FollowSymlinks::No);
        let mut file = parent
            .open_with(&name, &options)
            .map_err(|error| unsafe_io_path("write", path, error))?;
        if !file
            .metadata()
            .map_err(|error| io_error("inspect", Some(path), error))?
            .is_file()
        {
            return Err(unsafe_path(path));
        }
        file.write_all(bytes)
            .map_err(|error| io_error("write", Some(path), error))
    }

    pub(crate) fn remove_file(&self, path: &RepoPath) -> Result<(), TelosError> {
        path.validate()?;
        let (parents, name) = split(path)?;
        let Some(parent) = self.open_parent(&parents, false, path)? else {
            return Ok(());
        };
        match parent.remove_file(&name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(unsafe_io_path("delete", path, error)),
        }
    }

    fn open_parent(
        &self,
        components: &[OsString],
        create: bool,
        path: &RepoPath,
    ) -> Result<Option<Dir>, TelosError> {
        let mut current = self
            .root
            .open_dir(".")
            .map_err(|error| io_error("open repository root", None, error))?;
        for component in components {
            match current.open_dir_nofollow(component) {
                Ok(next) => current = next,
                Err(error) if !create && error.kind() == io::ErrorKind::NotFound => {
                    return Ok(None);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    current
                        .create_dir(component)
                        .map_err(|error| unsafe_io_path("create directory", path, error))?;
                    current = current
                        .open_dir_nofollow(component)
                        .map_err(|error| unsafe_io_path("open directory", path, error))?;
                }
                Err(error) => return Err(unsafe_io_path("open directory", path, error)),
            }
        }
        Ok(Some(current))
    }
}

fn split(path: &RepoPath) -> Result<(Vec<OsString>, OsString), TelosError> {
    let mut components = Path::new(path.as_str())
        .components()
        .map(|component| match component {
            Component::Normal(component) => Ok(component.to_os_string()),
            _ => Err(unsafe_path(path)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let name = components.pop().ok_or_else(|| unsafe_path(path))?;
    Ok((components, name))
}

fn unsafe_path(path: &RepoPath) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("repository path `{path}` is not safely contained"),
    )
}

fn io_error(action: &str, path: Option<&RepoPath>, error: io::Error) -> TelosError {
    let suffix = path.map_or_else(String::new, |path| format!(" `{path}`"));
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {action}{suffix}: {error}"),
    )
}

fn unsafe_io_path(action: &str, path: &RepoPath, error: io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("failed to safely {action} repository path `{path}`: {error}"),
    )
}
