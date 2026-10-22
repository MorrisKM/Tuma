use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Why a request could not be sent or its response not read. HTTP error statuses (4xx, 5xx) are not errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid URL \"{url}\": {reason}")]
    InvalidUrl { url: String, reason: String },
    #[error("unsupported URL scheme \"{0}\": use http or https")]
    UnsupportedScheme(String),
    #[error("invalid method \"{0}\"")]
    InvalidMethod(String),
    #[error("invalid header \"{0}\"")]
    InvalidHeader(String),
    #[error("could not read {}: {source}", path.display())]
    File { path: PathBuf, source: io::Error },
    #[error("could not resolve {host}: {source}")]
    Dns { host: String, source: io::Error },
    #[error("could not connect to {addr}: {source}")]
    Connect { addr: SocketAddr, source: io::Error },
    #[error("TLS setup failed: {0}")]
    TlsConfig(String),
    #[error("TLS handshake with {host} failed: {source}")]
    Tls { host: String, source: io::Error },
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),
    #[error("request timed out after {0} ms")]
    Timeout(u64),
    #[error("stopped after {0} redirects")]
    TooManyRedirects(u32),
    #[error("could not decode the {encoding} response body: {source}")]
    Decode { encoding: String, source: io::Error },
}

impl Error {
    /// A stable name for the kind of error, for the UI and for `--json` output.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::InvalidUrl { .. } | Error::UnsupportedScheme(_) => "invalidUrl",
            Error::InvalidMethod(_) | Error::InvalidHeader(_) => "invalidRequest",
            Error::File { .. } => "file",
            Error::Dns { .. } => "dns",
            Error::Connect { .. } => "connect",
            Error::TlsConfig(_) | Error::Tls { .. } => "tls",
            Error::Http(_) => "http",
            Error::Timeout(_) => "timeout",
            Error::TooManyRedirects(_) => "redirects",
            Error::Decode { .. } => "decode",
        }
    }
}
