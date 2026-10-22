//! Bruno collection exports (JSON): folders, HTTP and GraphQL requests, auth, and the environments exported with them.
//! Bruno's scripts use its own `bru.*` API, so they are kept as comments for the user to port, not run.

use serde_json::Value;

use super::Warnings;
use crate::http::ApiKeyLocation;
use crate::workspace::files::{
    Auth, Body, FolderFile, KeyValue, Part, RequestFile, Scripts, Variable,
};
use crate::workspace::{FolderTree, TreeItem};

/// A Bruno export: a `version` and a list of `items` whose entries have a Bruno `type`.
pub fn detect(json: &Value) -> bool {
    json.get("items").is_some_and(Value::is_array)
        && json.get("version").is_some()
        && json["items"].as_array().is_some_and(|items| {
            items.iter().all(|i| {
                matches!(
                    i["type"].as_str(),
                    Some(
                        "folder"
                            | "http-request"
                            | "graphql-request"
                            | "grpc-request"
                            | "ws-request"
                    )
                )
            })
        })
}

pub struct Parsed {
    pub collection: FolderTree,
    pub environments: Vec<(String, Vec<Variable>)>,
}

pub fn parse(json: &Value, warnings: &mut Warnings) -> Parsed {
    let name = json["name"].as_str().unwrap_or("Bruno collection");
    let mut collection = folder(name, &json["items"], warnings);
    let root = &json["root"]["request"];
    collection.folder.headers = key_values(&root["headers"]);
    collection.folder.auth = auth(&root["auth"], name, warnings);
    collection.folder.description = json["root"]["docs"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let environments = json["environments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|env| {
            let variables = env["variables"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| Variable {
                    name: v["name"].as_str().unwrap_or("").to_string(),
                    value: v["value"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v["value"].to_string()),
                    secret: v["secret"].as_bool().unwrap_or(false),
                    enabled: v["enabled"].as_bool().unwrap_or(true),
                })
                .filter(|v| !v.name.is_empty())
                .collect();
            (
                env["name"].as_str().unwrap_or("Environment").to_string(),
                variables,
            )
        })
        .collect();
    Parsed {
        collection,
        environments,
    }
}

fn folder(name: &str, items: &Value, warnings: &mut Warnings) -> FolderTree {
    let mut list: Vec<&Value> = items
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    // Bruno orders by `seq`; folders without one come first.
    list.sort_by_key(|i| i["seq"].as_i64().unwrap_or(0));
    let mut out = Vec::new();
    for item in list {
        let item_name = item["name"].as_str().unwrap_or("Untitled").to_string();
        match item["type"].as_str().unwrap_or("") {
            "folder" => {
                let mut child = folder(&item_name, &item["items"], warnings);
                let root = &item["root"]["request"];
                child.folder.headers = key_values(&root["headers"]);
                child.folder.auth = auth(&root["auth"], &item_name, warnings);
                child.folder.description = item["root"]["docs"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                out.push(TreeItem::Folder(child));
            }
            "http-request" | "graphql-request" => out.push(TreeItem::Request {
                file: request(item, &item_name, warnings),
                name: item_name,
            }),
            other => warnings.add(format!("{item_name}: {other} items are not imported")),
        }
    }
    FolderTree {
        name: name.to_string(),
        folder: FolderFile::default(),
        items: out,
    }
}

fn key_values(list: &Value) -> Vec<KeyValue> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|h| KeyValue {
            name: h["name"].as_str().unwrap_or("").to_string(),
            value: h["value"].as_str().unwrap_or("").to_string(),
            enabled: h["enabled"].as_bool().unwrap_or(true),
        })
        .filter(|h| !h.name.is_empty())
        .collect()
}

fn request(item: &Value, name: &str, warnings: &mut Warnings) -> RequestFile {
    let r = &item["request"];
    let mut url = r["url"].as_str().unwrap_or_default().to_string();
    // Bruno keeps the query in the URL too; path parameters (`:id`) are filled in from `params`.
    for p in r["params"].as_array().into_iter().flatten() {
        if p["type"].as_str() == Some("path") {
            let key = p["name"].as_str().unwrap_or("");
            let value = p["value"].as_str().unwrap_or("");
            if !key.is_empty() && !value.is_empty() {
                url = url.replace(&format!(":{key}"), value);
            }
        }
    }
    // Kept as line comments, so the request still sends, until someone ports them.
    let mut commented = |parts: &[(&str, Option<&str>)]| -> String {
        let mut out = String::new();
        for (label, text) in parts {
            if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
                out.push_str(&format!(
                    "// From Bruno ({label}); port bru.*, req.* and res.* to pm.* to run it:\n"
                ));
                for line in text.lines() {
                    out.push_str(&format!("// {line}\n"));
                }
                warnings.add(format!(
                    "{name}: its {label} uses Bruno's own API; it is kept as comments"
                ));
            }
        }
        out
    };
    let scripts = Scripts {
        pre_request: commented(&[("pre-request script", r["script"]["req"].as_str())]),
        test: commented(&[
            ("post-response script", r["script"]["res"].as_str()),
            ("tests", r["tests"].as_str()),
        ]),
    };
    RequestFile {
        method: r["method"].as_str().unwrap_or("GET").to_uppercase(),
        url,
        description: r["docs"].as_str().unwrap_or_default().to_string(),
        headers: key_values(&r["headers"]),
        body: body(&r["body"], name, warnings),
        auth: auth(&r["auth"], name, warnings),
        scripts,
        ..RequestFile::default()
    }
}

fn body(b: &Value, name: &str, warnings: &mut Warnings) -> Body {
    let text = |key: &str| b[key].as_str().unwrap_or("").to_string();
    match b["mode"].as_str().unwrap_or("none") {
        "json" => Body::Json {
            content: text("json"),
        },
        "text" => Body::Text {
            content: text("text"),
        },
        "xml" => Body::Xml {
            content: text("xml"),
        },
        "sparql" => Body::Raw {
            content: text("sparql"),
            content_type: "application/sparql-query".into(),
        },
        "formUrlEncoded" => Body::Form {
            fields: key_values(&b["formUrlEncoded"]),
        },
        "multipartForm" => Body::Multipart {
            parts: b["multipartForm"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let file = p["type"].as_str() == Some("file");
                    let value = match &p["value"] {
                        Value::Array(files) => files
                            .first()
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        v => v.as_str().unwrap_or("").to_string(),
                    };
                    Part {
                        name: p["name"].as_str().unwrap_or("").to_string(),
                        value,
                        file,
                        enabled: p["enabled"].as_bool().unwrap_or(true),
                        ..Part::default()
                    }
                })
                .collect(),
        },
        "graphql" => Body::Graphql {
            query: b["graphql"]["query"].as_str().unwrap_or("").to_string(),
            variables: b["graphql"]["variables"].as_str().unwrap_or("").to_string(),
            operation_name: None,
        },
        "none" => Body::None,
        other => {
            warnings.add(format!("{name}: its {other} body was not imported"));
            Body::None
        }
    }
}

fn auth(a: &Value, name: &str, warnings: &mut Warnings) -> Option<Auth> {
    let s = |section: &str, key: &str| a[section][key].as_str().unwrap_or("").to_string();
    match a["mode"].as_str()? {
        "inherit" => None,
        "none" => Some(Auth::None),
        "bearer" => Some(Auth::Bearer {
            token: s("bearer", "token"),
        }),
        "basic" => Some(Auth::Basic {
            username: s("basic", "username"),
            password: s("basic", "password"),
        }),
        "apikey" => Some(Auth::ApiKey {
            key: s("apikey", "key"),
            value: s("apikey", "value"),
            location: if a["apikey"]["placement"].as_str() == Some("queryparams") {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            },
        }),
        other => {
            warnings.add(format!(
                "{name}: {other} auth was not imported; set it in the Auth tab"
            ));
            None
        }
    }
}
