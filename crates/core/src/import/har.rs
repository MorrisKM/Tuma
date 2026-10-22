//! HAR files: network traffic saved from a browser's developer tools ("Save all as HAR") or a proxy.
//!
//! Pages load many images, scripts and fonts; only API calls are kept: requests the browser marks as `fetch` or
//! `xhr`, or, without that mark, responses that are not static files.

use serde_json::Value;

use super::Warnings;
use crate::workspace::files::{Body, KeyValue, RequestFile};
use crate::workspace::{FolderTree, TreeItem};

/// Headers that browsers set themselves or that only make sense for one connection.
const SKIP_HEADERS: [&str; 10] = [
    "host",
    "connection",
    "content-length",
    "accept-encoding",
    "cookie",
    "origin",
    "referer",
    "user-agent",
    "sec-fetch-mode",
    "priority",
];

/// Builds a collection named `name` from a HAR file: a folder per host, a request per call, in recorded order.
pub fn parse(json: &str, name: &str, warnings: &mut Warnings) -> Result<FolderTree, String> {
    let har: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let entries = har
        .pointer("/log/entries")
        .and_then(Value::as_array)
        .ok_or("not a HAR file (no log.entries)")?;
    let mut folders: Vec<FolderTree> = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        if !is_api_call(entry) {
            skipped += 1;
            continue;
        }
        let request = &entry["request"];
        let raw_url = request["url"].as_str().unwrap_or_default();
        let Ok(url) = url::Url::parse(raw_url) else {
            continue;
        };
        let host = url.host_str().unwrap_or("other").to_string();
        let method = request["method"].as_str().unwrap_or("GET").to_uppercase();
        let headers = request["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| Some((h["name"].as_str()?, h["value"].as_str().unwrap_or_default())))
            // HTTP/2 pseudo-headers such as :authority.
            .filter(|(n, _)| !n.starts_with(':') && !n.to_ascii_lowercase().starts_with("sec-ch-"))
            .filter(|(n, _)| !SKIP_HEADERS.contains(&n.to_ascii_lowercase().as_str()))
            .map(|(n, v)| KeyValue {
                name: n.into(),
                value: v.into(),
                enabled: true,
            })
            .collect::<Vec<_>>();
        let body = match request.get("postData") {
            Some(post) => {
                let mime = post["mimeType"].as_str().unwrap_or_default();
                let text = post["text"].as_str().unwrap_or_default().to_string();
                if mime.starts_with("application/x-www-form-urlencoded") {
                    Body::Form {
                        fields: url::form_urlencoded::parse(text.as_bytes())
                            .map(|(n, v)| KeyValue {
                                name: n.into_owned(),
                                value: v.into_owned(),
                                enabled: true,
                            })
                            .collect(),
                    }
                } else if mime.starts_with("multipart/") {
                    warnings.add(format!("{method} {raw_url}: multipart body not imported"));
                    Body::None
                } else if text.is_empty() {
                    Body::None
                } else {
                    Body::text(text, mime.split(';').next().unwrap_or("text/plain").trim())
                }
            }
            None => Body::None,
        };
        // The body's content type is part of the body.
        let headers = if body.is_none() {
            headers
        } else {
            headers
                .into_iter()
                .filter(|h| !h.name.eq_ignore_ascii_case("content-type"))
                .collect()
        };
        let path = url.path().trim_end_matches('/');
        let item = TreeItem::Request {
            name: format!("{method} {}", if path.is_empty() { "/" } else { path }),
            file: RequestFile {
                method,
                url: raw_url.to_string(),
                headers,
                body,
                ..RequestFile::default()
            },
        };
        match folders.iter_mut().find(|f| f.name == host) {
            Some(folder) => folder.items.push(item),
            None => folders.push(FolderTree {
                name: host,
                items: vec![item],
                ..Default::default()
            }),
        }
    }
    if skipped > 0 {
        warnings.add(format!(
            "{skipped} page loads (images, scripts, styles, fonts, documents) left out"
        ));
    }
    if folders.is_empty() {
        return Err("the HAR file has no API calls".into());
    }
    Ok(FolderTree {
        name: name.to_string(),
        items: folders.into_iter().map(TreeItem::Folder).collect(),
        ..Default::default()
    })
}

fn is_api_call(entry: &Value) -> bool {
    if let Some(kind) = entry["_resourceType"].as_str() {
        return kind == "fetch" || kind == "xhr";
    }
    let mime = entry
        .pointer("/response/content/mimeType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ![
        "image/",
        "font/",
        "text/css",
        "text/html",
        "javascript",
        "video/",
        "audio/",
    ]
    .iter()
    .any(|m| mime.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_api_calls() {
        let har = r#"{"log": {"entries": [
            {"_resourceType": "document", "request": {"method": "GET", "url": "https://app.example.com/", "headers": []}},
            {"_resourceType": "fetch", "request": {"method": "POST", "url": "https://api.example.com/v1/login",
              "headers": [{"name": ":authority", "value": "api.example.com"}, {"name": "Content-Type", "value": "application/json"},
                          {"name": "Authorization", "value": "Bearer x"}, {"name": "Cookie", "value": "a=b"}],
              "postData": {"mimeType": "application/json", "text": "{\"user\":\"ann\"}"}}},
            {"request": {"method": "GET", "url": "https://cdn.example.com/logo.png", "headers": []},
             "response": {"content": {"mimeType": "image/png"}}},
            {"request": {"method": "GET", "url": "https://api.example.com/v1/me", "headers": []},
             "response": {"content": {"mimeType": "application/json"}}}
        ]}}"#;
        let mut warnings = Warnings::default();
        let tree = parse(har, "Recording", &mut warnings).unwrap();
        assert_eq!(
            warnings.0,
            ["2 page loads (images, scripts, styles, fonts, documents) left out"]
        );
        let TreeItem::Folder(api) = &tree.items[0] else {
            panic!()
        };
        assert_eq!(api.name, "api.example.com");
        let TreeItem::Request { name, file } = &api.items[0] else {
            panic!()
        };
        assert_eq!(name, "POST /v1/login");
        assert_eq!(
            file.headers
                .iter()
                .map(|h| h.name.as_str())
                .collect::<Vec<_>>(),
            ["Authorization"]
        );
        assert_eq!(
            file.body,
            Body::Json {
                content: "{\"user\":\"ann\"}".into()
            }
        );
        assert_eq!(api.items.len(), 2);
    }
}
