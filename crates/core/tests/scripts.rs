//! Scripts end to end: a Postman collection with typical scripts, imported and run unchanged against a local server.
//!
//! Routes: `POST /login` returns `{"token": "tok-<user>"}`, `GET /profile` needs that token and returns
//! `{"user": ...}`, `GET /ping` returns `pong`, `/echo` returns what it was sent, anything else is a 500.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tuma_core::import;
use tuma_core::runs::{self, RunOptions};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::{Scope, VariableValue, Workspace};

async fn handle(
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let reply = |status: u16, body: String| {
        Ok(hyper::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap())
    };
    match (parts.method.as_str(), parts.uri.path()) {
        ("POST", "/login") => {
            let json: Value = serde_json::from_slice(&body).unwrap_or_default();
            // The collection's pre-request script adds this header to every request.
            if parts.headers.get("x-request-id").is_none() {
                return reply(400, json!({ "error": "no X-Request-Id" }).to_string());
            }
            reply(
                200,
                json!({ "token": format!("tok-{}", json["user"].as_str().unwrap_or("")) })
                    .to_string(),
            )
        }
        ("GET", "/profile") => {
            let auth = parts
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            match auth.strip_prefix("Bearer tok-") {
                Some(user) => reply(200, json!({ "user": user }).to_string()),
                None => reply(401, json!({ "error": "no token" }).to_string()),
            }
        }
        ("GET", "/ping") => Ok(hyper::Response::new(Full::new(Bytes::from("pong")))),
        (method, "/echo") => {
            let header = |name: &str| {
                parts
                    .headers
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            };
            reply(
                200,
                json!({
                    "method": method,
                    "query": parts.uri.query().unwrap_or(""),
                    "authorization": header("authorization"),
                    "contentType": header("content-type"),
                    "body": String::from_utf8_lossy(&body),
                })
                .to_string(),
            )
        }
        _ => reply(500, "{}".into()),
    }
}

async fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let _ = auto::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service_fn(handle))
                    .await;
            });
        }
    });
    format!("http://{addr}")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tuma-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scripts")
        .join(name)
}

fn options<'a>(
    ws: &'a Workspace,
    secrets: &'a MemoryStore,
    data: Vec<std::collections::BTreeMap<String, String>>,
) -> RunOptions<'a> {
    RunOptions {
        workspace: ws,
        path: "Accounts",
        environment: Some("Local"),
        overrides: &[],
        secrets,
        history: None,
        source: "test",
        allow_host: None,
        network: None,
        iterations: 1,
        data,
        delay: Duration::ZERO,
        only_tests: false,
        fail_fast: false,
        scripts: true,
        cancel: None,
        detail: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn imported_postman_scripts_run_unchanged() {
    let base = server().await;
    let dir = TempDir::new("scripts");
    let secrets = MemoryStore::default();
    let mut ws = Workspace::create(dir.0.join("ws"), "Test").unwrap();
    ws.set_local_dir(dir.0.join("local"));
    let (_, warnings) = import::postman(
        &mut ws,
        &fs::read_to_string(fixture("login.postman_collection.json")).unwrap(),
        &secrets,
    )
    .unwrap();
    assert!(warnings.0.is_empty(), "{warnings:?}");
    ws.write_environment(
        "Local",
        &[VariableValue {
            name: "baseUrl".into(),
            value: base.clone(),
            secret: false,
            enabled: true,
        }],
        &secrets,
    )
    .unwrap();

    let data = runs::read_data_file(&fixture("users.csv")).unwrap();
    let mut seen = Vec::new();
    let report = runs::run(options(&ws, &secrets, data), &mut |step| {
        seen.push(step.name.clone())
    })
    .await
    .unwrap();

    // setNextRequest skips "Never runs", in both iterations (one per row of the data file).
    assert_eq!(seen, ["Login", "Get profile", "Login", "Get profile"]);
    assert_eq!(report.iterations, 2);
    assert_eq!(report.stopped, None);
    for step in &report.steps {
        let o = &step.outcome;
        assert!(o.passed, "{}: {o:#?}", step.name);
        assert!(o.script_errors.is_empty(), "{:?}", o.script_errors);
    }
    let names = |i: usize| {
        report.steps[i]
            .outcome
            .tests
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(0),
        ["responds quickly", "Status code is 200", "has a token"]
    );
    assert_eq!(names(3), ["is the logged-in user", "ping works"]);
    assert_eq!(
        report.steps[3].outcome.console[0].text,
        "profile bob took true"
    );

    // The token is a current value: on this machine, not in the environment file.
    let values = ws.current_values();
    assert_eq!(
        values.environments["Local"]["token"],
        Some("tok-bob".into())
    );
    assert!(
        !fs::read_to_string(dir.0.join("ws/environments/Local.yaml"))
            .unwrap()
            .contains("tok-")
    );
    let resolved = ws
        .variables_in_scope(None, Some("Local"), &secrets)
        .unwrap();
    assert!(
        resolved
            .iter()
            .any(|v| v.name == "token" && v.value == "tok-bob")
    );
    ws.reset_current_values(Some(Scope::Environment("Local")))
        .unwrap();
    assert!(ws.current_values().is_empty());

    // JUnit for CI.
    let xml = runs::junit(&report, "Accounts");
    assert!(
        xml.contains(r#"<testsuites name="Accounts" tests="10" failures="0" errors="0""#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<testcase name="ping works" classname="Accounts/Get profile"/>"#),
        "{xml}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failing_scripts_fail_the_run() {
    let base = server().await;
    let dir = TempDir::new("scripts-fail");
    let secrets = MemoryStore::default();
    let mut ws = Workspace::create(dir.0.join("ws"), "Test").unwrap();
    ws.set_local_dir(dir.0.join("local"));
    import::postman(
        &mut ws,
        &fs::read_to_string(fixture("login.postman_collection.json")).unwrap(),
        &secrets,
    )
    .unwrap();
    ws.write_environment(
        "Local",
        &[VariableValue {
            name: "baseUrl".into(),
            value: base,
            secret: false,
            enabled: true,
        }],
        &secrets,
    )
    .unwrap();
    // No data file: {{user}} has no value, so the profile is for an empty user and the check fails.
    let report = runs::run(options(&ws, &secrets, vec![]), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(report.summary.requests, 2);
    assert_eq!(report.summary.failed, 1);
    let profile = &report.steps[1].outcome;
    let failed: Vec<_> = profile.tests.iter().filter(|t| !t.passed).collect();
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0]
            .error
            .as_deref()
            .unwrap()
            .starts_with("AssertionError"),
        "{failed:?}"
    );
    let xml = runs::junit(&report, "Accounts");
    assert!(xml.contains("failures=\"1\""), "{xml}");
}

/// A standalone script, as on the Scripts page: tuma.post with a query, headers and a JSON body, reading the
/// environment, and a file and a result coming back.
#[tokio::test(flavor = "multi_thread")]
async fn standalone_scripts_send_requests_with_tuma() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tuma_core::scripts::{self, Kind, Script, ScriptFile};

    let base = server().await;
    let dir = TempDir::new("standalone-script");
    let ws = Workspace::create(&dir.0, "Scripts").unwrap();
    let secrets = MemoryStore::default();
    ws.create_environment("UAT").unwrap();
    ws.write_environment(
        "UAT",
        &[
            VariableValue {
                name: "baseUrl".into(),
                value: base,
                enabled: true,
                secret: false,
            },
            VariableValue {
                name: "token".into(),
                value: "key:secret".into(),
                enabled: true,
                secret: true,
            },
        ],
        &secrets,
    )
    .unwrap();
    let source = r#"
        const token = pm.environment.get("token");
        const r = await tuma.post(`${pm.environment.get("baseUrl")}/echo`, {
          query: { filters: [["id", "=", 5]], limit: 1, "tag[]": ["a", "b"] },
          headers: { Authorization: `token ${token}` },
          json: { name: "Ann" },
        });
        const echo = r.json();
        tuma.saveFile("echo.json", echo);
        return { code: r.code, ...echo };
    "#;
    scripts::write(
        &ws,
        &Script {
            name: "Echo".into(),
            kind: Kind::Javascript,
            settings: ScriptFile::default(),
            source: source.into(),
        },
    )
    .unwrap();
    let outcome = scripts::run(
        scripts::RunOptions {
            workspace: &ws,
            name: "Echo",
            environment: Some("UAT"),
            inputs: &BTreeMap::new(),
            extra_args: "",
            secrets: &secrets,
            network: None,
            stop: Arc::default(),
        },
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    let result = outcome.result.unwrap();
    assert_eq!(result["code"], 200);
    assert_eq!(result["method"], "POST");
    assert_eq!(
        result["query"],
        "filters=%5B%5B%22id%22%2C%22%3D%22%2C5%5D%5D&limit=1&tag%5B%5D=a&tag%5B%5D=b"
    );
    assert_eq!(result["authorization"], "token key:secret");
    assert_eq!(result["contentType"], "application/json");
    assert_eq!(result["body"], r#"{"name":"Ann"}"#);
    assert_eq!(outcome.files[0].name, "echo.json");
}
