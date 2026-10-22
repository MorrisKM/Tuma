//! The on-disk format: what each YAML file in a workspace contains. Field order here is the key order in the files,
//! and empty or default fields are left out, so files stay short and diffs stay clean.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::expect::Expectations;
use crate::http::auth::{AwsAuth, DigestAuth, JwtAuth, OAuth1Auth, OAuth2Auth};
use crate::http::{self, ApiKeyLocation};

/// `workspace.yaml` at the workspace root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceFile {
    pub name: String,
    /// Identifies the workspace in the OS keychain, so clones of the same repo share secrets on one machine.
    pub id: String,
    /// Global variables, the lowest-priority scope.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

/// `_folder.yaml` in a collection or folder directory. Everything is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderFile {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Children in display order, by folder name or request name. Children not listed come after, sorted by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Applies to requests inside that inherit auth. Absent means inherit from the parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    /// Sent with every request inside, unless a request sets the same header.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
    #[serde(skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
    /// For a collection made from an OpenAPI spec: where the spec comes from (a URL, or a path relative to the
    /// workspace), so the collection can be synced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi: Option<String>,
}

/// `<name>.yaml`: one request. The file name (without `.yaml`) is the request's name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestFile {
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The OpenAPI operation this request calls, as `METHOD /path/{param}`, for sync and contract checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    #[serde(skip_serializing_if = "Body::is_none")]
    pub body: Body,
    /// Absent means inherit from the folder; `type: none` means send no auth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(skip_serializing_if = "is_default_settings")]
    pub settings: http::Settings,
    /// Checks that `tuma test` and `tuma run` apply to the response.
    #[serde(skip_serializing_if = "Expectations::is_empty")]
    pub expect: Expectations,
    /// Pre-request and test scripts (JavaScript, Postman's `pm.*` API). Collections and folders can have them too.
    #[serde(skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
    /// For a WebSocket request (a `ws://` or `wss://` URL) or a streaming gRPC call: messages to send with a click.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<SavedMessage>,
    /// For a gRPC request (a `grpc://` or `grpcs://` URL): the method, and `.proto` files if the server has no
    /// reflection. The message is the JSON body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grpc: Option<crate::grpc::GrpcSettings>,
}

/// A WebSocket message kept with its request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedMessage {
    pub name: String,
    /// Text; `{{variables}}` are resolved when it is sent.
    pub content: String,
}

impl RequestFile {
    /// A WebSocket request: its URL starts with `ws://` or `wss://`.
    pub fn is_websocket(&self) -> bool {
        let url = self.url.trim_start().to_ascii_lowercase();
        url.starts_with("ws://") || url.starts_with("wss://")
    }

    /// A gRPC request: its URL starts with `grpc://` or `grpcs://`.
    pub fn is_grpc(&self) -> bool {
        let url = self.url.trim_start().to_ascii_lowercase();
        url.starts_with("grpc://") || url.starts_with("grpcs://")
    }
}

impl Default for RequestFile {
    fn default() -> Self {
        Self {
            method: "GET".into(),
            url: String::new(),
            description: String::new(),
            operation: None,
            headers: Vec::new(),
            body: Body::None,
            auth: None,
            settings: http::Settings::default(),
            expect: Expectations::default(),
            scripts: Scripts::default(),
            messages: Vec::new(),
            grpc: None,
        }
    }
}

fn is_default_settings(settings: &http::Settings) -> bool {
    *settings == http::Settings::default()
}

/// `environments/<name>.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentFile {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

/// A variable. A secret's value lives in the OS keychain, never in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Variable {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(skip_serializing_if = "is_false")]
    pub secret: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl Default for Variable {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            secret: false,
            enabled: true,
        }
    }
}

impl Variable {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            secret: false,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyValue {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
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

impl From<&KeyValue> for http::KeyValue {
    fn from(kv: &KeyValue) -> Self {
        Self {
            name: kv.name.clone(),
            value: kv.value.clone(),
            enabled: kv.enabled,
        }
    }
}

impl From<&http::KeyValue> for KeyValue {
    fn from(kv: &http::KeyValue) -> Self {
        Self {
            name: kv.name.clone(),
            value: kv.value.clone(),
            enabled: kv.enabled,
        }
    }
}

/// A request body. Text bodies name their language (`type: json`) instead of a content type, because that is what
/// people pick in the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Body {
    #[default]
    None,
    Json {
        content: String,
    },
    Xml {
        content: String,
    },
    Html {
        content: String,
    },
    Text {
        content: String,
    },
    Javascript {
        content: String,
    },
    /// Text with any other content type.
    #[serde(rename_all = "camelCase")]
    Raw {
        content: String,
        content_type: String,
    },
    Form {
        fields: Vec<KeyValue>,
    },
    Multipart {
        parts: Vec<Part>,
    },
    /// A GraphQL operation, sent as JSON (`{"query", "variables", "operationName"}`).
    #[serde(rename_all = "camelCase")]
    Graphql {
        query: String,
        /// JSON text; `{{variables}}` are resolved before it is parsed.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        variables: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_name: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    File {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
    },
}

const TEXT_TYPES: [(&str, &str); 5] = [
    ("json", "application/json"),
    ("xml", "application/xml"),
    ("html", "text/html"),
    ("text", "text/plain"),
    ("javascript", "application/javascript"),
];

impl Body {
    pub fn is_none(&self) -> bool {
        matches!(self, Body::None)
    }

    /// A text body for a content type, using a named type such as `json` when there is one.
    pub fn text(content: String, content_type: &str) -> Self {
        match TEXT_TYPES
            .iter()
            .find(|(_, ct)| *ct == content_type)
            .map(|(name, _)| *name)
        {
            Some("json") => Body::Json { content },
            Some("xml") => Body::Xml { content },
            Some("html") => Body::Html { content },
            Some("text") => Body::Text { content },
            Some("javascript") => Body::Javascript { content },
            _ => Body::Raw {
                content,
                content_type: content_type.to_string(),
            },
        }
    }
}

impl From<&Body> for http::Body {
    fn from(body: &Body) -> Self {
        let raw = |content: &String, name: &str| {
            let content_type = TEXT_TYPES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, ct)| *ct)
                .unwrap_or_default();
            http::Body::Raw {
                content: content.clone(),
                content_type: content_type.to_string(),
            }
        };
        match body {
            Body::None => http::Body::None,
            // Comments are for the people reading the request, not the server.
            Body::Json { content } => raw(&crate::jsonc::strip(content), "json"),
            Body::Xml { content } => raw(content, "xml"),
            Body::Html { content } => raw(content, "html"),
            Body::Text { content } => raw(content, "text"),
            Body::Javascript { content } => raw(content, "javascript"),
            Body::Raw {
                content,
                content_type,
            } => http::Body::Raw {
                content: content.clone(),
                content_type: content_type.clone(),
            },
            Body::Form { fields } => http::Body::FormUrlEncoded {
                fields: fields.iter().map(Into::into).collect(),
            },
            Body::Multipart { parts } => http::Body::Multipart {
                parts: parts.iter().map(Into::into).collect(),
            },
            Body::File { path, content_type } => http::Body::File {
                path: path.clone(),
                content_type: content_type.clone(),
            },
            Body::Graphql {
                query,
                variables,
                operation_name,
            } => http::Body::Graphql {
                query: query.clone(),
                variables: variables.clone(),
                operation_name: operation_name.clone(),
            },
        }
    }
}

impl From<&http::Body> for Body {
    fn from(body: &http::Body) -> Self {
        match body {
            http::Body::None => Body::None,
            http::Body::Raw {
                content,
                content_type,
            } => Body::text(content.clone(), content_type),
            http::Body::FormUrlEncoded { fields } => Body::Form {
                fields: fields.iter().map(Into::into).collect(),
            },
            http::Body::Multipart { parts } => Body::Multipart {
                parts: parts.iter().map(Into::into).collect(),
            },
            http::Body::File { path, content_type } => Body::File {
                path: path.clone(),
                content_type: content_type.clone(),
            },
            http::Body::Graphql {
                query,
                variables,
                operation_name,
            } => Body::Graphql {
                query: query.clone(),
                variables: variables.clone(),
                operation_name: operation_name.clone(),
            },
        }
    }
}

/// A multipart field. With `file: true`, `value` is a file path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Part {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "is_false")]
    pub file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "is_true")]
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

impl From<&Part> for http::Part {
    fn from(p: &Part) -> Self {
        Self {
            name: p.name.clone(),
            value: p.value.clone(),
            file: p.file,
            content_type: p.content_type.clone(),
            enabled: p.enabled,
        }
    }
}

impl From<&http::Part> for Part {
    fn from(p: &http::Part) -> Self {
        Self {
            name: p.name.clone(),
            value: p.value.clone(),
            file: p.file,
            content_type: p.content_type.clone(),
            enabled: p.enabled,
        }
    }
}

/// Auth as written in files. Same shapes as [`http::Auth`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Auth {
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
        #[serde(default)]
        location: ApiKeyLocation,
    },
    /// HTTP Digest: answers the server's challenge.
    Digest(DigestAuth),
    /// AWS Signature Version 4.
    Aws(AwsAuth),
    /// A JWT signed from a payload, sent as a bearer token.
    Jwt(JwtAuth),
    #[serde(rename = "oauth1")]
    OAuth1(OAuth1Auth),
    /// Tokens are kept on this machine (in the keychain), not in the file.
    #[serde(rename = "oauth2")]
    OAuth2(OAuth2Auth),
}

impl From<&Auth> for http::Auth {
    fn from(auth: &Auth) -> Self {
        match auth.clone() {
            Auth::None => http::Auth::None,
            Auth::Bearer { token } => http::Auth::Bearer { token },
            Auth::Basic { username, password } => http::Auth::Basic { username, password },
            Auth::ApiKey {
                key,
                value,
                location,
            } => http::Auth::ApiKey {
                key,
                value,
                location,
            },
            Auth::Digest(a) => http::Auth::Digest(a),
            Auth::Aws(a) => http::Auth::Aws(a),
            Auth::Jwt(a) => http::Auth::Jwt(a),
            Auth::OAuth1(a) => http::Auth::OAuth1(a),
            Auth::OAuth2(a) => http::Auth::OAuth2(a),
        }
    }
}

impl From<&http::Auth> for Auth {
    fn from(auth: &http::Auth) -> Self {
        match auth.clone() {
            http::Auth::Digest(a) => Auth::Digest(a),
            http::Auth::Aws(a) => Auth::Aws(a),
            http::Auth::Jwt(a) => Auth::Jwt(a),
            http::Auth::OAuth1(a) => Auth::OAuth1(a),
            http::Auth::OAuth2(a) => Auth::OAuth2(a),
            http::Auth::None => Auth::None,
            http::Auth::Bearer { token } => Auth::Bearer { token },
            http::Auth::Basic { username, password } => Auth::Basic { username, password },
            http::Auth::ApiKey {
                key,
                value,
                location,
            } => Auth::ApiKey {
                key,
                value,
                location,
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Scripts {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pre_request: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub test: String,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.pre_request.is_empty() && self.test.is_empty()
    }
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Serializes to YAML with the workspace style: block scalars (`|`) for multi-line text and no folding of long lines,
/// which would make diffs noisy.
pub fn to_yaml<T: Serialize>(value: &T) -> Result<String, serde_saphyr::ser::Error> {
    let options = serde_saphyr::ser_options! { folded_wrap_chars: usize::MAX };
    serde_saphyr::to_string_with_options(value, options)
}
