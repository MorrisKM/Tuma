//! A collection as Postman Collection v2.1 JSON, for Postman, Insomnia, Bruno and others that import it.

use serde_json::{Value, json};

use crate::workspace::FolderTree;
use crate::workspace::TreeItem;
use crate::workspace::files::{Auth, Body, FolderFile, KeyValue, RequestFile, Scripts};

pub fn collection(tree: &FolderTree) -> Value {
    let mut out = json!({
        "info": {
            "name": tree.name,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
        },
        "item": items(&tree.items),
    });
    folder_fields(&mut out, &tree.folder);
    let variables: Vec<Value> = tree
        .folder
        .variables
        .iter()
        .map(|v| {
            let mut var = json!({ "key": v.name, "value": v.value });
            if !v.enabled {
                var["disabled"] = json!(true);
            }
            var
        })
        .collect();
    if !variables.is_empty() {
        out["variable"] = json!(variables);
    }
    if !tree.folder.description.is_empty() {
        out["info"]["description"] = json!(tree.folder.description);
        out.as_object_mut().map(|o| o.remove("description"));
    }
    out
}

fn items(items: &[TreeItem]) -> Vec<Value> {
    items
        .iter()
        .map(|item| match item {
            TreeItem::Folder(folder) => {
                let mut out = json!({ "name": folder.name, "item": self::items(&folder.items) });
                folder_fields(&mut out, &folder.folder);
                out
            }
            TreeItem::Request { name, file } => request(name, file),
        })
        .collect()
}

fn folder_fields(out: &mut Value, folder: &FolderFile) {
    if !folder.description.is_empty() {
        out["description"] = json!(folder.description);
    }
    if let Some(auth) = &folder.auth {
        out["auth"] = auth_json(auth);
    }
    if let Some(events) = events(&folder.scripts) {
        out["event"] = events;
    }
}

fn request(name: &str, file: &RequestFile) -> Value {
    let mut request = json!({
        "method": file.method,
        "header": file.headers.iter().map(key_value).collect::<Vec<_>>(),
        "url": { "raw": file.url },
    });
    if !file.description.is_empty() {
        request["description"] = json!(file.description);
    }
    if let Some(auth) = &file.auth {
        request["auth"] = auth_json(auth);
    }
    if let Some(body) = body(&file.body) {
        request["body"] = body;
    }
    let mut out = json!({ "name": name, "request": request });
    if let Some(events) = events(&file.scripts) {
        out["event"] = events;
    }
    out
}

fn key_value(kv: &KeyValue) -> Value {
    let mut out = json!({ "key": kv.name, "value": kv.value });
    if !kv.enabled {
        out["disabled"] = json!(true);
    }
    out
}

fn auth_json(auth: &Auth) -> Value {
    let pair = |k: &str, v: &str| json!({ "key": k, "value": v, "type": "string" });
    match auth {
        Auth::None => json!({ "type": "noauth" }),
        Auth::Bearer { token } => json!({ "type": "bearer", "bearer": [pair("token", token)] }),
        Auth::Basic { username, password } => {
            json!({ "type": "basic", "basic": [pair("username", username), pair("password", password)] })
        }
        Auth::ApiKey {
            key,
            value,
            location,
        } => {
            let location = match location {
                crate::http::ApiKeyLocation::Header => "header",
                crate::http::ApiKeyLocation::Query => "query",
            };
            json!({ "type": "apikey", "apikey": [pair("key", key), pair("value", value), pair("in", location)] })
        }
    }
}

fn body(body: &Body) -> Option<Value> {
    let raw = |content: &str, language: &str| json!({ "mode": "raw", "raw": content, "options": { "raw": { "language": language } } });
    Some(match body {
        Body::None => return None,
        Body::Json { content } => raw(content, "json"),
        Body::Xml { content } => raw(content, "xml"),
        Body::Html { content } => raw(content, "html"),
        Body::Javascript { content } => raw(content, "javascript"),
        Body::Text { content } | Body::Raw { content, .. } => raw(content, "text"),
        Body::Form { fields } => {
            json!({ "mode": "urlencoded", "urlencoded": fields.iter().map(key_value).collect::<Vec<_>>() })
        }
        Body::Multipart { parts } => json!({
            "mode": "formdata",
            "formdata": parts.iter().map(|p| {
                let mut part = if p.file {
                    json!({ "key": p.name, "type": "file", "src": p.value })
                } else {
                    json!({ "key": p.name, "type": "text", "value": p.value })
                };
                if !p.enabled {
                    part["disabled"] = json!(true);
                }
                part
            }).collect::<Vec<_>>(),
        }),
        Body::File { path, .. } => json!({ "mode": "file", "file": { "src": path } }),
    })
}

fn events(scripts: &Scripts) -> Option<Value> {
    let mut events = Vec::new();
    for (listen, script) in [
        ("prerequest", &scripts.pre_request),
        ("test", &scripts.test),
    ] {
        if !script.is_empty() {
            events.push(json!({
                "listen": listen,
                "script": { "type": "text/javascript", "exec": script.lines().collect::<Vec<_>>() },
            }));
        }
    }
    (!events.is_empty()).then(|| json!(events))
}
