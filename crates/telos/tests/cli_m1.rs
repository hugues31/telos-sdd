use assert_cmd::Command;

/// `telos --version` prints the crate version and exits successfully.
#[test]
fn version_flag_prints_telos_version() {
    Command::cargo_bin("telos")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout("telos 0.7.0\n");
}
