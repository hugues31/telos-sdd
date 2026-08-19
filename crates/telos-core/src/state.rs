//! Project state (spec §6): comparing a sealed [`Lock`] against the live
//! working tree, and a coverage summary of the spec itself.
//!
//! [`compute_state`] never parses a `.tel` file -- it compares git blob OIDs
//! only, which is exactly what lets it answer even when the spec on disk is
//! syntactically broken. A corrupted spec file *is* drift the caller needs
//! to be told about, not a reason for the check itself to fail.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::TelosError;
use crate::git::{GitRepo, Oid};
use crate::ids::RepoPath;
use crate::lock::Lock;
use crate::model::{Binding, IntentStatus, TelosModel};
use crate::workspace::Workspace;

/// The project's overall state, per spec §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStateKind {
    /// Nothing has drifted and no change is open.
    Coherent,
    /// A change is open. Never produced in M1: `open_changes` is always
    /// empty until M2 wires up the change flow.
    Changing,
    /// At least one sealed path was modified or went missing, or a spec
    /// file exists on disk that was never sealed.
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

/// The result of [`compute_state`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateReport {
    pub state: ProjectStateKind,
    /// Sorted by path.
    pub drift: Vec<DriftEntry>,
    /// Always empty in M1; typed `Vec<ChangeSummary>` from M2 onward.
    pub open_changes: Vec<()>,
}

/// Compares `lock` against the live working tree, by OID only.
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
/// - A code file not in `lock.code` is never drift: unlinked code is free in
///   M1 -- reconciling it against `telos.toml`'s globs is rule 5, an M2
///   reconcile-time concern, not checked here.
///
/// Deliberately does not parse anything -- see the module docs.
pub fn compute_state(
    ws: &Workspace,
    lock: &Lock,
    git: &GitRepo,
) -> Result<StateReport, TelosError> {
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

    drift.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    // Always empty in M1: no change flow exists yet to populate it.
    let open_changes: Vec<()> = Vec::new();

    let state = if !drift.is_empty() {
        ProjectStateKind::Drifted
    } else if !open_changes.is_empty() {
        ProjectStateKind::Changing
    } else {
        ProjectStateKind::Coherent
    };

    Ok(StateReport {
        state,
        drift,
        open_changes,
    })
}

/// A snapshot of how much of the spec has scenarios proved and intents
/// implemented -- the `coverage` object of the `status --json` schema
/// (Annex B). Every count is exact, computed directly off `model`.
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
        notions: model.notions.len() as u32,
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
