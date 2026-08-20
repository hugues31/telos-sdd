//! The witness protocol: `scn_NNNN` test discovery (D4), and the two
//! verdicts M3 builds on top of the journal (D7) -- whether one scenario's
//! red/green pair is still valid against the current bytes
//! ([`witness_verdict`]), and which scenarios a change's post-model owes a
//! witness to in the first place ([`required_witnesses`]).
//!
//! Both verdict functions are pure: neither touches the filesystem or git.
//! [`witness_verdict`] is handed the journal it must read and the current
//! OIDs it must compare against (the caller -- T5's reconcile gate -- is the
//! one that knows how to compute those); [`required_witnesses`] is handed
//! the base and post spec state directly. [`find_test_for`] is the one
//! function here that does I/O, since discovering a test file is
//! necessarily a question about the working tree.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::fs;

use crate::emit::emit_scenario_fragment;
use crate::error::{ErrorCode, TelosError};
use crate::git::Oid;
use crate::globs::glob_matches;
use crate::ids::{IntentId, RepoPath, ScenarioId};
use crate::model::{
    Intent, IntentStatus, JournalEntry, StagedOp, TelFile, TelosModel, TestRef, TestRun, Witness,
};
use crate::workspace::Workspace;

/// Whether one scenario's witness is still trustworthy at reconcile time
/// (D7). Read by reconcile's witness gate; the message
/// [`WitnessVerdict::Sealed`] carries
/// is the frozen wording of Annex F, minus the hint (which is fixed and
/// belongs to the caller that turns this into a [`TelosError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessVerdict {
    /// A red witness at the current oid, followed by a green run at the
    /// same oid: the pair proves the current bytes.
    Intact,
    /// No red run was ever recorded for this scenario.
    MissingRed,
    /// A red witness stands at the current oid, but no green run followed
    /// it on the same bytes -- implementation has not proven it yet.
    MissingGreen,
    /// A red witness was sealed, but the bytes moved since: the test file
    /// changed (or a green ran on different bytes than its red), or the
    /// file no longer exists.
    Sealed(String),
}

/// `format!("scn_{:04}", id.0)` -- the D4 naming convention a scenario's
/// test is discovered by. Grows past four digits rather than truncating,
/// matching the id types' own `Display` (`ids.rs`'s `entity_id!`).
pub fn scenario_pattern(id: ScenarioId) -> String {
    format!("scn_{:04}", id.0)
}

/// Discovers the test file (and, when found by scanning, the test function)
/// for one scenario (D4).
///
/// `file` explicit wins outright: it must exist, and is scanned only to
/// pick up a `name` if the pattern happens to appear in it -- its absence
/// from an explicit file is not an error, since `--file` *is* the filter
/// (Annex F names this "the whole file is the filter"). Without `file`,
/// every path [`glob_matches`] returns for `[tests] globs` is scanned for
/// the scenario's `scn_NNNN` pattern as a raw byte substring (CRLF-
/// insensitive by construction, since the pattern itself holds no `\r` or
/// `\n`) that starts an identifier -- `descn_0001x` does not count, only an
/// occurrence not itself preceded by an `[A-Za-z0-9_]` byte does (see
/// [`identifier_at`]); zero or more than one match is `TelosTestNotFound`,
/// worded exactly as Annex F freezes it. Note that a configured runner is
/// *not* checked here -- that gate belongs to the caller (`telos test`,
/// T3).
pub fn find_test_for(
    ws: &Workspace,
    id: ScenarioId,
    file: Option<&RepoPath>,
) -> Result<TestRef, TelosError> {
    let pattern = scenario_pattern(id);

    if let Some(path) = file {
        if ws.read_optional_bytes(path)?.is_none() {
            return Err(TelosError::new(
                ErrorCode::TelosTestNotFound,
                format!("the file passed with --file does not exist: `{path}`"),
            ));
        }
        let bytes = read_bytes(ws, path)?;
        let name = identifier_at(&bytes, pattern.as_bytes());
        return Ok(TestRef {
            path: path.clone(),
            name,
        });
    }

    let candidates = glob_matches(&ws.repo_root, &ws.config.tests.globs)?;
    let mut hits: Vec<(RepoPath, String)> = Vec::new();
    for path in candidates {
        let bytes = read_bytes(ws, &path)?;
        if let Some(name) = identifier_at(&bytes, pattern.as_bytes()) {
            hits.push((path, name));
        }
    }

    match hits.len() {
        0 => Err(no_match_error(&pattern)),
        1 => {
            let (path, name) = hits.into_iter().next().expect("len checked to be 1");
            Ok(TestRef {
                path,
                name: Some(name),
            })
        }
        _ => Err(multiple_match_error(&pattern, &hits)),
    }
}

/// Reads `path` (relative to `ws.repo_root`) as raw bytes, `TelosInternal`
/// on any I/O failure -- discovery reads whatever `[tests] globs` matched,
/// and a file vanishing between the glob walk and this read is a filesystem
/// race, not a modelling question.
fn read_bytes(ws: &Workspace, path: &RepoPath) -> Result<Vec<u8>, TelosError> {
    ws.read_bytes(path)
}

/// Finds `pattern` as a raw byte substring of `bytes`, at an *identifier
/// boundary* -- the byte immediately before the match, if any, must not
/// itself be `[A-Za-z0-9_]` -- and, if found, returns the longest
/// `[A-Za-z0-9_]*` run starting at the match: the identifier the pattern is
/// a prefix of (e.g. `scn_0001_settles` out of `fn scn_0001_settles()`).
///
/// The boundary check is what keeps `descn_0001x` or `xscn_0001` from
/// counting as a match: the pattern occurs inside those bytes, but not at
/// the start of an identifier, so neither discovery nor name extraction may
/// treat it as one -- the two are one decision, not two. `None` when no
/// boundary-respecting occurrence exists.
fn identifier_at(bytes: &[u8], pattern: &[u8]) -> Option<String> {
    if pattern.is_empty() || bytes.len() < pattern.len() {
        return None;
    }
    for start in 0..=(bytes.len() - pattern.len()) {
        if bytes[start..start + pattern.len()] != *pattern {
            continue;
        }
        let preceded_by_identifier_byte = start > 0 && is_identifier_byte(bytes[start - 1]);
        if preceded_by_identifier_byte {
            continue;
        }
        let mut end = start + pattern.len();
        while end < bytes.len() && is_identifier_byte(bytes[end]) {
            end += 1;
        }
        return Some(String::from_utf8_lossy(&bytes[start..end]).into_owned());
    }
    None
}

fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The frozen `TELOS_TEST_NOT_FOUND` message and hint for zero matches
/// (Annex F).
fn no_match_error(pattern: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosTestNotFound,
        format!("no file matched by the [tests] globs contains `{pattern}`"),
    )
    .hint(format!(
        "name the test after the scenario id (`{pattern}_\u{2026}`) in a file the [tests] \
         globs cover, or pass `--file <path>`"
    ))
}

/// The frozen `TELOS_TEST_NOT_FOUND` message and hint for more than one
/// match (Annex F): the files are listed backtick-quoted, in the order
/// [`glob_matches`] returned them (already sorted).
fn multiple_match_error(pattern: &str, hits: &[(RepoPath, String)]) -> TelosError {
    let list: Vec<String> = hits.iter().map(|(path, _)| format!("`{path}`")).collect();
    TelosError::new(
        ErrorCode::TelosTestNotFound,
        format!(
            "`{pattern}` appears in more than one test file: {}",
            list.join(", ")
        ),
    )
    .hint("pass `--file <path>` to pick one")
}

/// Whether scenario `scenario`'s witness (all its runs, in journal order) is
/// still valid against `current` (D7).
///
/// The read is: find the *last* red run whose recorded oid still matches
/// `current`'s oid for its test path (an absent path means the file is
/// gone). No red at all is [`WitnessVerdict::MissingRed`]; reds exist but
/// none matches `current` is [`WitnessVerdict::Sealed`], reported against
/// the most recent red attempt. Once a current red is found, every later
/// green run on the *same path* is considered together, not just the first:
/// if any of them shares the red's oid, that pair proves the current bytes
/// and the verdict is [`WitnessVerdict::Intact`] regardless of what other,
/// differently-`oid`'d greens sit between the red and it (a green re-run on
/// bytes that later moved back to the red's does not un-prove the pair).
/// Only when *no* later green shares the red's oid does an earlier,
/// different-oid one count: [`WitnessVerdict::Sealed`], since it was taken
/// on bytes the current red does not stand for. No later green at all on
/// that path is [`WitnessVerdict::MissingGreen`] -- also what a
/// red-green-red cycle collapses to, since the second red becomes the
/// current one and nothing follows it.
pub fn witness_verdict(
    journal: &[JournalEntry],
    scenario: ScenarioId,
    current: &BTreeMap<RepoPath, Oid>,
) -> WitnessVerdict {
    let runs: Vec<&TestRun> = journal
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Run(run) if run.scenario == scenario => Some(run),
            _ => None,
        })
        .collect();

    let red_indices: Vec<usize> = runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.witness == Witness::Red)
        .map(|(i, _)| i)
        .collect();

    let Some(&last_red_idx) = red_indices.last() else {
        return WitnessVerdict::MissingRed;
    };

    let current_red_idx = red_indices
        .iter()
        .rev()
        .copied()
        .find(|&i| current.get(&runs[i].test.path) == Some(&runs[i].oid));

    let Some(red_idx) = current_red_idx else {
        let last_red = runs[last_red_idx];
        return WitnessVerdict::Sealed(sealed_message(scenario, &last_red.test.path, current));
    };

    let red = runs[red_idx];
    let later_same_path_greens: Vec<&TestRun> = runs[red_idx + 1..]
        .iter()
        .copied()
        .filter(|run| run.witness == Witness::Green && run.test.path == red.test.path)
        .collect();

    if later_same_path_greens
        .iter()
        .any(|green| green.oid == red.oid)
    {
        WitnessVerdict::Intact
    } else if later_same_path_greens.is_empty() {
        WitnessVerdict::MissingGreen
    } else {
        WitnessVerdict::Sealed(sealed_message(scenario, &red.test.path, current))
    }
}

/// The frozen `TELOS_TEST_SEALED` message (Annex F), chosen by whether
/// `path` is still present in `current`: absent means the file is gone,
/// present with a different oid means it changed.
fn sealed_message(
    scenario: ScenarioId,
    path: &RepoPath,
    current: &BTreeMap<RepoPath, Oid>,
) -> String {
    if current.contains_key(path) {
        format!("the test file `{path}` changed after the red witness for {scenario} was sealed")
    } else {
        format!("the test file `{path}` sealed for {scenario} no longer exists")
    }
}

/// Which scenarios a change's post-model owes a witness to (D7 scope).
///
/// Only `add`/`edit intent` ops contribute, and only when the *post* status
/// of the intent they target (read from `post`, the folded model) is
/// `active` -- a draft intent, or one an op does not touch at all, owes
/// nothing. For each such intent, a scenario is required when it is absent
/// from `base` (by id) or present but its canonical emission
/// ([`emit_scenario_fragment`]) differs from the base's -- comparison is by
/// emission, never by structural equality, because two scenarios parsed
/// from different source strings never carry equal spans even when nothing
/// about them changed (this is what keeps `loop_merge`, which edits an
/// intent's `telos` without touching its scenarios, from demanding a
/// witness it has no reason to owe). The result is sorted and deduplicated.
pub fn required_witnesses(
    base: &[(RepoPath, TelFile)],
    post: &TelosModel,
    ops: &[StagedOp],
) -> Vec<ScenarioId> {
    let mut required: BTreeSet<ScenarioId> = BTreeSet::new();

    for op in ops {
        let intent_id = match op {
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => i.id,
            _ => continue,
        };
        let Some(post_intent) = post.intents.get(&intent_id) else {
            continue;
        };
        if post_intent.status != IntentStatus::Active {
            continue;
        }

        let base_intent = find_base_intent(base, intent_id);
        for scenario in &post_intent.scenarios {
            let unchanged = base_intent
                .and_then(|intent| intent.scenarios.iter().find(|s| s.id == scenario.id))
                .is_some_and(|base_scenario| {
                    emit_scenario_fragment(base_scenario) == emit_scenario_fragment(scenario)
                });
            if !unchanged {
                required.insert(scenario.id);
            }
        }
    }

    required.into_iter().collect()
}

/// The intent named `id` among `base`'s parsed files, if any declares it.
fn find_base_intent(base: &[(RepoPath, TelFile)], id: IntentId) -> Option<&Intent> {
    base.iter().find_map(|(_, file)| match file {
        TelFile::Intent(intent) if intent.id == id => Some(intent),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Globs};
    use crate::span::Span;

    // --- scenario_pattern --------------------------------------------------

    #[test]
    fn scenario_pattern_is_scn_underscore_zero_padded_to_four_digits() {
        assert_eq!(scenario_pattern(ScenarioId(1)), "scn_0001");
        assert_eq!(scenario_pattern(ScenarioId(108)), "scn_0108");
    }

    #[test]
    fn scenario_pattern_grows_past_four_digits_without_truncating() {
        assert_eq!(scenario_pattern(ScenarioId(12345)), "scn_12345");
    }

    // --- find_test_for -------------------------------------------------------

    fn workspace(root: &std::path::Path, tests_globs: &[&str]) -> Workspace {
        Workspace {
            repo_root: root.to_path_buf(),
            telos_dir: root.join("telos"),
            config: Config {
                tests: Globs {
                    globs: tests_globs.iter().map(|s| s.to_string()).collect(),
                },
                ..Config::default()
            },
        }
    }

    #[test]
    fn find_test_for_a_single_match_extracts_the_function_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("tests")).unwrap();
        fs::write(
            tmp.path().join("tests/billing.rs"),
            "fn scn_0001_settles() {}\n",
        )
        .unwrap();
        let ws = workspace(tmp.path(), &["tests/**/*.rs"]);

        let found = find_test_for(&ws, ScenarioId(1), None).unwrap();

        assert_eq!(
            found,
            TestRef {
                path: RepoPath::new("tests/billing.rs"),
                name: Some("scn_0001_settles".to_string()),
            }
        );
    }

    #[test]
    fn find_test_for_zero_matches_is_the_frozen_message() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("tests")).unwrap();
        fs::write(tmp.path().join("tests/other.rs"), "fn something() {}\n").unwrap();
        let ws = workspace(tmp.path(), &["tests/**/*.rs"]);

        let err = find_test_for(&ws, ScenarioId(108), None).unwrap_err();

        assert_eq!(err.code, ErrorCode::TelosTestNotFound);
        assert_eq!(
            err.message,
            "no file matched by the [tests] globs contains `scn_0108`"
        );
        assert_eq!(
            err.hint.as_deref(),
            Some(
                "name the test after the scenario id (`scn_0108_\u{2026}`) in a file the \
                 [tests] globs cover, or pass `--file <path>`"
            )
        );
    }

    #[test]
    fn find_test_for_with_empty_tests_globs_is_zero_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path(), &[]);

        let err = find_test_for(&ws, ScenarioId(1), None).unwrap_err();

        assert_eq!(err.code, ErrorCode::TelosTestNotFound);
        assert_eq!(
            err.message,
            "no file matched by the [tests] globs contains `scn_0001`"
        );
    }

    #[test]
    fn find_test_for_multiple_matches_lists_the_files_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("tests")).unwrap();
        fs::write(tmp.path().join("tests/b.rs"), "fn scn_0108_x() {}\n").unwrap();
        fs::write(tmp.path().join("tests/a.rs"), "fn scn_0108_y() {}\n").unwrap();
        let ws = workspace(tmp.path(), &["tests/**/*.rs"]);

        let err = find_test_for(&ws, ScenarioId(108), None).unwrap_err();

        assert_eq!(err.code, ErrorCode::TelosTestNotFound);
        assert_eq!(
            err.message,
            "`scn_0108` appears in more than one test file: `tests/a.rs`, `tests/b.rs`"
        );
        assert_eq!(
            err.hint.as_deref(),
            Some("pass `--file <path>` to pick one")
        );
    }

    #[test]
    fn find_test_for_does_not_match_the_pattern_embedded_in_a_longer_identifier() {
        // "descn_0001x" contains the pattern as a substring, but not at an
        // identifier boundary on either side -- neither discovery nor name
        // extraction may count it (controller ruling).
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("tests")).unwrap();
        fs::write(tmp.path().join("tests/only.rs"), "descn_0001x\n").unwrap();
        let ws = workspace(tmp.path(), &["tests/**/*.rs"]);

        let err = find_test_for(&ws, ScenarioId(1), None).unwrap_err();

        assert_eq!(err.code, ErrorCode::TelosTestNotFound);
        assert_eq!(
            err.message,
            "no file matched by the [tests] globs contains `scn_0001`"
        );
    }

    #[test]
    fn find_test_for_a_bare_pattern_with_no_suffix_is_its_own_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("chosen.rs"), "scn_0001").unwrap();
        let ws = workspace(tmp.path(), &[]);

        let found = find_test_for(&ws, ScenarioId(1), Some(&RepoPath::new("chosen.rs"))).unwrap();

        assert_eq!(found.name.as_deref(), Some("scn_0001"));
    }

    #[test]
    fn find_test_for_with_file_and_pattern_present_extracts_the_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("chosen.rs"), "fn scn_0001_x() {}\n").unwrap();
        let ws = workspace(tmp.path(), &[]);

        let found = find_test_for(&ws, ScenarioId(1), Some(&RepoPath::new("chosen.rs"))).unwrap();

        assert_eq!(found.path, RepoPath::new("chosen.rs"));
        assert_eq!(found.name.as_deref(), Some("scn_0001_x"));
    }

    #[test]
    fn find_test_for_with_file_and_no_pattern_leaves_name_none() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("chosen.rs"), "fn unrelated() {}\n").unwrap();
        let ws = workspace(tmp.path(), &[]);

        let found = find_test_for(&ws, ScenarioId(1), Some(&RepoPath::new("chosen.rs"))).unwrap();

        assert_eq!(found.path, RepoPath::new("chosen.rs"));
        assert_eq!(found.name, None);
    }

    #[test]
    fn find_test_for_with_a_missing_file_is_its_own_message() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(tmp.path(), &[]);

        let err = find_test_for(&ws, ScenarioId(1), Some(&RepoPath::new("nope.rs"))).unwrap_err();

        assert_eq!(err.code, ErrorCode::TelosTestNotFound);
        assert_eq!(
            err.message,
            "the file passed with --file does not exist: `nope.rs`"
        );
        assert_eq!(err.hint, None);
    }

    // --- witness_verdict -----------------------------------------------------

    fn oid(s: &str) -> Oid {
        Oid(s.to_string())
    }

    fn run(witness: Witness, path: &str, oid_str: &str) -> JournalEntry {
        JournalEntry::Run(TestRun {
            scenario: ScenarioId(1),
            witness,
            test: TestRef {
                path: RepoPath::new(path),
                name: None,
            },
            oid: oid(oid_str),
        })
    }

    fn current(entries: &[(&str, &str)]) -> BTreeMap<RepoPath, Oid> {
        entries
            .iter()
            .map(|(p, o)| (RepoPath::new(*p), oid(o)))
            .collect()
    }

    #[test]
    fn witness_verdict_is_intact_when_green_follows_red_on_the_current_oid() {
        let journal = vec![
            run(Witness::Red, "tests/billing.rs", "aaa"),
            run(Witness::Green, "tests/billing.rs", "aaa"),
        ];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::Intact
        );
    }

    #[test]
    fn witness_verdict_is_missing_red_when_no_red_run_was_ever_taken() {
        let journal = vec![];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::MissingRed
        );
    }

    #[test]
    fn witness_verdict_is_missing_green_when_the_current_red_has_no_green_after_it() {
        let journal = vec![run(Witness::Red, "tests/billing.rs", "aaa")];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::MissingGreen
        );
    }

    #[test]
    fn witness_verdict_is_missing_green_on_a_red_green_red_cycle() {
        // A settled pair, then the file changes and a fresh red is taken
        // at the new (current) oid with no green after it yet.
        let journal = vec![
            run(Witness::Red, "tests/billing.rs", "aaa"),
            run(Witness::Green, "tests/billing.rs", "aaa"),
            run(Witness::Red, "tests/billing.rs", "bbb"),
        ];
        let current = current(&[("tests/billing.rs", "bbb")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::MissingGreen
        );
    }

    #[test]
    fn witness_verdict_is_intact_when_a_same_oid_green_follows_a_different_oid_one() {
        // A green re-run on bytes that later moved back to the red's must
        // not mask the same-oid green that follows it: git diff shows
        // nothing wrong, so the verdict must not claim it does.
        let journal = vec![
            run(Witness::Red, "tests/billing.rs", "aaa"),
            run(Witness::Green, "tests/billing.rs", "bbb"),
            run(Witness::Green, "tests/billing.rs", "aaa"),
        ];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::Intact
        );
    }

    #[test]
    fn witness_verdict_is_missing_green_when_the_later_green_is_on_a_different_path() {
        let journal = vec![
            run(Witness::Red, "tests/x.rs", "aaa"),
            run(Witness::Green, "tests/y.rs", "ccc"),
        ];
        let current = current(&[("tests/x.rs", "aaa"), ("tests/y.rs", "ccc")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::MissingGreen
        );
    }

    #[test]
    fn witness_verdict_picks_the_red_matching_current_even_when_it_is_not_the_last_red() {
        // Two reds, no green at all: the earlier red (oid `aaa`) is the one
        // that matches `current`, the later one (oid `bbb`) does not -- the
        // verdict follows whichever red is valid against the current
        // bytes, and since no green was ever recorded, it is MissingGreen.
        let journal = vec![
            run(Witness::Red, "tests/billing.rs", "aaa"),
            run(Witness::Red, "tests/billing.rs", "bbb"),
        ];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::MissingGreen
        );
    }

    #[test]
    fn witness_verdict_is_sealed_when_the_red_oid_no_longer_matches_current() {
        let journal = vec![run(Witness::Red, "tests/billing.rs", "aaa")];
        let current = current(&[("tests/billing.rs", "bbb")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::Sealed(
                "the test file `tests/billing.rs` changed after the red witness for SCN-0001 \
                 was sealed"
                    .to_string()
            )
        );
    }

    #[test]
    fn witness_verdict_is_sealed_when_a_later_green_ran_on_different_bytes_than_the_red() {
        let journal = vec![
            run(Witness::Red, "tests/billing.rs", "aaa"),
            run(Witness::Green, "tests/billing.rs", "ccc"),
        ];
        let current = current(&[("tests/billing.rs", "aaa")]);

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::Sealed(
                "the test file `tests/billing.rs` changed after the red witness for SCN-0001 \
                 was sealed"
                    .to_string()
            )
        );
    }

    #[test]
    fn witness_verdict_is_sealed_naming_disappearance_when_the_path_is_gone() {
        let journal = vec![run(Witness::Red, "tests/billing.rs", "aaa")];
        let current: BTreeMap<RepoPath, Oid> = BTreeMap::new();

        assert_eq!(
            witness_verdict(&journal, ScenarioId(1), &current),
            WitnessVerdict::Sealed(
                "the test file `tests/billing.rs` sealed for SCN-0001 no longer exists".to_string()
            )
        );
    }

    // --- required_witnesses ----------------------------------------------

    use crate::model::change::fixtures::int_0017;

    fn base_with(intent: Intent) -> Vec<(RepoPath, TelFile)> {
        vec![(
            RepoPath::new(format!("telos/intents/{}.tel", intent.id)),
            TelFile::Intent(intent),
        )]
    }

    fn post_with(intent: Intent) -> TelosModel {
        let mut model = TelosModel::default();
        model.intents.insert(intent.id, intent);
        model
    }

    #[test]
    fn required_witnesses_exempts_a_scenario_unchanged_but_for_its_span() {
        let base = base_with(int_0017());

        let mut post_intent = int_0017();
        // Re-parsing the same source (or a different one carrying the
        // identical scenario) never reproduces the same spans -- mutate
        // one deep inside to model that, and nothing else.
        post_intent.scenarios[0].given[0].notion.span = Span {
            start: 999,
            end: 1010,
        };
        let post = post_with(post_intent.clone());
        let ops = vec![StagedOp::EditIntent(post_intent)];

        assert_eq!(
            required_witnesses(&base, &post, &ops),
            Vec::<ScenarioId>::new()
        );
    }

    #[test]
    fn required_witnesses_requires_a_brand_new_scenario() {
        let base = base_with(int_0017());

        let mut post_intent = int_0017();
        let mut new_scenario = post_intent.scenarios[0].clone();
        new_scenario.id = ScenarioId(92);
        post_intent.scenarios.push(new_scenario);
        let post = post_with(post_intent.clone());
        let ops = vec![StagedOp::EditIntent(post_intent)];

        assert_eq!(required_witnesses(&base, &post, &ops), vec![ScenarioId(92)]);
    }

    #[test]
    fn required_witnesses_requires_a_scenario_whose_fragment_changed() {
        let base = base_with(int_0017());

        let mut post_intent = int_0017();
        post_intent.scenarios[0].title = "a newly issued invoice starts open".to_string();
        let post = post_with(post_intent.clone());
        let ops = vec![StagedOp::EditIntent(post_intent)];

        assert_eq!(required_witnesses(&base, &post, &ops), vec![ScenarioId(91)]);
    }

    #[test]
    fn required_witnesses_exempts_a_draft_intent_entirely() {
        let base = base_with(int_0017());

        let mut post_intent = int_0017();
        post_intent.status = IntentStatus::Draft;
        let mut new_scenario = post_intent.scenarios[0].clone();
        new_scenario.id = ScenarioId(93);
        post_intent.scenarios.push(new_scenario);
        let post = post_with(post_intent.clone());
        let ops = vec![StagedOp::EditIntent(post_intent)];

        assert_eq!(
            required_witnesses(&base, &post, &ops),
            Vec::<ScenarioId>::new()
        );
    }

    #[test]
    fn required_witnesses_ignores_ops_that_are_not_add_or_edit_intent() {
        use crate::model::change::fixtures::{con_0003, invoice};

        let base: Vec<(RepoPath, TelFile)> = vec![];
        let post = TelosModel::default();
        let ops = vec![
            StagedOp::AddNotion(invoice()),
            StagedOp::AddConstraint(con_0003()),
            StagedOp::RemoveIntent(IntentId(17)),
        ];

        assert_eq!(
            required_witnesses(&base, &post, &ops),
            Vec::<ScenarioId>::new()
        );
    }

    #[test]
    fn required_witnesses_is_sorted_across_multiple_intents() {
        // Intent 17 gains a new scenario 95; intent 5 is brand new (absent
        // from base entirely) with a lower-numbered scenario 10. The merged
        // result must come back ascending regardless of staging order, and
        // scenario 91 (unchanged) must not appear. Each scenario id here is
        // distinct, so this pins ordering, not deduplication -- see
        // `required_witnesses_dedupes_two_ops_on_the_same_intent_id` for
        // the latter.
        let mut intent_a_post = int_0017();
        let mut new_a = intent_a_post.scenarios[0].clone();
        new_a.id = ScenarioId(95);
        intent_a_post.scenarios.push(new_a);

        let mut intent_b_post = int_0017();
        intent_b_post.id = IntentId(5);
        intent_b_post.scenarios[0].id = ScenarioId(10);

        let base = base_with(int_0017());
        let mut post = TelosModel::default();
        post.intents.insert(IntentId(17), intent_a_post.clone());
        post.intents.insert(IntentId(5), intent_b_post.clone());

        let ops = vec![
            StagedOp::EditIntent(intent_a_post),
            StagedOp::AddIntent(intent_b_post),
        ];

        assert_eq!(
            required_witnesses(&base, &post, &ops),
            vec![ScenarioId(10), ScenarioId(95)]
        );
    }

    #[test]
    fn required_witnesses_dedupes_two_ops_on_the_same_intent_id() {
        // Two ops in one change's staged list both target intent 17 (the
        // type system permits it even though the grammar discourages it):
        // the scenario they both make required must collapse to one entry,
        // not appear once per op.
        let base = base_with(int_0017());

        let mut post_intent = int_0017();
        let mut new_scenario = post_intent.scenarios[0].clone();
        new_scenario.id = ScenarioId(92);
        post_intent.scenarios.push(new_scenario);
        let post = post_with(post_intent.clone());

        let ops = vec![
            StagedOp::AddIntent(post_intent.clone()),
            StagedOp::EditIntent(post_intent),
        ];

        assert_eq!(required_witnesses(&base, &post, &ops), vec![ScenarioId(92)]);
    }
}
