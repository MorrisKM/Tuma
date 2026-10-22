//! Sends a request over a fresh connection and measures each phase. Built on hyper's connection-level API rather than
//! a pooled client, because a pool hides DNS, connect and TLS times.

use std::io::Read;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::header::{
    AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HOST, LOCATION,
    PROXY_AUTHORIZATION, SET_COOKIE, WWW_AUTHENTICATE,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode, Version};
use http_body_util::{BodyExt, Full};
use hyper::rt::{Read as HyperRead, Write as HyperWrite};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use url::{Host, Position, Url};

use super::Error;
use super::auth;
use super::model::{Auth, Header, Redirect, Request, Response, SentRequest, Timing};
use super::network::Network;
use super::prepare::prepare;
use super::tls;

/// Sends `request` directly, with no cookie jar, and reads the whole response. Dropping the future cancels it.
pub async fn send(request: &Request) -> Result<Response, Error> {
    send_with(request, &Network::default()).await
}

/// Sends `request` the way `network` says: through its proxy, with its certificates and cookie jar.
pub async fn send_with(request: &Request, network: &Network) -> Result<Response, Error> {
    match request.settings.timeout_ms {
        Some(ms) if ms > 0 => {
            tokio::time::timeout(Duration::from_millis(ms), send_inner(request, network))
                .await
                .map_err(|_| Error::Timeout(ms))?
        }
        _ => send_inner(request, network).await,
    }
}

async fn send_inner(request: &Request, network: &Network) -> Result<Response, Error> {
    let start = Instant::now();
    let prepared = prepare(request).await?;
    let (mut method, mut url, mut headers, mut body) = (
        prepared.method,
        prepared.url,
        prepared.headers,
        prepared.body,
    );
    let mut redirects = Vec::new();
    // Digest answers the server's challenge once per URL.
    let mut digest_answered = None::<Url>;
    // A Cookie header set on the request is sent as is, with the jar's cookies after it.
    let own_cookies = headers.get(COOKIE).cloned();

    loop {
        let mut hop_headers = headers.clone();
        if let Some(jar) = &network.cookies {
            let from_jar: Vec<String> = jar
                .lock()
                .unwrap()
                .get_request_values(&url)
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            if !from_jar.is_empty() {
                let mut value = own_cookies
                    .as_ref()
                    .filter(|_| headers.contains_key(COOKIE))
                    .and_then(|v| v.to_str().ok())
                    .map(|v| format!("{v}; "))
                    .unwrap_or_default();
                value.push_str(&from_jar.join("; "));
                if let Ok(value) = HeaderValue::from_str(&value) {
                    hop_headers.insert(COOKIE, value);
                }
            }
        }
        let exchange = exchange(
            &method,
            &url,
            &hop_headers,
            body.clone(),
            request.settings.verify_tls,
            network,
        )
        .await?;
        if let Some(jar) = &network.cookies {
            let cookies = exchange
                .headers
                .get_all(SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .filter_map(|v| cookie_store::RawCookie::parse(v.to_string()).ok());
            jar.lock().unwrap().store_response_cookies(cookies, &url);
        }
        let status = exchange.status;
        if let Auth::Digest(digest) = &request.auth
            && status == StatusCode::UNAUTHORIZED
            && digest_answered.as_ref() != Some(&url)
            && let Some(answer) = exchange
                .headers
                .get_all(WWW_AUTHENTICATE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find_map(|challenge| {
                    let uri = &url[Position::BeforePath..Position::AfterQuery];
                    auth::digest_authorization(challenge, digest, method.as_str(), uri, None)
                })
        {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&answer)
                    .map_err(|_| Error::InvalidHeader("Authorization".into()))?,
            );
            digest_answered = Some(url.clone());
            continue;
        }
        let location = exchange.headers.get(LOCATION).and_then(|l| l.to_str().ok());

        if let Some(next) =
            location.filter(|_| request.settings.follow_redirects && status.is_redirection())
            && let Ok(next) = url.join(next)
        {
            if redirects.len() as u32 >= request.settings.max_redirects {
                return Err(Error::TooManyRedirects(request.settings.max_redirects));
            }
            redirects.push(Redirect {
                status: status.as_u16(),
                url: url.to_string(),
            });
            // Browsers and curl turn POST into GET for 301 and 302, and anything but HEAD into GET for 303.
            let to_get = match status {
                StatusCode::SEE_OTHER => method != Method::HEAD,
                StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => method == Method::POST,
                _ => false,
            };
            if to_get {
                method = Method::GET;
                body = Bytes::new();
                headers.remove(CONTENT_TYPE);
                headers.remove(CONTENT_LENGTH);
            }
            // Credentials must not leak to another site.
            if next.origin() != url.origin() {
                headers.remove(AUTHORIZATION);
                headers.remove(COOKIE);
            }
            url = next;
            continue;
        }

        let encoded_size = exchange.body.len();
        let body = decode(&exchange.headers, exchange.body)?;
        let mut timing = exchange.timing;
        timing.total = start.elapsed();
        return Ok(Response {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_string(),
            version: version_name(exchange.version).to_string(),
            url: url.to_string(),
            remote_addr: exchange.remote_addr,
            headers: header_list(&exchange.headers),
            body,
            encoded_size,
            timing,
            redirects,
            sent: exchange.sent,
        });
    }
}

struct Exchange {
    status: StatusCode,
    version: Version,
    headers: HeaderMap,
    body: Bytes,
    remote_addr: Option<SocketAddr>,
    timing: Timing,
    sent: SentRequest,
}

/// One request and response over a new connection.
async fn exchange(
    method: &Method,
    url: &Url,
    headers: &HeaderMap,
    body: Bytes,
    verify_tls: bool,
    network: &Network,
) -> Result<Exchange, Error> {
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let mut timing = Timing::default();
    let proxy = network.proxy.as_ref().and_then(|p| p.for_url(url));

    // Through a proxy, DNS and connect times are the proxy's.
    let target = proxy.unwrap_or(url);
    let mut stream = connect_to(target, &mut timing).await?;
    let remote_addr = stream.peer_addr().ok();
    let mut headers = headers.clone();
    let mut absolute_form = false;
    if let Some(proxy) = proxy {
        let auth = proxy_authorization(proxy);
        if url.scheme() == "https" {
            tunnel(&mut stream, &host, port, auth.as_deref()).await?;
        } else {
            // A plain HTTP request goes to the proxy with the full URL.
            absolute_form = true;
            if let Some(auth) = auth {
                headers.insert(
                    PROXY_AUTHORIZATION,
                    HeaderValue::from_str(&auth)
                        .map_err(|_| Error::InvalidHeader("Proxy-Authorization".into()))?,
                );
            }
        }
    }
    let headers = &headers;

    if url.scheme() == "https" {
        let started = Instant::now();
        let server_name = match url.host() {
            Some(Host::Ipv4(ip)) => ServerName::IpAddress(std::net::IpAddr::from(ip).into()),
            Some(Host::Ipv6(ip)) => ServerName::IpAddress(std::net::IpAddr::from(ip).into()),
            _ => ServerName::try_from(host.clone()).map_err(|e| Error::InvalidUrl {
                url: url.to_string(),
                reason: e.to_string(),
            })?,
        };
        let connector = TlsConnector::from(tls::config(tls::Options {
            verify: verify_tls,
            client_certificate: network.client_certificate(&host, Some(port)),
            ca_certificates: &network.ca_certificates,
            http1_only: false,
        })?);
        let stream = connector
            .connect(server_name, stream)
            .await
            .map_err(|source| Error::Tls { host, source })?;
        timing.tls = Some(started.elapsed());
        let http2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
        exchange_over(
            TokioIo::new(stream),
            http2,
            false,
            method,
            url,
            headers,
            body,
            remote_addr,
            timing,
        )
        .await
    } else {
        exchange_over(
            TokioIo::new(stream),
            false,
            absolute_form,
            method,
            url,
            headers,
            body,
            remote_addr,
            timing,
        )
        .await
    }
}

/// Resolves and connects to a URL's host, recording DNS and connect times.
pub(crate) async fn connect_to(url: &Url, timing: &mut Timing) -> Result<TcpStream, Error> {
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
        _ => {
            let started = Instant::now();
            let found = tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|source| Error::Dns {
                    host: host.clone(),
                    source,
                })?;
            timing.dns = Some(started.elapsed());
            found.collect()
        }
    };
    let started = Instant::now();
    let stream = connect(&addrs, &host).await?;
    timing.connect = started.elapsed();
    // Nagle's algorithm would delay small request bodies.
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// `Basic ...` for a proxy URL with a user name, such as `http://user:pass@proxy:3128`.
pub(crate) fn proxy_authorization(proxy: &Url) -> Option<String> {
    use base64::Engine;
    if proxy.username().is_empty() {
        return None;
    }
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    let pair = format!(
        "{}:{}",
        decode(proxy.username()),
        decode(proxy.password().unwrap_or_default())
    );
    Some(format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(pair)
    ))
}

/// Asks an HTTP proxy for a tunnel to `host:port` (`CONNECT`), for TLS (and WebSockets) through it.
pub(crate) async fn tunnel(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    auth: Option<&str>,
) -> Result<(), Error> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(auth) = auth {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    let proxy_error = |message: String| Error::Proxy(message);
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| proxy_error(e.to_string()))?;
    // Read the proxy's answer up to the blank line; nothing else is sent before the tunnel is open.
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        if answer.len() > 16 * 1024 {
            return Err(proxy_error(
                "the proxy's answer to CONNECT is too long".into(),
            ));
        }
        let n = stream
            .read(&mut byte)
            .await
            .map_err(|e| proxy_error(e.to_string()))?;
        if n == 0 {
            return Err(proxy_error("the proxy closed the connection".into()));
        }
        answer.push(byte[0]);
    }
    let status_line = String::from_utf8_lossy(&answer)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let code = status_line.split_whitespace().nth(1).unwrap_or_default();
    match code {
        "200" => Ok(()),
        "407" => Err(proxy_error(format!(
            "the proxy wants credentials ({status_line}); add them to its URL or settings"
        ))),
        _ => Err(proxy_error(format!(
            "the proxy refused the tunnel to {authority}: {status_line}"
        ))),
    }
}

/// Tries each address in turn and returns the first connection that succeeds.
async fn connect(addrs: &[SocketAddr], host: &str) -> Result<TcpStream, Error> {
    let mut last_error = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(source) => {
                last_error = Some(Error::Connect {
                    addr: *addr,
                    source,
                })
            }
        }
    }
    Err(last_error.unwrap_or_else(|| Error::Dns {
        host: host.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "no addresses found"),
    }))
}

#[allow(clippy::too_many_arguments)]
async fn exchange_over<T>(
    io: T,
    http2: bool,
    // Send the full URL as the request target, for a plain HTTP proxy.
    absolute_form: bool,
    method: &Method,
    url: &Url,
    headers: &HeaderMap,
    body: Bytes,
    remote_addr: Option<SocketAddr>,
    mut timing: Timing,
) -> Result<Exchange, Error>
where
    T: HyperRead + HyperWrite + Unpin + Send + 'static,
{
    let mut headers = headers.clone();
    // HTTP/1.1 sends the path and a Host header; HTTP/2 sends the full URL and no Host header.
    let target = if http2 {
        headers.remove(HOST);
        url.as_str().to_string()
    } else {
        if !headers.contains_key(HOST) {
            let authority = &url[Position::BeforeHost..Position::AfterPort];
            headers.insert(
                HOST,
                HeaderValue::from_str(authority)
                    .map_err(|_| Error::InvalidHeader(authority.into()))?,
            );
        }
        if absolute_form {
            url[..Position::AfterQuery].to_string()
        } else {
            url[Position::BeforePath..Position::AfterQuery].to_string()
        }
    };

    let sent = SentRequest {
        method: method.to_string(),
        url: url.to_string(),
        headers: header_list(&headers),
        body_size: body.len(),
    };
    let mut builder = http::Request::builder().method(method.clone()).uri(&target);
    if let Some(map) = builder.headers_mut() {
        *map = headers;
    }
    let request = builder
        .body(Full::new(body))
        .map_err(|e| Error::InvalidUrl {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let started = Instant::now();
    let response = if http2 {
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await?;
        tokio::spawn(connection);
        sender.send_request(request).await?
    } else {
        let (mut sender, connection) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(connection);
        sender.send_request(request).await?
    };
    timing.wait = started.elapsed();

    let (parts, incoming) = response.into_parts();
    let started = Instant::now();
    let body = incoming.collect().await?.to_bytes();
    timing.download = started.elapsed();

    Ok(Exchange {
        status: parts.status,
        version: parts.version,
        headers: parts.headers,
        body,
        remote_addr,
        timing,
        sent,
    })
}

/// Undoes `Content-Encoding`. Unknown encodings are left as they are.
fn decode(headers: &HeaderMap, body: Bytes) -> Result<Bytes, Error> {
    let Some(encoding) = headers.get(CONTENT_ENCODING).and_then(|e| e.to_str().ok()) else {
        return Ok(body);
    };
    let encoding = encoding.trim().to_ascii_lowercase();
    if body.is_empty() {
        return Ok(body);
    }
    let mut out = Vec::new();
    let result = match encoding.as_str() {
        "gzip" | "x-gzip" => flate2::read::MultiGzDecoder::new(&body[..]).read_to_end(&mut out),
        // "deflate" is meant to be zlib-wrapped, but some servers send raw deflate.
        "deflate" => flate2::read::ZlibDecoder::new(&body[..])
            .read_to_end(&mut out)
            .or_else(|_| {
                out.clear();
                flate2::read::DeflateDecoder::new(&body[..]).read_to_end(&mut out)
            }),
        "br" => brotli::Decompressor::new(&body[..], 64 * 1024).read_to_end(&mut out),
        _ => return Ok(body),
    };
    result
        .map(|_| Bytes::from(out))
        .map_err(|source| Error::Decode { encoding, source })
}

fn header_list(headers: &HeaderMap) -> Vec<Header> {
    headers
        .iter()
        .map(|(name, value)| Header {
            name: name.to_string(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect()
}

fn version_name(version: Version) -> &'static str {
    match version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/1.1",
    }
}
