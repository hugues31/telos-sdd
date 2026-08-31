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

#[cfg(all(unix, not(target_os = "macos")))]
use common::{fake_browser, wait_for_browser_target};
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_telos"));
    command.args(args).current_dir(root);
    spawn_and_read_startup_command(command)
}

fn spawn_and_read_startup_command(mut command: Command) -> (ServerChild, String) {
    let mut child = ServerChild(Some(
        command
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

struct HttpResponse {
    status_line: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).expect("HTTP response body is UTF-8")
    }
}

fn request_with_headers(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> HttpResponse {
    let address = url
        .strip_prefix("http://")
        .and_then(|url| url.strip_suffix('/'))
        .expect("server URL uses http and ends in slash");
    let mut stream = TcpStream::connect(address).expect("connect to loopback view server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\n").unwrap();
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        write!(stream, "Host: {address}\r\n").unwrap();
    }
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    write!(stream, "Connection: close\r\n\r\n").unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response separates headers and body");
    let head = std::str::from_utf8(&response[..split]).expect("HTTP response head is UTF-8");
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .expect("HTTP response has a status line")
        .to_string();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').expect("HTTP header has a colon");
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect();
    HttpResponse {
        status_line,
        headers,
        body: response[split + 4..].to_vec(),
    }
}

fn request(url: &str, method: &str, path: &str) -> HttpResponse {
    request_with_headers(url, method, path, &[])
}

fn get(url: &str, path: &str) -> HttpResponse {
    request(url, "GET", path)
}

fn establish_session(url: &str) -> (HttpResponse, String) {
    let shell = get(url, "/");
    let cookie = shell
        .header("set-cookie")
        .expect("the live shell establishes a session")
        .split(';')
        .next()
        .expect("Set-Cookie starts with a cookie pair")
        .to_string();
    (shell, cookie)
}

fn session_get(url: &str, cookie: &str, path: &str) -> HttpResponse {
    request_with_headers(url, "GET", path, &[("Cookie", cookie)])
}

fn data_payload(response: &HttpResponse) -> Value {
    const PREFIX: &str = "window.__TELOS_DATA__ = ";
    const SUFFIX: &str = ";\n";

    let script = response.text();
    assert!(script.starts_with(PREFIX), "data.js prefix: {script}");
    assert!(script.ends_with(SUFFIX), "data.js suffix: {script}");
    serde_json::from_str(&script[PREFIX.len()..script.len() - SUFFIX.len()])
        .expect("data.js assignment contains JSON")
}

fn referenced_assets(index: &str) -> Vec<String> {
    let mut assets = Vec::new();
    for attribute in ["src=\"", "href=\""] {
        let mut remaining = index;
        while let Some(start) = remaining.find(attribute) {
            remaining = &remaining[start + attribute.len()..];
            let end = remaining.find('"').expect("quoted asset attribute");
            let value = &remaining[..end];
            if let Some(path) = value.strip_prefix("./assets/") {
                assets.push(format!("/assets/{path}"));
            }
            remaining = &remaining[end + 1..];
        }
    }
    assets.sort();
    assets.dedup();
    assets
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
fn live_data_requires_the_exact_loopback_session_and_browser_origin() {
    let tmp = with_fixture();
    let (mut server, url, _) = start_server(tmp.path());

    for path in ["/data.js", "/live.json"] {
        let unauthenticated = get(&url, path);
        assert!(
            unauthenticated.status_line.starts_with("HTTP/1.1 403 "),
            "{path}: {}",
            unauthenticated.status_line
        );
        assert_eq!(
            unauthenticated.header("cross-origin-resource-policy"),
            Some("same-origin")
        );
        assert_eq!(unauthenticated.header("cache-control"), Some("no-store"));
        assert!(!unauthenticated.text().contains("INT-0042"));
    }

    let wrong_host = request_with_headers(&url, "GET", "/", &[("Host", "attacker.invalid")]);
    assert!(
        wrong_host.status_line.starts_with("HTTP/1.1 421 "),
        "{}",
        wrong_host.status_line
    );

    let shell = get(&url, "/");
    assert!(shell.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        shell.header("cross-origin-resource-policy"),
        Some("same-origin")
    );
    assert_eq!(shell.header("cache-control"), Some("no-store"));
    let set_cookie = shell
        .header("set-cookie")
        .expect("the shell establishes a live-view session");
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("Set-Cookie starts with a cookie pair");
    let port = url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|url| url.strip_suffix('/'))
        .expect("the advertised URL carries its loopback port");
    let cookie_name = format!("telos_view_session_{port}");
    let credential = cookie
        .strip_prefix(&format!("{cookie_name}="))
        .expect("the session cookie name is scoped to the advertised port");
    assert_eq!(credential.len(), 64, "256-bit credential encoded as hex");
    assert!(credential.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        set_cookie,
        format!("{cookie_name}={credential}; HttpOnly; SameSite=Strict; Path=/")
    );
    assert!(!shell.text().contains(credential));

    let mut wrong_credential = credential.to_string();
    wrong_credential.replace_range(
        ..1,
        if credential.starts_with('0') {
            "1"
        } else {
            "0"
        },
    );
    let wrong_cookie = format!("{cookie_name}={wrong_credential}");
    let wrong_session = request_with_headers(&url, "GET", "/data.js", &[("Cookie", &wrong_cookie)]);
    assert!(wrong_session.status_line.starts_with("HTTP/1.1 403 "));

    for path in ["/data.js", "/live.json"] {
        let authenticated = request_with_headers(
            &url,
            "GET",
            path,
            &[("Cookie", cookie), ("Sec-Fetch-Site", "same-origin")],
        );
        assert!(
            authenticated.status_line.starts_with("HTTP/1.1 200 "),
            "{path}: {}",
            authenticated.status_line
        );
        assert_eq!(
            authenticated.header("cross-origin-resource-policy"),
            Some("same-origin")
        );
    }

    let raw_client = request_with_headers(&url, "GET", "/live.json", &[("Cookie", cookie)]);
    assert!(raw_client.status_line.starts_with("HTTP/1.1 200 "));

    for fetch_site in ["same-site", "cross-site"] {
        let foreign = request_with_headers(
            &url,
            "GET",
            "/data.js",
            &[("Cookie", cookie), ("Sec-Fetch-Site", fetch_site)],
        );
        assert!(
            foreign.status_line.starts_with("HTTP/1.1 403 "),
            "{fetch_site}: {}",
            foreign.status_line
        );
        assert!(!foreign.text().contains("INT-0042"));
    }

    let rebound = request_with_headers(
        &url,
        "GET",
        "/data.js",
        &[("Host", "localhost:3000"), ("Cookie", cookie)],
    );
    assert!(
        rebound.status_line.starts_with("HTTP/1.1 421 "),
        "{}",
        rebound.status_line
    );

    server.stop();
}

#[test]
fn concurrent_live_servers_keep_independent_host_cookie_names() {
    let first = with_fixture();
    let second = with_fixture();
    let (mut first_server, first_url, _) = start_server(first.path());
    let (mut second_server, second_url, _) = start_server(second.path());
    let (_, first_cookie) = establish_session(&first_url);
    let (_, second_cookie) = establish_session(&second_url);
    let first_name = first_cookie.split_once('=').unwrap().0;
    let second_name = second_cookie.split_once('=').unwrap().0;

    assert_ne!(first_name, second_name, "cookies are scoped by port name");
    assert_ne!(first_cookie, second_cookie, "credentials are per-process");

    let browser_cookie_header = format!("{first_cookie}; {second_cookie}");
    for url in [&first_url, &second_url] {
        let response = request_with_headers(
            url,
            "GET",
            "/live.json",
            &[("Cookie", &browser_cookie_header)],
        );
        assert!(
            response.status_line.starts_with("HTTP/1.1 200 "),
            "{url}: {}",
            response.status_line
        );
    }

    first_server.stop();
    second_server.stop();
}

#[test]
fn serves_the_spa_and_payload_on_loopback() {
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

    let (index, cookie) = establish_session(&url);
    assert!(index.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        index.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(index.text().contains("<!doctype html>"));
    assert!(index.text().contains("<div id=\"app\"></div>"));
    assert!(index.text().contains("<script src=\"./data.js\"></script>"));

    let assets = referenced_assets(index.text());
    assert!(!assets.is_empty(), "index.html references embedded assets");
    for path in assets {
        let response = get(&url, &path);
        assert!(
            response.status_line.starts_with("HTTP/1.1 200 "),
            "{path}: {}",
            response.status_line
        );
        let expected_type = if path.ends_with(".css") {
            "text/css; charset=utf-8"
        } else if path.ends_with(".js") {
            "text/javascript; charset=utf-8"
        } else {
            panic!("unexpected asset type in index.html: {path}");
        };
        assert_eq!(response.header("content-type"), Some(expected_type));
    }

    let data = session_get(&url, &cookie, "/data.js");
    assert!(data.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        data.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(data.header("cache-control"), Some("no-store"));
    let payload = data_payload(&data);
    assert_eq!(payload["meta"]["mode"], "live");
    assert!(data.text().contains("INT-0042"));
    assert!(data.text().contains("SCN-0107"));

    let live = session_get(&url, &cookie, "/live.json");
    assert!(live.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(live.header("content-type"), Some("application/json"));
    assert_eq!(live.header("cache-control"), Some("no-store"));
    assert_eq!(
        live.header("cross-origin-resource-policy"),
        Some("same-origin")
    );
    let status: Value = serde_json::from_slice(&live.body).expect("live.json is valid JSON");
    assert!(status["generation"].is_u64());
    assert_eq!(status["reload_error"], Value::Null);
    assert_eq!(status["watcher_error"], Value::Null);

    for path in [
        "/intent/INT-9999",
        "/graph",
        "/nope.js",
        "/assets/%2e%2e/index.html",
    ] {
        let missing = get(&url, path);
        assert!(
            missing.status_line.starts_with("HTTP/1.1 404 "),
            "{path}: {}",
            missing.status_line
        );
    }

    server.stop();
    assert_eq!(telos_bytes(tmp.path()), before);
}

#[test]
fn asset_paths_require_exactly_one_leading_slash() {
    let tmp = with_fixture();
    let (mut server, url, _) = start_server(tmp.path());

    let canonical = get(&url, "/assets/app.js");
    assert!(canonical.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        canonical.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(!canonical.body.is_empty());

    for path in ["//assets/app.js", "/assets//app.js"] {
        let response = get(&url, path);
        assert!(
            response.status_line.starts_with("HTTP/1.1 404 "),
            "{path}: {}",
            response.status_line
        );
    }

    server.stop();
}

#[test]
fn asset_fallback_rejects_mutating_http_methods() {
    let tmp = with_fixture();
    let (mut server, url, _) = start_server(tmp.path());

    for method in ["POST", "PUT"] {
        let response = request(&url, method, "/assets/app.js");
        assert!(
            response.status_line.starts_with("HTTP/1.1 405 "),
            "{method}: {}",
            response.status_line
        );
        let allow = response.header("allow").expect("405 includes Allow");
        assert!(allow.split(',').any(|entry| entry.trim() == "GET"));
        assert!(allow.split(',').any(|entry| entry.trim() == "HEAD"));
        assert!(response.body.is_empty());
    }

    server.stop();
}

#[test]
fn asset_head_matches_get_headers_without_a_body() {
    let tmp = with_fixture();
    let (mut server, url, _) = start_server(tmp.path());

    let get = get(&url, "/assets/app.js");
    let head = request(&url, "HEAD", "/assets/app.js");

    assert!(head.status_line.starts_with("HTTP/1.1 200 "));
    assert_eq!(head.header("content-type"), get.header("content-type"));
    assert_eq!(head.header("content-length"), get.header("content-length"));
    assert!(head.body.is_empty());

    server.stop();
}

#[test]
fn reloads_last_good_snapshot_and_recovers_after_invalid_edits() {
    let tmp = with_fixture();
    let before = telos_bytes(tmp.path());
    let intent_path = tmp
        .path()
        .join("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel");
    let original = fs::read_to_string(&intent_path).unwrap();
    let changed = original.replace(
        "Invoice payment marks it settled",
        "Invoice payment closes the balance",
    );
    assert_ne!(changed, original, "fixture title changed unexpectedly");
    let (mut server, url, _) = start_server(tmp.path());
    let (_, cookie) = establish_session(&url);
    let initial_status: Value =
        serde_json::from_slice(&session_get(&url, &cookie, "/live.json").body).unwrap();
    let initial_generation = initial_status["generation"].as_u64().unwrap();

    fs::write(&intent_path, &changed).unwrap();
    wait_until("valid drifted snapshot", || {
        let data = session_get(&url, &cookie, "/data.js");
        let live: Value =
            serde_json::from_slice(&session_get(&url, &cookie, "/live.json").body).unwrap();
        data.text().contains("Invoice payment closes the balance")
            && live["generation"].as_u64().unwrap() > initial_generation
            && live["reload_error"].is_null()
    });
    let valid_data = session_get(&url, &cookie, "/data.js");
    let valid_payload = data_payload(&valid_data);
    assert_eq!(valid_payload["snapshot"]["dashboard"]["state"], "drifted");
    assert!(
        valid_data
            .text()
            .contains("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel")
    );
    let valid_status: Value =
        serde_json::from_slice(&session_get(&url, &cookie, "/live.json").body).unwrap();
    let valid_generation = valid_status["generation"].as_u64().unwrap();

    fs::write(&intent_path, "&\n").unwrap();
    wait_until("last-good snapshot with reload error", || {
        let status: Value =
            serde_json::from_slice(&session_get(&url, &cookie, "/live.json").body).unwrap();
        status["reload_error"]
            .as_str()
            .is_some_and(|error| error.contains("unexpected character `&`"))
    });
    let invalid_status_response = session_get(&url, &cookie, "/live.json");
    let invalid_status: Value = serde_json::from_slice(&invalid_status_response.body).unwrap();
    assert_eq!(invalid_status["generation"], valid_generation);
    assert_eq!(invalid_status["watcher_error"], Value::Null);
    assert!(
        invalid_status["reload_error"]
            .as_str()
            .unwrap()
            .contains("unexpected character `&`")
    );
    assert!(!invalid_status_response.text().contains("&amp;"));
    assert!(
        session_get(&url, &cookie, "/data.js")
            .text()
            .contains("Invoice payment closes the balance")
    );

    fs::write(&intent_path, &original).unwrap();
    wait_until("recovered valid snapshot", || {
        let data = session_get(&url, &cookie, "/data.js");
        let live: Value =
            serde_json::from_slice(&session_get(&url, &cookie, "/live.json").body).unwrap();
        data.text().contains("Invoice payment marks it settled")
            && live["generation"].as_u64().unwrap() > valid_generation
            && live["reload_error"].is_null()
    });

    server.stop();
    assert_eq!(telos_bytes(tmp.path()), before);
}

#[test]
fn live_view_observes_drifted_and_changing_projects() {
    let drifted = with_fixture();
    let intent_path = drifted
        .path()
        .join("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel");
    let changed = fs::read_to_string(&intent_path).unwrap().replace(
        "Invoice payment marks it settled",
        "Drifted invoice payment",
    );
    fs::write(intent_path, changed).unwrap();
    let before_drifted_server = telos_bytes(drifted.path());
    let (mut drifted_server, drifted_url, _) = start_server(drifted.path());
    let (_, drifted_cookie) = establish_session(&drifted_url);
    let payload = data_payload(&session_get(&drifted_url, &drifted_cookie, "/data.js"));
    assert_eq!(payload["snapshot"]["dashboard"]["state"], "drifted");
    drifted_server.stop();
    assert_eq!(telos_bytes(drifted.path()), before_drifted_server);

    let changing = with_fixture();
    telos(changing.path(), &["change", "open", "observe in progress"])
        .assert()
        .success();
    let before_changing_server = telos_bytes(changing.path());
    let (mut changing_server, changing_url, _) = start_server(changing.path());
    let (_, changing_cookie) = establish_session(&changing_url);
    let payload = data_payload(&session_get(&changing_url, &changing_cookie, "/data.js"));
    assert_eq!(payload["snapshot"]["dashboard"]["state"], "changing");
    assert_eq!(
        payload["snapshot"]["dashboard"]["open_changes"][0]["id"],
        "CHG-0001"
    );
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

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn open_launches_the_default_browser_with_the_live_url() {
    let tmp = with_fixture();
    let (_browser_tmp, browser, target_log) = fake_browser();
    let mut command = Command::new(env!("CARGO_BIN_EXE_telos"));
    command
        .args(["view", "--port", "0", "--open", "--json"])
        .current_dir(tmp.path())
        .env("BROWSER", browser)
        .env("TELOS_TEST_BROWSER_TARGET", &target_log);

    let (mut server, line) = spawn_and_read_startup_command(command);
    let envelope: Value = serde_json::from_str(line.trim_end()).unwrap();
    let url = envelope["result"]["url"]
        .as_str()
        .unwrap_or_else(|| panic!("live view failed to start: {envelope}"));
    assert_eq!(wait_for_browser_target(&target_log), url);
    server.stop();
}
