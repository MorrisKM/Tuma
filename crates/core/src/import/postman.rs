//! Postman Collection v2.0/v2.1 and environment / globals exports.
//!
//! Exports in the wild vary (a URL is a string or an object, a description a string or an object, a script a string
//! or a list of lines), so this reads `serde_json::Value`s leniently. Anything that cannot be carried over is listed in
//! the warnings instead of failing the import.

use serde_json::Value;

use super::Warnings;
use crate::http::ApiKeyLocation;
use crate::workspace::FolderTree;
use crate::workspace::TreeItem;
use crate::workspace::files::{
    Auth, Body, FolderFile, KeyValue, Part, RequestFile, Scripts, Variable,
};

/// What a Postman export file holds.
pub enum Export {
    Collection(FolderTree),
    /// An environment, or Postman's globals (`name` is then "Globals").
    Environment {
        name: String,
        globals: bool,
        variables: Vec<(Variable, bool)>,
    },
}

/// Reads a Postman export. Environment variables come with a flag that says whether Postman marked them secret.
pub fn parse(json: &str, warnings: &mut Warnings) -> Result<Export, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    if value.get("info").is_some() && value.get("item").is_some() {
        return Ok(Export::Collection(collection(&value, warnings)));
    }
    if let Some(values) = value.get("values").and_then(Value::as_array) {
        let globals = str_at(&value, "_postman_variable_scope") == "globals";
        let name = if globals {
            "Globals".to_string()
        } else {
            str_at(&value, "name").to_string()
        };
        let variables = values
            .iter()
            .map(|v| {
                let variable = Variable {
                    name: str_at(v, "key").to_string(),
                    value: scalar(v.get("value")),
                    secret: false,
                    enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                };
                (variable, str_at(v, "type") == "secret")
            })
            .collect();
        return Ok(Export::Environment {
            name,
            globals,
            variables,
        });
    }
    Err("this is not a Postman collection or environment export".to_string())
}

fn collection(value: &Value, warnings: &mut Warnings) -> FolderTree {
    let info = &value["info"];
    let schema = str_at(info, "schema");
    if !schema.is_empty() && !schema.contains("v2.") {
        warnings.add(format!(
            "unexpected schema {schema}; only Postman v2.0 and v2.1 are supported"
        ));
    }
    let mut tree = folder(value, str_at(info, "name"), warnings);
    tree.folder.variables = variables(value.get("variable"));
    tree.folder.description = description(info.get("description"));
    tree
}

fn folder(value: &Value, name: &str, warnings: &mut Warnings) -> FolderTree {
    let items = array(value.get("item"))
        .iter()
        .map(|item| {
            let name = str_at(item, "name");
            if item.get("item").is_some() {
                TreeItem::Folder(folder(item, name, warnings))
            } else {
                TreeItem::Request {
                    name: name.to_string(),
                    file: request(item, name, warnings),
                }
            }
        })
        .collect();
    let folder = FolderFile {
        description: description(value.get("description")),
        auth: auth(value.get("auth"), name, warnings),
        scripts: scripts(value.get("event")),
        ..FolderFile::default()
    };
    FolderTree {
        name: name.to_string(),
        folder,
        items,
    }
}

fn request(item: &Value, name: &str, warnings: &mut Warnings) -> RequestFile {
    // A request can be just a URL string.
    let req = item.get("request").cloned().unwrap_or(Value::Null);
    if let Some(url) = req.as_str() {
        return RequestFile {
            url: url.to_string(),
            ..RequestFile::default()
        };
    }
    let examples = array(item.get("response")).len();
    if examples > 0 {
        warnings.add(format!(
            "\"{name}\": {examples} saved example(s) not imported"
        ));
    }
    RequestFile {
        method: match str_at(&req, "method") {
            "" => "GET".to_string(),
            method => method.to_uppercase(),
        },
        url: url(req.get("url"), name, warnings),
        description: description(req.get("description")),
        headers: array(req.get("header")).iter().map(key_value).collect(),
        body: body(req.get("body"), name, warnings),
        auth: auth(req.get("auth"), name, warnings),
        settings: Default::default(),
        scripts: scripts(item.get("event")),
    }
}

fn url(url: Option<&Value>, name: &str, warnings: &mut Warnings) -> String {
    let Some(url) = url else { return String::new() };
    if let Some(raw) = url.as_str() {
        return raw.to_string();
    }
    // Path variables (`/users/:id`) are not supported yet: put their values into the URL.
    let mut raw = str_at(url, "raw").to_string();
    for var in array(url.get("variable")) {
        let (key, value) = (str_at(var, "key"), scalar(var.get("value")));
        if !key.is_empty() && !value.is_empty() {
            raw = replace_path_variable(&raw, key, &value);
        }
    }
    let disabled = array(url.get("query"))
        .iter()
        .filter(|q| is_disabled(q))
        .count();
    if disabled > 0 {
        warnings.add(format!(
            "\"{name}\": {disabled} disabled query parameter(s) dropped"
        ));
    }
    raw
}

/// Replaces `:key` when it is a whole path segment.
fn replace_path_variable(url: &str, key: &str, value: &str) -> String {
    let pattern = format!(":{key}");
    let mut out = String::new();
    let mut rest = url;
    while let Some(i) = rest.find(&pattern) {
        let after = &rest[i + pattern.len()..];
        let ends_segment = after.is_empty() || after.starts_with(['/', '?', '#']);
        let starts_segment = rest[..i].ends_with('/');
        out.push_str(&rest[..i]);
        out.push_str(if ends_segment && starts_segment {
            value
        } else {
            &pattern
        });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn body(body: Option<&Value>, name: &str, warnings: &mut Warnings) -> Body {
    let Some(body) = body.filter(|b| !is_disabled(b)) else {
        return Body::None;
    };
    match str_at(body, "mode") {
        "raw" => {
            let content = str_at(body, "raw").to_string();
            if content.is_empty() {
                return Body::None;
            }
            match body
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
                .unwrap_or("text")
            {
                "json" => Body::Json { content },
                "xml" => Body::Xml { content },
                "html" => Body::Html { content },
                "javascript" => Body::Javascript { content },
                _ => Body::Text { content },
            }
        }
        "urlencoded" => Body::Form {
            fields: array(body.get("urlencoded"))
                .iter()
                .map(key_value)
                .collect(),
        },
        "formdata" => Body::Multipart {
            parts: array(body.get("formdata"))
                .iter()
                .map(|p| {
                    let file = str_at(p, "type") == "file";
                    let value = if file {
                        file_src(p.get("src"))
                    } else {
                        scalar(p.get("value"))
                    };
                    let content_type = Some(str_at(p, "contentType"))
                        .filter(|c| !c.is_empty())
                        .map(String::from);
                    Part {
                        name: str_at(p, "key").to_string(),
                        value,
                        file,
                        content_type,
                        enabled: !is_disabled(p),
                    }
                })
                .collect(),
        },
        "file" => match file_src(body.pointer("/file/src")) {
            src if src.is_empty() => Body::None,
            src => Body::File {
                path: src.into(),
                content_type: None,
            },
        },
        "graphql" => {
            let graphql = &body["graphql"];
            let variables: Value = match &graphql["variables"] {
                Value::String(s) if !s.trim().is_empty() => {
                    serde_json::from_str(s).unwrap_or(Value::Null)
                }
                Value::String(_) => Value::Null,
                other => other.clone(),
            };
            warnings.add(format!(
                "\"{name}\": GraphQL body imported as JSON until GraphQL support arrives"
            ));
            let json = serde_json::json!({ "query": graphql["query"], "variables": variables });
            Body::Json {
                content: serde_json::to_string_pretty(&json).unwrap_or_default(),
            }
        }
        "" => Body::None,
        mode => {
            warnings.add(format!("\"{name}\": body type \"{mode}\" not supported"));
            Body::None
        }
    }
}

fn file_src(src: Option<&Value>) -> String {
    match src {
        Some(Value::String(s)) => s.clone(),
        // Postman allows several files in one field; Tuma sends one.
        Some(Value::Array(files)) => files
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// `None` means inherit, which is what a missing `auth` means in Postman.
fn auth(auth: Option<&Value>, name: &str, warnings: &mut Warnings) -> Option<Auth> {
    let auth = auth.filter(|a| !a.is_null())?;
    let kind = str_at(auth, "type");
    // Each type's settings are a list of {key, value}.
    let field = |key: &str| {
        array(auth.get(kind))
            .iter()
            .find(|f| str_at(f, "key") == key)
            .map(|f| scalar(f.get("value")))
            .unwrap_or_default()
    };
    Some(match kind {
        "noauth" => Auth::None,
        "inherit" | "" => return None,
        "bearer" => Auth::Bearer {
            token: field("token"),
        },
        "basic" => Auth::Basic {
            username: field("username"),
            password: field("password"),
        },
        "apikey" => Auth::ApiKey {
            key: field("key"),
            value: field("value"),
            location: if field("in") == "query" {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            },
        },
        other => {
            warnings.add(format!(
                "\"{name}\": {other} auth is not supported yet; it was set to no auth"
            ));
            Auth::None
        }
    })
}

fn scripts(events: Option<&Value>) -> Scripts {
    let mut scripts = Scripts::default();
    for event in array(events) {
        let exec = match event.pointer("/script/exec") {
            Some(Value::Array(lines)) => lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        if exec.trim().is_empty() {
            continue;
        }
        match str_at(event, "listen") {
            "prerequest" => scripts.pre_request = exec,
            "test" => scripts.test = exec,
            _ => {}
        }
    }
    scripts
}

fn variables(vars: Option<&Value>) -> Vec<Variable> {
    array(vars)
        .iter()
        .map(|v| Variable {
            name: str_at(v, "key").to_string(),
            value: scalar(v.get("value")),
            secret: false,
            enabled: !is_disabled(v),
        })
        .filter(|v| !v.name.is_empty())
        .collect()
}

fn key_value(v: &Value) -> KeyValue {
    KeyValue {
        name: str_at(v, "key").to_string(),
        value: scalar(v.get("value")),
        enabled: !is_disabled(v),
    }
}

fn description(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(o)) => o
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A value as text: Postman stores numbers and booleans as JSON values in some exports.
fn scalar(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn is_disabled(value: &Value) -> bool {
    value
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_variables_only_replace_whole_segments() {
        assert_eq!(
            replace_path_variable("{{base}}/users/:id/posts?x=:id", "id", "7"),
            "{{base}}/users/7/posts?x=:id"
        );
        assert_eq!(replace_path_variable("/a/:idx", "id", "7"), "/a/:idx");
    }
}
