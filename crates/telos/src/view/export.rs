//! Atomic static export of a rendered [`ViewSnapshot`].
//!
//! The exporter prepares every page in memory, reserves the final directory
//! without replacement, marks it incomplete, and writes only through its held
//! capability. A failed export remains explicitly incomplete and is refused
//! on retry; the exporter never guesses that a pathname is still its own and
//! never recursively cleans it up.
//!
//! Unix has no operation that both creates a directory and returns its handle,
//! nor a rename operation whose source is a directory handle. The reservation
//! therefore rejects a substituted symlink or non-empty directory before page
//! writes, and completion verifies the held object around marker removal.

use std::ffi::OsString;
use std::io::{self, Read, Seek, SeekFrom, Write};
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

const INDEX_PATH: &str = "index.html";
const INCOMPLETE_INDEX: &[u8] = b"<!doctype html><title>Telos export incomplete</title>\n";

struct ReservedDestination {
    parent: Dir,
    parent_path: PathBuf,
    name: OsString,
    path: PathBuf,
    dir: Dir,
    index: cap_std::fs::File,
    identity: EntryIdentity,
}

/// Renders and publishes every static page below a create-only `destination`.
/// The returned paths are relative to `destination` and lexicographically
/// sorted, making both the answer and an exported tree deterministic.
pub(crate) fn export(
    snapshot: &ViewSnapshot,
    destination: &Path,
) -> Result<Vec<PathBuf>, TelosError> {
    export_with_writer_and_hooks(
        snapshot,
        destination,
        |_destination_path, destination, relative, bytes| {
            write_relative(destination, relative, bytes)
        },
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
fn export_with_writer_and_before_complete<F, H>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    write_file: F,
    before_complete: H,
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
        before_complete,
        |_destination| Ok(()),
    )
}

fn export_with_writer_and_hooks<F, B, R, H, J>(
    snapshot: &ViewSnapshot,
    destination: &Path,
    mut write_file: F,
    before_reserve: B,
    after_destination_create: R,
    before_complete: H,
    after_identity_check: J,
) -> Result<Vec<PathBuf>, TelosError>
where
    F: FnMut(&Path, &Dir, &Path, &[u8]) -> io::Result<()>,
    B: FnOnce(&Path) -> io::Result<()>,
    R: FnOnce(&Path) -> io::Result<()>,
    H: FnOnce(&Path) -> io::Result<()>,
    J: FnOnce(&Path) -> io::Result<()>,
{
    let rendered = rendered_files(snapshot)?;
    let mut reserved = ReservedDestination::reserve_with_hooks(
        destination,
        before_reserve,
        after_destination_create,
    )?;

    (|| {
        for (relative, bytes) in &rendered {
            if relative == Path::new(INDEX_PATH) {
                continue;
            }
            let path = reserved.path().join(relative);
            write_file(reserved.path(), reserved.dir(), relative, bytes)
                .map_err(|error| io_error("write", &path, error))?;
        }

        before_complete(reserved.path())
            .map_err(|error| io_error("prepare completion for", destination, error))?;
        reserved.verify_entry()?;
        after_identity_check(reserved.path())
            .map_err(|error| io_error("finish destination verification for", destination, error))?;
        reserved.verify_entry()?;
        reserved.complete(&rendered)?;
        Ok(rendered.into_iter().map(|(path, _)| path).collect())
    })()
}

fn write_relative(destination: &Dir, relative: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new_relative(destination, relative)?;
    file.write_all(bytes)
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

impl ReservedDestination {
    fn reserve_with_hooks<B, H>(
        destination: &Path,
        before_reserve: B,
        after_create: H,
    ) -> Result<Self, TelosError>
    where
        B: FnOnce(&Path) -> io::Result<()>,
        H: FnOnce(&Path) -> io::Result<()>,
    {
        let parent_path = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = destination.file_name().ok_or_else(|| {
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
        before_reserve(destination)
            .map_err(|error| io_error("prepare destination reservation for", destination, error))?;
        match parent.create_dir(name) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(existing_destination(destination));
            }
            Err(error) => return Err(io_error("reserve", destination, error)),
        }

        let path = parent_path.join(name);
        after_create(&path)
            .map_err(|error| io_error("finish destination reservation for", destination, error))?;
        let dir = parent.open_dir_nofollow(name).map_err(|error| {
            stale_destination(&path, format!("cannot open it no-follow: {error}"))
        })?;
        if dir
            .entries()
            .map_err(|error| stale_destination(&path, error))?
            .next()
            .transpose()
            .map_err(|error| stale_destination(&path, error))?
            .is_some()
        {
            return Err(stale_destination(
                &path,
                "the newly reserved directory was replaced with a non-empty owner",
            ));
        }
        let identity = identity(
            &dir.dir_metadata()
                .map_err(|error| stale_destination(&path, error))?,
        );
        let mut index = create_new_relative(&dir, Path::new(INDEX_PATH))
            .map_err(|error| io_error("mark incomplete", destination, error))?;
        index
            .write_all(INCOMPLETE_INDEX)
            .and_then(|()| index.sync_all())
            .map_err(|error| io_error("mark incomplete", destination, error))?;
        let reserved = Self {
            parent,
            parent_path: parent_path.to_path_buf(),
            name: name.to_os_string(),
            path,
            dir,
            index,
            identity,
        };
        reserved.verify_entry()?;
        Ok(reserved)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn dir(&self) -> &Dir {
        &self.dir
    }

    fn verify_entry(&self) -> Result<(), TelosError> {
        let entry = self.parent.open_dir_nofollow(&self.name).map_err(|error| {
            stale_destination(
                &self.parent_path.join(&self.name),
                format!("cannot reopen the reserved entry: {error}"),
            )
        })?;
        let entry_identity = identity(&entry.dir_metadata().map_err(|error| {
            stale_destination(
                &self.path,
                format!("cannot inspect the directory entry: {error}"),
            )
        })?);
        let handle_identity = identity(&self.dir().dir_metadata().map_err(|error| {
            stale_destination(
                &self.path,
                format!("cannot inspect the held directory: {error}"),
            )
        })?);
        if entry_identity != self.identity || handle_identity != self.identity {
            return Err(stale_destination(
                &self.parent_path.join(&self.name),
                "the directory entry no longer names the reserved destination directory",
            ));
        }
        Ok(())
    }

    fn complete(&mut self, rendered: &[(PathBuf, Vec<u8>)]) -> Result<(), TelosError> {
        self.validate_contents(rendered, false)?;
        let index = rendered
            .iter()
            .find(|(path, _)| path == Path::new(INDEX_PATH))
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or_else(|| stale_destination(&self.path, "rendered export has no index"))?;
        self.rewrite_index(index)?;
        if let Err(error) = self
            .verify_entry()
            .and_then(|()| self.validate_contents(rendered, true))
        {
            let _ = self.rewrite_index(INCOMPLETE_INDEX);
            return Err(error);
        }
        Ok(())
    }

    fn rewrite_index(&mut self, bytes: &[u8]) -> Result<(), TelosError> {
        self.index
            .set_len(0)
            .and_then(|()| self.index.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|()| self.index.write_all(bytes))
            .and_then(|()| self.index.sync_all())
            .map_err(|error| io_error("write", &self.path.join(INDEX_PATH), error))
    }

    fn validate_contents(
        &self,
        rendered: &[(PathBuf, Vec<u8>)],
        complete: bool,
    ) -> Result<(), TelosError> {
        let mut actual = Vec::new();
        collect_files(self.dir(), Path::new(""), &mut actual)
            .map_err(|error| io_error("inspect incomplete export", &self.path, error))?;
        actual.sort();
        let mut expected: Vec<_> = rendered.iter().map(|(path, _)| path.clone()).collect();
        expected.sort();
        if actual != expected {
            return Err(stale_destination(
                &self.path,
                "its contents changed before completion",
            ));
        }
        for (relative, bytes) in rendered {
            let expected = if !complete && relative == Path::new(INDEX_PATH) {
                INCOMPLETE_INDEX
            } else {
                bytes
            };
            let actual = read_relative(self.dir(), relative)
                .map_err(|error| io_error("verify", &self.path.join(relative), error))?;
            if actual != expected {
                return Err(stale_destination(
                    &self.path.join(relative),
                    "its bytes changed before completion",
                ));
            }
        }
        Ok(())
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

fn stale_destination(path: &Path, reason: impl std::fmt::Display) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!(
            "refusing stale export destination {}: {reason}",
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
        export, export_with_writer, export_with_writer_and_before_complete,
        export_with_writer_and_hooks, write_relative,
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
    fn a_write_failure_leaves_an_explicitly_incomplete_destination() {
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
        assert_eq!(
            fs::read(destination.join("index.html")).unwrap(),
            b"<!doctype html><title>Telos export incomplete</title>\n"
        );
        assert!(!destination.join(".telos-export-incomplete").exists());

        let retry = export(&fixture_snapshot(), &destination).unwrap_err();
        assert_eq!(
            retry.code,
            telos_core::error::ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(
            fs::read(destination.join("index.html")).unwrap(),
            b"<!doctype html><title>Telos export incomplete</title>\n"
        );
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
            |_destination_path, destination, relative, bytes| {
                write_relative(destination, relative, bytes)
            },
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
    fn a_substituted_destination_entry_is_never_completed() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("displaced-staging");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_before_complete(
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
        assert_eq!(
            fs::read_to_string(destination.join("hostile.txt")).unwrap(),
            "not the export"
        );
        assert!(!destination.join("index.html").exists());
        assert_eq!(
            fs::read(displaced.join("index.html")).unwrap(),
            b"<!doctype html><title>Telos export incomplete</title>\n"
        );
        let replacement = replacement.into_inner().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(replacement.join("hostile.txt")).unwrap(),
            "not the export"
        );
    }

    #[test]
    fn failed_completion_never_cleans_up_a_substituted_destination_entry() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("displaced-staging");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_before_complete(
            &fixture_snapshot(),
            &destination,
            |_staging_path, staging, relative, bytes| write_relative(staging, relative, bytes),
            |staging| {
                let staging = staging.to_path_buf();
                fs::rename(&staging, &displaced)?;
                fs::create_dir(&staging)?;
                fs::write(staging.join("hostile.txt"), "replacement owner")?;
                *replacement.lock().unwrap() = Some(staging);
                fs::write(destination.join("owner.txt"), "destination owner")
            },
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
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
    fn a_destination_entry_substituted_between_creation_and_open_is_not_adopted() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("created-destination");
        let replacement = std::sync::Mutex::new(None);

        let error = export_with_writer_and_hooks(
            &fixture_snapshot(),
            &destination,
            |_destination_path, destination, relative, bytes| {
                write_relative(destination, relative, bytes)
            },
            |_destination| Ok(()),
            |destination| {
                let destination = destination.to_path_buf();
                fs::rename(&destination, &displaced)?;
                fs::create_dir(&destination)?;
                fs::write(destination.join("hostile.txt"), "reservation replacement")?;
                *replacement.lock().unwrap() = Some(destination);
                Ok(())
            },
            |_destination| Ok(()),
            |_destination| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        let replacement = replacement.into_inner().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(replacement.join("hostile.txt")).unwrap(),
            "reservation replacement"
        );
        assert!(!replacement.join("index.html").exists());
    }

    #[test]
    fn a_destination_entry_substituted_after_identity_check_is_not_completed() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("site");
        let displaced = temporary.path().join("verified-destination");

        let error = export_with_writer_and_hooks(
            &fixture_snapshot(),
            &destination,
            |_destination_path, destination, relative, bytes| {
                write_relative(destination, relative, bytes)
            },
            |_destination| Ok(()),
            |_destination| Ok(()),
            |_destination| Ok(()),
            |destination| {
                let destination = destination.to_path_buf();
                fs::rename(&destination, &displaced)?;
                fs::create_dir(&destination)?;
                fs::write(destination.join("hostile.txt"), "post-check replacement")
            },
        )
        .unwrap_err();

        assert_eq!(error.code, telos_core::error::ErrorCode::TelosInternal);
        assert_eq!(
            fs::read_to_string(destination.join("hostile.txt")).unwrap(),
            "post-check replacement"
        );
        assert!(!destination.join("index.html").exists());
        assert!(displaced.join("index.html").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_destination_symlink_substituted_before_writes_never_receives_export_bytes() {
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
        assert!(
            fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        assert_eq!(
            fs::read(displaced.join("index.html")).unwrap(),
            b"<!doctype html><title>Telos export incomplete</title>\n"
        );
    }
}
