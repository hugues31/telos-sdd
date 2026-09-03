//! `telos.lock`: the content-addressed seal over the spec and
//! the code its bindings reference.
//!
//! Writing is hand-rendered TOML, deterministic to the byte -- fixed key
//! order, sorted tables (a free consequence of `BTreeMap`'s iteration
//! order), LF line endings, exactly one trailing newline. The `toml` crate
//! is deliberately *not* used to serialize a `Lock`: its formatting and key
//! ordering aren't under our control, and a lock file that isn't
//! byte-identical between two seals of the same content would make
//! `git diff telos.lock` noisy for no reason. Reading, by contrast, does
//! go through `toml` + `serde` -- a human or another tool may have
//! reformatted the file, and `read` has no reason to be stricter about
//! that than it needs to be.
//!
//! `seal()` -- computing a `Lock` from a live `Workspace` / `TelosModel` /
//! [`crate::git::GitRepo`] -- also lives here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{ErrorCode, TelosError};
use crate::git::{GitRepo, Oid};
use crate::ids::{ChangeId, RepoPath};
use crate::model::TelosModel;
use crate::repo_fs::RepoFs;
use crate::workspace::Workspace;

pub const LOCK_VERSION: u32 = 2;

/// A parsed or in-memory `telos.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    /// Lock file format version -- `2`.
    pub version: u32,
    /// The tool that produced this lock, e.g. `"telos 0.7.0"`.
    pub tool: String,
    /// The change that produced this seal, if any. `None` for the seal
    /// `telos init` writes `None`; a reconciled change writes `Some`.
    pub sealed_by: Option<ChangeId>,
    /// `"sha256:<hex>"`, over `spec` -- see [`Lock::compute_digest`].
    pub spec_digest: String,
    /// `telos.toml` plus every `.tel` file (excluding `telos/changes/` and
    /// `telos.lock` itself), by repo-relative path.
    pub spec: BTreeMap<RepoPath, Oid>,
    /// Every file referenced by a binding, by repo-relative path.
    pub code: BTreeMap<RepoPath, Oid>,
}

impl Lock {
    /// Reads and parses `path`. `Ok(None)` if the file does not exist (an
    /// unsealed project, not an error); a parse failure is
    /// `TelosParseError`, naming `path`.
    pub fn read(path: &Path) -> Result<Option<Lock>, TelosError> {
        let src = match fs::read_to_string(path) {
            Ok(src) => src,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(TelosError::new(
                    ErrorCode::TelosInternal,
                    format!("failed to read {}: {e}", path.display()),
                ));
            }
        };

        let raw: RawLock = toml::from_str(&src).map_err(|e| {
            TelosError::new(
                ErrorCode::TelosParseError,
                format!("{}: {e}", path.display()),
            )
        })?;
        if raw.version != LOCK_VERSION {
            return Err(TelosError::new(
                ErrorCode::TelosParseError,
                format!(
                    "{}: unsupported lock format version {}; expected {}",
                    path.display(),
                    raw.version,
                    LOCK_VERSION
                ),
            )
            .hint("run `telos reconcile --full` with Telos 0.9 to regenerate telos.lock"));
        }

        let sealed_by = raw
            .sealed_by
            .map(|s| {
                s.parse::<ChangeId>().map_err(|_| {
                    TelosError::new(
                        ErrorCode::TelosParseError,
                        format!("{}: invalid `sealed_by` value `{s}`", path.display()),
                    )
                })
            })
            .transpose()?;

        Ok(Some(Lock {
            version: raw.version,
            tool: raw.tool,
            sealed_by,
            spec_digest: raw.spec_digest,
            spec: into_oid_map(raw.spec, path)?,
            code: into_oid_map(raw.code, path)?,
        }))
    }

    /// Writes `self` to `path` as canonical, deterministic TOML: `version`,
    /// `tool`, `sealed_by` (omitted entirely when `None`, else the
    /// `"CHG-NNNN"` string), `spec_digest`, then a `[spec]` table and a
    /// `[code]` table, each with its entries sorted by path (guaranteed by
    /// `BTreeMap`'s iteration order) and quoted as TOML keys. LF line
    /// endings throughout, exactly one trailing newline.
    pub fn write(&self, path: &Path) -> Result<(), TelosError> {
        fs::write(path, self.render()).map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to write {}: {e}", path.display()),
            )
        })
    }

    pub(crate) fn write_to_workspace(&self, ws: &Workspace) -> Result<(), TelosError> {
        RepoFs::open(&ws.repo_root)?
            .write(&RepoPath::new("telos/telos.lock"), self.render().as_bytes())
    }

    fn render(&self) -> String {
        let mut out = String::new();
        // `write!`/`writeln!` on a `String` only fail on a `fmt::Display`
        // impl erroring, which none of these types do -- `.unwrap()` is
        // safe.
        writeln!(out, "version = {}", self.version).unwrap();
        writeln!(out, "tool = {}", quote(&self.tool)).unwrap();
        if let Some(id) = &self.sealed_by {
            writeln!(out, "sealed_by = {}", quote(&id.to_string())).unwrap();
        }
        writeln!(out, "spec_digest = {}", quote(&self.spec_digest)).unwrap();

        out.push('\n');
        out.push_str("[spec]\n");
        for (path, oid) in &self.spec {
            writeln!(out, "{} = {}", quote(path.as_str()), quote(&oid.0)).unwrap();
        }

        out.push('\n');
        out.push_str("[code]\n");
        for (path, oid) in &self.code {
            writeln!(out, "{} = {}", quote(path.as_str()), quote(&oid.0)).unwrap();
        }

        out
    }

    /// `"sha256:" + hex(sha256("path\0oid\n" for every entry, sorted))`.
    ///
    /// Sorted order comes for free from iterating a `BTreeMap`, so this is
    /// stable under any insertion-order permutation of `spec` -- two maps
    /// built by inserting the same entries in different orders are the
    /// same `BTreeMap`, and so digest identically.
    pub fn compute_digest(spec: &BTreeMap<RepoPath, Oid>) -> String {
        let mut hasher = Sha256::new();
        for (path, oid) in spec {
            hasher.update(path.as_str().as_bytes());
            hasher.update(b"\0");
            hasher.update(oid.0.as_bytes());
            hasher.update(b"\n");
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

/// Computes a fresh [`Lock`] from a live workspace: OIDs of
/// [`Workspace::spec_files`] as `spec`, and the deduplicated
/// [`crate::model::Binding::code_path`] of every binding in `model` as
/// `code`.
///
/// A binding that names a code file missing from disk at seal time is
/// `TelosIntegrityViolation`, naming the path -- a binding cannot be sealed
/// to a file that does not exist. (A missing *spec* file cannot happen here:
/// `spec_files()` only ever lists files that already exist on disk.)
///
/// `ws` and `git` are discovered independently by every caller, so the
/// first thing this does is [`GitRepo::ensure_matches_workspace_root`] --
/// sealing paths relative to a git root that isn't actually `ws.repo_root`
/// would silently hash the wrong tree.
///
/// Records the bytes on disk *right now*, and nothing else: that is the one
/// sentence this function means, and it is not qualified by who calls it. A
/// caller that must not seal some of those live OIDs corrects the `Lock`
/// this hands back rather than asking `seal` to lie -- see
/// [`crate::reconcile::reconcile_change`]'s carry-over, which keeps a path
/// another open change claims at its previously sealed OID.
///
/// Both maps come from [`GitRepo::store_blobs`], not `blob_oids`: the
/// `Lock` this hands back exists to be written, and every OID a written
/// lock records must name an object the store holds -- commit or no commit
/// -- or `telos revert` has nothing to restore from. See the `git` module
/// docs.
///
/// Does not write the lock to disk -- the caller does that with
/// [`Lock::write`].
pub fn seal(
    ws: &Workspace,
    model: &TelosModel,
    git: &GitRepo,
    sealed_by: Option<ChangeId>,
) -> Result<Lock, TelosError> {
    git.ensure_matches_workspace_root(&ws.repo_root)?;

    let spec_paths = ws.spec_files()?;
    let spec = git.store_blobs(&spec_paths)?;

    let code_paths: Vec<RepoPath> = model
        .bindings
        .iter()
        .map(|b| b.code_path().clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let code = git.store_blobs(&code_paths)?;

    for path in &code_paths {
        if !code.contains_key(path) {
            return Err(TelosError::new(
                ErrorCode::TelosIntegrityViolation,
                format!("binding references `{path}`, which does not exist"),
            ));
        }
    }

    let spec_digest = Lock::compute_digest(&spec);

    Ok(Lock {
        version: LOCK_VERSION,
        tool: format!("telos {}", crate::VERSION),
        sealed_by,
        spec_digest,
        spec,
        code,
    })
}

/// Quotes `s` as a TOML basic string: wraps it in `"..."`, escaping the
/// characters that would otherwise end the string or be misread. `Lock`'s
/// fields only ever hold ids, hex digests and repo-relative paths (`/`,
/// never `\`), so this doesn't need to handle the full basic-string escape
/// table -- just enough to be correct for what actually reaches it.
fn quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('"');
    for c in s.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\t' => q.push_str("\\t"),
            '\r' => q.push_str("\\r"),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

/// The shape `toml::from_str` deserializes into: plain strings throughout,
/// tolerant of whatever formatting produced the file. [`Lock::read`]
/// converts this into the typed [`Lock`], which is where a malformed
/// `sealed_by` id is caught.
#[derive(Debug, Deserialize)]
struct RawLock {
    version: u32,
    tool: String,
    #[serde(default)]
    sealed_by: Option<String>,
    spec_digest: String,
    #[serde(default)]
    spec: BTreeMap<String, String>,
    #[serde(default)]
    code: BTreeMap<String, String>,
}

fn into_oid_map(
    raw: BTreeMap<String, String>,
    lock_path: &Path,
) -> Result<BTreeMap<RepoPath, Oid>, TelosError> {
    raw.into_iter()
        .map(|(path, oid)| {
            RepoPath::parse(path.clone())
                .map(|path| (path, Oid(oid)))
                .map_err(|error| {
                    TelosError::new(
                        ErrorCode::TelosParseError,
                        format!(
                            "{}: invalid repository path `{path}`: {}",
                            lock_path.display(),
                            error.message
                        ),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_digest_of_empty_spec_is_the_empty_hash() {
        let digest = Lock::compute_digest(&BTreeMap::new());
        assert_eq!(
            digest,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn read_rejects_the_v1_lock_format_with_an_actionable_hint() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telos.lock");
        std::fs::write(
            &path,
            concat!(
                "version = 1\n",
                "tool = \"telos 0.12.0\"\n",
                "spec_digest = \"sha256:old\"\n",
                "\n[spec]\n",
                "\n[code]\n",
            ),
        )
        .unwrap();

        let error = Lock::read(&path).unwrap_err();
        assert_eq!(error.code, ErrorCode::TelosParseError);
        assert!(error.message.contains("lock format version 1"));
        assert_eq!(
            error.hint.as_deref(),
            Some("run `telos reconcile --full` with Telos 0.9 to regenerate telos.lock")
        );
    }

    #[test]
    fn compute_digest_is_stable_under_insertion_order_permutation() {
        let mut a = BTreeMap::new();
        a.insert(
            RepoPath::new("telos/notions/Invoice.tel"),
            Oid("aaaa".repeat(10)),
        );
        a.insert(RepoPath::new("telos/telos.toml"), Oid("bbbb".repeat(10)));

        let mut b = BTreeMap::new();
        b.insert(RepoPath::new("telos/telos.toml"), Oid("bbbb".repeat(10)));
        b.insert(
            RepoPath::new("telos/notions/Invoice.tel"),
            Oid("aaaa".repeat(10)),
        );

        assert_eq!(Lock::compute_digest(&a), Lock::compute_digest(&b));
    }
}
