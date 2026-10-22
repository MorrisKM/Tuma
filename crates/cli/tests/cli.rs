//! Runs the real `tuma` binary, as a terminal or an agent would, against a stub HTTP server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Answers every request with 201 and `{"id": 1, "echo": <request body>}`, and returns its base URL.
fn stub_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let echo: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let out = json!({ "id": 1, "echo": echo }).to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}",
                out.len()
            );
        }
    });
    format!("http://{addr}")
}

/// A project folder (with .git) and a data folder for history, removed when dropped.
struct Project(PathBuf);

impl Project {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tuma-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app/.git")).unwrap();
        Self(dir)
    }

    fn app(&self) -> PathBuf {
        self.0.join("app")
    }

    fn tuma(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_tuma"))
            .args(args)
            .current_dir(self.app())
            .env("TUMA_DATA_DIR", self.0.join("data"))
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        (out.status.code().unwrap_or(-1), text)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn send_checks_save_and_test() {
    let base = stub_server();
    let project = Project::new("send");
    let var = format!("baseUrl={base}");

    // A failing check: exit code 1, nothing saved.
    let (code, out) = project.tuma(&[
        "send",
        "POST",
        "/users",
        "--var",
        &var,
        "-d",
        r#"{"name":"Ann"}"#,
        "-x",
        "status=200",
    ]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("✗ status is 200 (got 201 Created)"), "{out}");

    // Passing checks, saved as a test, in JSON.
    let (code, out) = project.tuma(&[
        "send",
        "POST",
        "/users",
        "--var",
        &var,
        "-d",
        r#"{"name":"Ann"}"#,
        "-x",
        "status=201",
        "-x",
        "$.echo.name == Ann",
        "--save",
        "Users/create user",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let result: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(result["saved"], "Users/create user.yaml");
    let saved =
        std::fs::read_to_string(project.app().join(".tuma/Users/create user.yaml")).unwrap();
    assert!(saved.contains("expect:\n  status: 201\n"), "{saved}");

    // `tuma test` reruns it; a missing baseUrl fails it.
    let (code, out) = project.tuma(&["test", "--var", &var]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 request passed, 2 checks"), "{out}");
    let (code, out) = project.tuma(&["test"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("needs a baseUrl"), "{out}");

    // The three requests that were sent are in this test's history, tagged as coming from the CLI. The last run
    // stopped before sending.
    let history = std::fs::read_to_string(project.0.join("data/history.jsonl")).unwrap();
    assert_eq!(history.matches("\"source\":\"cli\"").count(), 3);
}

#[test]
fn usage_errors_exit_with_2() {
    let project = Project::new("usage");
    let (code, out) = project.tuma(&["send", "GET", "http://localhost:1/", "-x", "bogus check"]);
    assert_eq!(code, 2, "{out}");
    let (code, _) = project.tuma(&["run"]);
    assert_eq!(code, 2);
}

/// Talks JSON-RPC to `tuma mcp` over stdio.
struct Mcp {
    child: std::process::Child,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(project: &Path, data: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tuma"))
            .arg("mcp")
            .args(args)
            .current_dir(project)
            .env("TUMA_DATA_DIR", data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Self {
            child,
            stdout,
            next_id: 0,
        };
        let init = mcp.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test-agent", "version": "1" } }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "tuma");
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        mcp
    }

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "the server closed its output"
            );
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == id {
                return reply;
            }
        }
    }

    /// Calls a tool and returns (is_error, its JSON output or text).
    fn call(&mut self, tool: &str, args: Value) -> (bool, Value) {
        let reply = self.request("tools/call", json!({ "name": tool, "arguments": args }));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let value = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()));
        (result["isError"].as_bool().unwrap_or(false), value)
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn mcp_agent_loop() {
    let base = stub_server();
    let project = Project::new("mcp");
    let mut mcp = Mcp::start(&project.app(), &project.0.join("data"), &[]);

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for tool in [
        "send_request",
        "run_saved",
        "save_request",
        "wait_for",
        "set_variables",
        "list_requests",
        "read_request",
    ] {
        assert!(names.contains(&tool), "{tool} missing from {names:?}");
    }

    let (err, _) = mcp.call(
        "set_variables",
        json!({ "environment": "local", "vars": { "baseUrl": base } }),
    );
    assert!(!err);
    let (err, out) = mcp.call("wait_for", json!({ "url": "/", "environment": "local" }));
    assert!(!err && out["ready"] == true, "{out}");

    let send = json!({
        "method": "POST", "url": "/users", "body": { "name": "Ann" }, "environment": "local",
        "checks": ["status=201", "$.id exists", "$.echo.name == Ann"], "save_as": "Users/create user",
    });
    let (err, out) = mcp.call("send_request", send);
    assert!(!err, "{out}");
    assert_eq!(out["passed"], true, "{out}");
    assert_eq!(out["saved"], "Users/create user.yaml");

    let (_, out) = mcp.call(
        "run_saved",
        json!({ "environment": "local", "only_tests": true }),
    );
    assert_eq!(out["summary"]["passed"], 1, "{out}");

    // Hosts outside localhost and the environment are refused.
    let (_, out) = mcp.call(
        "send_request",
        json!({ "method": "GET", "url": "https://example.org/" }),
    );
    assert!(
        out["error"]
            .as_str()
            .unwrap()
            .contains("not an allowed host"),
        "{out}"
    );

    let history = std::fs::read_to_string(project.0.join("data/history.jsonl")).unwrap();
    assert!(history.contains("\"source\":\"mcp:test-agent\""));
}

#[test]
fn mcp_read_only_refuses_writes() {
    let project = Project::new("readonly");
    let mut mcp = Mcp::start(&project.app(), &project.0.join("data"), &["--read-only"]);
    let (err, out) = mcp.call(
        "save_request",
        json!({ "path": "A/b", "method": "GET", "url": "http://localhost/" }),
    );
    assert!(err, "{out}");
    assert!(!project.app().join(".tuma").exists());
}
