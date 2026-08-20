//! `globs.rs` / `exec.rs`, end to end against real filesystem I/O and a
//! real `git` invocation: rule-5 glob scanning and orphan detection (D8),
//! `{filter}` substitution (D10), and the cross-OS shell runner (D9).

use std::fs;
use std::path::{Path, PathBuf};

use telos_core::config::{Config, Globs};
use telos_core::error::ErrorCode;
use telos_core::exec::{run_shell, run_shell_with_filter, substitute_filter};
use telos_core::globs::{glob_matches, orphan_code};
use telos_core::ids::{IntentId, RepoPath, ScenarioId};
use telos_core::model::{Binding, TelosModel, TestRef};
use telos_core::span::{Sp, Span};
use telos_core::workspace::Workspace;

// --- test helpers --------------------------------------------------------

fn sp<T>(node: T) -> Sp<T> {
    Sp {
        node,
        span: Span::default(),
    }
}

fn write_file(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, "// placeholder\n").unwrap();
}

fn workspace_with_globs(root: &Path, code: &[&str], tests: &[&str]) -> Workspace {
    Workspace {
        repo_root: root.to_path_buf(),
        telos_dir: root.join("telos"),
        config: Config {
            code: Globs {
                globs: code.iter().map(|s| s.to_string()).collect(),
            },
            tests: Globs {
                globs: tests.iter().map(|s| s.to_string()).collect(),
            },
            ..Config::default()
        },
    }
}

fn implements(path: &str, intent: u32) -> Binding {
    Binding::Implements {
        path: RepoPath::new(path),
        intent: sp(IntentId(intent)),
    }
}

fn proves(path: &str, scenario: u32) -> Binding {
    Binding::Proves {
        test: TestRef {
            path: RepoPath::new(path),
            name: None,
        },
        scenario: sp(ScenarioId(scenario)),
    }
}

fn repo_paths(paths: &[&str]) -> Vec<RepoPath> {
    paths.iter().map(|p| RepoPath::new(*p)).collect()
}

// --- glob_matches ----------------------------------------------------------

#[test]
fn glob_matches_finds_files_under_root_matching_a_pattern() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), "src/b.rs");
    write_file(tmp.path(), "other/c.rs");

    let matches = glob_matches(tmp.path(), &["src/*.rs".to_string()]).unwrap();

    assert_eq!(matches, repo_paths(&["src/a.rs", "src/b.rs"]));
}

#[test]
fn glob_matches_result_is_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/z.rs");
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), "src/m.rs");

    let matches = glob_matches(tmp.path(), &["src/*.rs".to_string()]).unwrap();

    assert_eq!(matches, repo_paths(&["src/a.rs", "src/m.rs", "src/z.rs"]));
}

#[test]
fn glob_matches_a_bare_star_does_not_cross_a_path_separator() {
    // Regression: globset's *default* (`Glob::new`, no `literal_separator`)
    // lets a bare `*` match across `/`, so `"src/*.rs"` would also match a
    // nested `"src/sub/deep.rs"`. `build_glob_set` must build with
    // `literal_separator(true)` so `*` stays within one path component,
    // matching gitignore-style semantics; `**` is the one that spans
    // directories.
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/top.rs");
    write_file(tmp.path(), "src/sub/deep.rs");

    let star = glob_matches(tmp.path(), &["src/*.rs".to_string()]).unwrap();
    assert_eq!(star, repo_paths(&["src/top.rs"]));

    let globstar = glob_matches(tmp.path(), &["src/**/*.rs".to_string()]).unwrap();
    assert_eq!(globstar, repo_paths(&["src/sub/deep.rs", "src/top.rs"]));
}

#[test]
fn glob_matches_skips_dot_git_directories_entirely() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), ".git/hooks/fake.rs");
    write_file(tmp.path(), ".git/objects/pack/x.rs");

    // A broad, recursive pattern that -- were `.git/` not special-cased --
    // would happily match the files planted inside it too.
    let matches = glob_matches(tmp.path(), &["**/*.rs".to_string()]).unwrap();

    assert_eq!(matches, repo_paths(&["src/a.rs"]));
}

#[test]
fn glob_matches_of_empty_patterns_is_empty_without_walking() {
    // A root that does not exist: if `glob_matches` tried to walk it, this
    // would fail with an I/O error. An empty pattern list must short-circuit
    // before ever touching the filesystem.
    let nonexistent = PathBuf::from("/definitely/does/not/exist/telos-sdd-test");

    let matches = glob_matches(&nonexistent, &[]).unwrap();

    assert_eq!(matches, Vec::<RepoPath>::new());
}

#[test]
fn glob_matches_rejects_an_invalid_pattern_naming_it() {
    let tmp = tempfile::tempdir().unwrap();

    let err = glob_matches(tmp.path(), &["src/[".to_string()]).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(
        err.message.contains("src/["),
        "expected the message to name the offending pattern, got: {}",
        err.message
    );
}

// --- orphan_code -------------------------------------------------------

#[test]
fn orphan_code_reports_a_code_file_with_no_implements_binding() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), "src/b.rs");
    let ws = workspace_with_globs(tmp.path(), &["src/*.rs"], &[]);

    let mut model = TelosModel::default();
    model.bindings.push(implements("src/a.rs", 1));

    assert_eq!(orphan_code(&ws, &model).unwrap(), repo_paths(&["src/b.rs"]));
}

#[test]
fn orphan_code_covers_the_tests_family_independently() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), "tests/t1.rs");
    write_file(tmp.path(), "tests/t2.rs");
    let ws = workspace_with_globs(tmp.path(), &["src/*.rs"], &["tests/*.rs"]);

    let mut model = TelosModel::default();
    model.bindings.push(implements("src/a.rs", 1));
    model.bindings.push(proves("tests/t1.rs", 1));

    assert_eq!(
        orphan_code(&ws, &model).unwrap(),
        repo_paths(&["tests/t2.rs"])
    );
}

#[test]
fn orphan_code_ignores_files_under_dot_git() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    write_file(tmp.path(), ".git/hooks/fake.rs");
    // A pattern broad enough to match `.git/` contents, so the assertion
    // actually exercises the walk's special-case rather than the glob.
    let ws = workspace_with_globs(tmp.path(), &["**/*.rs"], &[]);

    let model = TelosModel::default();

    assert_eq!(orphan_code(&ws, &model).unwrap(), repo_paths(&["src/a.rs"]));
}

#[test]
fn orphan_code_of_empty_globs_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs");
    let ws = workspace_with_globs(tmp.path(), &[], &[]);

    let model = TelosModel::default();

    assert_eq!(orphan_code(&ws, &model).unwrap(), Vec::<RepoPath>::new());
}

#[test]
fn orphan_code_a_file_matched_by_both_families_must_satisfy_both() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "shared.rs");
    // Both globs match the same file -- it belongs to both families.
    let ws = workspace_with_globs(tmp.path(), &["*.rs"], &["*.rs"]);

    let mut model = TelosModel::default();
    // Bound on the [code] side only: the [tests] family's check must still
    // fail it independently, and the result must not be deduplicated away.
    model.bindings.push(implements("shared.rs", 1));

    assert_eq!(
        orphan_code(&ws, &model).unwrap(),
        repo_paths(&["shared.rs"])
    );
}

#[test]
fn orphan_code_a_file_bound_in_both_families_is_not_orphaned() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "shared.rs");
    let ws = workspace_with_globs(tmp.path(), &["*.rs"], &["*.rs"]);

    let mut model = TelosModel::default();
    model.bindings.push(implements("shared.rs", 1));
    model.bindings.push(proves("shared.rs", 1));

    assert_eq!(orphan_code(&ws, &model).unwrap(), Vec::<RepoPath>::new());
}

// --- substitute_filter ---------------------------------------------------

#[test]
fn substitute_filter_replaces_the_placeholder() {
    assert_eq!(
        substitute_filter("cargo test {filter}", "scn_x"),
        "cargo test scn_x"
    );
}

#[test]
fn substitute_filter_replaces_every_occurrence() {
    assert_eq!(
        substitute_filter("{filter} && echo {filter}", "scn_x"),
        "scn_x && echo scn_x"
    );
}

#[test]
fn substitute_filter_of_an_empty_filter_trims_the_trailing_space() {
    assert_eq!(substitute_filter("cargo test {filter}", ""), "cargo test");
}

#[test]
fn substitute_filter_with_no_placeholder_is_unchanged_but_still_trimmed() {
    assert_eq!(substitute_filter("cargo test  ", ""), "cargo test");
}

// --- run_shell -------------------------------------------------------------

#[test]
fn run_shell_reports_a_successful_command() {
    let tmp = tempfile::tempdir().unwrap();

    let result = run_shell("git --version", tmp.path()).unwrap();

    assert_eq!(result.status, 0);
    assert!(
        result.stdout.contains("git version"),
        "expected stdout to contain `git version`, got: {}",
        result.stdout
    );
}

#[test]
fn run_shell_captures_a_nonzero_exit_and_stderr_without_erroring() {
    let tmp = tempfile::tempdir().unwrap();

    let result = run_shell("git hash-object no-such-file", tmp.path()).unwrap();

    assert_ne!(result.status, 0);
    assert!(
        !result.stderr.is_empty(),
        "expected stderr to capture git's complaint"
    );
}

#[test]
fn run_shell_runs_with_the_given_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "marker.txt");

    let result = if cfg!(windows) {
        run_shell("dir /b marker.txt", tmp.path()).unwrap()
    } else {
        run_shell("ls marker.txt", tmp.path()).unwrap()
    };

    assert_eq!(result.status, 0);
    assert!(result.stdout.contains("marker.txt"));
}

#[test]
fn filtered_shell_run_preserves_display_but_passes_metacharacters_as_one_argument() {
    let tmp = tempfile::tempdir().unwrap();
    let (filter, template, displayed) = if cfg!(windows) {
        (
            "proof&mkdir injected",
            "if exist {filter} (exit 0) else (exit 1)",
            "if exist proof&mkdir injected (exit 0) else (exit 1)",
        )
    } else {
        (
            "proof;mkdir injected",
            "test -f {filter}",
            "test -f proof;mkdir injected",
        )
    };
    fs::write(tmp.path().join(filter), "proof\n").unwrap();

    let run = run_shell_with_filter(template, filter, tmp.path()).unwrap();

    assert_eq!(run.command, displayed);
    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected").exists());
}

#[test]
fn filtered_shell_run_keeps_leading_display_whitespace() {
    let tmp = tempfile::tempdir().unwrap();

    let run = run_shell_with_filter("  git --version {filter}  ", "", tmp.path()).unwrap();

    assert_eq!(run.command, "  git --version");
}

#[test]
fn filtered_shell_run_supports_a_placeholder_already_quoted_as_one_argument() {
    let tmp = tempfile::tempdir().unwrap();
    let (filter, template, displayed) = if cfg!(windows) {
        (
            "proof&mkdir injected",
            "if exist \"{filter}\" (exit 0) else (exit 1)",
            "if exist \"proof&mkdir injected\" (exit 0) else (exit 1)",
        )
    } else {
        (
            "proof;mkdir injected",
            "test -f \"{filter}\"",
            "test -f \"proof;mkdir injected\"",
        )
    };
    fs::write(tmp.path().join(filter), "proof\n").unwrap();

    let run = run_shell_with_filter(template, filter, tmp.path()).unwrap();

    assert_eq!(run.command, displayed);
    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected").exists());
}

#[test]
fn filtered_shell_run_supports_a_placeholder_embedded_in_double_quotes() {
    let tmp = tempfile::tempdir().unwrap();
    let (filter, template, file) = if cfg!(windows) {
        (
            "proof&mkdir injected",
            "if exist \"prefix-{filter}-suffix\" (exit 0) else (exit 1)",
            "prefix-proof&mkdir injected-suffix",
        )
    } else {
        (
            "proof;mkdir injected",
            "test -f \"prefix-{filter}-suffix\"",
            "prefix-proof;mkdir injected-suffix",
        )
    };
    fs::write(tmp.path().join(file), "proof\n").unwrap();

    let run = run_shell_with_filter(template, filter, tmp.path()).unwrap();

    assert_eq!(run.command, template.replace("{filter}", filter).trim_end());
    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected-suffix").exists());
}

#[cfg(unix)]
#[test]
fn filtered_shell_run_supports_a_placeholder_embedded_in_single_quotes() {
    let tmp = tempfile::tempdir().unwrap();
    let filter = "proof;mkdir injected";
    fs::write(
        tmp.path().join("prefix-proof;mkdir injected-suffix"),
        "proof\n",
    )
    .unwrap();

    let run =
        run_shell_with_filter("test -f 'prefix-{filter}-suffix'", filter, tmp.path()).unwrap();

    assert_eq!(run.command, "test -f 'prefix-proof;mkdir injected-suffix'");
    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected-suffix").exists());
}

#[test]
fn filtered_shell_run_supports_unquoted_prefix_suffix_and_multiple_occurrences() {
    let tmp = tempfile::tempdir().unwrap();
    let (filter, template, file) = if cfg!(windows) {
        (
            "proof&mkdir injected",
            "if exist prefix-{filter}-suffix (if exist \"prefix-{filter}-suffix\" (exit 0) else (exit 1)) else (exit 1)",
            "prefix-proof&mkdir injected-suffix",
        )
    } else {
        (
            "proof;mkdir injected",
            "test -f prefix-{filter}-suffix && test -f \"prefix-{filter}-suffix\"",
            "prefix-proof;mkdir injected-suffix",
        )
    };
    fs::write(tmp.path().join(file), "proof\n").unwrap();

    let run = run_shell_with_filter(template, filter, tmp.path()).unwrap();

    assert_eq!(run.command, template.replace("{filter}", filter).trim_end());
    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected-suffix").exists());
}

#[cfg(windows)]
#[test]
fn filtered_shell_run_disables_windows_injection_metacharacters() {
    let tmp = tempfile::tempdir().unwrap();
    let filter = "proof&mkdir injected|echo bad^caret%PATH%!delayed!";

    let run = run_shell_with_filter("echo {filter} > observed.txt", filter, tmp.path()).unwrap();

    assert_eq!(run.result.status, 0);
    assert!(!tmp.path().join("injected").exists());
    assert!(
        fs::read_to_string(tmp.path().join("observed.txt"))
            .unwrap()
            .contains(filter)
    );
}

#[cfg(windows)]
#[test]
fn filtered_shell_run_rejects_a_windows_quote_before_spawn() {
    let tmp = tempfile::tempdir().unwrap();

    let err =
        run_shell_with_filter("echo {filter}", "proof\"&mkdir injected", tmp.path()).unwrap_err();

    assert_eq!(err.code, ErrorCode::TelosParseError);
    assert!(!tmp.path().join("injected").exists());
}
