//! The docs model as an OpenAPI 3.1 document.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{Api, AuthDoc, Endpoint, Kind, Payload};

pub fn spec(api: &Api, warnings: &mut Vec<String>) -> Value {
    let mut paths: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut schemes: Map<String, Value> = Map::new();
    let mut operation_ids: Vec<String> = Vec::new();
    let mut tags = Vec::new();
    for group in &api.groups {
        let mut used = false;
        for e in group.endpoints.iter().filter(|e| e.kind != Kind::Http) {
            warnings.push(format!(
                "{}: OpenAPI can't describe {} requests; Markdown and HTML docs include it",
                e.name,
                if e.kind == Kind::WebSocket {
                    "WebSocket"
                } else {
                    "gRPC"
                }
            ));
        }
        for e in group.endpoints.iter().filter(|e| e.kind == Kind::Http) {
            let method = e.method.to_ascii_lowercase();
            if ![
                "get", "put", "post", "delete", "options", "head", "patch", "trace",
            ]
            .contains(&method.as_str())
            {
                warnings.push(format!(
                    "{}: OpenAPI has no {} method; left out",
                    e.name, e.method
                ));
                continue;
            }
            let item = paths.entry(e.path.clone()).or_default();
            if item.contains_key(&method) {
                warnings.push(format!(
                    "{}: another request is already {} {}; OpenAPI allows one, so this one is left out",
                    e.name, e.method, e.path
                ));
                continue;
            }
            item.insert(
                method,
                operation(e, &group.name, &mut schemes, &mut operation_ids),
            );
            used = true;
        }
        if used && !group.name.is_empty() {
            let mut tag = json!({ "name": group.name });
            if !group.description.is_empty() {
                tag["description"] = json!(group.description);
            }
            tags.push(tag);
        }
    }
    let mut info = json!({ "title": api.title, "version": "1.0.0" });
    if !api.description.is_empty() {
        info["description"] = json!(api.description);
    }
    let mut doc = json!({
        "openapi": "3.1.0",
        "info": info,
        "paths": paths.into_iter().map(|(k, v)| (k, Value::Object(v))).collect::<Map<_, _>>(),
    });
    if !api.servers.is_empty() {
        doc["servers"] = api
            .servers
            .iter()
            .map(|url| {
                // An unresolved {{variable}} becomes an OpenAPI server variable.
                if url.contains("{{") {
                    let name = url
                        .trim_start_matches("{{")
                        .split("}}")
                        .next()
                        .unwrap_or("baseUrl")
                        .to_string();
                    json!({
                        "url": url.replace("{{", "{").replace("}}", "}"),
                        "variables": { name: { "default": "http://localhost" } },
                    })
                } else {
                    json!({ "url": url })
                }
            })
            .collect();
    }
    if !tags.is_empty() {
        doc["tags"] = Value::Array(tags);
    }
    if !schemes.is_empty() {
        doc["components"] = json!({ "securitySchemes": schemes });
    }
    doc
}

fn operation(
    e: &Endpoint,
    tag: &str,
    schemes: &mut Map<String, Value>,
    ids: &mut Vec<String>,
) -> Value {
    let mut op = json!({ "summary": e.name, "operationId": operation_id(&e.name, ids) });
    if !tag.is_empty() {
        op["tags"] = json!([tag]);
    }
    if !e.description.is_empty() {
        op["description"] = json!(e.description);
    }
    let mut parameters = Vec::new();
    for p in &e.path_params {
        parameters.push(json!({ "name": p.name, "in": "path", "required": true, "schema": { "type": "string" } }));
    }
    for (place, list) in [("query", &e.query), ("header", &e.headers)] {
        for p in list {
            let mut param = json!({ "name": p.name, "in": place, "schema": { "type": "string" } });
            if !p.example.is_empty() && !p.example.contains("{{") {
                param["example"] = json!(p.example);
            }
            parameters.push(param);
        }
    }
    if !parameters.is_empty() {
        op["parameters"] = Value::Array(parameters);
    }
    if let Some(body) = &e.body {
        op["requestBody"] = json!({ "content": { body.content_type.clone(): media(body) } });
    }
    let mut responses: Map<String, Value> = Map::new();
    let mut by_status: BTreeMap<u16, Vec<&super::ResponseDoc>> = BTreeMap::new();
    for r in &e.responses {
        by_status.entry(r.status).or_default().push(r);
    }
    for (status, list) in by_status {
        let mut response = json!({ "description": list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ") });
        let mut content: Map<String, Value> = Map::new();
        for r in list.iter().filter(|r| r.body.is_some()) {
            let body = r.body.as_ref().unwrap();
            let entry = content
                .entry(body.content_type.clone())
                .or_insert_with(|| json!({ "examples": {} }));
            if entry.get("schema").is_none()
                && let Some(schema) = &body.schema
            {
                entry["schema"] = schema.clone();
            }
            entry["examples"][r.name.clone()] = json!({ "value": example_value(body) });
        }
        if !content.is_empty() {
            response["content"] = Value::Object(content);
        }
        responses.insert(status.to_string(), response);
    }
    if responses.is_empty() {
        responses.insert(
            "default".into(),
            json!({ "description": "Not documented yet: save an example response" }),
        );
    }
    op["responses"] = Value::Object(responses);
    if let Some(auth) = &e.auth {
        match scheme(auth) {
            Some((name, value)) => {
                schemes.insert(name.clone(), value);
                op["security"] = json!([{ name: [] }]);
            }
            None if *auth == AuthDoc::None => op["security"] = json!([]),
            None => {
                let note = format!("Auth: {}.", auth.describe());
                op["description"] = json!(match op["description"].as_str() {
                    Some(d) => format!("{d}\n\n{note}"),
                    None => note,
                });
            }
        }
    }
    op
}

fn media(body: &Payload) -> Value {
    let mut media = json!({ "example": example_value(body) });
    if let Some(schema) = &body.schema {
        media["schema"] = schema.clone();
    }
    media
}

/// A JSON example as JSON, anything else as text.
fn example_value(body: &Payload) -> Value {
    if body.schema.is_some() {
        serde_json::from_str(&body.example).unwrap_or_else(|_| json!(body.example))
    } else {
        json!(body.example)
    }
}

fn scheme(auth: &AuthDoc) -> Option<(String, Value)> {
    Some(match auth {
        AuthDoc::Bearer => (
            "bearerAuth".into(),
            json!({ "type": "http", "scheme": "bearer" }),
        ),
        AuthDoc::Basic => (
            "basicAuth".into(),
            json!({ "type": "http", "scheme": "basic" }),
        ),
        AuthDoc::Digest => (
            "digestAuth".into(),
            json!({ "type": "http", "scheme": "digest" }),
        ),
        AuthDoc::ApiKey { name, in_query } => (
            format!(
                "apiKey_{}",
                name.chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect::<String>()
            ),
            json!({ "type": "apiKey", "name": name, "in": if *in_query { "query" } else { "header" } }),
        ),
        AuthDoc::OAuth2 {
            flow,
            auth_url,
            token_url,
            scopes,
        } => {
            let scopes: Map<String, Value> =
                scopes.iter().map(|s| (s.clone(), json!(""))).collect();
            let mut f = json!({ "scopes": scopes });
            if matches!(*flow, "authorizationCode" | "implicit") {
                f["authorizationUrl"] = json!(auth_url);
            }
            if *flow != "implicit" {
                f["tokenUrl"] = json!(token_url);
            }
            (
                "oauth2".into(),
                json!({ "type": "oauth2", "flows": { *flow: f } }),
            )
        }
        AuthDoc::None | AuthDoc::Other(_) => return None,
    })
}

/// "List orders" → "listOrders", unique in the document.
fn operation_id(name: &str, ids: &mut Vec<String>) -> String {
    let mut id = String::new();
    for (i, word) in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        let lower = word.to_ascii_lowercase();
        if i == 0 {
            id.push_str(&lower);
        } else {
            let mut chars = lower.chars();
            if let Some(first) = chars.next() {
                id.push(first.to_ascii_uppercase());
                id.extend(chars);
            }
        }
    }
    if id.is_empty() {
        id = "operation".into();
    }
    let mut unique = id.clone();
    let mut n = 2;
    while ids.contains(&unique) {
        unique = format!("{id}{n}");
        n += 1;
    }
    ids.push(unique.clone());
    unique
}
