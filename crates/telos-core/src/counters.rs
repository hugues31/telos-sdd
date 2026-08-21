//! Persistent per-kind id counters and the never-reuse allocator built on
//! top of them.
//!
//! `telos/changes/counters.toml` is a CLI-managed side file: it lives under
//! `telos/changes/`, a directory [`Workspace::spec_files`] never scans, so
//! it sits outside the seal entirely and its churn never touches
//! `telos.lock`. It exists purely to remember the highest id of each kind
//! ever handed out, so a fresh allocation can never repeat one.
//!
//! That alone is not enough: a bad merge resolution can roll the file back
//! below what the spec (or an open change) already contains. [`floors`]
//! recomputes a *floor* -- the highest id actually in use, scanned fresh
//! from the sealed model and every open change -- and [`Alloc`] starts from
//! `max(persisted, floor)`, so the persisted file is only ever a fast path,
//! never a single point of truth: the next allocation self-heals a stale or
//! missing counters file for free.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{ErrorCode, TelosError};
use crate::ids::{ChangeId, ConstraintId, IntentId, ScenarioId};
use crate::model::{Change, StagedOp, TelosModel};
use crate::workspace::Workspace;

/// The four persisted high-water marks, one per id kind. Never decremented:
/// an id, once handed out, is never handed out again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    pub intent: u32,
    pub scenario: u32,
    pub constraint: u32,
    pub change: u32,
}

/// Reads `<telos_dir>/changes/counters.toml`.
///
/// A missing file is not an error -- it is the state of a project whose
/// counters were never persisted yet -- and reads back as all zeros. A
/// present but unparsable file is `TelosParseError`, naming the file.
pub fn read_counters(ws: &Workspace) -> Result<Counters, TelosError> {
    let path = counters_path(ws);
    let src = match fs::read_to_string(&path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Counters::default()),
        Err(e) => {
            return Err(TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to read {}: {e}", path.display()),
            ));
        }
    };

    let raw: RawCounters = toml::from_str(&src).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("{}: {e}", path.display()),
        )
    })?;

    Ok(Counters {
        intent: raw.intent,
        scenario: raw.scenario,
        constraint: raw.constraint,
        change: raw.change,
    })
}

/// Writes `c` to `<telos_dir>/changes/counters.toml` as deterministic,
/// hand-rendered TOML -- fixed key order (`intent`, `scenario`,
/// `constraint`, `change`), one `key = value` line per counter, LF line
/// endings, exactly one trailing newline and nothing besides. Creates
/// `telos/changes/` first if it does not exist yet: `telos init` creates it
/// up front, but nothing else requires that to already be true here.
pub fn write_counters(ws: &Workspace, c: &Counters) -> Result<(), TelosError> {
    let dir = changes_dir(ws);
    fs::create_dir_all(&dir).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to create {}: {e}", dir.display()),
        )
    })?;

    let path = counters_path(ws);
    let content = format!(
        "intent = {}\nscenario = {}\nconstraint = {}\nchange = {}\n",
        c.intent, c.scenario, c.constraint, c.change
    );
    fs::write(&path, content).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to write {}: {e}", path.display()),
        )
    })
}

/// The highest id of each kind actually in use: the sealed model's own
/// entities, plus every intent or constraint an open change's ops would add
/// or edit (and that intent's scenarios), plus -- for changes -- the ids of
/// the open changes themselves and the change that produced the current
/// seal.
///
/// A `remove`/`accept` op names an id (or a path) that, while the change
/// remains open, is still live in the sealed model -- so it is already
/// covered by the model scan and does not need its own case. Notion ops
/// never touch a counter: notions are named, not numbered.
///
/// Deliberately independent of the persisted [`Counters`] file: it is the
/// ground truth [`Alloc::new`] maxes the persisted value against, so a
/// stale or missing counters file can never cause an id to be reissued.
pub fn floors(model: &TelosModel, open: &[Change], sealed_by: Option<ChangeId>) -> Counters {
    let mut floor = Counters::default();

    for id in model.intents.keys() {
        floor.intent = floor.intent.max(id.0);
    }
    for id in model.scenario_owner.keys() {
        floor.scenario = floor.scenario.max(id.0);
    }
    for id in model.constraints.keys() {
        floor.constraint = floor.constraint.max(id.0);
    }

    for change in open {
        floor.change = floor.change.max(change.id.0);
        for op in &change.ops {
            match op {
                StagedOp::AddIntent(intent) | StagedOp::EditIntent(intent) => {
                    floor.intent = floor.intent.max(intent.id.0);
                    for scenario in &intent.scenarios {
                        floor.scenario = floor.scenario.max(scenario.id.0);
                    }
                }
                StagedOp::AddConstraint(constraint) | StagedOp::EditConstraint(constraint) => {
                    floor.constraint = floor.constraint.max(constraint.id.0);
                }
                StagedOp::AddNotion(_)
                | StagedOp::EditNotion(_)
                | StagedOp::RemoveNotion(_)
                | StagedOp::RemoveIntent(_)
                | StagedOp::RemoveConstraint(_)
                | StagedOp::EditConfig(_)
                | StagedOp::Accept { .. } => {}
            }
        }
    }

    if let Some(id) = sealed_by {
        floor.change = floor.change.max(id.0);
    }

    floor
}

/// A live id allocator: starts at `max(persisted, floor)` component-wise
/// ([`Alloc::new`]), then hands out ids by incrementing first and returning
/// the new value -- so after a floor of 42, the first `next_intent()`
/// returns `INT-0043`, never `INT-0042`.
#[derive(Debug, Clone, Copy)]
pub struct Alloc {
    counters: Counters,
}

impl Alloc {
    /// `max(persisted, floor)`, component-wise.
    pub fn new(persisted: Counters, floor: Counters) -> Alloc {
        Alloc {
            counters: Counters {
                intent: persisted.intent.max(floor.intent),
                scenario: persisted.scenario.max(floor.scenario),
                constraint: persisted.constraint.max(floor.constraint),
                change: persisted.change.max(floor.change),
            },
        }
    }

    pub fn next_intent(&mut self) -> IntentId {
        self.counters.intent += 1;
        IntentId(self.counters.intent)
    }

    pub fn next_scenario(&mut self) -> ScenarioId {
        self.counters.scenario += 1;
        ScenarioId(self.counters.scenario)
    }

    pub fn next_constraint(&mut self) -> ConstraintId {
        self.counters.constraint += 1;
        ConstraintId(self.counters.constraint)
    }

    pub fn next_change(&mut self) -> ChangeId {
        self.counters.change += 1;
        ChangeId(self.counters.change)
    }

    /// The current counters, ready to persist via [`write_counters`].
    pub fn counters(&self) -> Counters {
        self.counters
    }
}

fn changes_dir(ws: &Workspace) -> PathBuf {
    ws.telos_dir.join("changes")
}

fn counters_path(ws: &Workspace) -> PathBuf {
    changes_dir(ws).join("counters.toml")
}

/// The shape `toml::from_str` deserializes into. Every field defaults to
/// `0` when absent, so a hand-edited file that only overrides one counter
/// still reads back sanely instead of failing to parse.
#[derive(Debug, Deserialize)]
struct RawCounters {
    #[serde(default)]
    intent: u32,
    #[serde(default)]
    scenario: u32,
    #[serde(default)]
    constraint: u32,
    #[serde(default)]
    change: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Oid;
    use crate::ids::{NotionName, RepoPath};
    use crate::model::ChangeStatus;
    use crate::model::change::fixtures::invoice;
    use crate::model::{
        Action, Constraint, ConstraintKind, InstanceStep, Intent, IntentStatus, Rule, Scenario,
        Scope, Statement,
    };
    use crate::span::{Sp, Span};

    // --- test helpers ------------------------------------------------------

    fn sp<T>(node: T) -> Sp<T> {
        Sp {
            node,
            span: Span::default(),
        }
    }

    fn notion_name(s: &str) -> NotionName {
        NotionName::new(s).unwrap()
    }

    fn scenario(id: u32) -> Scenario {
        Scenario {
            id: ScenarioId(id),
            title: "sample".to_string(),
            given: vec![],
            when: InstanceStep {
                notion: sp(notion_name("Paid")),
                fields: vec![],
            },
            then: vec![],
        }
    }

    fn intent(id: u32, scenarios: Vec<Scenario>) -> Intent {
        Intent {
            id: IntentId(id),
            title: "sample intent".to_string(),
            status: IntentStatus::Active,
            telos: "because".to_string(),
            statement: Statement::Ubiquitous {
                action: Action::Free("do it".to_string()),
            },
            refines: vec![],
            requires: vec![],
            excludes: vec![],
            scenarios,
        }
    }

    fn constraint(id: u32) -> Constraint {
        Constraint {
            id: ConstraintId(id),
            kind: ConstraintKind::Architecture,
            title: "sample constraint".to_string(),
            rule: Rule::Text("keep it clean".to_string()),
            scope: Scope::Global,
            check: None,
        }
    }

    fn model_with(intents: Vec<Intent>, constraints: Vec<Constraint>) -> TelosModel {
        let mut model = TelosModel::default();
        for intent in intents {
            for scenario in &intent.scenarios {
                model.scenario_owner.insert(scenario.id, intent.id);
            }
            model.intents.insert(intent.id, intent);
        }
        for constraint in constraints {
            model.constraints.insert(constraint.id, constraint);
        }
        model
    }

    fn drafted_change(id: u32, ops: Vec<StagedOp>) -> Change {
        Change {
            id: ChangeId(id),
            motivation: "x".to_string(),
            status: ChangeStatus::Drafted,
            approved_digest: None,
            ops,
            journal: vec![],
        }
    }

    // --- floors: the sealed model -------------------------------------------

    #[test]
    fn floors_of_an_empty_model_and_no_open_changes_is_all_zero() {
        let model = TelosModel::default();
        assert_eq!(floors(&model, &[], None), Counters::default());
    }

    #[test]
    fn floors_scans_the_highest_intent_scenario_and_constraint_id_in_the_model() {
        let model = model_with(
            vec![
                intent(17, vec![scenario(91)]),
                intent(42, vec![scenario(107)]),
            ],
            vec![constraint(3)],
        );
        assert_eq!(
            floors(&model, &[], None),
            Counters {
                intent: 42,
                scenario: 107,
                constraint: 3,
                change: 0,
            }
        );
    }

    // --- floors: open changes' ops ------------------------------------------

    #[test]
    fn floors_scans_the_intent_and_scenarios_of_an_add_intent_op() {
        let model = TelosModel::default();
        let change = drafted_change(
            5,
            vec![StagedOp::AddIntent(intent(99, vec![scenario(500)]))],
        );

        let floor = floors(&model, &[change], None);

        assert_eq!(floor.intent, 99);
        assert_eq!(floor.scenario, 500);
        assert_eq!(floor.change, 5);
    }

    #[test]
    fn floors_scans_the_intent_and_scenarios_of_an_edit_intent_op() {
        let model = TelosModel::default();
        let change = drafted_change(
            1,
            vec![StagedOp::EditIntent(intent(60, vec![scenario(300)]))],
        );

        let floor = floors(&model, &[change], None);

        assert_eq!(floor.intent, 60);
        assert_eq!(floor.scenario, 300);
    }

    #[test]
    fn floors_scans_constraints_added_and_edited_by_open_changes() {
        let model = TelosModel::default();
        let added = drafted_change(1, vec![StagedOp::AddConstraint(constraint(12))]);
        let edited = drafted_change(2, vec![StagedOp::EditConstraint(constraint(20))]);

        let floor = floors(&model, &[added, edited], None);

        assert_eq!(floor.constraint, 20);
        assert_eq!(floor.change, 2);
    }

    #[test]
    fn floors_ignores_notion_remove_and_accept_ops() {
        let model = TelosModel::default();
        let change = drafted_change(
            9,
            vec![
                StagedOp::AddNotion(invoice()),
                StagedOp::RemoveNotion(notion_name("Invoice")),
                StagedOp::RemoveIntent(IntentId(999)),
                StagedOp::RemoveConstraint(ConstraintId(999)),
                StagedOp::Accept {
                    path: RepoPath::new("telos/telos.toml"),
                    oid: Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
                },
            ],
        );

        let floor = floors(&model, &[change], None);

        assert_eq!(
            floor,
            Counters {
                intent: 0,
                scenario: 0,
                constraint: 0,
                change: 9,
            }
        );
    }

    // --- floors: the change floor's two other sources -----------------------

    #[test]
    fn floors_includes_sealed_by_in_the_change_floor() {
        let model = TelosModel::default();
        assert_eq!(floors(&model, &[], Some(ChangeId(4))).change, 4);
    }

    #[test]
    fn floors_takes_the_max_of_open_change_ids_and_sealed_by() {
        let model = TelosModel::default();
        let change = drafted_change(2, vec![]);

        assert_eq!(
            floors(&model, std::slice::from_ref(&change), Some(ChangeId(10))).change,
            10
        );
        assert_eq!(floors(&model, &[change], Some(ChangeId(1))).change, 2);
    }

    // --- Alloc ---------------------------------------------------------------

    #[test]
    fn alloc_new_takes_the_component_wise_max_of_persisted_and_floor() {
        let persisted = Counters {
            intent: 50,
            scenario: 10,
            constraint: 1,
            change: 0,
        };
        let floor = Counters {
            intent: 42,
            scenario: 107,
            constraint: 3,
            change: 5,
        };
        let alloc = Alloc::new(persisted, floor);
        assert_eq!(
            alloc.counters(),
            Counters {
                intent: 50,
                scenario: 107,
                constraint: 3,
                change: 5,
            }
        );
    }

    #[test]
    fn next_intent_increments_before_returning() {
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: 42,
                ..Counters::default()
            },
        );
        assert_eq!(alloc.next_intent(), IntentId(43));
        assert_eq!(alloc.next_intent(), IntentId(44));
    }

    /// From the corpus's floor, one `next_*` call
    /// per kind, each landing exactly one past the floor.
    #[test]
    fn next_functions_each_allocate_their_own_component_past_the_corpus_floor() {
        let mut alloc = Alloc::new(
            Counters::default(),
            Counters {
                intent: 42,
                scenario: 107,
                constraint: 3,
                change: 0,
            },
        );
        assert_eq!(alloc.next_intent(), IntentId(43));
        assert_eq!(alloc.next_scenario(), ScenarioId(108));
        assert_eq!(alloc.next_constraint(), ConstraintId(4));
        assert_eq!(alloc.next_change(), ChangeId(1));
    }

    #[test]
    fn a_persisted_counter_above_the_floor_wins() {
        let mut alloc = Alloc::new(
            Counters {
                intent: 50,
                ..Counters::default()
            },
            Counters {
                intent: 42,
                ..Counters::default()
            },
        );
        assert_eq!(alloc.next_intent(), IntentId(51));
    }

    #[test]
    fn counters_reflects_allocations_made_so_far() {
        let mut alloc = Alloc::new(Counters::default(), Counters::default());
        alloc.next_intent();
        alloc.next_scenario();
        assert_eq!(
            alloc.counters(),
            Counters {
                intent: 1,
                scenario: 1,
                constraint: 0,
                change: 0,
            }
        );
    }
}
