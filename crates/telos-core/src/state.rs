//! Project state: comparing a sealed [`Lock`] against the live
//! working tree, and a coverage summary of the spec itself.
//!
//! [`compute_state`] itself never parses a `.tel` file -- it compares git
//! blob OIDs only, which is exactly what lets it answer even when the spec
//! on disk is syntactically broken. A corrupted spec file *is* drift the
//! caller needs to be told about, not a reason for the check itself to
//! fail. The `open_changes` it is handed, by contrast, already came from a
//! parse ([`crate::changes::open_change_infos`]) -- `compute_state` only
//! reads their `claims`, never touches a change file itself.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::changes::OpenChangeInfo;
use crate::error::TelosError;
use crate::git::{GitRepo, Oid};
use crate::ids::{ChangeId, RepoPath};
use crate::lock::Lock;
use crate::model::{Binding, IntentStatus, TelosModel};
use crate::workspace::Workspace;

/// The hint carried by every `TELOS_DRIFT_DETECTED`, frozen by
/// `docs/contracts.md` and consumed by agent tooling.
///
/// It lives next to the module that *computes* drift so that the two places
/// that raise the code -- the CLI drift gate on staging commands, and
/// [`crate::reconcile::reconcile_change`]'s own first gate -- can never
/// drift apart from one another.
pub const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

/// The project's overall state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStateKind {
    /// Nothing has drifted and no change is open.
    Coherent,
    /// At least one change is open and every current drift is claimed by
    /// one of them -- the paths it owns are expected to differ from
    /// what was sealed, so that is not drift, it is the change in
    /// progress.
    Changing,
    /// At least one sealed path was modified or went missing, or a spec
    /// file exists on disk that was never sealed, and no open change
    /// claims it.
    Drifted,
}

/// One path whose current state no longer matches what was sealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriftEntry {
    pub path: RepoPath,
    pub kind: DriftKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DriftKind {
    /// Sealed and still present, but its current OID no longer matches.
    Modified,
    /// Sealed but no longer present on disk.
    Missing,
    /// A spec file present on disk that was never sealed -- created outside
    /// the protocol.
    Untracked,
}

/// One open change, as reported in a [`StateReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeSummary {
    pub id: ChangeId,
    pub status: String,
    pub obligations: Vec<String>,
}

/// The result of [`compute_state`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateReport {
    pub state: ProjectStateKind,
    /// Sorted by path. Never includes a path claimed by an open change
    /// -- see [`compute_state`].
    pub drift: Vec<DriftEntry>,
    pub open_changes: Vec<ChangeSummary>,
}

/// Stable authorization token for one exact seal plus the live bytes and
/// unclaimed drift scope observed at the decision boundary.
pub fn drift_token(
    ws: &Workspace,
    git: &GitRepo,
    lock: &Lock,
    drift: &[DriftEntry],
) -> Result<String, TelosError> {
    git.ensure_matches_workspace_root(&ws.repo_root)?;
    let live_paths = drift
        .iter()
        .filter(|entry| entry.kind != DriftKind::Missing)
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    let live_oids = git.blob_oids(&live_paths)?;
    let mut hasher = Sha256::new();
    hasher.update(b"telos-drift-v2\0");
    hasher.update(lock.spec_digest.as_bytes());
    hasher.update(b"\0");
    for (table, entries) in [(b's', &lock.spec), (b'c', &lock.code)] {
        for (path, oid) in entries {
            hasher.update([table]);
            hasher.update(path.as_str().as_bytes());
            hasher.update(b"\0");
            hasher.update(oid.0.as_bytes());
            hasher.update(b"\n");
        }
    }
    for entry in drift {
        hasher.update(entry.path.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(match entry.kind {
            DriftKind::Modified => b"modified".as_slice(),
            DriftKind::Missing => b"missing".as_slice(),
            DriftKind::Untracked => b"untracked".as_slice(),
        });
        hasher.update(b"\0");
        if let Some(oid) = live_oids.get(&entry.path) {
            hasher.update(oid.0.as_bytes());
        } else {
            hasher.update(b"absent");
        }
        hasher.update(b"\n");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Compares `lock` against the live working tree, by OID only, then folds
/// in `open_changes` while tolerating unreadable change files.
///
/// Re-hashes, in one [`GitRepo::blob_oids`] batch, the union of the
/// workspace's current [`Workspace::spec_files`] and every path `lock.spec`
/// and `lock.code` sealed.
///
/// - A sealed path (`spec` or `code`) absent from that batch is
///   [`DriftKind::Missing`].
/// - A sealed path present with a different OID is [`DriftKind::Modified`].
/// - A *current* spec file not present in `lock.spec` is
///   [`DriftKind::Untracked`].
/// - A code file not in `lock.code` is never drift: reconciling unlinked code
///   against `telos.toml`'s globs is a reconcile-time concern, not checked
///   here.
///
/// That raw drift is then filtered: a path claimed by *any* entry of
/// `open_changes` is dropped, because the change owns it -- what looks
/// like drift there is really the change in progress, not damage. `state`
/// follows this priority: unclaimed drift still present wins
/// (`Drifted`), else at least one open change wins (`Changing`), else
/// `Coherent`.
///
/// `ws` and `git` are discovered independently by every caller (`status`,
/// `check --sealed`) -- both from `cwd`, but by unrelated code paths -- so
/// this starts with [`GitRepo::ensure_matches_workspace_root`], the very
/// first check, before any of the above. Without it, a nested git
/// repository under an initialized workspace would have `blob_oids` hash
/// paths against the *nested* repo's blobs while `lock` was sealed against
/// the outer one, and every sealed path would report as spuriously
/// `Missing` or `Modified` instead of the real problem.
///
/// Deliberately does not parse anything itself -- see the module docs.
pub fn compute_state(
    ws: &Workspace,
    lock: &Lock,
    git: &GitRepo,
    open_changes: &[OpenChangeInfo],
) -> Result<StateReport, TelosError> {
    git.ensure_matches_workspace_root(&ws.repo_root)?;

    let current_spec_files = ws.spec_files()?;

    let mut sealed: BTreeMap<RepoPath, Oid> = lock.spec.clone();
    sealed.extend(
        lock.code
            .iter()
            .map(|(path, oid)| (path.clone(), oid.clone())),
    );

    let mut batch: BTreeSet<RepoPath> = current_spec_files.iter().cloned().collect();
    batch.extend(sealed.keys().cloned());
    let batch: Vec<RepoPath> = batch.into_iter().collect();

    let current_oids = git.blob_oids(&batch)?;

    let mut drift = Vec::new();

    for (path, sealed_oid) in &sealed {
        match current_oids.get(path) {
            None => drift.push(DriftEntry {
                path: path.clone(),
                kind: DriftKind::Missing,
            }),
            Some(current_oid) if current_oid != sealed_oid => drift.push(DriftEntry {
                path: path.clone(),
                kind: DriftKind::Modified,
            }),
            _ => {}
        }
    }

    for path in &current_spec_files {
        if !lock.spec.contains_key(path) {
            drift.push(DriftEntry {
                path: path.clone(),
                kind: DriftKind::Untracked,
            });
        }
    }

    // A path claimed by an open change is work in progress, not drift.
    let claimed: BTreeSet<&RepoPath> = open_changes.iter().flat_map(|c| c.claims.iter()).collect();
    drift.retain(|entry| !claimed.contains(&entry.path));

    drift.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    let state = if !drift.is_empty() {
        ProjectStateKind::Drifted
    } else if !open_changes.is_empty() {
        ProjectStateKind::Changing
    } else {
        ProjectStateKind::Coherent
    };

    let open_changes = open_changes
        .iter()
        .map(|c| ChangeSummary {
            id: c.id,
            status: c.status.as_str().to_string(),
            obligations: c.obligations.clone(),
        })
        .collect();

    Ok(StateReport {
        state,
        drift,
        open_changes,
    })
}

/// A snapshot of how much of the spec has scenarios proved and intents
/// implemented -- the `coverage` object of the `status --json` schema. Every
/// count is exact, computed directly from `model`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub notions: u32,
    pub constraints: u32,
    pub intents_total: u32,
    pub intents_active: u32,
    pub scenarios_total: u32,
    /// Scenarios with at least one `Proves` binding.
    pub scenarios_proved: u32,
    /// Intents with at least one `Implements` binding.
    pub intents_implemented: u32,
}

/// Computes [`Coverage`] for `model`.
pub fn coverage(model: &TelosModel) -> Coverage {
    let scenarios_total: u32 = model
        .intents
        .values()
        .map(|intent| intent.scenarios.len() as u32)
        .sum();

    let proved: BTreeSet<_> = model
        .bindings
        .iter()
        .filter_map(|b| match b {
            Binding::Proves { scenario, .. } => Some(scenario.node),
            Binding::Implements { .. } => None,
        })
        .collect();
    let implemented: BTreeSet<_> = model
        .bindings
        .iter()
        .filter_map(|b| match b {
            Binding::Implements { intent, .. } => Some(intent.node),
            Binding::Proves { .. } => None,
        })
        .collect();

    Coverage {
        notions: if model.domain_notions.is_empty() {
            model.notions.len()
        } else {
            model.domain_notions.len()
        } as u32,
        constraints: model.constraints.len() as u32,
        intents_total: model.intents.len() as u32,
        intents_active: model
            .intents
            .values()
            .filter(|intent| intent.status == IntentStatus::Active)
            .count() as u32,
        scenarios_total,
        scenarios_proved: proved.len() as u32,
        intents_implemented: implemented.len() as u32,
    }
}
