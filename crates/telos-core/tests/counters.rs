//! `counters.rs`, end to end against a copy of the `billing` corpus and
//! real filesystem I/O: `read_counters`/`write_counters`'s exact bytes and
//! roundtrip, `floors` over the real corpus, and the corpus-derived `Alloc`
//! sequence the task brief pins.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use telos_core::counters::{Alloc, Counters, floors, read_counters, write_counters};
use telos_core::error::ErrorCode;
use telos_core::ids::{ChangeId, ConstraintId, IntentId, ScenarioId};
use telos_core::workspace::Workspace;

// --- fixture plumbing --------------------------------------------------

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

// --- read_counters -------------------------------------------------------

#[test]
fn read_counters_is_all_zeros_when_the_file_is_absent() {
    let tmp = copied_corpus();
    assert!(
        !tmp.path().join("telos/changes/counters.toml").exists(),
        "the corpus fixture must not already carry a counters.toml"
    );
    let ws = Workspace::discover(tmp.path()).unwrap();

    assert_eq!(read_counters(&ws).unwrap(), Counters::default());
}

#[test]
fn read_counters_reports_a_parse_error_naming_the_file() {
    let tmp = copied_corpus();
    fs::create_dir_all(tmp.path().join("telos/changes")).unwrap();
    fs::write(
        tmp.path().join("telos/changes/counters.toml"),
        "this is not valid toml [[[",
    )
    .unwrap();
    let ws = Workspace::discover(tmp.path()).unwrap();

    let err = read_counters(&ws).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(
        err.message.contains("counters.toml"),
        "expected the message to name counters.toml, got: {}",
        err.message
    );
}

// --- write_counters --------------------------------------------------------

#[test]
fn write_counters_produces_the_exact_deterministic_bytes() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();

    write_counters(&ws, &Counters::default()).unwrap();

    assert_eq!(
        fs::read_to_string(tmp.path().join("telos/changes/counters.toml")).unwrap(),
        "intent = 0\nscenario = 0\nconstraint = 0\nchange = 0\n"
    );
}

#[test]
fn write_counters_creates_the_changes_directory_when_it_is_missing() {
    let tmp = copied_corpus();
    assert!(
        !tmp.path().join("telos/changes").exists(),
        "the raw corpus fixture must not already carry a telos/changes/ directory"
    );
    let ws = Workspace::discover(tmp.path()).unwrap();

    write_counters(&ws, &Counters::default()).unwrap();

    assert!(tmp.path().join("telos/changes/counters.toml").is_file());
}

#[test]
fn write_counters_then_read_counters_round_trips() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let c = Counters {
        intent: 43,
        scenario: 108,
        constraint: 4,
        change: 1,
    };

    write_counters(&ws, &c).unwrap();

    assert_eq!(read_counters(&ws).unwrap(), c);
}

// --- floors ------------------------------------------------------------

#[test]
fn floors_of_the_corpus_matches_the_expected_high_water_marks() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));

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

// --- Alloc, chained from the corpus's real floor ------------------------

#[test]
fn alloc_from_the_corpus_floor_allocates_ids_above_every_existing_one() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));

    let floor = floors(&model, &[], None);
    let mut alloc = Alloc::new(Counters::default(), floor);

    assert_eq!(alloc.next_intent(), IntentId(43));
    assert_eq!(alloc.next_scenario(), ScenarioId(108));
    assert_eq!(alloc.next_constraint(), ConstraintId(4));
    assert_eq!(alloc.next_change(), ChangeId(1));
}

#[test]
fn a_persisted_counter_above_the_corpus_floor_wins() {
    let tmp = copied_corpus();
    let ws = Workspace::discover(tmp.path()).unwrap();
    let model = ws
        .load_model()
        .unwrap_or_else(|diags| panic!("expected the corpus to load cleanly, got {diags:?}"));

    let floor = floors(&model, &[], None);
    let persisted = Counters {
        intent: 50,
        ..Counters::default()
    };
    let mut alloc = Alloc::new(persisted, floor);

    assert_eq!(alloc.next_intent(), IntentId(51));
}
