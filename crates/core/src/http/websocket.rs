//! WebSocket connections: the handshake goes the way requests do (proxy, client certificates, extra CAs, auth headers,
//! cookies), then messages flow both ways until either side closes. Everything that happens is reported as a
//! [`WsEvent`], for a message log.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use http::header::{COOKIE, HeaderName, SET_COOKIE};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

use super::client::{connect_to, proxy_authorization, tunnel};
use super::model::{Header, Request, Timing};
use super::network::Network;
use super::prepare::prepare;
use super::{Error, tls};

/// What happens on a connection, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WsEvent {
    /// The handshake succeeded.
    #[serde(rename_all = "camelCase")]
    Open {
        url: String,
        status: u16,
        headers: Vec<Header>,
        /// The subprotocol the server chose, if any.
        protocol: Option<String>,
    },
    /// A message, sent or received. Binary data is base64.
    #[serde(rename_all = "camelCase")]
    Message {
        incoming: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        binary: Option<String>,
        size: usize,
        /// Unix time in milliseconds.
        at: u64,
    },
    #[serde(rename_all = "camelCase")]
    Closed {
        code: Option<u16>,
        reason: String,
        by_server: bool,
    },
    Error {
        message: String,
    },
}

/// What a connection can be told to do.
#[derive(Debug, Clone)]
pub enum Outgoing {
    Text(String),
    Binary(Vec<u8>),
    Close,
}

/// An open connection. Dropping it closes the connection.
pub struct Socket {
    tx: mpsc::UnboundedSender<Outgoing>,
}

impl Socket {
    /// Queues a message; it shows up as a [`WsEvent::Message`] once sent. Fails if the connection is gone.
    pub fn send(&self, message: Outgoing) -> Result<(), String> {
        self.tx
            .send(message)
            .map_err(|_| "the connection is closed".to_string())
    }

    pub fn close(&self) {
        let _ = self.tx.send(Outgoing::Close);
    }
}

pub type EventSink = Arc<dyn Fn(WsEvent) + Send + Sync>;

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// The handshake's headers that the WebSocket library sets itself.
const OWN_HEADERS: [&str; 8] = [
    "host",
    "connection",
    "upgrade",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-extensions",
    "accept-encoding",
    "content-length",
];

/// Opens a connection to `request`'s `ws://` or `wss://` URL, with its headers and auth (variables already resolved).
/// Events go to `events`, starting with [`WsEvent::Open`].
pub async fn connect(
    request: &Request,
    network: &Network,
    events: EventSink,
) -> Result<Socket, Error> {
    let raw = request.url.trim();
    let (secure, rest) = match raw.split_once("://") {
        Some(("wss", rest)) => (true, rest),
        Some(("ws", rest)) => (false, rest),
        Some((other, _)) => return Err(Error::UnsupportedScheme(other.to_string())),
        None => (false, raw),
    };
    // Headers and auth are built as for an HTTP request to the same place.
    let as_http = Request {
        method: "GET".into(),
        url: format!("{}://{rest}", if secure { "https" } else { "http" }),
        body: super::Body::None,
        ..request.clone()
    };
    let prepared = prepare(&as_http).await?;
    let http_url = prepared.url;
    let mut ws_url = http_url.clone();
    let _ = ws_url.set_scheme(if secure { "wss" } else { "ws" });

    let mut handshake = ws_url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::WebSocket(e.to_string()))?;
    for (name, value) in prepared.headers.iter() {
        if !OWN_HEADERS.contains(&name.as_str()) {
            handshake.headers_mut().append(name.clone(), value.clone());
        }
    }
    if let Some(jar) = &network.cookies {
        let cookies: Vec<String> = jar
            .lock()
            .unwrap()
            .get_request_values(&http_url)
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        if !cookies.is_empty()
            && let Ok(value) = cookies.join("; ").parse()
        {
            handshake.headers_mut().insert(COOKIE, value);
        }
    }

    let host = http_url.host_str().unwrap_or_default().to_string();
    let port = http_url.port_or_known_default().unwrap_or(80);
    let proxy = network.proxy.as_ref().and_then(|p| p.for_url(&http_url));
    let mut timing = Timing::default();
    let mut tcp = connect_to(proxy.unwrap_or(&http_url), &mut timing).await?;
    if let Some(proxy) = proxy {
        tunnel(&mut tcp, &host, port, proxy_authorization(proxy).as_deref()).await?;
    }
    let stream: Box<dyn Io> = if secure {
        let config = tls::config(tls::Options {
            verify: request.settings.verify_tls,
            client_certificate: network.client_certificate(&host, Some(port)),
            ca_certificates: &network.ca_certificates,
            http1_only: true,
        })?;
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| {
            Error::InvalidUrl {
                url: request.url.clone(),
                reason: e.to_string(),
            }
        })?;
        let tls = TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .map_err(|source| Error::Tls {
                host: host.clone(),
                source,
            })?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };

    let (socket, response) = tokio_tungstenite::client_async(handshake, stream)
        .await
        .map_err(|e| match e {
            tungstenite::Error::Http(response) => {
                let body = response
                    .body()
                    .as_ref()
                    .map(|b| String::from_utf8_lossy(b).trim().to_string())
                    .unwrap_or_default();
                Error::WebSocket(format!(
                    "the server refused the WebSocket: {}{}",
                    response.status(),
                    if body.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", body.chars().take(300).collect::<String>())
                    }
                ))
            }
            other => Error::WebSocket(other.to_string()),
        })?;
    if let Some(jar) = &network.cookies {
        let cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| cookie_store::RawCookie::parse(v.to_string()).ok());
        jar.lock()
            .unwrap()
            .store_response_cookies(cookies, &http_url);
    }
    let protocol_header = HeaderName::from_static("sec-websocket-protocol");
    events(WsEvent::Open {
        url: ws_url.to_string(),
        status: response.status().as_u16(),
        headers: response
            .headers()
            .iter()
            .map(|(n, v)| Header {
                name: n.to_string(),
                value: String::from_utf8_lossy(v.as_bytes()).into_owned(),
            })
            .collect(),
        protocol: response
            .headers()
            .get(&protocol_header)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
    });

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(run(socket, rx, events));
    Ok(Socket { tx })
}

fn message_event(message: &Message, incoming: bool) -> Option<WsEvent> {
    let at = now_ms();
    match message {
        Message::Text(text) => Some(WsEvent::Message {
            incoming,
            size: text.len(),
            text: Some(text.to_string()),
            binary: None,
            at,
        }),
        Message::Binary(data) => Some(WsEvent::Message {
            incoming,
            size: data.len(),
            text: None,
            binary: Some(base64::engine::general_purpose::STANDARD.encode(data)),
            at,
        }),
        _ => None,
    }
}

/// Moves messages both ways until the connection ends.
async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    mut rx: mpsc::UnboundedReceiver<Outgoing>,
    events: EventSink,
) {
    let mut closing = false;
    loop {
        tokio::select! {
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Close(frame))) => {
                    events(WsEvent::Closed {
                        code: frame.as_ref().map(|f| u16::from(f.code)),
                        reason: frame.map(|f| f.reason.to_string()).unwrap_or_default(),
                        by_server: !closing,
                    });
                    // Answer the close and let the library finish the closing handshake.
                    let _ = socket.close(None).await;
                    return;
                }
                Some(Ok(message)) => {
                    if let Some(event) = message_event(&message, true) {
                        events(event);
                    }
                }
                Some(Err(e)) => {
                    events(WsEvent::Error { message: e.to_string() });
                    events(WsEvent::Closed { code: None, reason: String::new(), by_server: true });
                    return;
                }
                None => {
                    events(WsEvent::Closed { code: None, reason: String::new(), by_server: !closing });
                    return;
                }
            },
            outgoing = rx.recv(), if !closing => {
                let message = match outgoing {
                    Some(Outgoing::Text(text)) => Message::text(text),
                    Some(Outgoing::Binary(data)) => Message::binary(data),
                    // Asked to close, or every handle is gone.
                    Some(Outgoing::Close) | None => {
                        closing = true;
                        let frame = CloseFrame { code: CloseCode::Normal, reason: "".into() };
                        if socket.send(Message::Close(Some(frame))).await.is_err() {
                            events(WsEvent::Closed { code: Some(1000), reason: String::new(), by_server: false });
                            return;
                        }
                        continue;
                    }
                };
                let event = message_event(&message, false);
                match socket.send(message).await {
                    Ok(()) => {
                        if let Some(event) = event {
                            events(event);
                        }
                    }
                    Err(e) => events(WsEvent::Error { message: format!("could not send: {e}") }),
                }
            }
        }
    }
}
