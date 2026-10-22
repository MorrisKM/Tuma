//! End-to-end tests of the HTTP engine against a local server.
//!
//! Routes: `/echo` returns the request as JSON, `/redirect/{status}?to=` redirects, `/gzip` and `/br` send
//! compressed bodies, `/slow` waits two seconds.

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tuma_core::http::{ApiKeyLocation, Auth, Body, Error, KeyValue, Part, Request, Settings, send};

async fn handle(
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let response = hyper::Response::builder();

    let response = if let Some(status) = path.strip_prefix("/redirect/") {
        let to = query.strip_prefix("to=").unwrap_or("/echo");
        response
            .status(status.parse::<u16>().unwrap())
            .header("location", to)
            .body(Full::default())
    } else if path == "/gzip" {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"hello gzip").unwrap();
        response
            .header("content-encoding", "gzip")
            .body(Full::new(encoder.finish().unwrap().into()))
    } else if path == "/br" {
        let mut out = Vec::new();
        brotli::CompressorWriter::new(&mut out, 4096, 5, 22)
            .write_all(b"hello brotli")
            .unwrap();
        response
            .header("content-encoding", "br")
            .body(Full::new(out.into()))
    } else if path == "/slow" {
        tokio::time::sleep(Duration::from_secs(2)).await;
        response.body(Full::default())
    } else {
        let headers: serde_json::Map<String, Value> = parts
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    Value::String(v.to_str().unwrap().to_string()),
                )
            })
            .collect();
        let echo = json!({
            "method": parts.method.as_str(),
            "path": path,
            "query": query,
            "version": format!("{:?}", parts.version),
            "headers": headers,
            "body": String::from_utf8_lossy(&body),
        });
        response
            .header("content-type", "application/json")
            .body(Full::new(echo.to_string().into()))
    };
    Ok(response.unwrap())
}

/// Starts a plain HTTP server and returns its base URL.
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

/// Starts an HTTPS server with a self-signed certificate for `localhost`, offering HTTP/2 and HTTP/1.1.
async fn tls_server() -> SocketAddr {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert.cert.der().clone()], key)
    .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A failed handshake is expected in the verification test.
                if let Ok(stream) = acceptor.accept(stream).await {
                    let _ = auto::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service_fn(handle))
                        .await;
                }
            });
        }
    });
    addr
}

fn get(url: impl Into<String>) -> Request {
    Request {
        method: "GET".into(),
        url: url.into(),
        ..Default::default()
    }
}

fn echo(response: &tuma_core::http::Response) -> Value {
    serde_json::from_slice(&response.body).unwrap()
}

#[tokio::test]
async fn get_with_headers_and_timing() {
    let base = server().await;
    let mut request = get(format!("{base}/echo?a=1"));
    request.headers = vec![KeyValue::new("X-Trace", "abc")];

    let response = send(&request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.reason, "OK");
    assert_eq!(response.version, "HTTP/1.1");
    let echo = echo(&response);
    assert_eq!(echo["method"], "GET");
    assert_eq!(echo["query"], "a=1");
    assert_eq!(echo["headers"]["x-trace"], "abc");
    assert_eq!(echo["headers"]["host"], base.trim_start_matches("http://"));
    assert!(
        echo["headers"]["user-agent"]
            .as_str()
            .unwrap()
            .starts_with("Tuma/")
    );
    // 127.0.0.1 needs no DNS lookup, and plain HTTP has no TLS.
    assert!(response.timing.dns.is_none());
    assert!(response.timing.tls.is_none());
    assert!(response.timing.total >= response.timing.wait);
    assert_eq!(response.sent.method, "GET");
}

#[tokio::test]
async fn dns_lookup_is_timed() {
    let base = server().await.replace("127.0.0.1", "localhost");
    let response = send(&get(format!("{base}/echo"))).await.unwrap();
    assert!(response.timing.dns.is_some());
}

#[tokio::test]
async fn post_json() {
    let base = server().await;
    let request = Request {
        method: "post".into(),
        url: format!("{base}/echo"),
        body: Body::Raw {
            content: r#"{"name":"Ann"}"#.into(),
            content_type: "application/json".into(),
        },
        auth: Auth::Bearer {
            token: "t0k".into(),
        },
        ..Default::default()
    };

    let echo = echo(&send(&request).await.unwrap());

    assert_eq!(echo["method"], "POST");
    assert_eq!(echo["body"], r#"{"name":"Ann"}"#);
    assert_eq!(echo["headers"]["content-type"], "application/json");
    assert_eq!(echo["headers"]["content-length"], "14");
    assert_eq!(echo["headers"]["authorization"], "Bearer t0k");
}

#[tokio::test]
async fn url_encoded_form_and_api_key_header() {
    let base = server().await;
    let request = Request {
        method: "POST".into(),
        url: format!("{base}/echo"),
        body: Body::FormUrlEncoded {
            fields: vec![
                KeyValue::new("a", "1 2"),
                KeyValue::new("b", "&"),
                KeyValue {
                    enabled: false,
                    ..KeyValue::new("c", "3")
                },
            ],
        },
        auth: Auth::ApiKey {
            key: "X-Api-Key".into(),
            value: "k".into(),
            location: ApiKeyLocation::Header,
        },
        ..Default::default()
    };

    let echo = echo(&send(&request).await.unwrap());

    assert_eq!(echo["body"], "a=1+2&b=%26");
    assert_eq!(
        echo["headers"]["content-type"],
        "application/x-www-form-urlencoded"
    );
    assert_eq!(echo["headers"]["x-api-key"], "k");
}

#[tokio::test]
async fn multipart_with_a_file() {
    let base = server().await;
    let dir = std::env::temp_dir().join(format!("tuma-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("note.txt");
    std::fs::write(&file, "file contents").unwrap();
    let request = Request {
        method: "POST".into(),
        url: format!("{base}/echo"),
        body: Body::Multipart {
            parts: vec![
                Part {
                    name: "title".into(),
                    value: "Hi".into(),
                    ..Default::default()
                },
                Part {
                    name: "doc".into(),
                    value: file.to_string_lossy().into(),
                    file: true,
                    ..Default::default()
                },
            ],
        },
        ..Default::default()
    };

    let echo = echo(&send(&request).await.unwrap());
    std::fs::remove_dir_all(&dir).unwrap();

    let content_type = echo["headers"]["content-type"].as_str().unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let expected = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHi\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"doc\"; filename=\"note.txt\"\r\nContent-Type: text/plain\r\n\r\nfile contents\r\n\
         --{boundary}--\r\n"
    );
    assert_eq!(echo["body"], expected);
}

#[tokio::test]
async fn missing_file_is_an_error() {
    let request = Request {
        method: "POST".into(),
        url: "http://127.0.0.1:1/".into(),
        body: Body::File {
            path: "/no/such/file".into(),
            content_type: None,
        },
        ..Default::default()
    };
    assert!(matches!(send(&request).await, Err(Error::File { .. })));
}

#[tokio::test]
async fn follows_redirects_and_turns_post_into_get_on_303() {
    let base = server().await;
    let request = Request {
        method: "POST".into(),
        url: format!("{base}/redirect/303?to=/redirect/307"),
        body: Body::Raw {
            content: "x".into(),
            content_type: "text/plain".into(),
        },
        ..Default::default()
    };

    let response = send(&request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        response
            .redirects
            .iter()
            .map(|r| r.status)
            .collect::<Vec<_>>(),
        [303, 307]
    );
    assert!(response.url.ends_with("/echo"));
    let echo = echo(&response);
    assert_eq!(echo["method"], "GET");
    assert_eq!(echo["body"], "");
}

#[tokio::test]
async fn redirects_can_be_turned_off_or_limited() {
    let base = server().await;
    let mut request = get(format!("{base}/redirect/302"));
    request.settings.follow_redirects = false;
    assert_eq!(send(&request).await.unwrap().status, 302);

    let mut request = get(format!("{base}/redirect/302?to=/redirect/302"));
    request.settings.max_redirects = 1;
    assert!(matches!(
        send(&request).await,
        Err(Error::TooManyRedirects(1))
    ));
}

#[tokio::test]
async fn authorization_is_dropped_on_cross_origin_redirect() {
    let base = server().await;
    let other = base.replace("127.0.0.1", "localhost");
    let mut request = get(format!("{base}/redirect/302?to={other}/echo"));
    request.auth = Auth::Bearer {
        token: "secret".into(),
    };

    let echo = echo(&send(&request).await.unwrap());

    assert!(echo["headers"].get("authorization").is_none());
}

#[tokio::test]
async fn decodes_compressed_bodies() {
    let base = server().await;
    let response = send(&get(format!("{base}/gzip"))).await.unwrap();
    assert_eq!(&response.body[..], b"hello gzip");
    assert!(response.encoded_size > 0 && response.encoded_size != response.body.len());
    let response = send(&get(format!("{base}/br"))).await.unwrap();
    assert_eq!(&response.body[..], b"hello brotli");
}

#[tokio::test]
async fn times_out() {
    let base = server().await;
    let mut request = get(format!("{base}/slow"));
    request.settings = Settings {
        timeout_ms: Some(100),
        ..Default::default()
    };
    assert!(matches!(send(&request).await, Err(Error::Timeout(100))));
}

#[tokio::test]
async fn connection_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let error = send(&get(format!("http://{addr}/"))).await.unwrap_err();
    assert_eq!(error.kind(), "connect");
}

#[tokio::test]
async fn https_with_http2_when_verification_is_off() {
    let addr = tls_server().await;
    let mut request = get(format!("https://localhost:{}/echo", addr.port()));
    request.settings.verify_tls = false;

    let response = send(&request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.version, "HTTP/2");
    assert!(response.timing.tls.is_some());
    let echo = echo(&response);
    assert_eq!(echo["version"], "HTTP/2.0");
    assert!(echo["headers"].get("host").is_none());
}

#[tokio::test]
async fn self_signed_certificate_is_rejected_by_default() {
    let addr = tls_server().await;
    let error = send(&get(format!("https://localhost:{}/echo", addr.port())))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "tls");
}
