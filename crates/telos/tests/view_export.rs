//! End-to-end contract tests for `telos view --export`: it publishes the
//! same sealed snapshot the renderer uses, but only after every byte has been
//! prepared and without ever replacing a caller-owned destination.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

#[cfg(unix)]
use common::{fake_browser, wait_for_browser_target};
use common::{telos, with_fixture};

const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";
const DATA_PREFIX: &str = "window.__TELOS_DATA__ = ";
const DATA_SUFFIX: &str = ";\n";

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

fn data_payload(script: &[u8]) -> Value {
    let script = std::str::from_utf8(script).expect("data.js is UTF-8");
    assert!(script.starts_with(DATA_PREFIX), "unexpected data.js prefix");
    assert!(script.ends_with(DATA_SUFFIX), "unexpected data.js suffix");
    serde_json::from_str(&script[DATA_PREFIX.len()..script.len() - DATA_SUFFIX.len()])
        .expect("data.js assignment contains JSON")
}

fn is_build_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn is_export_path(path: &str) -> bool {
    matches!(path, ".nojekyll" | "index.html" | "data.js")
        || path
            .strip_prefix("assets/")
            .is_some_and(|relative| !relative.is_empty())
}

#[test]
fn export_writes_the_embedded_spa_and_sealed_billing_payload() {
    let tmp = with_fixture();

    let output = export(tmp.path(), "site");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let site = tmp.path().join("site");
    let envelope = json_stdout(&output);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["command"], "view");
    assert_eq!(envelope["result"]["mode"], "export");
    assert_eq!(envelope["result"]["destination"], "site");
    assert_eq!(envelope["error"], Value::Null);
    assert_eq!(envelope["next_actions"], json!([]));

    let announced = envelope["result"]["files"]
        .as_array()
        .expect("files is an array")
        .iter()
        .map(|path| path.as_str().expect("file path is a string"))
        .collect::<Vec<_>>();
    let mut sorted = announced.clone();
    sorted.sort_unstable();
    assert_eq!(
        announced, sorted,
        "CLI file list is lexicographically sorted"
    );
    assert!(announced.iter().all(|path| is_export_path(path)));
    assert!(
        announced
            .iter()
            .any(|path| { path.starts_with("assets/") && path.ends_with(".js") })
    );
    assert!(announced.contains(&"assets/app.css"));
    assert!(announced.contains(&"assets/logo.png"));

    let disk_paths = exported_paths(&site);
    assert_eq!(
        announced
            .iter()
            .copied()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        disk_paths
    );
    assert_eq!(
        disk_paths,
        [
            PathBuf::from(".nojekyll"),
            PathBuf::from("assets/app.css"),
            PathBuf::from("assets/app.js"),
            PathBuf::from("assets/logo.png"),
            PathBuf::from("data.js"),
            PathBuf::from("index.html"),
        ]
    );

    assert_eq!(
        fs::read(site.join("index.html")).unwrap(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../frontend/dist/index.html"
        ))
    );
    assert!(fs::read(site.join(".nojekyll")).unwrap().is_empty());

    let payload = data_payload(&fs::read(site.join("data.js")).unwrap());
    assert_eq!(payload["meta"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(is_build_date(
        payload["meta"]["build_date"]
            .as_str()
            .expect("build_date is a string")
    ));
    assert_eq!(payload["meta"]["mode"], "export");
    assert_eq!(payload["snapshot"]["dashboard"]["state"], "coherent");
    assert!(
        payload["snapshot"]["dashboard"]["drift"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        payload["snapshot"]["dashboard"]["open_changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        payload["snapshot"]["coverage"],
        json!({
            "notions": 4,
            "constraints": 1,
            "intents_total": 2,
            "intents_active": 2,
            "intents_implemented": 1,
            "scenarios_total": 2,
            "scenarios_proved": 2,
            "rows": [
                {
                    "intent": "INT-0017",
                    "scenario": "SCN-0091",
                    "test": "tests/billing.rs"
                },
                {
                    "intent": "INT-0042",
                    "scenario": "SCN-0107",
                    "test": "tests/billing.rs::scn_0107_full_payment_settles_the_invoice"
                }
            ]
        })
    );
    assert!(
        payload["snapshot"]["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent["id"] == "INT-0042")
    );
    assert!(
        payload["snapshot"]["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .any(|scenario| scenario["id"] == "SCN-0107")
    );

    for path in ["index.html", "assets/app.css"] {
        let text = fs::read_to_string(site.join(path)).unwrap();
        for forbidden in [
            "http://",
            "https://",
            "href=\"//",
            "src=\"//",
            "href=\"http",
            "src=\"http",
        ] {
            assert!(!text.contains(forbidden), "found {forbidden:?} in {path}");
        }
    }
}

#[test]
fn export_refuses_drift_before_creating_the_destination() {
    let tmp = with_fixture();
    let intent = tmp
        .path()
        .join("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel");
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
fn open_launches_the_default_browser_with_the_exported_index() {
    let tmp = with_fixture();
    let (_browser_tmp, browser, target_log) = fake_browser();
    let output = telos(
        tmp.path(),
        &["view", "--export", "site", "--open", "--json"],
    )
    .env("BROWSER", browser)
    .env("TELOS_TEST_BROWSER_TARGET", &target_log)
    .output()
    .expect("run telos view --export --open");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        wait_for_browser_target(&target_log),
        format!("file://{}", tmp.path().join("site/index.html").display())
    );
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
