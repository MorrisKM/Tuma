//! Insomnia exports (format 4, JSON): workspaces become collections, request groups folders, and environments
//! environments. Insomnia writes variables as `{{ _.name }}`; they become `{{name}}`.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::Warnings;
use crate::http::ApiKeyLocation;
use crate::workspace::files::{
    Auth, Body, FolderFile, KeyValue, Part, RequestFile, Scripts, Variable,
};
use crate::workspace::{FolderTree, TreeItem};

/// An Insomnia export file: `_type: export` with a list of resources.
pub fn detect(json: &Value) -> bool {
    json.get("_type").and_then(Value::as_str) == Some("export")
        && json.get("resources").is_some_and(Value::is_array)
}

pub struct Parsed {
    pub collections: Vec<FolderTree>,
    pub environments: Vec<(String, Vec<Variable>)>,
}

static TEMPLATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*_\.([\w.-]+)\s*\}\}").unwrap());

/// `{{ _.baseUrl }}` → `{{baseUrl}}`.
fn vars(text: &str) -> String {
    TEMPLATE.replace_all(text, "{{$1}}").into_owned()
}

pub fn parse(json: &Value, warnings: &mut Warnings) -> Parsed {
    let resources: Vec<&Value> = json["resources"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let kind = |r: &Value| {
        r.get("_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut children: HashMap<String, Vec<&Value>> = HashMap::new();
    for r in &resources {
        if let Some(parent) = r.get("parentId").and_then(Value::as_str) {
            children.entry(parent.to_string()).or_default().push(r);
        }
    }
    // Insomnia orders siblings by metaSortKey.
    for list in children.values_mut() {
        list.sort_by(|a, b| {
            let key = |v: &Value| v.get("metaSortKey").and_then(Value::as_f64).unwrap_or(0.0);
            key(a).total_cmp(&key(b))
        });
    }

    let mut collections = Vec::new();
    let mut environments = Vec::new();
    for workspace in resources.iter().filter(|r| kind(r) == "workspace") {
        let id = workspace["_id"].as_str().unwrap_or_default();
        let name = workspace["name"].as_str().unwrap_or("Insomnia");
        let mut tree = group(id, name, &children, warnings);
        tree.folder.description = workspace["description"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        collections.push(tree);

        // The base environment, and its sub-environments with the base values underneath theirs.
        for base in children
            .get(id)
            .into_iter()
            .flatten()
            .filter(|r| kind(r) == "environment")
        {
            let base_vars = flatten(&base["data"]);
            let subs: Vec<_> = children
                .get(base["_id"].as_str().unwrap_or_default())
                .into_iter()
                .flatten()
                .filter(|r| kind(r) == "environment")
                .collect();
            if subs.is_empty() {
                if !base_vars.is_empty() {
                    environments.push((format!("{name} base"), base_vars));
                }
                continue;
            }
            for sub in subs {
                let mut all = base_vars.clone();
                for v in flatten(&sub["data"]) {
                    all.retain(|b| b.name != v.name);
                    all.push(v);
                }
                environments.push((
                    sub["name"].as_str().unwrap_or("Environment").to_string(),
                    all,
                ));
            }
        }
    }
    Parsed {
        collections,
        environments,
    }
}

/// Scalar values as they are, nested ones by dotted name (`{{ _.api.url }}` becomes `{{api.url}}`).
fn flatten(data: &Value) -> Vec<Variable> {
    fn walk(prefix: &str, value: &Value, out: &mut Vec<Variable>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let name = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&name, v, out);
                }
            }
            Value::String(s) => out.push(Variable::new(prefix, vars(s))),
            Value::Null => out.push(Variable::new(prefix, "")),
            other => out.push(Variable::new(prefix, other.to_string())),
        }
    }
    let mut out = Vec::new();
    walk("", data, &mut out);
    out
}

fn group(
    id: &str,
    name: &str,
    children: &HashMap<String, Vec<&Value>>,
    warnings: &mut Warnings,
) -> FolderTree {
    let mut items = Vec::new();
    for child in children.get(id).into_iter().flatten() {
        let child_name = child["name"].as_str().unwrap_or("Untitled").to_string();
        let child_id = child["_id"].as_str().unwrap_or_default();
        match child["_type"].as_str().unwrap_or("") {
            "request_group" => {
                let mut folder = group(child_id, &child_name, children, warnings);
                folder.folder.description = child["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                // A request group's environment is variables for the requests in it.
                folder.folder.variables = flatten(&child["environment"]);
                items.push(TreeItem::Folder(folder));
            }
            "request" => items.push(TreeItem::Request {
                file: request(child, &child_name, warnings),
                name: child_name,
            }),
            "grpc_request" | "websocket_request" => warnings.add(format!(
                "{child_name}: {} requests are not imported",
                child["_type"].as_str().unwrap_or("")
            )),
            _ => {}
        }
    }
    FolderTree {
        name: name.to_string(),
        folder: FolderFile::default(),
        items,
    }
}

fn request(r: &Value, name: &str, warnings: &mut Warnings) -> RequestFile {
    let mut url = vars(r["url"].as_str().unwrap_or_default());
    let query: Vec<String> = r["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| !p["disabled"].as_bool().unwrap_or(false))
        .map(|p| {
            format!(
                "{}={}",
                vars(p["name"].as_str().unwrap_or("")),
                vars(p["value"].as_str().unwrap_or(""))
            )
        })
        .collect();
    if !query.is_empty() {
        url = format!(
            "{url}{}{}",
            if url.contains('?') { "&" } else { "?" },
            query.join("&")
        );
    }
    let headers = r["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| KeyValue {
            name: vars(h["name"].as_str().unwrap_or("")),
            value: vars(h["value"].as_str().unwrap_or("")),
            enabled: !h["disabled"].as_bool().unwrap_or(false),
        })
        .filter(|h| !h.name.is_empty())
        .collect();
    let mut scripts = Scripts::default();
    for (key, slot) in [
        ("preRequestScript", &mut scripts.pre_request),
        ("afterResponseScript", &mut scripts.test),
    ] {
        if let Some(text) = r[key].as_str().filter(|t| !t.trim().is_empty()) {
            // Insomnia's script API is Postman's under another name.
            *slot = text.replace("insomnia.", "pm.");
            warnings.add(format!(
                "{name}: its {key} was converted from insomnia.* to pm.*; check that it still works"
            ));
        }
    }
    RequestFile {
        method: r["method"].as_str().unwrap_or("GET").to_uppercase(),
        url,
        description: r["description"].as_str().unwrap_or_default().to_string(),
        headers,
        body: body(&r["body"], name, warnings),
        auth: auth(&r["authentication"], name, warnings),
        scripts,
        ..RequestFile::default()
    }
}

fn body(b: &Value, name: &str, warnings: &mut Warnings) -> Body {
    let mime = b["mimeType"].as_str().unwrap_or("");
    let text = vars(b["text"].as_str().unwrap_or(""));
    let fields = || b["params"].as_array().into_iter().flatten();
    match mime {
        "" if text.is_empty() => Body::None,
        "application/x-www-form-urlencoded" => Body::Form {
            fields: fields()
                .map(|p| KeyValue {
                    name: vars(p["name"].as_str().unwrap_or("")),
                    value: vars(p["value"].as_str().unwrap_or("")),
                    enabled: !p["disabled"].as_bool().unwrap_or(false),
                })
                .collect(),
        },
        "multipart/form-data" => Body::Multipart {
            parts: fields()
                .map(|p| {
                    let file = p["type"].as_str() == Some("file");
                    Part {
                        name: vars(p["name"].as_str().unwrap_or("")),
                        value: if file {
                            p["fileName"].as_str().unwrap_or("").to_string()
                        } else {
                            vars(p["value"].as_str().unwrap_or(""))
                        },
                        file,
                        enabled: !p["disabled"].as_bool().unwrap_or(false),
                        ..Part::default()
                    }
                })
                .collect(),
        },
        "application/graphql" => {
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            Body::Graphql {
                query: parsed["query"].as_str().unwrap_or(&text).to_string(),
                variables: match &parsed["variables"] {
                    Value::Null => String::new(),
                    v => serde_json::to_string_pretty(v).unwrap_or_default(),
                },
                operation_name: parsed["operationName"].as_str().map(String::from),
            }
        }
        "application/octet-stream" => {
            warnings.add(format!(
                "{name}: its file body was not imported; choose the file again"
            ));
            Body::None
        }
        "" => Body::Text { content: text },
        mime => Body::text(text, mime),
    }
}

fn auth(a: &Value, name: &str, warnings: &mut Warnings) -> Option<Auth> {
    let s = |key: &str| vars(a[key].as_str().unwrap_or(""));
    if a["disabled"].as_bool() == Some(true) {
        return None;
    }
    match a["type"].as_str()? {
        "bearer" => Some(Auth::Bearer { token: s("token") }),
        "basic" => Some(Auth::Basic {
            username: s("username"),
            password: s("password"),
        }),
        "apikey" => Some(Auth::ApiKey {
            key: s("key"),
            value: s("value"),
            location: if a["addTo"].as_str() == Some("queryParams") {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            },
        }),
        "none" => Some(Auth::None),
        other => {
            warnings.add(format!(
                "{name}: {other} auth was not imported; set it in the Auth tab"
            ));
            None
        }
    }
}
