//! Code that sends a request: for copying into a script or an app. Variables must already be resolved.

use serde_json::Value;

use crate::curl::{auth_headers, full_url};
use crate::http::{Body, Request};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    Curl,
    /// Python with `requests`.
    Python,
    /// JavaScript with `fetch` (browsers, Node 18+, Deno, Bun).
    JavaScript,
}

pub fn generate(request: &Request, language: Language) -> String {
    match language {
        Language::Curl => crate::curl::to_curl(request),
        Language::Python => python(request),
        Language::JavaScript => javascript(request),
    }
}

/// The headers to write out: the request's own, its content type when the body implies one and none is set, and auth.
fn headers(request: &Request, implied_content_type: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = request
        .headers
        .iter()
        .filter(|h| h.enabled && !h.name.is_empty())
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    if let Some(ct) = implied_content_type.filter(|ct| !ct.is_empty())
        && !out
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"))
    {
        out.push(("Content-Type".into(), ct.to_string()));
    }
    out.extend(auth_headers(request).0);
    out
}

/// A JSON body as data, when it parses; it is then written as a literal instead of a string.
fn json_body(request: &Request) -> Option<Value> {
    match &request.body {
        Body::Raw {
            content,
            content_type,
        } if content_type.contains("json") => serde_json::from_str(content).ok(),
        _ => None,
    }
}

fn string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn python(request: &Request) -> String {
    let method = request.method.to_lowercase();
    let known =
        ["get", "post", "put", "patch", "delete", "head", "options"].contains(&method.as_str());
    let mut args = vec![string(&full_url(request))];
    if !known {
        args.insert(0, string(&request.method.to_uppercase()));
    }
    let json = json_body(request);
    let mut files_comment = false;
    let body_arg = match (&request.body, &json) {
        (_, Some(value)) => Some(format!("json={}", python_value(value, 1))),
        (Body::Raw { content, .. }, None) => Some(format!("data={}", string(content))),
        (Body::FormUrlEncoded { fields }, _) => {
            let items: Vec<String> = fields
                .iter()
                .filter(|f| f.enabled && !f.name.is_empty())
                .map(|f| format!("        {}: {},", string(&f.name), string(&f.value)))
                .collect();
            Some(format!("data={{\n{}\n    }}", items.join("\n")))
        }
        (Body::Multipart { parts }, _) => {
            files_comment = parts.iter().any(|p| p.file);
            let fields: Vec<String> = parts
                .iter()
                .filter(|p| p.enabled && !p.name.is_empty() && !p.file)
                .map(|p| format!("        {}: {},", string(&p.name), string(&p.value)))
                .collect();
            let files: Vec<String> = parts
                .iter()
                .filter(|p| p.enabled && !p.name.is_empty() && p.file)
                .map(|p| {
                    format!(
                        "        {}: open({}, \"rb\"),",
                        string(&p.name),
                        string(&p.value)
                    )
                })
                .collect();
            let mut out = Vec::new();
            if !fields.is_empty() {
                out.push(format!("data={{\n{}\n    }}", fields.join("\n")));
            }
            if !files.is_empty() {
                out.push(format!("files={{\n{}\n    }}", files.join("\n")));
            }
            (!out.is_empty()).then(|| out.join(",\n    "))
        }
        (Body::File { path, .. }, _) => Some(format!(
            "data=open({}, \"rb\")",
            string(&path.to_string_lossy())
        )),
        (Body::None, _) => None,
    };
    // requests sets the content type itself for json=, data= dicts and files=.
    let implied = match &request.body {
        Body::Raw { content_type, .. } if json.is_none() => Some(content_type.as_str()),
        Body::File { content_type, .. } => content_type.as_deref(),
        _ => None,
    };
    let headers = headers(request, implied);
    if !headers.is_empty() {
        let items: Vec<String> = headers
            .iter()
            .map(|(n, v)| format!("        {}: {},", string(n), string(v)))
            .collect();
        args.push(format!("headers={{\n{}\n    }}", items.join("\n")));
    }
    args.extend(body_arg);
    if !request.settings.verify_tls {
        args.push("verify=False".into());
    }
    if let Some(ms) = request.settings.timeout_ms {
        args.push(format!("timeout={}", ms as f64 / 1000.0));
    }
    let call = if known {
        format!("requests.{method}")
    } else {
        "requests.request".into()
    };
    let mut out = String::from("import requests\n\n");
    if files_comment {
        out.push_str("# File paths are as they are on the machine that made this snippet.\n");
    }
    out.push_str(&format!(
        "response = {call}(\n    {},\n)\n",
        args.join(",\n    ")
    ));
    out.push_str("print(response.status_code)\nprint(response.text)\n");
    out
}

/// A JSON value as a Python literal, indented for an argument list.
fn python_value(value: &Value, depth: usize) -> String {
    let pad = "    ".repeat(depth + 1);
    let end = "    ".repeat(depth);
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => string(s),
        Value::Array(items) if items.is_empty() => "[]".into(),
        Value::Array(items) => {
            let items: Vec<String> = items
                .iter()
                .map(|v| format!("{pad}{},", python_value(v, depth + 1)))
                .collect();
            format!("[\n{}\n{end}]", items.join("\n"))
        }
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{pad}{}: {},", string(k), python_value(v, depth + 1)))
                .collect();
            format!("{{\n{}\n{end}}}", items.join("\n"))
        }
    }
}

fn javascript(request: &Request) -> String {
    let json = json_body(request);
    let mut prelude = Vec::new();
    let body = match (&request.body, &json) {
        (_, Some(value)) => Some(format!(
            "JSON.stringify({})",
            indent(&serde_json::to_string_pretty(value).unwrap_or_default(), 1)
        )),
        (Body::Raw { content, .. }, None) => Some(string(content)),
        (Body::FormUrlEncoded { fields }, _) => {
            let items: Vec<String> = fields
                .iter()
                .filter(|f| f.enabled && !f.name.is_empty())
                .map(|f| format!("  [{}, {}],", string(&f.name), string(&f.value)))
                .collect();
            Some(format!(
                "new URLSearchParams([\n{}\n  ])",
                items
                    .iter()
                    .map(|i| format!("  {i}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        }
        (Body::Multipart { parts }, _) => {
            prelude.push("const form = new FormData();".to_string());
            for p in parts.iter().filter(|p| p.enabled && !p.name.is_empty()) {
                if p.file {
                    prelude.push(format!(
                        "// In Node: new Blob([await fs.promises.readFile({})])\nform.append({}, fileFromInput, {});",
                        string(&p.value),
                        string(&p.name),
                        string(std::path::Path::new(&p.value).file_name().and_then(|n| n.to_str()).unwrap_or("file"))
                    ));
                } else {
                    prelude.push(format!(
                        "form.append({}, {});",
                        string(&p.name),
                        string(&p.value)
                    ));
                }
            }
            Some("form".into())
        }
        (Body::File { path, .. }, _) => {
            prelude.push(format!(
                "// In a browser, use a File from an <input type=\"file\">.\nconst file = await fs.promises.readFile({});",
                string(&path.to_string_lossy())
            ));
            Some("file".into())
        }
        (Body::None, _) => None,
    };
    // fetch sets the content type itself for URLSearchParams and FormData.
    let implied = match &request.body {
        Body::Raw { content_type, .. } => Some(content_type.as_str()),
        Body::File { content_type, .. } => content_type.as_deref(),
        _ => None,
    };
    let mut options = Vec::new();
    let method = request.method.to_uppercase();
    if method != "GET" {
        options.push(format!("  method: {},", string(&method)));
    }
    let headers = headers(request, implied);
    if !headers.is_empty() {
        let items: Vec<String> = headers
            .iter()
            .map(|(n, v)| format!("    {}: {},", string(n), string(v)))
            .collect();
        options.push(format!("  headers: {{\n{}\n  }},", items.join("\n")));
    }
    if let Some(body) = body {
        options.push(format!("  body: {body},"));
    }
    if !request.settings.follow_redirects {
        options.push("  redirect: \"manual\",".into());
    }
    let mut out = String::new();
    for line in &prelude {
        out.push_str(line);
        out.push('\n');
    }
    if !prelude.is_empty() {
        out.push('\n');
    }
    let url = string(&full_url(request));
    if options.is_empty() {
        out.push_str(&format!("const response = await fetch({url});\n"));
    } else {
        out.push_str(&format!(
            "const response = await fetch({url}, {{\n{}\n}});\n",
            options.join("\n")
        ));
    }
    out.push_str("console.log(response.status, await response.text());\n");
    out
}

fn indent(text: &str, levels: usize) -> String {
    let pad = "  ".repeat(levels);
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.to_string()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Auth, KeyValue};

    fn post() -> Request {
        Request {
            method: "POST".into(),
            url: "https://api.example.com/users".into(),
            headers: vec![KeyValue::new("Accept", "application/json")],
            body: Body::Raw {
                content: r#"{"name":"Ann","admin":false,"tags":["a"],"boss":null}"#.into(),
                content_type: "application/json".into(),
            },
            auth: Auth::Bearer {
                token: "t0k".into(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn python_requests() {
        assert_eq!(
            generate(&post(), Language::Python),
            r#"import requests

response = requests.post(
    "https://api.example.com/users",
    headers={
        "Accept": "application/json",
        "Authorization": "Bearer t0k",
    },
    json={
        "name": "Ann",
        "admin": False,
        "tags": [
            "a",
        ],
        "boss": None,
    },
)
print(response.status_code)
print(response.text)
"#
        );
    }

    #[test]
    fn javascript_fetch() {
        assert_eq!(
            generate(&post(), Language::JavaScript),
            r#"const response = await fetch("https://api.example.com/users", {
  method: "POST",
  headers: {
    "Accept": "application/json",
    "Content-Type": "application/json",
    "Authorization": "Bearer t0k",
  },
  body: JSON.stringify({
    "name": "Ann",
    "admin": false,
    "tags": [
      "a"
    ],
    "boss": null
  }),
});
console.log(response.status, await response.text());
"#
        );
        let get = Request {
            method: "GET".into(),
            url: "http://x/".into(),
            ..Default::default()
        };
        assert_eq!(
            generate(&get, Language::JavaScript),
            "const response = await fetch(\"http://x/\");\nconsole.log(response.status, await response.text());\n"
        );
    }

    #[test]
    fn forms() {
        let request = Request {
            method: "PURGE".into(),
            url: "http://x/".into(),
            body: Body::FormUrlEncoded {
                fields: vec![KeyValue::new("a", "1")],
            },
            ..Default::default()
        };
        let python = generate(&request, Language::Python);
        assert!(python.contains("requests.request(\n    \"PURGE\",\n    \"http://x/\",\n    data={\n        \"a\": \"1\",\n    },"), "{python}");
        let js = generate(&request, Language::JavaScript);
        assert!(
            js.contains("body: new URLSearchParams([\n    [\"a\", \"1\"],\n  ]),"),
            "{js}"
        );
    }
}
