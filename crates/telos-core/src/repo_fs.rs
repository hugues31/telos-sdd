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
    pub(crate) fn list_files(&self, path: &RepoPath) -> Result<Vec<String>, TelosError> {
        path.validate()?;
        let components = Path::new(path.as_str())
            .components()
            .map(|c| c.as_os_str().to_owned())
            .collect::<Vec<_>>();
        let Some(dir) = self.open_parent(&components, false, path)? else {
            return Ok(vec![]);
        };
        let mut names = Vec::new();
        for entry in dir.entries().map_err(|e| io_error("list", Some(path), e))? {
            let entry = entry.map_err(|e| io_error("list entry", Some(path), e))?;
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| unsafe_path(path))?,
            );
        }
        names.sort();
        Ok(names)
    }

    pub(crate) fn writer_lock(&self) -> Result<std::fs::File, TelosError> {
        let path = RepoPath::new("telos/.runtime/write.lock");
        let (parents, name) = split(&path)?;
        let parent = self
            .open_parent(&parents, true, &path)?
            .expect("created parent");
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .follow(FollowSymlinks::No);
        let file = parent
            .open_with(name, &options)
            .map_err(|e| unsafe_io_path("open writer lock", &path, e))?
            .into_std();
        if !file
            .metadata()
            .map_err(|e| io_error("inspect lock", Some(&path), e))?
            .is_file()
        {
            return Err(unsafe_path(&path));
        }
        file.try_lock().map_err(|e| {
            TelosError::new(
                ErrorCode::TelosWorkspaceBusy,
                format!("another Telos writer holds this worktree: {e}"),
            )
            .hint("wait for the current command; a terminated process releases its lock")
        })?;
        Ok(file)
    }

    pub(crate) fn validate_writable(&self, path: &RepoPath) -> Result<(), TelosError> {
        self.read_optional(path)?;
        let (parents, name) = split(path)?;
        if let Some(parent) = self.open_parent(&parents, false, path)? {
            #[cfg(unix)]
            if parent
                .dir_metadata()
                .map_err(|e| io_error("inspect parent", Some(path), e))?
                .permissions()
                .readonly()
            {
                return Err(io_error(
                    "write read-only directory",
                    Some(path),
                    io::Error::new(io::ErrorKind::PermissionDenied, "directory is read-only"),
                ));
            }
            if let Ok(metadata) = parent.symlink_metadata(name)
                && metadata.permissions().readonly()
            {
                return Err(io_error(
                    "replace read-only file",
                    Some(path),
                    io::Error::new(io::ErrorKind::PermissionDenied, "file is read-only"),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn atomic_write(&self, path: &RepoPath, bytes: &[u8]) -> Result<(), TelosError> {
        path.validate()?;
        // Reject symlinks and non-files even though rename itself could replace them.
        self.read_optional(path)?;
        let (parents, name) = split(path)?;
        let parent = self
            .open_parent(&parents, true, path)?
            .expect("created parent");
        let temporary = format!(".{}.tmp", crate::work::new_id("write")?);
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let result = (|| {
            let mut file = parent
                .open_with(&temporary, &options)
                .map_err(|e| unsafe_io_path("prepare write", path, e))?;
            if let Ok(metadata) = parent.symlink_metadata(&name) {
                if metadata.permissions().readonly() {
                    return Err(io_error(
                        "replace read-only file",
                        Some(path),
                        io::Error::new(io::ErrorKind::PermissionDenied, "file is read-only"),
                    ));
                }
                file.set_permissions(metadata.permissions())
                    .map_err(|e| io_error("preserve permissions", Some(path), e))?;
            }
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| io_error("sync write", Some(path), e))?;
            parent
                .rename(&temporary, &parent, &name)
                .map_err(|e| io_error("publish write", Some(path), e))?;
            sync_directory(&parent).map_err(|e| io_error("sync parent", Some(path), e))
        })();
        if result.is_err() {
            let _ = parent.remove_file(&temporary);
        }
        result
    }

    pub(crate) fn open(root: &Path) -> Result<Self, TelosError> {
        Dir::open_ambient_dir(root, ambient_authority())
            .map(|root| Self { root })
            .map_err(|error| io_error("open repository root", None, error))
    }

    pub(crate) fn validate_parents(&self, path: &RepoPath) -> Result<(), TelosError> {
        path.validate()?;
        let (parents, _) = split(path)?;
        self.open_parent(&parents, false, path)?;
        Ok(())
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

    #[cfg(test)]
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
            Ok(()) => sync_directory(&parent).map_err(|e| io_error("sync deletion", Some(path), e)),
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
                    sync_directory(&current)
                        .map_err(|error| io_error("sync directory creation", Some(path), error))?;
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

fn sync_directory(dir: &Dir) -> io::Result<()> {
    #[cfg(unix)]
    dir.open(".")?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
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
