//! `telos gherkin`: the rendered `.feature` projection of the sealed spec.

mod common;

use std::path::Path;

use common::{telos, with_fixture};

fn json_stdout(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not valid JSON ({error}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Runs a command that must succeed and returns its `result`.
fn result_of(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = telos(dir, args).output().unwrap();
    assert!(out.status.success(), "{args:?} failed: {out:?}");
    json_stdout(&out)["result"].clone()
}

#[test]
fn gherkin_renders_every_intent_in_path_order() {
    let tmp = with_fixture();
    let result = result_of(tmp.path(), &["gherkin", "--json"]);

    let features = result["features"].as_array().expect("features is an array");
    let paths: Vec<&str> = features
        .iter()
        .map(|f| f["path"].as_str().expect("path is a string"))
        .collect();
    assert_eq!(
        paths,
        vec![
            "telos/features/billing/invoicing/INT-0017.feature",
            "telos/features/billing/settlement/INT-0042.feature",
        ]
    );
}

#[test]
fn gherkin_content_is_runnable_cucumber() {
    let tmp = with_fixture();
    let result = result_of(tmp.path(), &["gherkin", "--json"]);
    let content = result["features"][1]["content"]
        .as_str()
        .expect("content is a string");

    assert!(content.starts_with("@INT-0042\nFeature: "), "{content}");
    assert!(content.contains("\n  @SCN-0107\n"), "{content}");
    assert!(
        content.contains("    Given the invoice with state open and balance 120.00 EUR\n"),
        "{content}"
    );
}

#[test]
fn gherkin_writes_nothing() {
    let tmp = with_fixture();
    let lock = tmp.path().join("telos/telos.lock");
    let before = std::fs::read_to_string(&lock).unwrap();
    result_of(tmp.path(), &["gherkin", "--json"]);
    assert_eq!(
        before,
        std::fs::read_to_string(&lock).unwrap(),
        "gherkin must not touch the lock"
    );
    assert!(
        !tmp.path().join("telos/features").exists(),
        "gherkin renders but does not write -- sealing feature files is a later phase"
    );
}
