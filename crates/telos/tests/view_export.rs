//! End-to-end contract tests for `telos view --export`: it publishes the
//! same sealed snapshot the renderer uses, but only after every byte has been
//! prepared and without ever replacing a caller-owned destination.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use common::{telos, with_fixture};

const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

fn json_stdout(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not a JSON envelope ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn export(root: &Path, destination: &str) -> std::process::Output {
    telos(root, &["view", "--export", destination, "--json"])
        .output()
        .expect("run telos view --export")
}

fn exported_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_paths(root, root, &mut paths);
    paths.sort();
    paths
}

fn collect_paths(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_paths(root, &path, paths);
        } else {
            paths.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

#[test]
fn export_writes_the_sealed_billing_snapshot_with_the_exact_envelope() {
    let tmp = with_fixture();

    let output = export(tmp.path(), "site");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json_stdout(&output),
        json!({
            "ok": true,
            "command": "view",
            "result": {
                "mode": "export",
                "destination": "site",
                "files": [
                    "coverage.html",
                    "glossary.html",
                    "graph.html",
                    "index.html",
                    "intents/INT-0017.html",
                    "intents/INT-0042.html"
                ]
            },
            "error": null,
            "next_actions": []
        })
    );

    let site = tmp.path().join("site");
    assert_eq!(
        exported_paths(&site),
        [
            PathBuf::from("coverage.html"),
            PathBuf::from("glossary.html"),
            PathBuf::from("graph.html"),
            PathBuf::from("index.html"),
            PathBuf::from("intents/INT-0017.html"),
            PathBuf::from("intents/INT-0042.html"),
        ]
    );

    let dashboard = fs::read_to_string(site.join("index.html")).unwrap();
    let graph = fs::read_to_string(site.join("graph.html")).unwrap();
    let intent = fs::read_to_string(site.join("intents/INT-0042.html")).unwrap();
    let glossary = fs::read_to_string(site.join("glossary.html")).unwrap();
    let coverage = fs::read_to_string(site.join("coverage.html")).unwrap();

    assert!(dashboard.contains("INT-0017"));
    assert!(dashboard.contains("INT-0042"));
    assert!(dashboard.contains("Project state: <strong>coherent</strong>"));
    assert!(graph.contains("id=\"relation-filter\""));
    assert!(graph.contains("requires"));
    assert!(intent.contains("Customers must see immediately that their debt is cleared."));
    assert!(intent.contains("CON-0003"));
    assert!(intent.contains("src/billing/invoice.rs"));
    assert!(intent.contains("tests/billing.rs::scn_0107_full_payment_settles_the_invoice"));
    assert!(intent.contains("href=\"../graph.html\""));
    assert!(intent.contains("href=\"INT-0017.html\""));
    assert!(glossary.contains("Invoice"));
    assert!(glossary.contains("href=\"intents/INT-0042.html\""));
    assert!(coverage.contains("Intent × scenario × test"));
    assert!(coverage.contains("SCN-0107"));
    assert!(coverage.contains("href=\"intents/INT-0042.html#scenario-SCN-0107\""));

    for path in exported_paths(&site) {
        let html = fs::read_to_string(site.join(path)).unwrap();
        for forbidden in [
            "http://",
            "https://",
            "href=\"//",
            "src=\"//",
            "href=\"http",
            "src=\"http",
        ] {
            assert!(!html.contains(forbidden), "found {forbidden:?} in {html}");
        }
    }
}

#[test]
fn export_refuses_drift_before_creating_the_destination() {
    let tmp = with_fixture();
    let intent = tmp.path().join("telos/intents/INT-0042.tel");
    fs::write(
        &intent,
        format!("{}\n", fs::read_to_string(&intent).unwrap()),
    )
    .unwrap();

    let output = export(tmp.path(), "site");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json_stdout(&output),
        json!({
            "ok": false,
            "command": "view",
            "result": null,
            "error": {
                "code": "TELOS_DRIFT_DETECTED",
                "message": "the project has drifted from its seal",
                "hint": DRIFT_HINT
            },
            "next_actions": []
        })
    );
    assert!(!tmp.path().join("site").exists());
}

#[test]
fn export_refuses_open_changes_before_creating_the_destination() {
    let tmp = with_fixture();
    telos(tmp.path(), &["change", "open", "view adjustment"])
        .assert()
        .success();

    let output = export(tmp.path(), "site");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json_stdout(&output),
        json!({
            "ok": false,
            "command": "view",
            "result": null,
            "error": {
                "code": "TELOS_CHANGE_STATE_INVALID",
                "message": "open changes; reconcile or abandon them",
                "hint": "run `telos change list`"
            },
            "next_actions": []
        })
    );
    assert!(!tmp.path().join("site").exists());
}

#[test]
fn export_never_replaces_an_existing_destination() {
    let tmp = with_fixture();
    let destination = tmp.path().join("site");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep.txt"), "leave me alone").unwrap();

    let output = export(tmp.path(), "site");

    assert_eq!(output.status.code(), Some(1));
    let envelope = json_stdout(&output);
    assert_eq!(envelope["error"]["code"], "TELOS_CHANGE_STATE_INVALID");
    assert_eq!(
        envelope["error"]["message"],
        "export destination `site` already exists"
    );
    assert_eq!(
        envelope["error"]["hint"],
        "choose an empty path that does not exist"
    );
    assert_eq!(
        fs::read_to_string(destination.join("keep.txt")).unwrap(),
        "leave me alone"
    );
}

#[cfg(unix)]
#[test]
fn export_never_replaces_an_existing_destination_symlink() {
    use std::os::unix::fs::symlink;

    let tmp = with_fixture();
    let target = tmp.path().join("owned-by-someone-else");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), "leave me alone").unwrap();
    symlink(&target, tmp.path().join("site")).unwrap();

    let output = export(tmp.path(), "site");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json_stdout(&output)["error"]["code"],
        "TELOS_CHANGE_STATE_INVALID"
    );
    assert_eq!(
        fs::read_to_string(target.join("keep.txt")).unwrap(),
        "leave me alone"
    );
    assert!(
        fs::symlink_metadata(tmp.path().join("site"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn two_exports_have_identical_sorted_paths_and_bytes() {
    let tmp = with_fixture();

    assert!(export(tmp.path(), "site-a").status.success());
    assert!(export(tmp.path(), "site-b").status.success());

    let left = tmp.path().join("site-a");
    let right = tmp.path().join("site-b");
    let paths = exported_paths(&left);
    assert_eq!(paths, exported_paths(&right));
    for path in paths {
        assert_eq!(
            fs::read(left.join(&path)).unwrap(),
            fs::read(right.join(path)).unwrap()
        );
    }
}

#[cfg(unix)]
#[test]
fn export_rejects_a_non_utf8_destination_without_creating_any_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = with_fixture();
    let before = directory_names(tmp.path());
    let destination = OsString::from_vec(b"site-\xff".to_vec());
    let mut command = telos(tmp.path(), &[]);
    let output = command
        .arg("view")
        .arg("--export")
        .arg(&destination)
        .arg("--json")
        .output()
        .expect("run telos view with a non-UTF-8 destination");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json_stdout(&output),
        json!({
            "ok": false,
            "command": "view",
            "result": null,
            "error": {
                "code": "TELOS_PARSE_ERROR",
                "message": "export destination must be valid UTF-8",
                "hint": null
            },
            "next_actions": []
        })
    );
    assert_eq!(directory_names(tmp.path()), before);
}

#[cfg(unix)]
fn directory_names(directory: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}
