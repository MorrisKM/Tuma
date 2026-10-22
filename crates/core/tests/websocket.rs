//! WebSocket connections against a local server: auth and cookies on the handshake, messages both ways, closing from
//! either side, TLS, and a refused handshake.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{
    Request as HsRequest, Response as HsResponse,
};
use tuma_core::http::websocket::{self, Outgoing, WsEvent};
use tuma_core::http::{Auth, Network, Request, Settings};

/// Echoes text in upper case, says who it is talking to, and closes with 4000 on "bye". Refuses clients without a
/// bearer token.
// The handshake callback's error type is tungstenite's.
#[allow(clippy::result_large_err)]
async fn ws_server(tls: Option<tokio_rustls::TlsAcceptor>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let tls = tls.clone();
            tokio::spawn(async move {
                let seen = Arc::new(Mutex::new(String::new()));
                let record = seen.clone();
                let check = move |req: &HsRequest, response: HsResponse| {
                    let auth = req
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let cookie = req
                        .headers()
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    *record.lock().unwrap() = format!("{auth}|{cookie}");
                    if auth != "Bearer t0k" {
                        let mut refused = tokio_tungstenite::tungstenite::http::Response::new(
                            Some("no token".to_string()),
                        );
                        *refused.status_mut() =
                            tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED;
                        return Err(refused);
                    }
                    Ok(response)
                };
                macro_rules! serve {
                    ($stream:expr) => {{
                        let Ok(mut ws) = tokio_tungstenite::accept_hdr_async($stream, check).await
                        else {
                            return;
                        };
                        let hello = seen.lock().unwrap().clone();
                        let _ = ws.send(Message::text(format!("hello {hello}"))).await;
                        while let Some(Ok(message)) = ws.next().await {
                            match message {
                                Message::Text(t) if t.as_str() == "bye" => {
                                    let frame =
                                        tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                            code: 4000u16.into(),
                                            reason: "see you".into(),
                                        };
                                    let _ = ws.close(Some(frame)).await;
                                }
                                Message::Text(t) => {
                                    let _ = ws.send(Message::text(t.to_uppercase())).await;
                                }
                                Message::Binary(b) => {
                                    let _ = ws.send(Message::binary(b)).await;
                                }
                                _ => {}
                            }
                        }
                    }};
                }
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(stream).await {
                            serve!(stream)
                        }
                    }
                    None => serve!(stream),
                }
            });
        }
    });
    port
}

struct Log(Arc<Mutex<Vec<WsEvent>>>);

impl Log {
    fn new() -> (Self, websocket::EventSink) {
        let events: Arc<Mutex<Vec<WsEvent>>> = Arc::default();
        let sink = events.clone();
        (
            Self(events),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        )
    }

    async fn until_closed(&self) -> Vec<WsEvent> {
        for _ in 0..100 {
            if self
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, WsEvent::Closed { .. }))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.0.lock().unwrap().clone()
    }

    async fn wait_for(&self, count: usize) -> Vec<WsEvent> {
        for _ in 0..100 {
            if self.0.lock().unwrap().len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.0.lock().unwrap().clone()
    }
}

fn text(e: &WsEvent) -> Option<(bool, String)> {
    match e {
        WsEvent::Message {
            incoming,
            text: Some(t),
            ..
        } => Some((*incoming, t.clone())),
        _ => None,
    }
}

fn request(url: String) -> Request {
    Request {
        method: "GET".into(),
        url,
        auth: Auth::Bearer {
            token: "t0k".into(),
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn messages_both_ways_and_server_close() {
    let port = ws_server(None).await;
    let jar = tuma_core::http::network::CookieJar::default();
    jar.lock()
        .unwrap()
        .parse(
            "session=abc; Path=/",
            &url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        )
        .unwrap();
    let network = Network {
        cookies: Some(jar),
        ..Default::default()
    };
    let (log, sink) = Log::new();
    let socket = websocket::connect(
        &request(format!("ws://127.0.0.1:{port}/chat")),
        &network,
        sink,
    )
    .await
    .unwrap();
    // The server speaks first.
    log.wait_for(2).await;
    socket.send(Outgoing::Text("hi there".into())).unwrap();
    socket.send(Outgoing::Binary(vec![1, 2, 3])).unwrap();
    let events = log.wait_for(6).await;
    assert!(
        matches!(&events[0], WsEvent::Open { status: 101, .. }),
        "{events:?}"
    );
    let texts: Vec<(bool, String)> = events.iter().filter_map(text).collect();
    assert_eq!(
        texts,
        [
            (true, "hello Bearer t0k|session=abc".into()),
            (false, "hi there".into()),
            (true, "HI THERE".into())
        ]
    );
    assert!(events.iter().any(
        |e| matches!(e, WsEvent::Message { incoming: true, binary: Some(b), .. } if b == "AQID")
    ));

    socket.send(Outgoing::Text("bye".into())).unwrap();
    let events = log.until_closed().await;
    assert_eq!(
        events.last(),
        Some(&WsEvent::Closed {
            code: Some(4000),
            reason: "see you".into(),
            by_server: true
        }),
        "{events:?}"
    );
    // The connection is gone, and says so.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(socket.send(Outgoing::Text("late".into())).is_err());
}

#[tokio::test]
async fn client_close_tls_and_refusal() {
    // Closing from Tuma's side.
    let port = ws_server(None).await;
    let (log, sink) = Log::new();
    let socket = websocket::connect(
        &request(format!("ws://127.0.0.1:{port}/")),
        &Network::default(),
        sink,
    )
    .await
    .unwrap();
    socket.close();
    let events = log.until_closed().await;
    assert!(
        matches!(
            events.last(),
            Some(WsEvent::Closed {
                by_server: false,
                ..
            })
        ),
        "{events:?}"
    );

    // No token: the handshake is refused, with the server's status and words.
    let mut anonymous = request(format!("ws://127.0.0.1:{port}/"));
    anonymous.auth = Auth::None;
    let (_, sink) = Log::new();
    let error = websocket::connect(&anonymous, &Network::default(), sink)
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("401") && error.to_string().contains("no token"),
        "{error}"
    );

    // wss:// with a self-signed certificate, verification off.
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into()),
    )
    .unwrap();
    let port = ws_server(Some(tokio_rustls::TlsAcceptor::from(Arc::new(config)))).await;
    let mut secure = request(format!("wss://localhost:{port}/"));
    secure.settings = Settings {
        verify_tls: false,
        ..Default::default()
    };
    let (log, sink) = Log::new();
    let _socket = websocket::connect(&secure, &Network::default(), sink)
        .await
        .unwrap();
    let events = log.wait_for(2).await;
    assert_eq!(text(&events[1]), Some((true, "hello Bearer t0k|".into())));
}
