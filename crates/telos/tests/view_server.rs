//! End-to-end contracts for the foreground loopback view server.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use common::{telos, with_fixture};

struct ServerChild(Option<Child>);

impl ServerChild {
    fn stop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_server(root: &Path) -> (ServerChild, String, Value) {
    let (child, line) = spawn_and_read_startup(root, &["view", "--port", "0", "--json"]);
    let envelope: Value = serde_json::from_str(line.trim_end())
        .unwrap_or_else(|error| panic!("startup line is not JSON ({error}): {line:?}"));
    let url = envelope["result"]["url"]
        .as_str()
        .unwrap_or_else(|| panic!("startup envelope has no result.url: {envelope}"))
        .to_string();
    (child, url, envelope)
}

fn spawn_and_read_startup(root: &Path, args: &[&str]) -> (ServerChild, String) {
    let mut child = ServerChild(Some(
        Command::new(env!("CARGO_BIN_EXE_telos"))
            .args(args)
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn telos view"),
    ));
    let stdout = child
        .0
        .as_mut()
        .expect("server child is running")
        .stdout
        .take()
        .expect("server stdout is piped");
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut stdout = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let result = std::io::BufRead::read_line(&mut stdout, &mut line).map(|_| line);
        let _ = sender.send(result);
    });

    let line = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("server did not flush its startup line within five seconds")
        .expect("read server startup line");
    (child, line)
}

fn get(url: &str, path: &str) -> String {
    let address = url
        .strip_prefix("http://")
        .and_then(|url| url.strip_suffix('/'))
        .expect("server URL uses http and ends in slash");
    let mut stream = TcpStream::connect(address).expect("connect to loopback view server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn telos_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let telos = root.join("telos");
    let mut files = BTreeMap::new();
    collect(&telos, &telos, &mut files);
    files
}

fn wait_until(description: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {description}");
}

#[test]
fn serves_every_page_on_loopback() {
    let tmp = with_fixture();
    let before = telos_bytes(tmp.path());

    let (mut server, url, envelope) = start_server(tmp.path());

    assert_eq!(
        envelope,
        json!({
            "ok": true,
            "command": "view",
            "result": {"mode": "server", "url": url},
            "error": null,
            "next_actions": []
        })
    );
    assert!(url.starts_with("http://127.0.0.1:"), "url: {url}");

    for (path, expected) in [
        ("/", "Telos dashboard"),
        ("/graph", "requires"),
        (
            "/intent/INT-0042",
            "Customers must see immediately that their debt is cleared.",
        ),
        ("/glossary", "Invoice"),
        ("/coverage", "SCN-0107"),
    ] {
        let response = get(&url, path);
        assert!(response.starts_with("HTTP/1.1 200 "), "{path}: {response}");
        assert!(
            response.contains("content-type: text/html; charset=utf-8\r\n"),
            "{path}: {response}"
        );
        assert!(response.contains(expected), "{path}: {response}");
    }

    let missing = get(&url, "/intent/INT-9999");
    assert!(missing.starts_with("HTTP/1.1 404 "), "{missing}");

    server.stop();
    assert_eq!(telos_bytes(tmp.path()), before);
}

#[test]
fn reloads_last_good_snapshot_and_recovers_after_invalid_edits() {
    let tmp = with_fixture();
    let before = telos_bytes(tmp.path());
    let intent_path = tmp.path().join("telos/intents/INT-0042.tel");
    let original = fs::read_to_string(&intent_path).unwrap();
    let changed = original.replace(
        "Invoice payment marks it settled",
        "Invoice payment closes the balance",
    );
    assert_ne!(changed, original, "fixture title changed unexpectedly");
    let (mut server, url, _) = start_server(tmp.path());

    fs::write(&intent_path, &changed).unwrap();
    wait_until("valid drifted snapshot", || {
        let intent = get(&url, "/intent/INT-0042");
        let dashboard = get(&url, "/");
        intent.contains("Invoice payment closes the balance")
            && dashboard.contains("Project state: <strong>drifted</strong>")
            && dashboard.contains("telos/intents/INT-0042.tel")
    });

    fs::write(&intent_path, "&\n").unwrap();
    wait_until("last-good snapshot with reload error", || {
        let intent = get(&url, "/intent/INT-0042");
        intent.contains("Invoice payment closes the balance")
            && intent.contains("Reload error:")
            && intent.contains("unexpected character `&amp;`")
            && !intent.contains("unexpected character `&`")
    });

    fs::write(&intent_path, &original).unwrap();
    wait_until("recovered valid snapshot", || {
        let intent = get(&url, "/intent/INT-0042");
        intent.contains("Invoice payment marks it settled") && !intent.contains("Reload error:")
    });

    server.stop();
    assert_eq!(telos_bytes(tmp.path()), before);
}

#[test]
fn live_view_observes_drifted_and_changing_projects() {
    let drifted = with_fixture();
    let intent_path = drifted.path().join("telos/intents/INT-0042.tel");
    let changed = fs::read_to_string(&intent_path).unwrap().replace(
        "Invoice payment marks it settled",
        "Drifted invoice payment",
    );
    fs::write(intent_path, changed).unwrap();
    let before_drifted_server = telos_bytes(drifted.path());
    let (mut drifted_server, drifted_url, _) = start_server(drifted.path());
    let dashboard = get(&drifted_url, "/");
    assert!(dashboard.contains("Project state: <strong>drifted</strong>"));
    drifted_server.stop();
    assert_eq!(telos_bytes(drifted.path()), before_drifted_server);

    let changing = with_fixture();
    telos(changing.path(), &["change", "open", "observe in progress"])
        .assert()
        .success();
    let before_changing_server = telos_bytes(changing.path());
    let (mut changing_server, changing_url, _) = start_server(changing.path());
    let dashboard = get(&changing_url, "/");
    assert!(dashboard.contains("Project state: <strong>changing</strong>"));
    assert!(dashboard.contains("CHG-0001"));
    changing_server.stop();
    assert_eq!(telos_bytes(changing.path()), before_changing_server);
}

#[test]
fn startup_failure_is_one_normal_envelope_and_a_failure_exit() {
    let tmp = with_fixture();
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = occupied.local_addr().unwrap().port().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_telos"))
        .args(["view", "--port", &port, "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("run telos view on an occupied port");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "stdout: {stdout:?}");
    let envelope: Value = serde_json::from_str(stdout.trim_end()).unwrap();
    let object = envelope.as_object().unwrap();
    assert_eq!(object.len(), 5);
    for key in ["ok", "command", "result", "error", "next_actions"] {
        assert!(object.contains_key(key));
    }
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "view");
    assert_eq!(envelope["result"], Value::Null);
    assert_eq!(envelope["error"]["code"], "TELOS_INTERNAL");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("failed to bind the loopback view server:")
    );
    assert_eq!(envelope["next_actions"], json!([]));
}

#[test]
fn human_startup_line_is_the_loopback_url() {
    let tmp = with_fixture();
    let (mut server, line) = spawn_and_read_startup(tmp.path(), &["view", "--port", "0"]);

    assert!(line.starts_with("http://127.0.0.1:"), "line: {line:?}");
    assert!(line.ends_with("/\n"), "line: {line:?}");
    server.stop();
}
