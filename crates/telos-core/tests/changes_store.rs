//! `changes.rs`, the change store, seen from outside the crate: filesystem
//! CRUD over `telos/changes/*.tel` (`list_change_ids`, `read_change`,
//! `write_change`, `delete_change`), and `open_change_infos`'s best-effort
//! scan (D15).

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use telos_core::changes::{
    delete_change, list_change_ids, open_change_infos, read_change, write_change,
};
use telos_core::error::ErrorCode;
use telos_core::ids::{ChangeId, NotionName};
use telos_core::model::{Change, ChangeStatus, Notion, NotionKind, StagedOp};
use telos_core::workspace::Workspace;

// --- fixture plumbing (mirrors tests/counters.rs) --------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

/// Recursively copies every file and subdirectory of `src` into `dst`,
/// creating `dst` (and any nested directory) as needed.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {}: {e}", dst.display()));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display())) {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("copy {}: {e}", entry.path().display()));
        }
    }
}

/// A fresh tempdir holding a copy of the `billing` corpus's `telos/` tree.
fn copied_corpus() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&corpus_root(), tmp.path());
    tmp
}

/// A minimal valid notion, distinct from anything the corpus already
/// declares -- just enough for a `StagedOp::AddNotion` to hang off.
fn ledger_notion() -> Notion {
    Notion {
        name: NotionName::new("Ledger").unwrap(),
        kind: NotionKind::Entity,
        def: "A record of postings.".to_string(),
        attrs: vec![],
        rels: vec![],
    }
}

/// A drafted change with one `add notion` op, at `id`.
fn sample_change(id: u32) -> Change {
    Change {
        id: ChangeId(id),
        motivation: "Introduce the ledger".to_string(),
        status: ChangeStatus::Drafted,
        approved_digest: None,
        ops: vec![StagedOp::AddNotion(ledger_notion())],
    }
}

// --- write / read / delete round trip --------------------------------------

#[test]
fn write_read_delete_round_trips_through_the_store() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let change = sample_change(1);

    write_change(&ws, &change).unwrap();
    assert!(
        tmp.path().join("telos/changes/CHG-0001.tel").is_file(),
        "write_change must create telos/changes/CHG-0001.tel"
    );

    let read_back = read_change(&ws, ChangeId(1)).unwrap();
    assert_eq!(read_back, change);

    delete_change(&ws, ChangeId(1)).unwrap();
    assert!(!tmp.path().join("telos/changes/CHG-0001.tel").exists());

    let err = read_change(&ws, ChangeId(1)).unwrap_err();
    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown change `CHG-0001`");
}

#[test]
fn delete_change_on_an_absent_id_is_the_same_unknown_error_as_read() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    let err = delete_change(&ws, ChangeId(1)).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown change `CHG-0001`");
}

// --- list_change_ids ---------------------------------------------------

#[test]
fn list_change_ids_is_empty_when_the_changes_directory_does_not_exist() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    assert!(
        !tmp.path().join("telos/changes").exists(),
        "the raw corpus fixture must not already carry a telos/changes/ directory"
    );

    assert!(list_change_ids(&ws).unwrap().is_empty());
}

#[test]
fn list_change_ids_sorts_ascending_and_filters_out_counters_toml() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    write_change(&ws, &sample_change(2)).unwrap();
    write_change(&ws, &sample_change(1)).unwrap();
    fs::write(
        tmp.path().join("telos/changes/counters.toml"),
        "intent = 0\nscenario = 0\nconstraint = 0\nchange = 0\n",
    )
    .unwrap();

    assert_eq!(
        list_change_ids(&ws).unwrap(),
        vec![ChangeId(1), ChangeId(2)]
    );
}

// --- read_change: unknown id --------------------------------------------

#[test]
fn read_change_names_the_nearest_existing_id_by_numeric_distance() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    write_change(&ws, &sample_change(2)).unwrap();
    write_change(&ws, &sample_change(9000)).unwrap();

    let err = read_change(&ws, ChangeId(9999)).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.message, "unknown change `CHG-9999`");
    assert_eq!(err.hint.as_deref(), Some("closest is CHG-9000"));
}

#[test]
fn read_change_has_no_hint_when_no_other_change_exists() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    let err = read_change(&ws, ChangeId(1)).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosReferenceUnknown);
    assert_eq!(err.hint, None);
}

// --- read_change: parse diagnostics -------------------------------------

#[test]
fn read_change_converts_parse_diagnostics_via_the_first_diagnostic_policy() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(
        tmp.path().join("telos/changes/CHG-0001.tel"),
        "change CHG-0001 \"x\" {\n  status finished\n}\n",
    )
    .unwrap();

    let err = read_change(&ws, ChangeId(1)).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(
        err.message
            .contains("expected one of `open`, `drafted`, `approved`, `implementing`, `abandoned`"),
        "{}",
        err.message
    );
}

// --- open_change_infos ---------------------------------------------------

#[test]
fn open_change_infos_reports_a_parsed_changes_status_claims_and_obligations() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let change = sample_change(1);
    write_change(&ws, &change).unwrap();

    let infos = open_change_infos(&ws).unwrap();

    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id, ChangeId(1));
    assert_eq!(infos[0].status, ChangeStatus::Drafted);
    assert_eq!(infos[0].claims, change.claims());
    assert_eq!(infos[0].obligations, change.obligations());
}

#[test]
fn open_change_infos_treats_an_unparseable_file_as_a_best_effort_open_change() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(
        tmp.path().join("telos/changes/CHG-0001.tel"),
        b"\x00not even a change file{{{".as_slice(),
    )
    .unwrap();

    let infos = open_change_infos(&ws).unwrap();

    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id, ChangeId(1));
    assert_eq!(infos[0].status, ChangeStatus::Open);
    assert!(infos[0].claims.is_empty());
    assert_eq!(
        infos[0].obligations,
        vec!["repair telos/changes/CHG-0001.tel (unparseable)".to_string()]
    );
}

#[test]
fn open_change_infos_is_empty_when_no_change_is_open() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    assert!(open_change_infos(&ws).unwrap().is_empty());
}
