#![cfg_attr(not(test), allow(dead_code))]

use serde::Serialize;

use super::model::ViewSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DataMode {
    Live,
    Export,
}

#[derive(Serialize)]
struct DataEnvelope<'a> {
    meta: DataMeta,
    snapshot: &'a ViewSnapshot,
}

#[derive(Serialize)]
struct DataMeta {
    version: &'static str,
    build_date: &'static str,
    mode: DataMode,
}

pub(crate) fn data_js(snapshot: &ViewSnapshot, mode: DataMode) -> String {
    let envelope = DataEnvelope {
        meta: DataMeta {
            version: env!("CARGO_PKG_VERSION"),
            build_date: env!("TELOS_BUILD_DATE"),
            mode,
        },
        snapshot,
    };
    let json = serde_json::to_string(&envelope)
        .expect("ViewSnapshot and its metadata serialize to JSON")
        .replace('<', r"\u003c")
        .replace('\u{2028}', r"\u2028")
        .replace('\u{2029}', r"\u2029");
    format!("window.__TELOS_DATA__ = {json};\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;
    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::{DataMode, data_js};
    use crate::view::model::ViewSnapshot;

    const PREFIX: &str = "window.__TELOS_DATA__ = ";
    const SUFFIX: &str = ";\n";

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

    fn payload(script: &str) -> Value {
        assert!(script.starts_with(PREFIX));
        assert!(script.ends_with(SUFFIX));
        serde_json::from_str(&script[PREFIX.len()..script.len() - SUFFIX.len()]).unwrap()
    }

    #[test]
    fn data_script_has_exact_assignment_shape_and_live_metadata() {
        let script = data_js(&fixture_snapshot(), DataMode::Live);
        let value = payload(&script);

        assert_eq!(value["meta"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["meta"]["build_date"], env!("TELOS_BUILD_DATE"));
        assert_eq!(value["meta"]["mode"], "live");
        assert_eq!(value["snapshot"]["dashboard"]["state"], "coherent");
        assert_eq!(value["snapshot"]["intents"][0]["id"], "INT-0017");
        assert_eq!(
            value["snapshot"]["intents"][0]["statement"]["template"],
            "event-driven"
        );
        assert!(
            value["snapshot"]["intents"][0]["statement"]["canonical"]
                .as_str()
                .unwrap()
                .contains("statement event-driven")
        );
        assert!(
            value["snapshot"]["scenarios"][0]["canonical"]
                .as_str()
                .unwrap()
                .contains("scenario SCN-0091")
        );
    }

    #[test]
    fn data_script_serializes_export_mode() {
        let value = payload(&data_js(&fixture_snapshot(), DataMode::Export));

        assert_eq!(value["meta"]["mode"], "export");
    }

    #[test]
    fn build_date_is_iso_utc_and_matches_the_behavioral_build_expectation() {
        let build_date = env!("TELOS_BUILD_DATE");
        assert_eq!(build_date.len(), 10);
        assert_eq!(&build_date[4..5], "-");
        assert_eq!(&build_date[7..8], "-");
        assert!(
            build_date
                .chars()
                .enumerate()
                .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
        );

        let Ok(expected) = std::env::var("EXPECTED_TELOS_BUILD_DATE") else {
            return;
        };

        assert_eq!(build_date, expected);
    }

    #[test]
    fn hostile_fields_are_script_safe_and_round_trip() {
        let mut snapshot = fixture_snapshot();
        let hostile = "</script>\u{2028}left\u{2029}right";
        snapshot.intents[0].title = hostile.to_string();

        let script = data_js(&snapshot, DataMode::Live);
        assert!(!script.contains('<'));
        assert!(!script.contains('\u{2028}'));
        assert!(!script.contains('\u{2029}'));
        assert!(script.contains(r"\u003c/script>"));
        assert!(script.contains(r"\u2028"));
        assert!(script.contains(r"\u2029"));

        let value = payload(&script);
        assert_eq!(value["snapshot"]["intents"][0]["title"], hostile);
    }
}
