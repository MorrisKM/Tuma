//! The network settings end to end: cookies, an HTTP proxy (plain and CONNECT, with credentials), client certificates
//! (PEM and PKCS#12) and a private CA, each against a real local server.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tuma_core::http::network::{self, ClientCertificate, Proxy};
use tuma_core::http::{Error, Network, Request, send_with};
use url::Url;

/// `/set` sets a cookie, `/login` sets one and redirects to `/cookies`, `/cookies` returns the Cookie header.
async fn handle(
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let cookies = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let response = match request.uri().path() {
        "/set" => hyper::Response::builder()
            .header("set-cookie", "session=abc; Path=/; HttpOnly")
            .header("set-cookie", "theme=dark; Path=/; Max-Age=3600")
            .body(Full::new(Bytes::from("set"))),
        "/login" => hyper::Response::builder()
            .status(302)
            .header("location", "/cookies")
            .header("set-cookie", "token=t1; Path=/")
            .body(Full::new(Bytes::new())),
        _ => hyper::Response::builder().body(Full::new(Bytes::from(cookies))),
    };
    Ok(response.unwrap())
}

async fn serve(listener: TcpListener, tls: Option<tokio_rustls::TlsAcceptor>) {
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let tls = tls.clone();
        tokio::spawn(async move {
            match tls {
                Some(tls) => {
                    if let Ok(stream) = tls.accept(stream).await {
                        let _ = auto::Builder::new(TokioExecutor::new())
                            .serve_connection(TokioIo::new(stream), service_fn(handle))
                            .await;
                    }
                }
                None => {
                    let _ = auto::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service_fn(handle))
                        .await;
                }
            }
        });
    }
}

async fn http_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, None));
    format!("http://{addr}")
}

fn get(url: &str) -> Request {
    Request {
        method: "GET".into(),
        url: url.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn cookies_are_kept_and_sent_back() {
    let base = http_server().await;
    let jar = network::CookieJar::default();
    let net = Network {
        cookies: Some(jar.clone()),
        ..Default::default()
    };
    send_with(&get(&format!("{base}/set")), &net).await.unwrap();
    let response = send_with(&get(&format!("{base}/cookies")), &net)
        .await
        .unwrap();
    let sent = String::from_utf8_lossy(&response.body).to_string();
    assert!(
        sent.contains("session=abc") && sent.contains("theme=dark"),
        "{sent}"
    );

    // A cookie set on a redirect is sent on the next hop.
    let response = send_with(&get(&format!("{base}/login")), &net)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&response.body).contains("token=t1"));

    // The request's own Cookie header comes first.
    let mut request = get(&format!("{base}/cookies"));
    request
        .headers
        .push(tuma_core::http::KeyValue::new("Cookie", "mine=1"));
    let response = send_with(&request, &net).await.unwrap();
    assert!(String::from_utf8_lossy(&response.body).starts_with("mine=1; "));

    let listed = tuma_core::settings::list_cookies(&jar);
    assert_eq!(listed["127.0.0.1"].len(), 3, "{listed:?}");
    tuma_core::settings::remove_cookies(&jar, Some("127.0.0.1"), None, Some("theme"));
    assert_eq!(
        tuma_core::settings::list_cookies(&jar)["127.0.0.1"].len(),
        2
    );

    // No jar, no cookies.
    let response = send_with(&get(&format!("{base}/cookies")), &Network::default())
        .await
        .unwrap();
    assert!(response.body.is_empty());
}

/// A forwarding proxy that wants `u:p`. It records each request line it gets.
async fn proxy_server(seen: Arc<Mutex<Vec<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if client.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let line = text.lines().next().unwrap_or_default().to_string();
                seen.lock().unwrap().push(line.clone());
                if !text
                    .to_ascii_lowercase()
                    .contains("proxy-authorization: basic dtpw")
                {
                    let _ = client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n").await;
                    return;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default();
                let target = parts.next().unwrap_or_default().to_string();
                let authority = if method == "CONNECT" {
                    target.clone()
                } else {
                    Url::parse(&target)
                        .map(|u| {
                            format!(
                                "{}:{}",
                                u.host_str().unwrap_or_default(),
                                u.port_or_known_default().unwrap_or(80)
                            )
                        })
                        .unwrap_or_default()
                };
                let Ok(mut upstream) = tokio::net::TcpStream::connect(&authority).await else {
                    return;
                };
                if method == "CONNECT" {
                    let _ = client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await;
                } else {
                    let _ = upstream.write_all(&head).await;
                }
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    format!("http://{addr}")
}

struct Pki {
    ca: rcgen::Certificate,
    ca_params: CertificateParams,
    ca_key: KeyPair,
}

impl Pki {
    fn new() -> Self {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        Self {
            ca,
            ca_params,
            ca_key,
        }
    }

    /// A certificate signed by the CA, and its key.
    fn issue(&self, names: &[&str]) -> (rcgen::Certificate, KeyPair) {
        let key = KeyPair::generate().unwrap();
        let params =
            CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .unwrap();
        let issuer = Issuer::from_params(&self.ca_params, &self.ca_key);
        (params.signed_by(&key, &issuer).unwrap(), key)
    }
}

/// An HTTPS server for localhost with a certificate from `pki`, which requires a client certificate from it too when
/// `mutual` is set.
async fn tls_server(pki: &Pki, mutual: bool) -> String {
    let (cert, key) = pki.issue(&["localhost"]);
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap();
    let builder = if mutual {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(pki.ca.der().clone()).unwrap();
        let verifier =
            rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .unwrap();
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    let config = builder
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve(
        listener,
        Some(tokio_rustls::TlsAcceptor::from(Arc::new(config))),
    ));
    format!("https://localhost:{port}")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tuma-net-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ca_network(dir: &TempDir, pki: &Pki) -> Network {
    let ca = dir.write("ca.pem", pki.ca.pem());
    Network {
        ca_certificates: network::read_ca_certificates(&ca).unwrap(),
        ..Default::default()
    }
}

#[tokio::test]
async fn proxies_plain_http_and_tunnels_https_with_credentials() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let proxy = proxy_server(seen.clone()).await;
    let plain = http_server().await;
    let pki = Pki::new();
    let secure = tls_server(&pki, false).await;
    let dir = TempDir::new("proxy");

    let with_auth = Url::parse(&proxy.replace("http://", "http://u:p@")).unwrap();
    let mut net = ca_network(&dir, &pki);
    net.proxy = Some(Proxy {
        http: Some(with_auth.clone()),
        https: Some(with_auth),
        bypass: vec![],
    });

    let response = send_with(&get(&format!("{plain}/cookies")), &net)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let response = send_with(&get(&format!("{secure}/cookies")), &net)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let lines = seen.lock().unwrap().clone();
    assert!(
        lines[0].starts_with(&format!("GET {plain}/cookies HTTP/1.1")),
        "{lines:?}"
    );
    assert!(lines[1].starts_with("CONNECT localhost:"), "{lines:?}");

    // Without credentials the tunnel is refused, with a message that says why.
    net.proxy = Some(Proxy {
        http: Some(Url::parse(&proxy).unwrap()),
        https: Some(Url::parse(&proxy).unwrap()),
        bypass: vec![],
    });
    let error = send_with(&get(&format!("{secure}/x")), &net)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, Error::Proxy(m) if m.contains("wants credentials")),
        "{error}"
    );

    // Bypassed hosts go direct.
    net.proxy.as_mut().unwrap().bypass = vec!["localhost".into(), "127.0.0.1".into()];
    let before = seen.lock().unwrap().len();
    send_with(&get(&format!("{secure}/x")), &net).await.unwrap();
    send_with(&get(&format!("{plain}/x")), &net).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), before);
}

#[tokio::test]
async fn private_ca_and_client_certificates() {
    let pki = Pki::new();
    let dir = TempDir::new("mtls");
    let public = tls_server(&pki, false).await;
    let mutual = tls_server(&pki, true).await;

    // The CA isn't trusted by default.
    let error = send_with(&get(&format!("{public}/x")), &Network::default())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Tls { .. }), "{error}");
    let net = ca_network(&dir, &pki);
    assert_eq!(
        send_with(&get(&format!("{public}/x")), &net)
            .await
            .unwrap()
            .status,
        200
    );

    // The mutual-TLS server refuses a client with no certificate.
    assert!(send_with(&get(&format!("{mutual}/x")), &net).await.is_err());

    // PEM files.
    let (client, client_key) = pki.issue(&["tuma client"]);
    let cert = dir.write("client.crt", client.pem());
    let key = dir.write("client.key", client_key.serialize_pem());
    let mut net = ca_network(&dir, &pki);
    net.client_certificates = vec![ClientCertificate::from_pem("localhost", &cert, &key).unwrap()];
    assert_eq!(
        send_with(&get(&format!("{mutual}/x")), &net)
            .await
            .unwrap()
            .status,
        200
    );

    // A certificate for another host isn't offered.
    net.client_certificates =
        vec![ClientCertificate::from_pem("api.example.com", &cert, &key).unwrap()];
    assert!(send_with(&get(&format!("{mutual}/x")), &net).await.is_err());

    // PKCS#12, with its passphrase.
    let pfx = dir.write("client.pfx", pfx(&client, &client_key, "s3cret"));
    assert!(ClientCertificate::from_pfx("localhost", &pfx, "wrong").is_err());
    net.client_certificates =
        vec![ClientCertificate::from_pfx("localhost", &pfx, "s3cret").unwrap()];
    assert_eq!(
        send_with(&get(&format!("{mutual}/x")), &net)
            .await
            .unwrap()
            .status,
        200
    );
    assert!(ClientCertificate::from_pem("x", Path::new("/no/such.crt"), &key).is_err());
}

fn pfx(cert: &rcgen::Certificate, key: &KeyPair, passphrase: &str) -> Vec<u8> {
    use p12_keystore::{Certificate, KeyStore, KeyStoreEntry, PrivateKey, PrivateKeyChain};
    let chain = PrivateKeyChain::new(
        vec![1u8; 20],
        PrivateKey::from_der(&key.serialize_der()).unwrap(),
        [Certificate::from_der(cert.der()).unwrap()],
    );
    let mut store = KeyStore::new();
    store.add_entry("client", KeyStoreEntry::PrivateKeyChain(chain));
    store.writer(passphrase).write().unwrap()
}
