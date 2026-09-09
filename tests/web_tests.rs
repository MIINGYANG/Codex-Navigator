use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const DEADLINE: Duration = Duration::from_secs(10);

struct Server {
    child: Child,
    root: TempDir,
    address: String,
    token: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Response {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Response {
    fn json(&self) -> Value {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        serde_json::from_slice(&self.body).unwrap()
    }

    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }
}

fn lines(records: &[Value]) -> String {
    records.iter().map(|record| format!("{record}\n")).collect()
}

fn user(text: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
}

fn meta(id: &str) -> Value {
    json!({"type":"session_meta","payload":{"id":id,"cwd":"/synthetic/project","source":"cli"}})
}

fn fixture(root: &Path, id: &str, records: &[Value]) -> PathBuf {
    let directory = root.join("codex/sessions/2026/09/06");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("rollout-{id}.jsonl"));
    fs::write(&path, lines(records)).unwrap();
    path
}

impl Server {
    fn start(root: TempDir, arguments: &[&str]) -> Self {
        Self::start_entry(root, arguments, false)
    }

    fn start_entry(root: TempDir, arguments: &[&str], alias: bool) -> Self {
        let mut command = Command::new(if alias {
            env!("CARGO_BIN_EXE_codex-trail")
        } else {
            env!("CARGO_BIN_EXE_codex-nav")
        });
        if !alias {
            command.arg("--web");
        }
        let mut child = command
            .args(["--port", "0", "--no-open"])
            .args(arguments)
            .env("CODEX_HOME", root.path().join("codex"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(start) = line.find("http://127.0.0.1:") {
                    let url = line[start..].split_whitespace().next().unwrap();
                    if sender.send(url.to_string()).is_err() {
                        break;
                    }
                }
            }
        });
        let url = match receiver.recv_timeout(DEADLINE) {
            Ok(url) => url,
            Err(error) => {
                let state = child.try_wait().unwrap();
                let _ = child.kill();
                let _ = child.wait();
                panic!("web server did not print startup URL: {error}; exit={state:?}");
            }
        };
        let (address, token) = url
            .strip_prefix("http://")
            .unwrap()
            .split_once("/#token=")
            .unwrap();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        Self {
            child,
            root,
            address: address.to_string(),
            token: token.to_string(),
        }
    }

    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
        let mut stream = TcpStream::connect(&self.address).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        let host = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map_or(self.address.as_str(), |(_, value)| *value);
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("host") {
                request.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let split = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let header_text = std::str::from_utf8(&bytes[..split]).unwrap();
        let mut header_lines = header_text.split("\r\n");
        let status = header_lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers: BTreeMap<_, _> = header_lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
            .collect();
        let mut body = bytes[split + 4..].to_vec();
        if headers
            .get("transfer-encoding")
            .is_some_and(|value| value == "chunked")
        {
            let mut remaining = body.as_slice();
            let mut decoded = Vec::new();
            loop {
                let end = remaining
                    .windows(2)
                    .position(|part| part == b"\r\n")
                    .unwrap();
                let size = usize::from_str_radix(
                    std::str::from_utf8(&remaining[..end])
                        .unwrap()
                        .split(';')
                        .next()
                        .unwrap(),
                    16,
                )
                .unwrap();
                if size == 0 {
                    break;
                }
                remaining = &remaining[end + 2..];
                decoded.extend_from_slice(&remaining[..size]);
                remaining = &remaining[size + 2..];
            }
            body = decoded;
        }
        Response {
            status,
            headers,
            body,
        }
    }

    fn get(&self, path: &str) -> Response {
        self.request("GET", path, &[("X-Codex-Nav-Token", &self.token)])
    }

    fn wait(&self, path: &str, ready: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let value = self.get(path).json();
            if ready(&value) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "API did not reach expected state: {path}: {value}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn sessions(&self) -> Value {
        self.wait("/api/sessions?all=1", |value| value["loading"] == false)
    }

    fn key(&self, id: &str) -> String {
        self.sessions()["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == id)
            .unwrap()["key"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn loaded(&self, key: &str, count: usize) -> Value {
        self.wait(&format!("/api/session/{key}"), |value| {
            value["loading"] == false && value["turn_count"] == count
        })
    }
}

#[test]
fn web_static_assets_and_security_headers_expose_no_session_data() {
    let root = TempDir::new().unwrap();
    let path = fixture(
        root.path(),
        "main",
        &[meta("main"), user("private-prompt-marker")],
    );
    let original = fs::read(&path).unwrap();
    let server = Server::start(root, &[]);
    for path in [
        "/",
        "/trail/",
        "/trail/assets/trail.js",
        "/trail/assets/trail.css",
        "/trail/theme-init.js",
        "/theme-init.js",
        "/favicon.svg",
        "/trail/favicon.svg",
    ] {
        let response = server.request("GET", path, &[]);
        assert_eq!(response.status, 200, "{path}");
        assert!(!response.text().contains("private-prompt-marker"));
        assert!(!response.text().contains(&server.token));
        assert_eq!(response.headers["x-content-type-options"], "nosniff");
        assert_eq!(response.headers["referrer-policy"], "no-referrer");
        assert!(!response.headers.contains_key("access-control-allow-origin"));
        assert!(response.headers["cache-control"].contains("no-store"));
    }
    let home = server.request("GET", "/", &[]);
    let favicon = server.request("GET", "/favicon.svg", &[]);
    assert_eq!(favicon.headers["content-type"], "image/svg+xml");
    assert!(favicon.text().contains("<svg"));
    assert_eq!(
        favicon.body,
        server.request("GET", "/trail/favicon.svg", &[]).body
    );
    for entry in ["/", "/index.html", "/trail", "/trail/", "/trail/index.html"] {
        let html = server.request("GET", entry, &[]).text();
        assert!(html.contains("rel=\"icon\""));
        assert!(html.contains("href=\"/trail/favicon.svg\""));
    }
    let csp = &home.headers["content-security-policy"];
    for rule in [
        "default-src 'none'",
        "frame-ancestors 'none'",
        "connect-src 'self'",
    ] {
        assert!(csp.contains(rule), "missing CSP rule: {rule}");
    }
    let info = server.get("/api/info").json();
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(info["watch"], true);
    assert_eq!(info["default_all"], true);
    assert!(info["initial_session"].is_null());
    assert!(!server.get("/api/info").headers["content-security-policy"].contains("unsafe-inline"));
    for path in ["/app.js", "/app.css", "/state.mjs", "/flow.js"] {
        assert_eq!(
            server.request("GET", path, &[]).status,
            404,
            "legacy asset {path}"
        );
    }
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(!server.root.path().join("config").exists());
}

#[test]
fn web_cross_site_top_level_navigation_serves_public_shell_not_api_data() {
    let root = TempDir::new().unwrap();
    fixture(
        root.path(),
        "main",
        &[meta("main"), user("private-navigation-marker")],
    );
    let server = Server::start(root, &[]);
    let mut headers = vec![
        ("Sec-Fetch-Site", "cross-site"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Dest", "document"),
    ];
    let response = server.request("GET", "/", &headers);
    assert_eq!(response.status, 200);
    assert!(!response.text().contains("private-navigation-marker"));
    assert!(!response.text().contains(&server.token));
    headers.push(("X-Codex-Nav-Token", server.token.as_str()));
    assert_eq!(server.request("GET", "/api/info", &headers).status, 403);
    headers.push(("Origin", "https://evil.invalid"));
    assert_eq!(server.request("GET", "/", &headers).status, 403);
}

#[test]
fn web_api_rejects_unauthenticated_cross_origin_rebinding_and_mutations() {
    let root = TempDir::new().unwrap();
    fixture(
        root.path(),
        "main",
        &[meta("main"), user("private-prompt-marker")],
    );
    let server = Server::start(root, &[]);
    for headers in [
        vec![],
        vec![("X-Codex-Nav-Token", "incorrect")],
        vec![
            ("X-Codex-Nav-Token", server.token.as_str()),
            ("Origin", "https://evil.invalid"),
        ],
        vec![
            ("X-Codex-Nav-Token", server.token.as_str()),
            ("Origin", "null"),
        ],
        vec![
            ("X-Codex-Nav-Token", server.token.as_str()),
            ("Host", "evil.invalid"),
        ],
        vec![
            ("X-Codex-Nav-Token", server.token.as_str()),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        vec![
            ("X-Codex-Nav-Token", server.token.as_str()),
            ("X-Codex-Nav-Token", server.token.as_str()),
        ],
    ] {
        let response = server.request("GET", "/api/sessions?all=1", &headers);
        assert_eq!(response.status, 403, "headers={headers:?}");
        assert!(!response.text().contains("private-prompt-marker"));
        assert!(!response.headers.contains_key("access-control-allow-origin"));
    }
    for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
        assert_eq!(
            server
                .request(method, "/api/info", &[("X-Codex-Nav-Token", &server.token)])
                .status,
            405
        );
    }
    assert_eq!(
        server
            .request("GET", &format!("/api/info?token={}", server.token), &[],)
            .status,
        403
    );
    assert_eq!(
        server
            .request(
                "GET",
                "/api/info",
                &[
                    ("X-Codex-Nav-Token", &server.token),
                    ("Content-Length", "1"),
                ],
            )
            .status,
        400
    );
    let origin = format!("http://{}", server.address);
    assert_eq!(
        server
            .request(
                "GET",
                "/api/info",
                &[("X-Codex-Nav-Token", &server.token), ("Origin", &origin)]
            )
            .status,
        200
    );
}

#[test]
fn web_discovery_lists_only_main_and_rejects_browser_supplied_paths() {
    let root = TempDir::new().unwrap();
    let main_path = fixture(root.path(), "main", &[meta("main"), user("main prompt")]);
    fixture(
        root.path(),
        "child",
        &[
            json!({"type":"session_meta","payload":{"id":"child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"main","agent_nickname":"test"}}}}}),
            user("child prompt"),
        ],
    );
    fixture(
        root.path(),
        "unknown",
        &[
            json!({"type":"session_meta","payload":{"id":"unknown","source":"future-unknown-source"}}),
            user("unknown prompt"),
        ],
    );
    fs::write(root.path().join("private.txt"), "arbitrary-private-marker").unwrap();
    let server = Server::start(root, &[]);
    let sessions = server.sessions();
    assert_eq!(sessions["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(sessions["sessions"][0]["id"], "main");
    let key = sessions["sessions"][0]["key"].as_str().unwrap();
    assert!(!key.contains('/'));
    assert_ne!(key, main_path.to_string_lossy());
    for path in [
        "/api/session/unknown-key",
        "/api/session/..%2F..%2Fprivate.txt",
        "/api/session/%2Fetc%2Fpasswd",
        "/..%2Fprivate.txt",
        "/Cargo.toml",
    ] {
        let response = server.get(path);
        assert!(response.status >= 400, "path={path}");
        assert!(!response.text().contains("arbitrary-private-marker"));
    }
}

#[test]
fn web_explicit_cli_session_is_registered_without_redirecting_to_parent() {
    let root = TempDir::new().unwrap();
    fixture(root.path(), "main", &[meta("main"), user("parent prompt")]);
    let child = fixture(
        root.path(),
        "child",
        &[
            json!({"type":"session_meta","payload":{"id":"child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"main"}}}}}),
            user("child-only prompt"),
        ],
    );
    let server = Server::start(root, &["--session", child.to_str().unwrap()]);
    let info = server.get("/api/info").json();
    let key = info["initial_session"].as_str().unwrap();
    let session = server.loaded(key, 1);
    assert_eq!(session["meta"]["id"], "child");
    let turn = server.get(&format!("/api/session/{key}/turn/0")).json();
    assert_eq!(turn["turn"]["prompt"]["text"], "child-only prompt");
    assert_eq!(server.sessions()["sessions"].as_array().unwrap().len(), 1);
}

#[test]
fn web_normalized_turns_separate_lifecycle_warnings_final_and_paginate() {
    let root = TempDir::new().unwrap();
    let mut records = vec![meta("main"), user("修复 authentication regression")];
    for index in 0..12 {
        records.push(json!({"type":"response_item","payload":{"type":"function_call_output","call_id":format!("tool-{index}"),"output":{"exit_code":if index == 0 {1} else {0},"output":format!("activity {index}")}}}));
    }
    records.push(json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"# 最终回复\n保留 <script>synthetic()</script> 作为文本。"}]}}));
    records.push(json!({"type":"event_msg","payload":{"type":"task_complete"}}));
    records.push(user("第二轮等待操作"));
    records.push(json!({"type":"event_msg","payload":{"type":"turn_aborted"}}));
    let path = fixture(root.path(), "main", &records);
    let original = fs::read(&path).unwrap();
    let server = Server::start(root, &[]);
    let key = server.key("main");
    server.loaded(&key, 2);
    let route = format!("/api/session/{key}");
    let turns = server.get(&format!("{route}/turns?limit=1")).json();
    assert_eq!(turns["total"], 2);
    assert_eq!(turns["turns"].as_array().unwrap().len(), 1);
    assert_eq!(turns["turns"][0]["status"], "completed");
    assert_eq!(turns["turns"][0]["errors"], 1);
    assert_eq!(turns["turns"][0]["has_final"], true);
    let second = server
        .get(&format!("{route}/turns?offset=1&limit=1"))
        .json();
    assert_eq!(second["turns"][0]["index"], 1);
    assert_eq!(second["turns"][0]["status"], "interrupted");
    let search = server
        .get(&format!("{route}/turns?q=authentication"))
        .json();
    assert_eq!(search["total"], 1);
    assert_eq!(search["turns"][0]["index"], 0);
    let turn = server.get(&format!("{route}/turn/0?limit=999")).json();
    assert_eq!(turn["items"].as_array().unwrap().len(), 8);
    assert!(turn["items_total"].as_u64().unwrap() > 8);
    assert_eq!(turn["next_offset"], 8);
    assert!(turn["final_answer"]["index"].as_u64().unwrap() >= 8);
    assert!(turn["final_answer"]["text"]
        .as_str()
        .unwrap()
        .contains("最终回复"));
    assert_eq!(turn["turn"]["activity"]["errors"], 1);
    let remaining = server.get(&format!("{route}/turn/0?offset=8")).json();
    assert_eq!(remaining["items"][0]["index"], 8);
    assert!(remaining["next_offset"].is_null());
    let no_final = server.get(&format!("{route}/turn/1")).json();
    assert!(no_final["final_answer"].is_null());
    let text = server.get(&format!("{route}/turn/0/text"));
    assert_eq!(text.status, 200);
    assert!(text.text().contains("FINAL ANSWER"));
    assert!(text.text().contains("activity 11"));
    for suffix in [
        "turns?offset=bad",
        "turns?limit=-1",
        "turn/0?offset=bad",
        "turn/0?limit=-1",
    ] {
        assert_eq!(server.get(&format!("{route}/{suffix}")).status, 400);
    }
    assert_eq!(server.get(&format!("{route}/turn/99")).status, 404);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn web_watch_appends_partial_records_and_replacement_changes_generation() {
    let root = TempDir::new().unwrap();
    let path = fixture(root.path(), "main", &[meta("main"), user("first")]);
    let server = Server::start(root, &[]);
    let key = server.key("main");
    let initial = server.loaded(&key, 1);
    let record = lines(&[user("中文 second")]);
    let split = record.find('中').unwrap() + 1;
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&record.as_bytes()[..split])
        .unwrap();
    thread::sleep(Duration::from_millis(200));
    let partial = server.get(&format!("/api/session/{key}")).json();
    assert_eq!(partial["turn_count"], 1);
    assert_eq!(partial["stats"]["malformed_records"], 0);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&record.as_bytes()[split..])
        .unwrap();
    let appended = server.loaded(&key, 2);
    assert_eq!(appended["generation"], initial["generation"]);
    assert!(appended["revision"].as_u64().unwrap() > initial["revision"].as_u64().unwrap());
    let replacement = path.with_extension("replacement");
    fs::write(&replacement, lines(&[meta("main"), user("replacement")])).unwrap();
    fs::rename(&replacement, &path).unwrap();
    let changed = server.wait(&format!("/api/session/{key}"), |value| {
        value["loading"] == false
            && value["turn_count"] == 1
            && value["generation"] != initial["generation"]
    });
    assert_ne!(changed["generation"], initial["generation"]);
    let turn = server.get(&format!("/api/session/{key}/turn/0")).json();
    assert_eq!(turn["turn"]["prompt"]["text"], "replacement");
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        lines(&[meta("main"), user("replacement")])
    );
}

#[test]
fn web_no_watch_requires_manual_refresh_and_remains_read_only() {
    let root = TempDir::new().unwrap();
    let path = fixture(root.path(), "main", &[meta("main"), user("first")]);
    let server = Server::start(root, &["--no-watch"]);
    let key = server.key("main");
    assert_eq!(server.loaded(&key, 1)["watch"], false);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(lines(&[user("second")]).as_bytes())
        .unwrap();
    let expected = fs::read(&path).unwrap();
    thread::sleep(Duration::from_millis(350));
    assert_eq!(
        server.get(&format!("/api/session/{key}")).json()["turn_count"],
        1
    );
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        server
            .get(&format!("/api/trail/session/{key}?refresh=1"))
            .status,
        200
    );
    server.loaded(&key, 2);
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(fs::read(path).unwrap(), expected);
    assert!(!server.root.path().join("config").exists());
}

#[test]
fn web_no_watch_search_snapshot_changes_only_after_manual_refresh() {
    let root = TempDir::new().unwrap();
    let path = fixture(
        root.path(),
        "main",
        &[meta("main"), user("original keyword")],
    );
    let server = Server::start(root, &["--no-watch"]);
    let key = server.key("main");
    server.loaded(&key, 1);
    server.wait("/api/trail/search?q=original", |value| {
        value["loading"] == false
            && value["results"]
                .as_array()
                .is_some_and(|items| items.len() == 1)
    });
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(lines(&[user("appended keyword")]).as_bytes())
        .unwrap();
    // Exceed the normal search index refresh interval: manual mode must still
    // return its existing snapshot and leave the opened graph unchanged.
    thread::sleep(Duration::from_millis(3100));
    assert_eq!(
        server.get("/api/trail/search?q=appended").json()["results"],
        json!([])
    );
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.get(&format!("/api/trail/session/{key}?refresh=1"));
    server.loaded(&key, 2);
    server.get("/api/sessions?refresh=1");
    server.sessions();
    let updated = server.wait("/api/trail/search?q=appended", |value| {
        value["loading"] == false
            && value["results"]
                .as_array()
                .is_some_and(|items| items.len() == 1)
    });
    assert_eq!(updated["results"][0]["title"], "appended keyword");
}

#[test]
fn web_defaults_to_all_dates_and_latest_project_not_current_directory() {
    let root = TempDir::new().unwrap();
    fixture(
        root.path(),
        "related",
        &[meta("related"), user("current project")],
    );
    let archive = root.path().join("codex/sessions/2010/01/01");
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("rollout-unrelated.jsonl"), lines(&[
        json!({"type":"session_meta","payload":{"id":"unrelated","cwd":"/synthetic/other","source":"cli"}}),
        user("other project newest question"),
    ])).unwrap();
    fs::write(
        root.path().join("codex/session_index.jsonl"),
        lines(&[
            json!({"id":"related","thread_name":"Related","updated_at":"2090-01-01T00:00:00Z"}),
            json!({"id":"unrelated","thread_name":"Unrelated","updated_at":"2090-01-02T00:00:00Z"}),
        ]),
    )
    .unwrap();
    let server = Server::start(root, &["--cwd", "/synthetic/project"]);
    let sessions = server.wait("/api/sessions", |value| value["loading"] == false);
    assert_eq!(sessions["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(sessions["sessions"][0]["id"], "unrelated");
}

#[test]
fn web_empty_installation_is_a_valid_picker_not_an_error() {
    let server = Server::start(TempDir::new().unwrap(), &[]);
    let sessions = server.sessions();
    assert_eq!(sessions["sessions"], json!([]));
    assert!(sessions["error"].is_null());
    assert!(!server.root.path().join("codex").exists());
}

#[cfg(unix)]
#[test]
fn web_discovery_rejects_symlink_escape_but_cli_may_explicitly_open_external_file() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let main_path = fixture(root.path(), "main", &[meta("main"), user("main prompt")]);
    let external = root.path().join("external.jsonl");
    let private = lines(&[meta("outside"), user("outside-private-marker")]);
    fs::write(&external, &private).unwrap();
    symlink(
        &external,
        main_path.parent().unwrap().join("rollout-escape.jsonl"),
    )
    .unwrap();
    let server = Server::start(root, &[]);
    let sessions = server.sessions();
    assert_eq!(sessions["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(sessions["sessions"][0]["id"], "main");
    assert!(!sessions.to_string().contains("outside-private-marker"));
    assert_eq!(fs::read_to_string(&external).unwrap(), private);

    let explicit_root = TempDir::new().unwrap();
    let explicit = Server::start(explicit_root, &["--session", external.to_str().unwrap()]);
    let info = explicit.get("/api/info").json();
    let key = info["initial_session"].as_str().unwrap();
    assert_eq!(explicit.loaded(key, 1)["meta"]["id"], "outside");
    let graph = explicit.get(&format!("/api/trail/session/{key}")).json();
    assert_eq!(graph["meta"]["cwd"], "/synthetic/project");
    assert!(explicit.sessions()["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        explicit.get(&format!("/api/session/{key}/turn/0")).json()["turn"]["prompt"]["text"],
        "outside-private-marker"
    );
    assert_eq!(fs::read_to_string(external).unwrap(), private);
}

#[test]
fn web_turn_directory_is_bounded_and_refresh_discovers_new_main_sessions() {
    let root = TempDir::new().unwrap();
    let mut records = vec![meta("main")];
    records.extend((0..120).map(|index| user(&format!("unique prompt {index}"))));
    fixture(root.path(), "main", &records);
    let server = Server::start(root, &[]);
    let key = server.key("main");
    server.loaded(&key, 120);
    let first = server
        .get(&format!("/api/session/{key}/turns?limit=9999"))
        .json();
    assert_eq!(first["total"], 120);
    assert_eq!(first["turns"].as_array().unwrap().len(), 100);
    let last = server
        .get(&format!("/api/session/{key}/turns?offset=100&limit=100"))
        .json();
    assert_eq!(last["turns"].as_array().unwrap().len(), 20);
    assert_eq!(last["turns"][0]["index"], 100);
    assert_eq!(
        server
            .get(&format!("/api/session/{key}/turns?offset=999"))
            .json()["turns"],
        json!([])
    );
    fixture(
        server.root.path(),
        "new-main",
        &[meta("new-main"), user("newly discovered")],
    );
    assert_eq!(server.get("/api/sessions?all=1&refresh=1").status, 200);
    server.wait("/api/sessions?all=1", |value| {
        value["loading"] == false
            && value["sessions"]
                .as_array()
                .is_some_and(|sessions| sessions.len() == 2)
    });
}

#[test]
fn web_flow_search_orders_before_pagination_without_changing_relevance_default() {
    let root = TempDir::new().unwrap();
    let mut records = vec![meta("flow-order")];
    records.extend((0..120).map(|index| user(&format!("shared question {index}"))));
    let path = fixture(root.path(), "flow-order", &records);
    let original = fs::read(&path).unwrap();
    let server = Server::start(root, &[]);
    let key = server.key("flow-order");
    server.loaded(&key, 120);
    let route = format!("/api/session/{key}/turns?q=shared");
    assert_eq!(server.get(&route).json()["turns"][0]["index"], 119);
    let first = server
        .get(&format!("{route}&order=chronological&limit=100"))
        .json();
    assert_eq!(first["total"], 120);
    assert_eq!(first["turns"][0]["index"], 0);
    assert_eq!(first["turns"][99]["index"], 99);
    let next = server
        .get(&format!("{route}&order=chronological&offset=100"))
        .json();
    assert_eq!(next["turns"][0]["index"], 100);
    assert_eq!(next["turns"][19]["index"], 119);
    assert_eq!(server.get(&format!("{route}&order=invalid")).status, 400);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn web_cache_eviction_reloads_with_new_generation_and_correct_session() {
    let root = TempDir::new().unwrap();
    for id in ["first", "second", "third"] {
        fixture(root.path(), id, &[meta(id), user(&format!("{id} prompt"))]);
    }
    let server = Server::start(root, &[]);
    let first_key = server.key("first");
    let initial = server.loaded(&first_key, 1);
    for id in ["second", "third"] {
        let key = server.key(id);
        assert_eq!(server.loaded(&key, 1)["meta"]["id"], id);
    }
    let reopened = server.loaded(&first_key, 1);
    assert_ne!(reopened["generation"], initial["generation"]);
    assert_eq!(reopened["meta"]["id"], "first");
    assert_eq!(
        server
            .get(&format!("/api/session/{first_key}/turn/0"))
            .json()["turn"]["prompt"]["text"],
        "first prompt"
    );
}

#[test]
fn trail_graph_search_are_deterministic_prompt_only_and_do_not_evict_live_sessions() {
    let root = TempDir::new().unwrap();
    let branch_path = fixture(root.path(), "synthetic-trail", &[]);
    fs::write(&branch_path, include_str!("fixtures/trail_branch.jsonl")).unwrap();
    fixture(
        root.path(),
        "other",
        &[
            meta("other"),
            user("crosssession keyword marker"),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"ASSISTANT_ONLY_SECRET"}]}}),
        ],
    );
    fixture(root.path(), "third", &[meta("third"), user("third prompt")]);
    let original = fs::read(&branch_path).unwrap();
    let server = Server::start(root, &[]);
    let key = server.key("synthetic-trail");
    let graph = server.wait(&format!("/api/trail/session/{key}"), |v| {
        v["loading"] == false
    });
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 4);
    assert_eq!(graph["edges"][2]["source"], "q1");
    assert_eq!(graph["edges"][2]["type"], "branch");
    assert!(!graph.to_string().contains("PRIVATE_REASONING"));
    let results = server.wait("/api/trail/search?q=crosssession", |v| {
        v["loading"] == false
    });
    assert_eq!(results["results"].as_array().unwrap().len(), 1);
    assert_eq!(results["results"][0]["nodeId"], "q1");
    assert_eq!(
        server
            .get("/api/trail/search?q=ASSISTANT_ONLY_SECRET")
            .json()["results"],
        json!([])
    );
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["generation"],
        graph["generation"]
    );
    assert_eq!(
        server
            .request("GET", "/api/trail/search?q=secret", &[])
            .status,
        403
    );
    assert_eq!(
        server
            .request("GET", &format!("/api/trail/events?session={key}"), &[])
            .status,
        403
    );
    assert_eq!(server.get("/api/trail/session/notregistered").status, 404);
    assert_eq!(fs::read(&branch_path).unwrap(), original);
}

#[test]
fn trail_project_path_preserves_recorded_text_and_missing_values() {
    let root = TempDir::new().unwrap();
    let cwd = "/home/example/项目/Personal Project/codex-navigator";
    let mut recorded = meta("recorded");
    recorded["payload"]["cwd"] = json!(cwd);
    fixture(
        root.path(),
        "recorded",
        &[recorded, user("recorded project")],
    );
    let mut missing = meta("missing");
    missing["payload"].as_object_mut().unwrap().remove("cwd");
    fixture(root.path(), "missing", &[missing, user("missing project")]);
    let server = Server::start(root, &[]);
    for (id, expected) in [("recorded", json!(cwd)), ("missing", Value::Null)] {
        let key = server.key(id);
        let listing = server.get("/api/sessions?all=1").json();
        let summary = listing["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["key"] == key)
            .unwrap();
        assert_eq!(summary["cwd"], expected);
        let graph = server.wait(&format!("/api/trail/session/{key}"), |v| {
            v["loading"] == false
        });
        assert_eq!(graph["meta"]["cwd"], expected);
        assert_eq!(server.loaded(&key, 1)["meta"]["cwd"], expected);
    }
}

#[test]
fn trail_sse_updates_only_complete_appended_records_and_is_header_authenticated() {
    let root = TempDir::new().unwrap();
    let path = fixture(root.path(), "live", &[meta("live"), user("first question")]);
    let server = Server::start(root, &[]);
    let key = server.key("live");
    server.loaded(&key, 1);
    let mut stream = TcpStream::connect(&server.address).unwrap();
    stream.set_read_timeout(Some(DEADLINE)).unwrap();
    write!(stream,"GET /api/trail/events?session={key} HTTP/1.1\r\nHost: {}\r\nX-Codex-Nav-Token: {}\r\nConnection: close\r\n\r\n",server.address,server.token).unwrap();
    let mut reader = BufReader::new(stream);
    fn event(reader: &mut BufReader<TcpStream>) -> Value {
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "SSE ended unexpectedly"
            );
            if let Some(json) = line.strip_prefix("data: ") {
                return serde_json::from_str(json.trim()).unwrap();
            }
        }
    }
    assert_eq!(event(&mut reader)["turn_count"], 1);
    let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
    writer.write_all(b"{malformed}\n").unwrap();
    let malformed = event(&mut reader);
    assert_eq!(malformed["turn_count"], 1);
    assert_eq!(malformed["stats"]["malformed_records"], 1);
    let record = user("second question").to_string();
    let split = record.len() / 2;
    writer.write_all(&record.as_bytes()[..split]).unwrap();
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    writer.write_all(&record.as_bytes()[split..]).unwrap();
    writer.write_all(b"\n").unwrap();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let value = event(&mut reader);
        if value["turn_count"] == 2 {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"][1]["title"],
        "second question"
    );
}

#[test]
fn trail_embedded_assets_allow_only_inline_style_attributes_not_inline_scripts() {
    let server = Server::start(TempDir::new().unwrap(), &[]);
    for path in [
        "/",
        "/trail/",
        "/trail/assets/trail.js",
        "/trail/assets/trail.css",
        "/trail/theme-init.js",
        "/theme-init.js",
    ] {
        let response = server.request("GET", path, &[]);
        assert_eq!(response.status, 200);
        let csp = &response.headers["content-security-policy"];
        assert!(csp.contains("script-src 'self';"));
        assert!(csp.contains("style-src-attr 'unsafe-inline';"));
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
    }
    let bootstrap = server.request("GET", "/trail/theme-init.js", &[]);
    assert_eq!(
        bootstrap.body,
        server.request("GET", "/theme-init.js", &[]).body
    );
    assert!(bootstrap.headers["content-type"].starts_with("text/javascript"));
    let html = server.request("GET", "/", &[]).text();
    assert!(html.contains("src=\"/trail/theme-init.js\""));
    assert!(!html.contains("<script>"));
}

#[test]
fn web_and_trail_entrypoints_serve_the_same_app_and_honor_explicit_session() {
    for alias in [false, true] {
        let root = TempDir::new().unwrap();
        let path = fixture(
            root.path(),
            "selected",
            &[meta("selected"), user("selected question")],
        );
        fixture(
            root.path(),
            "other",
            &[meta("other"), user("other question")],
        );
        let original = fs::read(&path).unwrap();
        let server = Server::start_entry(
            root,
            &["--session", path.to_str().unwrap(), "--no-watch"],
            alias,
        );
        let home = server.request("GET", "/", &[]);
        assert_eq!(home.status, 200);
        assert!(home.text().contains("/trail/assets/trail.js"));
        for path in ["/index.html", "/trail", "/trail/", "/trail/index.html"] {
            assert_eq!(server.request("GET", path, &[]).body, home.body);
        }
        let info = server.get("/api/info").json();
        assert_eq!(info["default_all"], true);
        assert_eq!(info["watch"], false);
        let key = info["initial_session"].as_str().unwrap();
        let graph = server.wait(&format!("/api/trail/session/{key}"), |value| {
            value["loading"] == false
        });
        assert_eq!(graph["nodes"][0]["title"], "selected question");
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[test]
fn trail_sse_connection_budget_does_not_starve_regular_api_requests() {
    let root = TempDir::new().unwrap();
    fixture(root.path(), "live", &[meta("live"), user("question")]);
    let server = Server::start(root, &[]);
    let key = server.key("live");
    server.loaded(&key, 1);
    let mut readers = Vec::new();
    for _ in 0..8 {
        let mut stream = TcpStream::connect(&server.address).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        write!(stream,"GET /api/trail/events?session={key} HTTP/1.1\r\nHost: {}\r\nX-Codex-Nav-Token: {}\r\nConnection: close\r\n\r\n",server.address,server.token).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("200 OK"));
        readers.push(reader);
    }
    assert_eq!(server.get("/api/health").json()["ok"], true);
    assert_eq!(
        server
            .get(&format!("/api/trail/events?session={key}"))
            .status,
        503
    );
    assert_eq!(
        server.get(&format!("/api/trail/session/{key}")).json()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(readers);
}

#[cfg(unix)]
#[test]
fn web_ctrl_c_gracefully_stops_server_and_session_worker() {
    let root = TempDir::new().unwrap();
    let path = fixture(root.path(), "main", &[meta("main"), user("first")]);
    let original = fs::read(&path).unwrap();
    let mut server = Server::start(root, &[]);
    let key = server.key("main");
    server.loaded(&key, 1);
    // An incomplete HTTP header must not keep graceful shutdown waiting indefinitely.
    let mut stalled = TcpStream::connect(&server.address).unwrap();
    stalled.write_all(b"GET / HTTP/1.1\r\nHost:").unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(Command::new("kill")
        .args(["-INT", &server.child.id().to_string()])
        .status()
        .unwrap()
        .success());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(status.success(), "Ctrl+C did not exit gracefully: {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Ctrl+C left server or worker running"
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert!(TcpStream::connect(&server.address).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}
