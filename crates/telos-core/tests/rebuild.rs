use telos_core::ids::IntentId;
use telos_core::model::{Action, Intent, IntentStatus, Statement, TelosModel};
use telos_core::rebuild::{RebuildStep, plan};
use telos_core::span::{Sp, Span};

fn intent(id: u32, status: IntentStatus, requires: &[u32]) -> Intent {
    Intent {
        id: IntentId(id),
        title: format!("Intent {id}"),
        status,
        telos: format!("Purpose {id}"),
        statement: Statement::Ubiquitous {
            action: Action::Free(format!("implement {id}")),
        },
        refines: Vec::new(),
        requires: requires
            .iter()
            .map(|id| Sp {
                node: IntentId(*id),
                span: Span::default(),
            })
            .collect(),
        excludes: Vec::new(),
        scenarios: Vec::new(),
    }
}

fn model(intents: Vec<Intent>) -> TelosModel {
    let mut model = TelosModel::default();
    for intent in intents {
        model.intents.insert(intent.id, intent);
    }
    model
}

fn ids(steps: &[RebuildStep]) -> Vec<IntentId> {
    steps.iter().map(|step| step.intent).collect()
}

#[test]
fn independent_intents_use_ascending_ids_and_include_every_status() {
    let model = model(vec![
        intent(30, IntentStatus::Deprecated, &[]),
        intent(10, IntentStatus::Draft, &[]),
        intent(20, IntentStatus::Active, &[]),
    ]);

    assert_eq!(
        ids(&plan(&model)),
        [IntentId(10), IntentId(20), IntentId(30)]
    );
}

#[test]
fn a_chain_places_each_prerequisite_before_its_dependent() {
    let model = model(vec![
        intent(1, IntentStatus::Active, &[]),
        intent(2, IntentStatus::Active, &[1]),
        intent(3, IntentStatus::Active, &[2]),
    ]);

    assert_eq!(ids(&plan(&model)), [IntentId(1), IntentId(2), IntentId(3)]);
}

#[test]
fn a_diamond_is_prerequisite_first_with_ascending_ties_and_sorted_direct_requires() {
    let model = model(vec![
        intent(4, IntentStatus::Active, &[3, 2]),
        intent(2, IntentStatus::Active, &[1]),
        intent(3, IntentStatus::Active, &[1]),
        intent(1, IntentStatus::Active, &[]),
    ]);

    let steps = plan(&model);

    assert_eq!(
        ids(&steps),
        [IntentId(1), IntentId(2), IntentId(3), IntentId(4)]
    );
    assert_eq!(steps[0].requires, []);
    assert_eq!(steps[1].requires, [IntentId(1)]);
    assert_eq!(steps[2].requires, [IntentId(1)]);
    assert_eq!(steps[3].requires, [IntentId(2), IntentId(3)]);
}
