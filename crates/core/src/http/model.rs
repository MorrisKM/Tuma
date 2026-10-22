//! What a request is made of and what comes back. These types are the engine's API; the desktop app and the CLI build
//! a [`Request`] and display a [`Response`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use super::auth::{AwsAuth, DigestAuth, JwtAuth, OAuth1Auth, OAuth2Auth};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// A request as the user wrote it. Variables (`{{var}}`) must already be resolved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    /// Any method, including custom ones. Sent upper case.
    pub method: String,
    /// The full URL, query string included. `http://` is assumed when there is no scheme.
    pub url: String,
    pub headers: Vec<KeyValue>,
    pub body: Body,
    pub auth: Auth,
    pub settings: Settings,
}

/// A row in a headers, query or form table. Disabled rows and rows with an empty name are not sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyValue {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

impl KeyValue {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            enabled: true,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.enabled && !self.name.is_empty()
    }
}

impl Default for KeyValue {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Body {
    #[default]
    None,
    /// Text sent as is: JSON, XML, HTML, plain text, JavaScript.
    #[serde(rename_all = "camelCase")]
    Raw {
        content: String,
        content_type: String,
    },
    /// `application/x-www-form-urlencoded`.
    FormUrlEncoded { fields: Vec<KeyValue> },
    /// `multipart/form-data`, with text fields and files.
    Multipart { parts: Vec<Part> },
    /// A GraphQL operation, sent as `application/json`.
    #[serde(rename_all = "camelCase")]
    Graphql {
        query: String,
        variables: String,
        operation_name: Option<String>,
    },
    /// The contents of a file. The content type is guessed from the extension when not given.
    #[serde(rename_all = "camelCase")]
    File {
        path: PathBuf,
        content_type: Option<String>,
    },
}

/// A multipart field. When `file` is set, `value` is a path and the file's contents are sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Part {
    pub name: String,
    pub value: String,
    pub file: bool,
    pub content_type: Option<String>,
    pub enabled: bool,
}

impl Default for Part {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            file: false,
            content_type: None,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Auth {
    #[default]
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        key: String,
        value: String,
        location: ApiKeyLocation,
    },
    Digest(DigestAuth),
    Aws(AwsAuth),
    Jwt(JwtAuth),
    #[serde(rename = "oauth1")]
    OAuth1(OAuth1Auth),
    #[serde(rename = "oauth2")]
    OAuth2(OAuth2Auth),
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ApiKeyLocation {
    #[default]
    Header,
    Query,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Give up after this long, for the whole exchange including redirects. `None` waits forever.
    pub timeout_ms: Option<u64>,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    /// Check the server's certificate against the operating system's trust store.
    pub verify_tls: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            follow_redirects: true,
            max_redirects: 10,
            verify_tls: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// The standard reason phrase for the status, such as "Not Found". Empty for unknown codes.
    pub reason: String,
    /// `HTTP/1.1` or `HTTP/2`.
    pub version: String,
    /// The URL that answered, after redirects.
    pub url: String,
    pub remote_addr: Option<SocketAddr>,
    pub headers: Vec<Header>,
    /// The body after undoing `Content-Encoding` (gzip, deflate, br).
    pub body: Bytes,
    /// The body's size on the wire, before decoding.
    pub encoded_size: usize,
    pub timing: Timing,
    /// Redirects followed to get here, in order.
    pub redirects: Vec<Redirect>,
    /// The last request exactly as it went out, for the console.
    pub sent: SentRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Redirect {
    pub status: u16,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body_size: usize,
}

/// Where the time went in the last exchange. Phases that did not happen are `None`: no DNS lookup for an IP address,
/// no TLS for plain HTTP.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    #[serde(with = "millis_opt")]
    pub dns: Option<Duration>,
    #[serde(with = "millis")]
    pub connect: Duration,
    #[serde(with = "millis_opt")]
    pub tls: Option<Duration>,
    /// From sending the request to the first byte of the response.
    #[serde(with = "millis")]
    pub wait: Duration,
    #[serde(with = "millis")]
    pub download: Duration,
    /// Everything, including any redirects.
    #[serde(with = "millis")]
    pub total: Duration,
}

/// Durations are sent to the UI as fractional milliseconds.
mod millis {
    use std::time::Duration;

    pub fn serialize<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(d.as_secs_f64() * 1000.0)
    }
}

mod millis_opt {
    use std::time::Duration;

    pub fn serialize<S: serde::Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&(d.as_secs_f64() * 1000.0)),
            None => s.serialize_none(),
        }
    }
}

impl Body {
    /// A GraphQL operation as the JSON it is sent as. Empty variables are left out.
    pub fn graphql_json(
        query: &str,
        variables: &str,
        operation_name: Option<&str>,
    ) -> Result<String, String> {
        let mut out = serde_json::json!({ "query": query });
        if !variables.trim().is_empty() {
            let variables: serde_json::Value = serde_json::from_str(variables)
                .map_err(|e| format!("the GraphQL variables are not valid JSON: {e}"))?;
            out["variables"] = variables;
        }
        if let Some(name) = operation_name.filter(|n| !n.trim().is_empty()) {
            out["operationName"] = name.into();
        }
        Ok(out.to_string())
    }

    /// The body with GraphQL turned into its JSON, for code that only knows plain bodies (snippets, cURL).
    pub fn plain(&self) -> Body {
        match self {
            Body::Graphql {
                query,
                variables,
                operation_name,
            } => Body::Raw {
                content: Body::graphql_json(query, variables, operation_name.as_deref())
                    .unwrap_or_else(|_| {
                        serde_json::json!({ "query": query, "variables": variables }).to_string()
                    }),
                content_type: "application/json".into(),
            },
            other => other.clone(),
        }
    }
}

impl Request {
    /// The request with a plain body; see [`Body::plain`].
    pub fn plain(&self) -> Request {
        Request {
            body: self.body.plain(),
            ..self.clone()
        }
    }
}
