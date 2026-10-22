//! Encrypted fields end to end: the server receives ciphertext, and the response's fields come back decrypted.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuma_core::crypto::CryptoSettings;
use tuma_core::runner::{self, Job};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::files::{Body, RequestFile};

/// Answers every request with its own body, and keeps the last body it received.
async fn echo_server() -> (String, Arc<Mutex<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let keep = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 65536];
            let mut data = Vec::new();
            // Read the head, then as much body as Content-Length says.
            loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&data).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&data).to_string();
            let body = text
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            *keep.lock().unwrap() = body.clone();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        }
    });
    (format!("http://{addr}"), seen)
}

#[tokio::test]
async fn fields_go_out_encrypted_and_come_back_decrypted() {
    let (url, seen) = echo_server().await;
    let request = RequestFile {
        method: "POST".into(),
        url: format!("{url}/pay"),
        body: Body::Json {
            content: "{\n  // a test card\n  \"card\": \"4111111111111111\",\n  \"amount\": 10\n}"
                .into(),
        },
        crypto: Some(CryptoSettings {
            key: "{{encryptionKey}}".into(),
            encrypt: vec!["$.card".into()],
            decrypt: vec!["$.card".into()],
            ..CryptoSettings::default()
        }),
        ..RequestFile::default()
    };
    let secrets = MemoryStore::default();
    let overrides = [(
        "encryptionKey".to_string(),
        "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=".to_string(),
    )];
    let job = |request: &RequestFile, overrides: &[(String, String)]| {
        let request = request.clone();
        let overrides = overrides.to_vec();
        let secrets = &secrets;
        async move {
            runner::execute(Job {
                workspace: None,
                request: &request,
                path: None,
                context: None,
                environment: None,
                overrides: &overrides,
                extra_checks: None,
                secrets,
                history: None,
                source: "",
                allow_host: None,
                scripting: Default::default(),
                network: None,
                observer: None,
            })
            .await
        }
    };

    let execution = job(&request, &overrides).await;
    let response = execution.response.as_ref().expect("sent");
    // The server saw ciphertext, never the card number.
    let wire: serde_json::Value = serde_json::from_str(&seen.lock().unwrap()).unwrap();
    assert_ne!(wire["card"], "4111111111111111");
    assert_eq!(wire["amount"], 10);
    // The response's field is decrypted; the body as it came is kept.
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["card"], "4111111111111111");
    let decryption = execution.decryption.unwrap().unwrap();
    assert_eq!(decryption.fields, [("$.card".to_string(), 1)]);
    assert!(!decryption.original.contains("4111111111111111"));

    // A key of the wrong size stops the request before it is sent.
    let short = [("encryptionKey".to_string(), "c2hvcnQ=".to_string())];
    let failed = job(&request, &short).await;
    let failure = failed.response.unwrap_err();
    assert_eq!(failure.kind, "crypto");
    assert!(failure.message.contains("needs 32"), "{}", failure.message);
}
