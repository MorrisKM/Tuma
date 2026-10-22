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
    if !file.examples.is_empty() {
        out["response"] = file
            .examples
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "code": e.status,
                    "header": e.headers.iter().map(key_value).collect::<Vec<_>>(),
                    "body": e.body,
                })
            })
            .collect();
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
        Auth::Digest(a) => {
            json!({ "type": "digest", "digest": [pair("username", &a.username), pair("password", &a.password)] })
        }
        Auth::Aws(a) => json!({ "type": "awsv4", "awsv4": [
            pair("accessKey", &a.access_key), pair("secretKey", &a.secret_key), pair("sessionToken", &a.session_token),
            pair("region", &a.region), pair("service", &a.service),
        ] }),
        Auth::Jwt(a) => json!({ "type": "jwt", "jwt": [
            pair("algorithm", &format!("{:?}", a.algorithm)), pair("secret", &a.secret),
            pair("isSecretBase64Encoded", &a.secret_base64.to_string()), pair("payload", &a.payload),
            pair("header", &a.headers), pair("headerPrefix", &a.prefix),
            pair("addTokenTo", if a.query_param.is_some() { "queryParam" } else { "header" }),
            pair("queryParamKey", a.query_param.as_deref().unwrap_or("token")),
        ] }),
        Auth::OAuth1(a) => {
            let method = serde_json::to_value(a.signature_method)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            json!({ "type": "oauth1", "oauth1": [
                pair("consumerKey", &a.consumer_key), pair("consumerSecret", &a.consumer_secret),
                pair("token", &a.token), pair("tokenSecret", &a.token_secret), pair("signatureMethod", &method),
                pair("addParamsToHeader", &(a.location == crate::http::auth::OAuth1Location::Header).to_string()),
                pair("realm", &a.realm), pair("callback", &a.callback), pair("verifier", &a.verifier), pair("version", "1.0"),
            ] })
        }
        Auth::OAuth2(a) => {
            use crate::http::auth::{ClientAuthentication, OAuth2Grant};
            let grant = match a.grant {
                OAuth2Grant::AuthorizationCode if a.pkce => "authorization_code_with_pkce",
                OAuth2Grant::AuthorizationCode => "authorization_code",
                OAuth2Grant::Implicit => "implicit",
                OAuth2Grant::ClientCredentials => "client_credentials",
                OAuth2Grant::Password => "password_credentials",
            };
            json!({ "type": "oauth2", "oauth2": [
                pair("grant_type", grant), pair("authUrl", &a.auth_url), pair("accessTokenUrl", &a.access_token_url),
                pair("clientId", &a.client_id), pair("clientSecret", &a.client_secret), pair("scope", &a.scope),
                pair("redirect_uri", &a.redirect_uri), pair("username", &a.username), pair("password", &a.password),
                pair("client_authentication", if a.client_authentication == ClientAuthentication::Body { "body" } else { "header" }),
                pair("headerPrefix", &a.prefix), pair("addTokenTo", "header"),
            ] })
        }
    }
}

fn body(body: &Body) -> Option<Value> {
    let raw = |content: &str, language: &str| json!({ "mode": "raw", "raw": content, "options": { "raw": { "language": language } } });
    Some(match body {
        Body::None => return None,
        Body::Graphql {
            query, variables, ..
        } => {
            json!({ "mode": "graphql", "graphql": { "query": query, "variables": variables } })
        }
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
