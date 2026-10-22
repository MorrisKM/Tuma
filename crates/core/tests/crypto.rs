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

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/crypto")
            .join(name),
    )
    .unwrap()
}

fn rsa_settings(hash: tuma_core::crypto::Algorithm) -> CryptoSettings {
    CryptoSettings {
        algorithm: hash,
        ..CryptoSettings::default()
    }
}

#[test]
fn rsa_oaep_matches_openssl_and_takes_single_line_keys() {
    use tuma_core::crypto::{Algorithm, Cipher};
    let public = fixture("test-public.pem");
    let private = fixture("test-private.pem");
    for algorithm in [Algorithm::RsaOaepSha1, Algorithm::RsaOaepSha256] {
        let cipher =
            Cipher::with_private_key(&rsa_settings(algorithm), &public, "", &private).unwrap();
        let sealed = cipher.encrypt(b"1234").unwrap();
        assert_ne!(
            sealed,
            cipher.encrypt(b"1234").unwrap(),
            "OAEP is randomized"
        );
        assert_eq!(cipher.decrypt(&sealed).unwrap(), b"1234");
    }
    // A PIN that OpenSSL encrypted with OAEP SHA-1, as node-forge and encrypt_pin_rsa.py do.
    let cipher =
        Cipher::with_private_key(&rsa_settings(Algorithm::RsaOaepSha1), "", "", &private).unwrap();
    assert_eq!(
        cipher.decrypt(&fixture("pin-oaep-sha1.txt")).unwrap(),
        b"1234"
    );
    // Keys as .env files keep them: on one line, or with literal \n, or in quotes; and PKCS#1 private keys.
    let one_line = public.replace('\n', "");
    let escaped = format!("\"{}\"", public.replace('\n', "\\n"));
    for key in [one_line, escaped] {
        let c = Cipher::with_private_key(
            &rsa_settings(Algorithm::RsaOaepSha1),
            &key,
            "",
            &fixture("test-private-pkcs1.pem"),
        )
        .unwrap();
        assert_eq!(c.decrypt(&c.encrypt(b"9876").unwrap()).unwrap(), b"9876");
    }
    // Mistakes say what is wrong.
    let c =
        Cipher::with_private_key(&rsa_settings(Algorithm::RsaOaepSha1), &public, "", "").unwrap();
    assert!(c.encrypt(&[b'x'; 300]).unwrap_err().contains("too long"));
    assert!(
        Cipher::with_private_key(&rsa_settings(Algorithm::RsaOaepSha1), &private, "", "")
            .unwrap_err()
            .contains("private key")
    );
    let needs_private = CryptoSettings {
        decrypt: vec!["$".into()],
        ..rsa_settings(Algorithm::RsaOaepSha1)
    };
    assert!(Cipher::with_private_key(&needs_private, &public, "", "").is_err());
}

#[test]
fn hybrid_responses_decrypt_like_decrypt_py() {
    use tuma_core::crypto::{Cipher, HybridFormat, decrypt_body};
    let settings = CryptoSettings {
        hybrid: Some(HybridFormat::default()),
        decrypt: vec!["$".into()],
        ..CryptoSettings::default()
    };
    let cipher = Cipher::with_private_key(&settings, "", "", &fixture("test-private.pem")).unwrap();
    let response = fixture("hybrid-response.txt");
    // The whole body, bare or as a JSON string.
    for body in [response.clone(), format!("\"{}\"", response.trim())] {
        let (plain, fields) = decrypt_body(&body, &["$".into()], &cipher).unwrap();
        let json: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(json["name"], "Ada Lovelace");
        assert_eq!(json["services"][1], "licence");
        assert_eq!(fields, [("$".to_string(), 1)]);
    }
    // A field of a JSON response.
    let wrapped = format!("{{\"status\": \"ok\", \"data\": \"{}\"}}", response.trim());
    let (plain, _) = decrypt_body(&wrapped, &["$.data".into()], &cipher).unwrap();
    let json: serde_json::Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(json["data"]["idNumber"], "12345678");
    assert_eq!(json["status"], "ok");

    // Without the private key, or with the wrong separator, it says so.
    assert!(
        Cipher::with_private_key(&settings, "", "", "")
            .unwrap_err()
            .contains("private key")
    );
    let wrong = CryptoSettings {
        hybrid: Some(HybridFormat {
            separator: "|".into(),
            ..HybridFormat::default()
        }),
        ..settings.clone()
    };
    let c = Cipher::with_private_key(&wrong, "", "", &fixture("test-private.pem")).unwrap();
    assert!(c.decrypt(&response).unwrap_err().contains("three parts"));
}

#[tokio::test]
async fn an_rsa_encrypted_pin_goes_out_and_comes_back() {
    let (url, seen) = echo_server().await;
    let request = RequestFile {
        method: "POST".into(),
        url: format!("{url}/login"),
        body: Body::Json {
            content: "{\"user\": \"ada\", \"pin\": \"1234\"}".into(),
        },
        crypto: Some(CryptoSettings {
            algorithm: tuma_core::crypto::Algorithm::RsaOaepSha1,
            key: "{{rsaPublicKey}}".into(),
            private_key: "{{rsaPrivateKey}}".into(),
            encrypt: vec!["$.pin".into()],
            decrypt: vec!["$.pin".into()],
            ..CryptoSettings::default()
        }),
        ..RequestFile::default()
    };
    let overrides = [
        (
            "rsaPublicKey".to_string(),
            fixture("test-public.pem").replace('\n', ""),
        ),
        ("rsaPrivateKey".to_string(), fixture("test-private.pem")),
    ];
    let secrets = MemoryStore::default();
    let execution = runner::execute(Job {
        workspace: None,
        request: &request,
        path: None,
        context: None,
        environment: None,
        overrides: &overrides,
        extra_checks: None,
        secrets: &secrets,
        history: None,
        source: "",
        allow_host: None,
        scripting: Default::default(),
        network: None,
        observer: None,
    })
    .await;
    let response = execution.response.as_ref().expect("sent");
    let wire: serde_json::Value = serde_json::from_str(&seen.lock().unwrap()).unwrap();
    assert_eq!(wire["user"], "ada");
    assert!(
        wire["pin"].as_str().unwrap().len() > 300,
        "a 2048-bit RSA ciphertext in base64"
    );
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["pin"], "1234");
}
