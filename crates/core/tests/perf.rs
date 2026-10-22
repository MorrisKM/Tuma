//! A performance test against a local mock server: counts, latency, security checks, thresholds, reports, stopping.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tuma_core::mock;
use tuma_core::perf::{self, Options};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::Workspace;
use tuma_core::workspace::files::{Example, KeyValue, RequestFile};

async fn setup(name: &str) -> (std::path::PathBuf, Workspace, mock::MockServer) {
    let root = std::env::temp_dir().join(format!("tuma-perf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let workspace = Workspace::create(&root, "Perf").unwrap();
    let api = workspace.create_folder(None, "API").unwrap();
    for (name, url, status) in [
        ("List", "{{baseUrl}}/items", 200),
        ("Missing", "{{baseUrl}}/missing", 404),
    ] {
        workspace
            .create_request(
                &api,
                name,
                &RequestFile {
                    url: url.into(),
                    examples: vec![Example {
                        name: "Example".into(),
                        status,
                        headers: vec![KeyValue {
                            name: "Content-Type".into(),
                            value: "application/json".into(),
                            enabled: true,
                        }],
                        body: "[]".into(),
                    }],
                    ..RequestFile::default()
                },
            )
            .unwrap();
    }
    let server = mock::start(
        mock::Options {
            root: root.clone(),
            scope: Some("API".into()),
            port: 0,
            delay_ms: 2,
        },
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let env = workspace.create_environment("Local").unwrap();
    workspace
        .write_environment(
            &env,
            &[tuma_core::workspace::VariableValue {
                name: "baseUrl".into(),
                value: server.url(),
                secret: false,
                enabled: true,
            }],
            &MemoryStore::default(),
        )
        .unwrap();
    (root, workspace, server)
}

#[tokio::test]
async fn measures_a_run_and_checks_the_responses() {
    let (root, workspace, _server) = setup("run").await;
    let secrets = MemoryStore::default();
    let (targets, notes) = perf::prepare(&workspace, Some("API"), Some("Local"), &secrets, None)
        .await
        .unwrap();
    assert_eq!(targets.len(), 2);
    assert!(notes.is_empty(), "{notes:?}");
    let progress: Arc<Mutex<u32>> = Arc::default();
    let seen = progress.clone();
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let report = perf::run(
        "API",
        targets,
        Options {
            users: 4,
            duration_s: 0,
            iterations: 5,
            max_error_rate: Some(10.0),
            ..Options::default()
        },
        Some("Local".into()),
        notes,
        Arc::new(tuma_core::http::Network::default()),
        Arc::new(move |_| *seen.lock().unwrap() += 1),
        stopped,
    )
    .await;

    // 4 users × 5 iterations × 2 requests.
    assert_eq!(report.requests, 40);
    assert_eq!(report.per_request[0].requests, 20);
    // The 404s are errors: half of everything, above the 10% threshold.
    assert_eq!(report.errors, 20);
    assert_eq!(report.per_request[1].statuses.get("404"), Some(&20));
    assert_eq!(report.passed, Some(false));
    assert!(report.failures[0].contains("error rate"));
    assert!(report.latency.p95 >= report.latency.p50 && report.latency.min > 0.0);
    assert!(report.findings.iter().any(|f| f.check == "No nosniff"));
    assert!(*progress.lock().unwrap() >= 1);

    let html = perf::report::html(&report);
    assert!(
        html.contains("<h1>API</h1>") && html.contains("Security checks") && html.contains("<svg")
    );
    let json: serde_json::Value = serde_json::from_str(&perf::report::json(&report)).unwrap();
    assert_eq!(json["requests"], 40);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn stops_when_asked() {
    let (root, workspace, _server) = setup("stop").await;
    let secrets = MemoryStore::default();
    let (targets, notes) = perf::prepare(
        &workspace,
        Some("API/List.yaml"),
        Some("Local"),
        &secrets,
        None,
    )
    .await
    .unwrap();
    assert_eq!(targets.len(), 1);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = stop.send(true);
    });
    let began = Instant::now();
    let report = perf::run(
        "List",
        targets,
        Options {
            users: 3,
            duration_s: 60,
            ..Options::default()
        },
        None,
        notes,
        Arc::new(tuma_core::http::Network::default()),
        Arc::new(|_| {}),
        stopped,
    )
    .await;
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "a 60 s run stops early"
    );
    assert!(report.stopped && report.finished);
    assert!(report.requests > 0);
    let _ = std::fs::remove_dir_all(root);
}
