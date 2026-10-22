//! A mock server started on a real port, answering from a workspace's saved examples.

use std::fs;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuma_core::mock::{self, MockEvent, Options};
use tuma_core::workspace::Workspace;
use tuma_core::workspace::files::{Example, KeyValue, RequestFile};

async fn get(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    out
}

#[tokio::test]
async fn answers_with_examples_and_follows_edits() {
    let root = std::env::temp_dir().join(format!("tuma-mock-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let workspace = Workspace::create(&root, "Mock").unwrap();
    let api = workspace.create_folder(None, "Shop").unwrap();
    let mut request = RequestFile {
        url: "{{baseUrl}}/products/:id".into(),
        examples: vec![Example {
            name: "Found".into(),
            status: 200,
            headers: vec![KeyValue {
                name: "Content-Type".into(),
                value: "application/json".into(),
                enabled: true,
            }],
            body: r#"{"id": 42, "name": "Lamp"}"#.into(),
        }],
        ..RequestFile::default()
    };
    let path = workspace
        .create_request(&api, "Get product", &request)
        .unwrap();

    let events: Arc<Mutex<Vec<MockEvent>>> = Arc::default();
    let sink = events.clone();
    let server = mock::start(
        Options {
            root: root.clone(),
            scope: Some("Shop".into()),
            port: 0,
            delay_ms: 0,
        },
        Arc::new(move |e| sink.lock().unwrap().push(e)),
    )
    .await
    .unwrap();

    let reply = get(
        server.addr,
        "GET /products/42 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(
        reply
            .to_lowercase()
            .contains("access-control-allow-origin: *")
    );
    assert!(reply.ends_with(r#"{"id": 42, "name": "Lamp"}"#), "{reply}");

    // An edited example is used right away.
    request.examples[0].body = r#"{"id": 42, "name": "Desk lamp"}"#.into();
    workspace.write_request(&path, &request).unwrap();
    let reply = get(
        server.addr,
        "GET /products/1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(reply.contains("Desk lamp"), "{reply}");

    let reply = get(
        server.addr,
        "GET /nope HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");

    let logged = events.lock().unwrap().clone();
    assert_eq!(logged.len(), 3);
    assert_eq!(logged[0].request.as_deref(), Some("Get product"));
    assert_eq!(logged[2].status, 404);

    let addr = server.addr;
    server.stop();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "stopped"
    );
    let _ = fs::remove_dir_all(&root);
}
