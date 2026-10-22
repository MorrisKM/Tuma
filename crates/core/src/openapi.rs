//! OpenAPI 3.0 / 3.1 and Swagger 2.0: import a spec as a collection, keep it in sync, find one in a project or on a
//! running server, and check responses against it (contract checks).
//!
//! A collection made from a spec remembers where the spec came from (`openapi:` in its `_folder.yaml`) and keeps a
//! copy of it (`_openapi.json`), which contract checks use. Each request remembers its operation (`operation: POST
//! /users/{id}`), which is how sync matches requests to operations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::expect::CheckResult;
use crate::http::{self, ApiKeyLocation};
use crate::import::Warnings;
use crate::workspace::files::{Auth, Body, FolderFile, KeyValue, Part, RequestFile, Variable};
use crate::workspace::{self, FolderTree, NodeKind, TreeItem, Workspace};

/// The name of a collection's copy of its spec.
pub const SNAPSHOT: &str = "_openapi.json";

/// Paths frameworks serve their spec at: FastAPI, Spring (springdoc), ASP.NET (Swashbuckle), NestJS, Express
/// (swagger-jsdoc) and others.
pub const WELL_KNOWN_PATHS: [&str; 8] = [
    "/openapi.json",
    "/v3/api-docs",
    "/swagger/v1/swagger.json",
    "/swagger.json",
    "/api-docs",
    "/api-docs-json",
    "/openapi.yaml",
    "/docs/openapi.json",
];

const METHODS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Swagger2,
    OpenApi30,
    OpenApi31,
}

/// A parsed spec.
#[derive(Debug, Clone)]
pub struct Spec {
    doc: Value,
    version: Version,
}

/// One operation, with path-level parameters merged in.
struct Operation<'a> {
    method: &'a str,
    path: &'a str,
    op: &'a Value,
    parameters: Vec<&'a Value>,
}

impl Spec {
    /// Parses a spec in JSON or YAML.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: Value = match serde_json::from_str(text) {
            Ok(doc) => doc,
            Err(_) => {
                serde_saphyr::from_str(text).map_err(|e| format!("neither JSON nor YAML: {e}"))?
            }
        };
        let version = if doc.get("swagger").is_some() {
            Version::Swagger2
        } else {
            match doc.get("openapi").and_then(Value::as_str) {
                Some(v) if v.starts_with("3.0") => Version::OpenApi30,
                Some(v) if v.starts_with('3') => Version::OpenApi31,
                _ => {
                    return Err(
                        "not an OpenAPI or Swagger document (no openapi: or swagger: field)".into(),
                    );
                }
            }
        };
        if !doc.get("paths").is_some_and(Value::is_object) {
            return Err("the spec has no paths".into());
        }
        Ok(Self { doc, version })
    }

    pub fn title(&self) -> String {
        self.doc
            .pointer("/info/title")
            .and_then(Value::as_str)
            .unwrap_or("API")
            .to_string()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.doc).unwrap_or_default()
    }

    /// Follows `$ref`s within the document.
    fn resolve<'a>(&'a self, mut value: &'a Value) -> &'a Value {
        for _ in 0..16 {
            match value
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix('#'))
            {
                Some(pointer) => match self.doc.pointer(pointer) {
                    Some(target) => value = target,
                    None => break,
                },
                None => break,
            }
        }
        value
    }

    fn operations(&self) -> Vec<Operation<'_>> {
        let mut out = Vec::new();
        let Some(paths) = self.doc.get("paths").and_then(Value::as_object) else {
            return out;
        };
        for (path, item) in paths {
            let item = self.resolve(item);
            let shared = array(item.get("parameters"));
            for method in METHODS {
                let Some(op) = item.get(method) else { continue };
                let own = array(op.get("parameters"));
                // Operation parameters override path-level ones with the same name and location.
                let mut parameters: Vec<&Value> = own.iter().map(|p| self.resolve(p)).collect();
                for p in shared.iter().map(|p| self.resolve(p)) {
                    if !parameters
                        .iter()
                        .any(|o| o.get("name") == p.get("name") && o.get("in") == p.get("in"))
                    {
                        parameters.push(p);
                    }
                }
                out.push(Operation {
                    method,
                    path,
                    op,
                    parameters,
                });
            }
        }
        out
    }

    /// The base URL from `servers` (or Swagger's host and basePath). Relative URLs resolve against `source`.
    fn base_url(&self, source: Option<&str>) -> Option<String> {
        let url = match self.version {
            Version::Swagger2 => {
                let host = self.doc.get("host").and_then(Value::as_str)?;
                let scheme = array(self.doc.get("schemes"))
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("https");
                let base = self
                    .doc
                    .get("basePath")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                format!("{scheme}://{host}{base}")
            }
            _ => {
                // Without servers, the API is where the spec was served from (resolved below), as with FastAPI.
                let Some(server) = array(self.doc.get("servers")).first() else {
                    return source
                        .filter(|s| s.contains("://"))
                        .and_then(|s| url::Url::parse(s).ok())
                        .map(|u| u.origin().ascii_serialization());
                };
                let mut url = server.get("url").and_then(Value::as_str)?.to_string();
                // Server variables take their defaults.
                if let Some(vars) = server.get("variables").and_then(Value::as_object) {
                    for (name, var) in vars {
                        let default = var.get("default").map(scalar).unwrap_or_default();
                        url = url.replace(&format!("{{{name}}}"), &default);
                    }
                }
                url
            }
        };
        let url = url.trim_end_matches('/').to_string();
        if url.contains("://") {
            return Some(url);
        }
        // A relative server URL such as /api is relative to where the spec was found.
        let source = source.filter(|s| s.contains("://"))?;
        let parsed = url::Url::parse(source).ok()?;
        Some(
            parsed
                .join(if url.is_empty() { "/" } else { &url })
                .ok()?
                .as_str()
                .trim_end_matches('/')
                .to_string(),
        )
    }

    /// Builds a collection: a folder per tag, a request per operation, auth from the security schemes and a
    /// `baseUrl` variable from the servers.
    pub fn to_tree(&self, source: Option<&str>, warnings: &mut Warnings) -> FolderTree {
        let mut folders: BTreeMap<String, FolderTree> = BTreeMap::new();
        let mut root_items = Vec::new();
        let global_security = self.doc.get("security");
        let collection_auth = global_security.and_then(|s| self.auth_for(s, warnings));

        for operation in self.operations() {
            let (name, file) = self.request(&operation, collection_auth.as_ref(), warnings);
            let item = TreeItem::Request { name, file };
            match array(operation.op.get("tags"))
                .first()
                .and_then(Value::as_str)
            {
                Some(tag) => folders
                    .entry(tag.to_string())
                    .or_insert_with(|| FolderTree {
                        name: tag.to_string(),
                        ..Default::default()
                    })
                    .items
                    .push(item),
                None => root_items.push(item),
            }
        }

        // Folders in the order the spec lists its tags, then the rest alphabetically.
        let mut ordered = Vec::new();
        for tag in array(self.doc.get("tags")) {
            let name = tag.get("name").and_then(Value::as_str).unwrap_or_default();
            if let Some(mut folder) = folders.remove(name) {
                folder.folder.description = tag
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into();
                ordered.push(TreeItem::Folder(folder));
            }
        }
        ordered.extend(folders.into_values().map(TreeItem::Folder));
        ordered.extend(root_items);

        let mut variables = Vec::new();
        match self.base_url(source) {
            Some(url) => variables.push(Variable::new("baseUrl", url)),
            None => {
                warnings.add(
                    "the spec has no server URL; set the baseUrl variable to where the API runs"
                        .into(),
                );
                variables.push(Variable::new("baseUrl", "http://localhost:3000"));
            }
        }
        FolderTree {
            name: self.title(),
            folder: FolderFile {
                description: self
                    .doc
                    .pointer("/info/description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                auth: collection_auth,
                variables,
                openapi: source.map(String::from),
                ..FolderFile::default()
            },
            items: ordered,
        }
    }

    fn request(
        &self,
        o: &Operation<'_>,
        collection_auth: Option<&Auth>,
        warnings: &mut Warnings,
    ) -> (String, RequestFile) {
        let method = o.method.to_uppercase();
        let name = [o.op.get("summary"), o.op.get("operationId")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|s| !s.trim().is_empty())
            .map(|s| s.trim().trim_end_matches('.').to_string())
            .unwrap_or_else(|| format!("{method} {}", o.path));

        let mut url = format!("{{{{baseUrl}}}}{}", o.path);
        let mut query = Vec::new();
        let mut headers = Vec::new();
        let mut form = Vec::new();
        let mut body_schema = None;
        for p in &o.parameters {
            let pname = p.get("name").and_then(Value::as_str).unwrap_or_default();
            let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
            let example = self.parameter_example(p);
            match p.get("in").and_then(Value::as_str) {
                Some("path") => {
                    let value = example.unwrap_or_else(|| format!("{{{{{pname}}}}}"));
                    url = url.replace(&format!("{{{pname}}}"), &value);
                }
                Some("query") if required => {
                    query.push(format!("{pname}={}", example.unwrap_or_default()))
                }
                Some("header") if required => headers.push(KeyValue {
                    name: pname.into(),
                    value: example.unwrap_or_default(),
                    enabled: true,
                }),
                // Swagger 2 bodies and forms are parameters.
                Some("body") => body_schema = p.get("schema"),
                Some("formData") => form.push((
                    pname.to_string(),
                    example.unwrap_or_default(),
                    p.get("type") == Some(&json!("file")),
                )),
                _ => {}
            }
        }
        if !query.is_empty() {
            url = format!("{url}?{}", query.join("&"));
        }

        let body = match self.version {
            Version::Swagger2 if !form.is_empty() => {
                let consumes = array(o.op.get("consumes").or(self.doc.get("consumes")));
                if consumes
                    .iter()
                    .any(|c| c.as_str() == Some("multipart/form-data"))
                    || form.iter().any(|f| f.2)
                {
                    Body::Multipart {
                        parts: form
                            .into_iter()
                            .map(|(name, value, file)| Part {
                                name,
                                value,
                                file,
                                ..Default::default()
                            })
                            .collect(),
                    }
                } else {
                    Body::Form {
                        fields: form
                            .into_iter()
                            .map(|(name, value, _)| KeyValue {
                                name,
                                value,
                                enabled: true,
                            })
                            .collect(),
                    }
                }
            }
            Version::Swagger2 => body_schema.map(|s| self.json_body(s)).unwrap_or_default(),
            _ => {
                o.op.get("requestBody")
                    .map(|b| self.body(self.resolve(b)))
                    .unwrap_or_default()
            }
        };

        // Auth: the operation's own security when it differs from the spec's.
        let auth = match o.op.get("security") {
            Some(Value::Array(list)) if list.is_empty() => Some(Auth::None),
            Some(security) => {
                let auth = self.auth_for(security, warnings);
                if auth.as_ref() == collection_auth {
                    None
                } else {
                    auth
                }
            }
            None => None,
        };

        let description = [o.op.get("description"), o.op.get("summary")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|s| !s.trim().is_empty() && *s != name)
            .unwrap_or_default()
            .to_string();
        let file = RequestFile {
            method,
            url,
            description,
            operation: Some(operation_key(o.method, o.path)),
            headers,
            body,
            auth,
            ..RequestFile::default()
        };
        (name, file)
    }

    fn parameter_example(&self, p: &Value) -> Option<String> {
        let value = p
            .get("example")
            .or_else(|| {
                p.get("examples")
                    .and_then(Value::as_object)
                    .and_then(|e| e.values().next())
                    .map(|e| self.resolve(e).get("value").unwrap_or(e))
            })
            .cloned()
            .or_else(|| {
                p.get("schema")
                    .and_then(|s| self.declared_example(self.resolve(s)))
            })
            .or_else(|| p.get("default").cloned())?;
        Some(scalar(&value))
    }

    /// An example the schema states (example, examples, default, enum, const), without inventing one.
    fn declared_example(&self, schema: &Value) -> Option<Value> {
        schema
            .get("example")
            .cloned()
            .or_else(|| array(schema.get("examples")).first().cloned())
            .or_else(|| schema.get("default").cloned())
            .or_else(|| array(schema.get("enum")).first().cloned())
            .or_else(|| schema.get("const").cloned())
    }

    fn body(&self, body: &Value) -> Body {
        let Some(content) = body.get("content").and_then(Value::as_object) else {
            return Body::None;
        };
        let pick = |pred: &dyn Fn(&str) -> bool| content.iter().find(|(ct, _)| pred(ct.as_str()));
        if let Some((_, media)) = pick(&|ct| ct == "application/json" || ct.ends_with("+json")) {
            return match media.get("example").or_else(|| {
                media
                    .get("examples")
                    .and_then(Value::as_object)
                    .and_then(|e| e.values().next())
                    .map(|e| self.resolve(e).get("value").unwrap_or(e))
            }) {
                Some(example) => Body::Json {
                    content: pretty(example),
                },
                None => media
                    .get("schema")
                    .map(|s| self.json_body(s))
                    .unwrap_or(Body::Json {
                        content: "{}".into(),
                    }),
            };
        }
        if let Some((ct, media)) =
            pick(&|ct| ct == "application/x-www-form-urlencoded" || ct == "multipart/form-data")
        {
            let schema = media
                .get("schema")
                .map(|s| self.resolve(s))
                .unwrap_or(&Value::Null);
            let props = self.properties(schema);
            if ct == "multipart/form-data" {
                return Body::Multipart {
                    parts: props
                        .into_iter()
                        .map(|(name, s)| {
                            let file = s.get("format").and_then(Value::as_str) == Some("binary")
                                || s.get("contentMediaType").is_some();
                            let value = if file {
                                String::new()
                            } else {
                                scalar(&self.example(s, 0))
                            };
                            Part {
                                name,
                                value,
                                file,
                                ..Default::default()
                            }
                        })
                        .collect(),
                };
            }
            return Body::Form {
                fields: props
                    .into_iter()
                    .map(|(name, s)| KeyValue {
                        name,
                        value: scalar(&self.example(s, 0)),
                        enabled: true,
                    })
                    .collect(),
            };
        }
        match content.iter().next() {
            Some((ct, media)) => {
                let example = media.get("example").map(scalar).unwrap_or_default();
                Body::text(example, ct)
            }
            None => Body::None,
        }
    }

    fn json_body(&self, schema: &Value) -> Body {
        Body::Json {
            content: pretty(&self.example(schema, 0)),
        }
    }

    /// An object schema's properties, including those from `allOf`.
    fn properties<'a>(&'a self, schema: &'a Value) -> Vec<(String, &'a Value)> {
        let schema = self.resolve(schema);
        let mut out: Vec<(String, &Value)> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| {
                p.iter()
                    .map(|(k, v)| (k.clone(), self.resolve(v)))
                    .collect()
            })
            .unwrap_or_default();
        for part in array(schema.get("allOf")) {
            out.extend(self.properties(part));
        }
        out
    }

    /// An example value for a schema: the one it declares, or one made up from its type and format.
    pub fn example(&self, schema: &Value, depth: usize) -> Value {
        let schema = self.resolve(schema);
        if let Some(example) = self.declared_example(schema) {
            return example;
        }
        if depth > 6 {
            return Value::Null;
        }
        if let Some(first) = array(schema.get("oneOf"))
            .first()
            .or_else(|| array(schema.get("anyOf")).first())
        {
            return self.example(first, depth + 1);
        }
        let props = self.properties(schema);
        let kind = match schema.get("type") {
            Some(Value::String(t)) => t.as_str(),
            // OpenAPI 3.1 types can be lists, such as ["string", "null"].
            Some(Value::Array(types)) => types
                .iter()
                .filter_map(Value::as_str)
                .find(|t| *t != "null")
                .unwrap_or("null"),
            _ if !props.is_empty() => "object",
            _ if schema.get("items").is_some() => "array",
            _ => "",
        };
        match kind {
            "object" => {
                let mut map = Map::new();
                for (name, prop) in props {
                    // Read-only fields are set by the server, not sent.
                    if prop.get("readOnly") == Some(&Value::Bool(true)) {
                        continue;
                    }
                    map.insert(name, self.example(prop, depth + 1));
                }
                Value::Object(map)
            }
            "array" => {
                json!([self.example(schema.get("items").unwrap_or(&Value::Null), depth + 1)])
            }
            "integer" => json!(schema.get("minimum").and_then(Value::as_i64).unwrap_or(1)),
            "number" => json!(schema.get("minimum").and_then(Value::as_f64).unwrap_or(1.5)),
            "boolean" => json!(true),
            "string" => json!(match schema.get("format").and_then(Value::as_str) {
                Some("date-time") => "2026-01-01T12:00:00Z",
                Some("date") => "2026-01-01",
                Some("time") => "12:00:00",
                Some("email") => "user@example.com",
                Some("uuid") => "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                Some("uri" | "url") => "https://example.com",
                Some("hostname") => "example.com",
                Some("ipv4") => "192.0.2.1",
                Some("ipv6") => "2001:db8::1",
                Some("byte") => "dGV4dA==",
                Some("password") => "secret",
                _ => "string",
            }),
            _ => Value::Null,
        }
    }

    /// Turns a security requirement into auth with `{{variables}}` for the credentials.
    fn auth_for(&self, security: &Value, warnings: &mut Warnings) -> Option<Auth> {
        let requirement = array(Some(security)).first()?.as_object()?;
        let name = requirement.keys().next()?;
        let schemes = match self.version {
            Version::Swagger2 => self.doc.get("securityDefinitions"),
            _ => self.doc.pointer("/components/securitySchemes"),
        };
        let scheme = self.resolve(schemes?.get(name)?);
        let kind = scheme
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let http_scheme = scheme
            .get("scheme")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        Some(match (kind, http_scheme.as_str()) {
            ("http", "bearer") => Auth::Bearer {
                token: "{{token}}".into(),
            },
            ("http", "basic") | ("basic", _) => Auth::Basic {
                username: "{{username}}".into(),
                password: "{{password}}".into(),
            },
            ("apiKey", _) => Auth::ApiKey {
                key: scheme
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("X-Api-Key")
                    .into(),
                value: "{{apiKey}}".into(),
                location: if scheme.get("in").and_then(Value::as_str) == Some("query") {
                    ApiKeyLocation::Query
                } else {
                    ApiKeyLocation::Header
                },
            },
            ("oauth2" | "openIdConnect", _) => {
                warnings.add(format!("\"{name}\" uses {kind}; set up as a bearer token in {{{{token}}}} until OAuth arrives"));
                Auth::Bearer {
                    token: "{{token}}".into(),
                }
            }
            _ => {
                warnings.add(format!(
                    "security scheme \"{name}\" ({kind}) is not supported"
                ));
                return None;
            }
        })
    }

    /// The operation a request matches, by method and URL path. Templates such as `/users/{id}` match any segment,
    /// and the URL may have a prefix (a base path such as `/api/v1`).
    pub fn find_operation(&self, method: &str, url_path: &str) -> Option<String> {
        let method = method.to_ascii_lowercase();
        let actual: Vec<&str> = url_path.split('/').filter(|s| !s.is_empty()).collect();
        self.operations()
            .into_iter()
            .filter(|o| o.method == method)
            .filter(|o| {
                let template: Vec<&str> = o.path.split('/').filter(|s| !s.is_empty()).collect();
                template.len() <= actual.len()
                    && template
                        .iter()
                        .zip(&actual[actual.len() - template.len()..])
                        .all(|(t, a)| (t.starts_with('{') && t.ends_with('}')) || t == a)
            })
            // The most specific match: the one with the fewest template segments.
            .min_by_key(|o| o.path.matches('{').count())
            .map(|o| operation_key(o.method, o.path))
    }

    /// Checks a response body against the schema the spec gives for this operation and status.
    pub fn check_response(
        &self,
        operation: &str,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) -> CheckResult {
        let check = format!("response matches the spec for {operation} ({status})");
        let fail = |expected: &str, actual: String| CheckResult {
            check: check.clone(),
            passed: false,
            expected: expected.to_string(),
            actual,
        };
        let Some(o) = self
            .operations()
            .into_iter()
            .find(|o| operation_key(o.method, o.path) == operation)
        else {
            return fail(
                "the operation in the spec",
                format!("{operation} is not in the spec"),
            );
        };
        let responses = self.resolve(o.op.get("responses").unwrap_or(&Value::Null));
        let class = format!("{}XX", status / 100);
        let Some(response) = responses
            .get(status.to_string())
            .or_else(|| responses.get(&class))
            .or_else(|| responses.get(class.to_lowercase()))
            .or_else(|| responses.get("default"))
            .map(|r| self.resolve(r))
        else {
            let documented: Vec<&String> = responses
                .as_object()
                .map(|m| m.keys().collect())
                .unwrap_or_default();
            return fail(
                &format!("a documented status ({documented:?})"),
                status.to_string(),
            );
        };

        let schema = match self.version {
            Version::Swagger2 => response.get("schema"),
            _ => {
                let content = response.get("content").and_then(Value::as_object);
                let media_type = content_type.split(';').next().unwrap_or_default().trim();
                content.and_then(|c| {
                    c.get(media_type)
                        .or_else(|| {
                            c.iter()
                                .find(|(ct, _)| {
                                    ct.ends_with("json") && media_type.ends_with("json")
                                })
                                .map(|(_, m)| m)
                        })
                        .or_else(|| c.get("*/*"))
                        .and_then(|m| m.get("schema"))
                })
            }
        };
        let Some(schema) = schema else {
            // Nothing to check the body against: the status alone is documented.
            return CheckResult {
                check,
                passed: true,
                expected: "a documented status".into(),
                actual: status.to_string(),
            };
        };
        let Ok(instance) = serde_json::from_slice::<Value>(body) else {
            return fail("a JSON body", "not JSON".into());
        };
        match self.validate(schema, &instance) {
            Ok(()) => CheckResult {
                check,
                passed: true,
                expected: "matches the schema".into(),
                actual: "matches".into(),
            },
            Err(problems) => fail("matches the schema", problems),
        }
    }

    /// Validates against a schema from this spec. Its `$ref`s point into the spec, so the spec's definitions travel
    /// with it.
    fn validate(&self, schema: &Value, instance: &Value) -> Result<(), String> {
        let mut root = schema.clone();
        if let Value::Object(map) = &mut root {
            for key in ["components", "definitions"] {
                if let Some(defs) = self.doc.get(key) {
                    map.insert(key.into(), defs.clone());
                }
            }
        }
        let draft = match self.version {
            Version::OpenApi31 => jsonschema::Draft::Draft202012,
            // OpenAPI 3.0 and Swagger schemas are close to draft 4, except for `nullable`.
            _ => {
                root = nullable_to_type(root);
                jsonschema::Draft::Draft4
            }
        };
        let validator = jsonschema::options()
            .with_draft(draft)
            .build(&root)
            .map_err(|e| format!("invalid schema: {e}"))?;
        let problems: Vec<String> = validator
            .iter_errors(instance)
            .take(5)
            .map(|e| {
                let at = e.instance_path().to_string();
                if at.is_empty() {
                    e.to_string()
                } else {
                    format!("{at}: {e}")
                }
            })
            .collect();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }
}

/// `POST /users/{id}`: how a request names its operation.
pub fn operation_key(method: &str, path: &str) -> String {
    format!("{} {path}", method.to_uppercase())
}

/// OpenAPI 3.0's `nullable: true` as JSON Schema: the type plus `null`.
fn nullable_to_type(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            if map.remove("nullable") == Some(Value::Bool(true)) {
                match map.remove("type") {
                    Some(Value::String(t)) => {
                        map.insert("type".into(), json!([t, "null"]));
                    }
                    Some(other) => {
                        map.insert("type".into(), other);
                    }
                    None => {}
                }
                if let Some(Value::Array(values)) = map.get_mut("enum") {
                    values.push(Value::Null);
                }
            }
            Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, nullable_to_type(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(nullable_to_type).collect()),
        other => other,
    }
}

fn array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

// Loading, discovery, import and sync

/// Reads a spec from a URL, or a file (relative paths are relative to `base`). Returns its text.
pub async fn load(source: &str, base: &Path) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let request = http::Request {
            method: "GET".into(),
            url: source.into(),
            ..Default::default()
        };
        let response = http::send(&request).await.map_err(|e| e.to_string())?;
        if response.status >= 400 {
            return Err(format!(
                "{source} answered {} {}",
                response.status, response.reason
            ));
        }
        return String::from_utf8(response.body.to_vec())
            .map_err(|_| format!("{source} is not text"));
    }
    let path = base.join(source);
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Looks for a spec on a running server at the paths frameworks use. Returns the URL that answered with one.
pub async fn discover_on_server(base_url: &str) -> Option<(String, Spec)> {
    let base = base_url.trim_end_matches('/');
    for path in WELL_KNOWN_PATHS {
        let url = format!("{base}{path}");
        let request = http::Request {
            method: "GET".into(),
            url: url.clone(),
            settings: http::Settings {
                timeout_ms: Some(5000),
                ..Default::default()
            },
            ..Default::default()
        };
        if let Ok(response) = http::send(&request).await
            && response.status < 400
            && let Ok(spec) = Spec::parse(&String::from_utf8_lossy(&response.body))
        {
            return Some((url, spec));
        }
    }
    None
}

/// Spec files in a project: openapi.* and swagger.* (JSON or YAML), skipping dependency and build folders.
pub fn discover_files(root: &Path) -> Vec<PathBuf> {
    const SKIP: [&str; 8] = [
        "node_modules",
        "target",
        ".git",
        "dist",
        "build",
        "vendor",
        ".venv",
        "venv",
    ];
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if path.is_dir() {
                if depth < 4 && !SKIP.contains(&name.as_str()) && !name.starts_with('.') {
                    walk(&path, depth + 1, out);
                }
            } else if (name.starts_with("openapi") || name.starts_with("swagger"))
                && [".json", ".yaml", ".yml"]
                    .iter()
                    .any(|ext| name.ends_with(ext))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort();
    out
}

/// A spec location as stored in the collection: URLs as they are, files relative to the workspace folder so the
/// workspace can be cloned elsewhere.
pub fn source_for(source: &str, cwd: &Path, workspace_root: &Path) -> String {
    if source.contains("://") {
        return source.to_string();
    }
    let absolute = cwd.join(source);
    let absolute = absolute.canonicalize().unwrap_or(absolute);
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    relative_path(&absolute, &root).unwrap_or_else(|| absolute.to_string_lossy().into_owned())
}

/// `path` relative to `base`, using `..` where needed, with `/` separators.
fn relative_path(path: &Path, base: &Path) -> Option<String> {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let ups = std::iter::repeat_n("..".to_string(), base.len() - common);
    let downs = path[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    Some(ups.chain(downs).collect::<Vec<_>>().join("/"))
}

/// Imports a spec as a new collection, keeping a copy of it for contract checks and sync. `source` is recorded so
/// the collection can be synced later.
pub fn import(
    workspace: &Workspace,
    spec: &Spec,
    source: Option<&str>,
) -> Result<(String, Warnings), workspace::Error> {
    let mut warnings = Warnings::default();
    let tree = spec.to_tree(source, &mut warnings);
    let path = workspace.add_collection(&tree)?;
    workspace.write_file(&format!("{path}/{SNAPSHOT}"), &spec.to_json())?;
    Ok((path, warnings))
}

/// What a sync changed.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SyncReport {
    /// New requests, for operations that were not in the collection.
    pub added: Vec<String>,
    /// Requests whose method, URL or description changed with the spec.
    pub updated: Vec<String>,
    /// Requests whose operation is no longer in the spec. They are kept, not deleted.
    pub removed: Vec<String>,
}

/// Brings a collection made from a spec up to date with a new version of it. Requests keep their body, headers, auth,
/// checks and scripts; only the method, URL and description follow the spec.
pub fn sync(
    workspace: &Workspace,
    collection: &str,
    spec: &Spec,
) -> Result<SyncReport, workspace::Error> {
    let settings = workspace.read_folder(collection)?;
    let mut warnings = Warnings::default();
    let tree = spec.to_tree(settings.openapi.as_deref(), &mut warnings);

    // The collection's requests, by operation.
    let mut existing: BTreeMap<String, String> = BTreeMap::new();
    for path in crate::runner::requests_under(workspace, collection)? {
        if let Some(op) = workspace.read_request(&path)?.operation {
            existing.insert(op, path);
        }
    }

    let mut report = SyncReport::default();
    let mut seen = Vec::new();
    fn flatten(items: &[TreeItem], folder: &str, out: &mut Vec<(String, String, RequestFile)>) {
        for item in items {
            match item {
                TreeItem::Folder(f) => flatten(
                    &f.items,
                    &format!("{folder}/{}", workspace::file_name(&f.name)),
                    out,
                ),
                TreeItem::Request { name, file } => {
                    out.push((folder.to_string(), name.clone(), file.clone()))
                }
            }
        }
    }
    let mut wanted = Vec::new();
    flatten(&tree.items, collection, &mut wanted);
    for (folder, name, file) in wanted {
        let Some(op) = file.operation.clone() else {
            continue;
        };
        seen.push(op.clone());
        match existing.get(&op) {
            Some(path) => {
                let mut current = workspace.read_request(path)?;
                if current.method != file.method
                    || current.url != file.url
                    || current.description != file.description
                {
                    current.method = file.method;
                    current.url = file.url;
                    current.description = file.description;
                    workspace.write_request(path, &current)?;
                    report.updated.push(path.clone());
                }
            }
            None => {
                let folder = workspace.ensure_folder(&folder)?;
                report
                    .added
                    .push(workspace.create_request(&folder, &name, &file)?);
            }
        }
    }
    report.removed = existing
        .into_iter()
        .filter(|(op, _)| !seen.contains(op))
        .map(|(_, path)| path)
        .collect();
    workspace.write_file(&format!("{collection}/{SNAPSHOT}"), &spec.to_json())?;
    Ok(report)
}

/// The spec copy kept with the collection a request belongs to, if that collection came from a spec.
pub fn snapshot_for(workspace: &Workspace, request_path: &str) -> Option<Spec> {
    let collection = request_path.split('/').next()?;
    let text = workspace
        .read_file(&format!("{collection}/{SNAPSHOT}"))
        .ok()?;
    Spec::parse(&text).ok()
}

/// Every spec copy in the workspace, for requests that are not saved.
pub fn snapshots(workspace: &Workspace) -> Vec<Spec> {
    workspace
        .tree()
        .unwrap_or_default()
        .iter()
        .filter(|n| n.kind == NodeKind::Collection)
        .filter_map(|n| workspace.read_file(&format!("{}/{SNAPSHOT}", n.path)).ok())
        .filter_map(|text| Spec::parse(&text).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PETSTORE: &str = r##"
openapi: 3.0.3
info:
  title: Pet Store
servers:
  - url: http://localhost:{port}/v1
    variables:
      port:
        default: "8080"
tags:
  - name: pets
    description: Everything about pets
security:
  - bearer: []
paths:
  /pets:
    get:
      tags: [pets]
      summary: List pets
      parameters:
        - { name: limit, in: query, required: true, schema: { type: integer, example: 10 } }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: array, items: { $ref: "#/components/schemas/Pet" } }
    post:
      tags: [pets]
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema: { $ref: "#/components/schemas/NewPet" }
      responses:
        "201":
          description: created
          content:
            application/json:
              schema: { $ref: "#/components/schemas/Pet" }
  /pets/{petId}:
    parameters:
      - { name: petId, in: path, required: true, schema: { type: integer } }
    delete:
      tags: [pets]
      summary: Delete a pet.
      security: []
      responses:
        "204": { description: gone }
  /health:
    get:
      responses:
        "200": { description: ok }
components:
  securitySchemes:
    bearer: { type: http, scheme: bearer }
  schemas:
    NewPet:
      type: object
      required: [name]
      properties:
        name: { type: string, example: Rex }
        birthday: { type: string, format: date }
        tag: { type: string, nullable: true }
    Pet:
      allOf:
        - $ref: "#/components/schemas/NewPet"
        - type: object
          required: [id]
          properties:
            id: { type: integer, readOnly: true }
"##;

    fn requests(tree: &FolderTree) -> Vec<(String, RequestFile)> {
        let mut out = Vec::new();
        for item in &tree.items {
            match item {
                TreeItem::Folder(f) => out.extend(requests(f)),
                TreeItem::Request { name, file } => out.push((name.clone(), file.clone())),
            }
        }
        out
    }

    #[test]
    fn builds_a_collection() {
        let spec = Spec::parse(PETSTORE).unwrap();
        let mut warnings = Warnings::default();
        let tree = spec.to_tree(Some("openapi.yaml"), &mut warnings);
        assert!(warnings.0.is_empty(), "{warnings:?}");
        assert_eq!(tree.name, "Pet Store");
        assert_eq!(
            tree.folder.variables[0],
            Variable::new("baseUrl", "http://localhost:8080/v1")
        );
        assert_eq!(
            tree.folder.auth,
            Some(Auth::Bearer {
                token: "{{token}}".into()
            })
        );
        assert_eq!(tree.folder.openapi.as_deref(), Some("openapi.yaml"));

        let TreeItem::Folder(pets) = &tree.items[0] else {
            panic!()
        };
        assert_eq!(pets.folder.description, "Everything about pets");
        let all = requests(&tree);
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["List pets", "createPet", "Delete a pet", "GET /health"]
        );

        let (_, list) = &all[0];
        assert_eq!(list.url, "{{baseUrl}}/pets?limit=10");
        assert_eq!(list.operation.as_deref(), Some("GET /pets"));
        let (_, create) = &all[1];
        // Read-only fields are left out of request bodies.
        assert_eq!(
            create.body,
            Body::Json {
                content: pretty(&json!({"name": "Rex", "birthday": "2026-01-01", "tag": "string"}))
            }
        );
        let (_, delete) = &all[2];
        assert_eq!(delete.url, "{{baseUrl}}/pets/{{petId}}");
        assert_eq!(delete.auth, Some(Auth::None));
    }

    #[test]
    fn contract_checks() {
        let spec = Spec::parse(PETSTORE).unwrap();
        let check = |status, body: &str| {
            spec.check_response("POST /pets", status, "application/json", body.as_bytes())
        };

        assert!(check(201, r#"{"id": 1, "name": "Rex", "tag": null}"#).passed);
        let missing_id = check(201, r#"{"name": "Rex"}"#);
        assert!(!missing_id.passed);
        assert!(missing_id.actual.contains("id"), "{}", missing_id.actual);
        let wrong_type = check(201, r#"{"id": "one", "name": "Rex"}"#);
        assert!(wrong_type.actual.contains("/id"), "{}", wrong_type.actual);
        assert!(!check(500, "{}").passed, "500 is not documented");
    }

    #[test]
    fn finds_operations_by_url() {
        let spec = Spec::parse(PETSTORE).unwrap();
        assert_eq!(
            spec.find_operation("DELETE", "/v1/pets/42").as_deref(),
            Some("DELETE /pets/{petId}")
        );
        assert_eq!(
            spec.find_operation("GET", "/v1/pets").as_deref(),
            Some("GET /pets")
        );
        assert_eq!(spec.find_operation("PUT", "/v1/pets"), None);
    }

    #[test]
    fn swagger_2() {
        let spec = Spec::parse(
            r#"{"swagger": "2.0", "info": {"title": "Old"}, "host": "api.example.com", "basePath": "/v2", "schemes": ["https"],
                "paths": {"/login": {"post": {"consumes": ["application/x-www-form-urlencoded"],
                  "parameters": [{"name": "user", "in": "formData", "type": "string"}],
                  "responses": {"200": {"description": "ok", "schema": {"type": "object", "required": ["token"],
                    "properties": {"token": {"type": "string"}}}}}}}}}"#,
        )
        .unwrap();
        let tree = spec.to_tree(None, &mut Warnings::default());
        assert_eq!(tree.folder.variables[0].value, "https://api.example.com/v2");
        let all = requests(&tree);
        assert!(matches!(&all[0].1.body, Body::Form { fields } if fields[0].name == "user"));
        assert!(
            spec.check_response("POST /login", 200, "application/json", br#"{"token": "t"}"#)
                .passed
        );
        assert!(
            !spec
                .check_response("POST /login", 200, "application/json", b"{}")
                .passed
        );
    }

    #[test]
    fn no_servers_means_where_the_spec_came_from() {
        let spec =
            Spec::parse(r#"{"openapi": "3.1.0", "info": {"title": "T"}, "paths": {}}"#).unwrap();
        let tree = spec.to_tree(
            Some("http://localhost:8000/openapi.json"),
            &mut Warnings::default(),
        );
        assert_eq!(tree.folder.variables[0].value, "http://localhost:8000");
    }

    #[test]
    fn relative_servers_use_the_spec_url() {
        let spec = Spec::parse(r#"{"openapi": "3.1.0", "info": {"title": "T"}, "servers": [{"url": "/api"}], "paths": {}}"#).unwrap();
        let tree = spec.to_tree(
            Some("http://localhost:8000/openapi.json"),
            &mut Warnings::default(),
        );
        assert_eq!(tree.folder.variables[0].value, "http://localhost:8000/api");
    }
}
