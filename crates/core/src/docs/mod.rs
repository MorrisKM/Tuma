//! API documentation generated from requests: an OpenAPI 3.1 spec (YAML or JSON), Markdown, or a standalone HTML
//! page, for one request, a folder, a collection or the whole workspace.
//!
//! Everything comes from what the requests already say: the method and URL (path parameters are `{{name}}` or
//! `:name` segments), query parameters, headers, the body, auth (inherited from folders), descriptions, and saved
//! examples, whose JSON bodies also give the response schemas. Secret values never appear: auth is described by its
//! kind, and headers that carry credentials are shown without their values. Coding agents fill in what is missing by
//! writing request descriptions and saving examples (over MCP), then generating again.

mod html;
mod markdown;
mod openapi;

use serde_json::{Map, Value, json};

use crate::secrets::SecretStore;
use crate::workspace::files::{Auth, Body, RequestFile};
use crate::workspace::{Node, NodeKind, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    OpenApiYaml,
    OpenApiJson,
    Markdown,
    Html,
}

impl Format {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "openapi" | "openapi-yaml" | "yaml" | "swagger" => Format::OpenApiYaml,
            "openapi-json" | "json" => Format::OpenApiJson,
            "markdown" | "md" => Format::Markdown,
            "html" => Format::Html,
            _ => return None,
        })
    }

    /// The usual file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Format::OpenApiYaml => "yaml",
            Format::OpenApiJson => "json",
            Format::Markdown => "md",
            Format::Html => "html",
        }
    }
}

/// The API as the docs describe it.
#[derive(Debug, Clone, Default)]
pub struct Api {
    pub title: String,
    pub description: String,
    /// Base URLs, with variables filled in from the environment where they have a (non-secret) value.
    pub servers: Vec<String>,
    pub groups: Vec<Group>,
}

/// A collection or folder, with the requests directly in it.
#[derive(Debug, Clone, Default)]
pub struct Group {
    /// Such as "Shop API / Orders".
    pub name: String,
    pub description: String,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Http,
    WebSocket,
    Grpc,
}

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub name: String,
    pub kind: Kind,
    pub method: String,
    /// In OpenAPI's form: `/users/{id}`.
    pub path: String,
    /// The URL as written in the request.
    pub url: String,
    pub description: String,
    pub path_params: Vec<Param>,
    pub query: Vec<Param>,
    pub headers: Vec<Param>,
    pub body: Option<Payload>,
    pub auth: Option<AuthDoc>,
    pub responses: Vec<ResponseDoc>,
    /// For gRPC: the method, such as shop.v1.Shop/GetItem.
    pub grpc_method: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// The value as written, as an example; empty for credentials.
    pub example: String,
}

/// A body, as written or as saved in an example.
#[derive(Debug, Clone)]
pub struct Payload {
    pub content_type: String,
    /// Indented JSON when it is JSON (with comments removed), otherwise as written.
    pub example: String,
    /// Inferred from a JSON example.
    pub schema: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ResponseDoc {
    pub status: u16,
    pub name: String,
    pub body: Option<Payload>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthDoc {
    None,
    Bearer,
    Basic,
    ApiKey {
        name: String,
        in_query: bool,
    },
    Digest,
    OAuth2 {
        flow: &'static str,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
    /// Described in words: AWS Signature v4, JWT, OAuth 1.0.
    Other(String),
}

impl AuthDoc {
    pub fn describe(&self) -> String {
        match self {
            AuthDoc::None => "None".into(),
            AuthDoc::Bearer => "Bearer token (`Authorization: Bearer …`)".into(),
            AuthDoc::Basic => "HTTP Basic".into(),
            AuthDoc::ApiKey { name, in_query } => {
                format!(
                    "API key in the `{name}` {}",
                    if *in_query {
                        "query parameter"
                    } else {
                        "header"
                    }
                )
            }
            AuthDoc::Digest => "HTTP Digest".into(),
            AuthDoc::OAuth2 { flow, scopes, .. } => {
                let scopes = if scopes.is_empty() {
                    String::new()
                } else {
                    format!(", scopes {}", scopes.join(" "))
                };
                format!("OAuth 2.0 ({flow}{scopes})")
            }
            AuthDoc::Other(text) => text.clone(),
        }
    }
}

/// What a generation produced: the text, and what could not be described.
pub struct Generated {
    pub text: String,
    pub warnings: Vec<String>,
}

/// Generates docs for `scope`, a request's or folder's path, or the whole workspace when `None`. `environment` gives
/// the base URLs their values.
pub fn generate(
    workspace: &Workspace,
    scope: Option<&str>,
    environment: Option<&str>,
    format: Format,
    secrets: &dyn SecretStore,
) -> Result<Generated, String> {
    let mut warnings = Vec::new();
    let api = build(workspace, scope, environment, secrets)?;
    let text = match format {
        Format::OpenApiYaml => {
            crate::workspace::files::to_yaml(&openapi::spec(&api, &mut warnings))
                .map_err(|e| e.to_string())?
        }
        Format::OpenApiJson => {
            serde_json::to_string_pretty(&openapi::spec(&api, &mut warnings)).unwrap_or_default()
                + "\n"
        }
        Format::Markdown => markdown::render(&api),
        Format::Html => html::render(&api),
    };
    Ok(Generated { text, warnings })
}

/// Reads the requests in `scope` into the docs model.
pub fn build(
    workspace: &Workspace,
    scope: Option<&str>,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
) -> Result<Api, String> {
    let tree = workspace.tree().map_err(|e| e.to_string())?;
    let mut api = Api::default();
    let mut groups = Vec::new();
    match scope.filter(|s| !s.is_empty()) {
        None => {
            api.title = workspace.name().to_string();
            for collection in &tree {
                walk(workspace, collection, &[], &mut groups)?;
            }
            if tree.len() == 1 {
                api.description = workspace
                    .read_folder(&tree[0].path)
                    .map(|f| f.description)
                    .unwrap_or_default();
            }
        }
        Some(path) => {
            let node =
                find(&tree, path).ok_or_else(|| format!("{path} is not in the workspace"))?;
            api.title = node.name.clone();
            if node.kind == NodeKind::Request {
                let parents: Vec<String> = path
                    .split('/')
                    .collect::<Vec<_>>()
                    .split_last()
                    .map(|(_, p)| p.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default();
                let file = workspace.read_request(path).map_err(|e| e.to_string())?;
                let endpoint = endpoint(workspace, node, &file)?;
                api.description = file.description.clone();
                groups.push(Group {
                    name: parents.join(" / "),
                    description: String::new(),
                    endpoints: vec![endpoint],
                });
            } else {
                api.description = workspace
                    .read_folder(path)
                    .map(|f| f.description)
                    .unwrap_or_default();
                let parents: Vec<String> = path.split('/').map(String::from).collect();
                walk(workspace, node, &parents[..parents.len() - 1], &mut groups)?;
            }
        }
    }
    groups.retain(|g| !g.endpoints.is_empty());
    api.groups = groups;
    api.servers = servers(workspace, &api, scope, environment, secrets);
    Ok(api)
}

fn find<'a>(nodes: &'a [Node], path: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|n| {
        if n.path == path {
            Some(n)
        } else {
            find(&n.children, path)
        }
    })
}

fn walk(
    workspace: &Workspace,
    folder: &Node,
    parents: &[String],
    groups: &mut Vec<Group>,
) -> Result<(), String> {
    let mut names = parents.to_vec();
    names.push(folder.name.clone());
    let mut group = Group {
        name: names.join(" / "),
        description: workspace
            .read_folder(&folder.path)
            .map(|f| f.description)
            .unwrap_or_default(),
        endpoints: Vec::new(),
    };
    let mut subfolders = Vec::new();
    for child in &folder.children {
        if child.kind == NodeKind::Request {
            let file = workspace
                .read_request(&child.path)
                .map_err(|e| e.to_string())?;
            group.endpoints.push(endpoint(workspace, child, &file)?);
        } else {
            subfolders.push(child);
        }
    }
    groups.push(group);
    for child in subfolders {
        walk(workspace, child, &names, groups)?;
    }
    Ok(())
}

/// Header names whose values are credentials.
fn sensitive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "key",
        "token",
        "secret",
        "password",
        "signature",
    ]
    .iter()
    .any(|s| n.contains(s))
}

fn endpoint(workspace: &Workspace, node: &Node, file: &RequestFile) -> Result<Endpoint, String> {
    let kind = if file.is_websocket() {
        Kind::WebSocket
    } else if file.is_grpc() {
        Kind::Grpc
    } else {
        Kind::Http
    };
    let (_, path, path_params, query) = split_url(&file.url);
    // Auth set on the request, or else the nearest folder's.
    let auth = match &file.auth {
        Some(auth) => Some(auth.clone()),
        None => workspace
            .ancestors(&node.path)
            .map_err(|e| e.to_string())?
            .into_iter()
            .rev()
            .find_map(|a| a.folder.auth),
    };
    let headers = file
        .headers
        .iter()
        .filter(|h| h.enabled && !h.name.is_empty())
        .filter(|h| !h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| Param {
            name: h.name.clone(),
            example: if sensitive(&h.name) && !h.value.starts_with("{{") {
                String::new()
            } else {
                h.value.clone()
            },
        })
        .collect();
    let responses = file
        .examples
        .iter()
        .map(|e| ResponseDoc {
            status: e.status,
            name: e.name.clone(),
            body: (!e.body.is_empty())
                .then(|| payload(e.content_type().unwrap_or("text/plain"), &e.body)),
        })
        .collect();
    Ok(Endpoint {
        name: node.name.clone(),
        kind,
        method: match kind {
            Kind::WebSocket => "WS".into(),
            Kind::Grpc => "GRPC".into(),
            Kind::Http => file.method.to_uppercase(),
        },
        path,
        url: file.url.clone(),
        description: file.description.clone(),
        path_params,
        query,
        headers,
        body: request_body(file),
        auth: auth.map(|a| auth_doc(&a)),
        responses,
        grpc_method: file
            .grpc
            .as_ref()
            .map(|g| g.method.clone())
            .filter(|m| !m.is_empty()),
    })
}

fn request_body(file: &RequestFile) -> Option<Payload> {
    let content_type = file
        .headers
        .iter()
        .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.clone());
    let text =
        |ct: &str, content: &str| Some(payload(content_type.as_deref().unwrap_or(ct), content));
    match &file.body {
        Body::None => None,
        Body::Json { content } => text("application/json", content),
        Body::Xml { content } => text("application/xml", content),
        Body::Html { content } => text("text/html", content),
        Body::Text { content } => text("text/plain", content),
        Body::Javascript { content } => text("application/javascript", content),
        Body::Raw {
            content,
            content_type: ct,
        } => Some(payload(ct, content)),
        Body::Form { fields } => {
            let example = fields
                .iter()
                .filter(|f| f.enabled)
                .map(|f| format!("{}={}", f.name, f.value))
                .collect::<Vec<_>>()
                .join("&");
            let properties: Map<String, Value> = fields
                .iter()
                .filter(|f| f.enabled)
                .map(|f| (f.name.clone(), json!({ "type": "string" })))
                .collect();
            Some(Payload {
                content_type: "application/x-www-form-urlencoded".into(),
                example,
                schema: Some(json!({ "type": "object", "properties": properties })),
            })
        }
        Body::Multipart { parts } => {
            let properties: Map<String, Value> = parts
                .iter()
                .filter(|p| p.enabled)
                .map(|p| {
                    let schema = if p.file {
                        json!({ "type": "string", "format": "binary" })
                    } else {
                        json!({ "type": "string" })
                    };
                    (p.name.clone(), schema)
                })
                .collect();
            let example = parts
                .iter()
                .filter(|p| p.enabled)
                .map(|p| format!("{}: {}", p.name, if p.file { "(a file)" } else { &p.value }))
                .collect::<Vec<_>>()
                .join("\n");
            Some(Payload {
                content_type: "multipart/form-data".into(),
                example,
                schema: Some(json!({ "type": "object", "properties": properties })),
            })
        }
        Body::Graphql {
            query, variables, ..
        } => {
            let variables: Value =
                serde_json::from_str(&crate::jsonc::strip(variables)).unwrap_or(Value::Null);
            let example = json!({ "query": query, "variables": variables });
            Some(Payload {
                content_type: "application/json".into(),
                schema: Some(infer(&example)),
                example: serde_json::to_string_pretty(&example).unwrap_or_default(),
            })
        }
        Body::File {
            content_type: ct, ..
        } => Some(Payload {
            content_type: ct
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into()),
            example: "(a file)".into(),
            schema: Some(json!({ "type": "string", "format": "binary" })),
        }),
    }
}

fn payload(content_type: &str, content: &str) -> Payload {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    match serde_json::from_str::<Value>(&crate::jsonc::strip(content)) {
        Ok(value)
            if content_type.contains("json") || content.trim_start().starts_with(['{', '[']) =>
        {
            Payload {
                content_type: if content_type.is_empty() {
                    "application/json".into()
                } else {
                    content_type
                },
                example: serde_json::to_string_pretty(&value).unwrap_or_default(),
                schema: Some(infer(&value)),
            }
        }
        _ => Payload {
            content_type,
            example: content.to_string(),
            schema: None,
        },
    }
}

fn auth_doc(auth: &Auth) -> AuthDoc {
    match auth {
        Auth::None => AuthDoc::None,
        Auth::Bearer { .. } | Auth::RequestToken(_) => AuthDoc::Bearer,
        Auth::Basic { .. } => AuthDoc::Basic,
        Auth::ApiKey { key, location, .. } => AuthDoc::ApiKey {
            name: key.clone(),
            in_query: *location == crate::http::ApiKeyLocation::Query,
        },
        Auth::Digest(_) => AuthDoc::Digest,
        Auth::OAuth2(o) => {
            use crate::http::auth::OAuth2Grant;
            AuthDoc::OAuth2 {
                flow: match o.grant {
                    OAuth2Grant::AuthorizationCode => "authorizationCode",
                    OAuth2Grant::Implicit => "implicit",
                    OAuth2Grant::ClientCredentials => "clientCredentials",
                    OAuth2Grant::Password => "password",
                },
                auth_url: o.auth_url.clone(),
                token_url: o.access_token_url.clone(),
                scopes: o.scope.split_whitespace().map(String::from).collect(),
            }
        }
        Auth::Aws(_) => AuthDoc::Other("AWS Signature Version 4".into()),
        Auth::Jwt(_) => AuthDoc::Other("A signed JWT as a bearer token".into()),
        Auth::OAuth1(_) => AuthDoc::Other("OAuth 1.0".into()),
    }
}

/// Splits a request URL into its base (`https://host` or a leading `{{baseUrl}}`), its path in OpenAPI's form, the
/// path parameters, and the query parameters.
pub fn split_url(url: &str) -> (String, String, Vec<Param>, Vec<Param>) {
    let url = url.trim();
    let (without_query, query) = url.split_once('?').unwrap_or((url, ""));
    let (base, rest) = if let Some(after) = without_query.strip_prefix("{{") {
        match after.find("}}") {
            Some(end) => (
                format!("{{{{{}}}}}", &after[..end]),
                after[end + 2..].to_string(),
            ),
            None => (String::new(), without_query.to_string()),
        }
    } else if let Some((scheme, after)) = without_query.split_once("://") {
        match after.find('/') {
            Some(i) => (
                format!("{scheme}://{}", &after[..i]),
                after[i..].to_string(),
            ),
            None => (without_query.to_string(), String::new()),
        }
    } else {
        (String::new(), without_query.to_string())
    };
    let mut params = Vec::new();
    let segments: Vec<String> = rest
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            let name = segment
                .strip_prefix(':')
                .or_else(|| {
                    segment
                        .strip_prefix("{{")
                        .and_then(|s| s.strip_suffix("}}"))
                })
                .or_else(|| segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')));
            match name {
                Some(name) if !name.is_empty() => {
                    if !params.iter().any(|p: &Param| p.name == name) {
                        params.push(Param {
                            name: name.to_string(),
                            example: String::new(),
                        });
                    }
                    format!("{{{name}}}")
                }
                _ => segment.to_string(),
            }
        })
        .collect();
    let path = format!("/{}", segments.join("/"));
    let query = query
        .split('&')
        .filter(|q| !q.is_empty())
        .map(|q| {
            let (name, value) = q.split_once('=').unwrap_or((q, ""));
            Param {
                name: name.to_string(),
                example: value.to_string(),
            }
        })
        .collect();
    (base, path, params, query)
}

/// The base URLs used, each with its `{{variables}}` filled in where the environment or the collection gives a value
/// that is not secret.
fn servers(
    workspace: &Workspace,
    api: &Api,
    scope: Option<&str>,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
) -> Vec<String> {
    let mut bases: Vec<String> = Vec::new();
    for e in api
        .groups
        .iter()
        .flat_map(|g| &g.endpoints)
        .filter(|e| e.kind == Kind::Http)
    {
        let (base, ..) = split_url(&e.url);
        if !base.is_empty() && !bases.contains(&base) {
            bases.push(base);
        }
    }
    // Variables as a request in the scope sees them.
    let probe = scope
        .filter(|s| s.ends_with(".yaml"))
        .map(String::from)
        .or_else(|| first_request(&workspace.tree().unwrap_or_default(), scope));
    let vars = workspace
        .variables_in_scope(probe.as_deref(), environment, secrets)
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for base in bases {
        let mut resolved = base.clone();
        for v in vars.iter().filter(|v| !v.secret && !v.value.is_empty()) {
            resolved = resolved.replace(&format!("{{{{{}}}}}", v.name), &v.value);
        }
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

fn first_request(nodes: &[Node], scope: Option<&str>) -> Option<String> {
    for n in nodes {
        let inside = scope.is_none_or(|s| {
            n.path == s
                || n.path.starts_with(&format!("{s}/"))
                || s.starts_with(&format!("{}/", n.path))
        });
        if !inside {
            continue;
        }
        if n.kind == NodeKind::Request {
            return Some(n.path.clone());
        }
        if let Some(found) = first_request(&n.children, scope) {
            return Some(found);
        }
    }
    None
}

/// A JSON Schema describing `value`: types, object properties and array items, with common string formats.
pub fn infer(value: &Value) -> Value {
    match value {
        Value::Null => json!({ "type": "null" }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) => {
            json!({ "type": if n.is_i64() || n.is_u64() { "integer" } else { "number" } })
        }
        Value::String(s) => match string_format(s) {
            Some(format) => json!({ "type": "string", "format": format }),
            None => json!({ "type": "string" }),
        },
        Value::Array(items) => match items.first() {
            Some(first) => json!({ "type": "array", "items": infer(first) }),
            None => json!({ "type": "array" }),
        },
        Value::Object(map) => {
            let properties: Map<String, Value> =
                map.iter().map(|(k, v)| (k.clone(), infer(v))).collect();
            json!({ "type": "object", "properties": properties })
        }
    }
}

fn string_format(s: &str) -> Option<&'static str> {
    let bytes = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| {
        r.into_iter()
            .all(|i| bytes.get(i).is_some_and(u8::is_ascii_digit))
    };
    if s.len() >= 20 && digits(0..4) && bytes[4] == b'-' && s.as_bytes().get(10) == Some(&b'T') {
        Some("date-time")
    } else if s.len() == 10 && digits(0..4) && bytes[4] == b'-' && bytes[7] == b'-' {
        Some("date")
    } else if s.len() == 36
        && s.chars().filter(|c| *c == '-').count() == 4
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        Some("uuid")
    } else if s.contains('@')
        && !s.contains(' ')
        && s.split('@').nth(1).is_some_and(|d| d.contains('.'))
    {
        Some("email")
    } else if s.starts_with("http://") || s.starts_with("https://") {
        Some("uri")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_urls_into_base_path_and_parameters() {
        let (base, path, params, query) =
            split_url("{{baseUrl}}/users/:id/posts/{{postId}}?page=2&q=");
        assert_eq!(base, "{{baseUrl}}");
        assert_eq!(path, "/users/{id}/posts/{postId}");
        assert_eq!(
            params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["id", "postId"]
        );
        assert_eq!(query.len(), 2);
        assert_eq!(query[0].example, "2");
        let (base, path, ..) = split_url("https://api.example.com:8443/v1/health");
        assert_eq!(
            (base.as_str(), path.as_str()),
            ("https://api.example.com:8443", "/v1/health")
        );
        assert_eq!(split_url("https://example.com").1, "/");
    }

    #[test]
    fn infers_schemas_with_formats() {
        let schema = infer(
            &json!({ "id": 7, "price": 9.5, "email": "a@b.co", "at": "2026-10-22T10:00:00Z", "tags": ["x"], "x": null }),
        );
        assert_eq!(schema["properties"]["id"]["type"], "integer");
        assert_eq!(schema["properties"]["price"]["type"], "number");
        assert_eq!(schema["properties"]["email"]["format"], "email");
        assert_eq!(schema["properties"]["at"]["format"], "date-time");
        assert_eq!(schema["properties"]["tags"]["items"]["type"], "string");
        assert_eq!(schema["properties"]["x"]["type"], "null");
    }
}
