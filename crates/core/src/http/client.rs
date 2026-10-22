//! Sends a request over a fresh connection and measures each phase. Built on hyper's connection-level API rather than
//! a pooled client, because a pool hides DNS, connect and TLS times.

use std::io::Read;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::header::{
    AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HOST, LOCATION,
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
use super::model::{Header, Redirect, Request, Response, SentRequest, Timing};
use super::prepare::prepare;
use super::tls;

/// Sends `request` and reads the whole response. Dropping the future cancels the request.
pub async fn send(request: &Request) -> Result<Response, Error> {
    match request.settings.timeout_ms {
        Some(ms) if ms > 0 => tokio::time::timeout(Duration::from_millis(ms), send_inner(request))
            .await
            .map_err(|_| Error::Timeout(ms))?,
        _ => send_inner(request).await,
    }
}

async fn send_inner(request: &Request) -> Result<Response, Error> {
    let start = Instant::now();
    let prepared = prepare(request).await?;
    let (mut method, mut url, mut headers, mut body) = (
        prepared.method,
        prepared.url,
        prepared.headers,
        prepared.body,
    );
    let mut redirects = Vec::new();

    loop {
        let exchange = exchange(
            &method,
            &url,
            &headers,
            body.clone(),
            request.settings.verify_tls,
        )
        .await?;
        let status = exchange.status;
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
) -> Result<Exchange, Error> {
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let mut timing = Timing::default();

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
    let remote_addr = stream.peer_addr().ok();
    // Nagle's algorithm would delay small request bodies.
    let _ = stream.set_nodelay(true);

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
        let connector = TlsConnector::from(tls::config(verify_tls)?);
        let stream = connector
            .connect(server_name, stream)
            .await
            .map_err(|source| Error::Tls { host, source })?;
        timing.tls = Some(started.elapsed());
        let http2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
        exchange_over(
            TokioIo::new(stream),
            http2,
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
        url[Position::BeforePath..Position::AfterQuery].to_string()
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
