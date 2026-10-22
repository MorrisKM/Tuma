//! The docs model as Markdown: a contents list, then each folder's requests with their parameters and examples.

use std::fmt::Write;

use super::{Api, Endpoint, Kind, Param};

pub fn render(api: &Api) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", api.title);
    if !api.description.is_empty() {
        let _ = writeln!(out, "{}\n", api.description.trim());
    }
    if !api.servers.is_empty() {
        let _ = writeln!(
            out,
            "**Base URL{}:** {}\n",
            if api.servers.len() > 1 { "s" } else { "" },
            api.servers
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    // Contents.
    let _ = writeln!(out, "## Contents\n");
    for group in &api.groups {
        if !group.name.is_empty() {
            let _ = writeln!(out, "- **{}**", group.name);
        }
        for e in &group.endpoints {
            let indent = if group.name.is_empty() { "" } else { "  " };
            let _ = writeln!(
                out,
                "{indent}- [{} `{}`](#{})",
                e.name,
                signature(e),
                anchor(&e.name)
            );
        }
    }
    out.push('\n');
    for group in &api.groups {
        if !group.name.is_empty() {
            let _ = writeln!(out, "## {}\n", group.name);
            if !group.description.is_empty() {
                let _ = writeln!(out, "{}\n", group.description.trim());
            }
        }
        for e in &group.endpoints {
            endpoint(&mut out, e);
        }
    }
    out.trim_end().to_string() + "\n"
}

fn signature(e: &Endpoint) -> String {
    match e.kind {
        Kind::Http => format!("{} {}", e.method, e.path),
        Kind::WebSocket => format!("WS {}", e.path),
        Kind::Grpc => format!("gRPC {}", e.grpc_method.as_deref().unwrap_or(&e.path)),
    }
}

/// GitHub's heading anchors: lower case, spaces as dashes, most punctuation dropped.
fn anchor(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn endpoint(out: &mut String, e: &Endpoint) {
    let _ = writeln!(out, "### {}\n", e.name);
    let _ = writeln!(out, "`{}`\n", signature(e));
    if !e.description.is_empty() {
        let _ = writeln!(out, "{}\n", e.description.trim());
    }
    if let Some(auth) = &e.auth {
        let _ = writeln!(out, "**Auth:** {}\n", auth.describe());
    }
    table(out, "Path parameters", &e.path_params);
    table(out, "Query parameters", &e.query);
    table(out, "Headers", &e.headers);
    if let Some(body) = &e.body {
        let _ = writeln!(out, "**Request body** (`{}`)\n", body.content_type);
        code(out, &body.content_type, &body.example);
    }
    if e.responses.is_empty() {
        if e.kind == Kind::Http {
            let _ = writeln!(out, "_No example responses saved yet._\n");
        }
    } else {
        let _ = writeln!(out, "**Responses**\n");
        for r in &e.responses {
            let _ = writeln!(out, "- `{}` {}", r.status, r.name);
        }
        out.push('\n');
        for r in e.responses.iter().filter(|r| r.body.is_some()) {
            let body = r.body.as_ref().unwrap();
            let _ = writeln!(
                out,
                "<details><summary>{} {} (<code>{}</code>)</summary>\n",
                r.status,
                html_escape(&r.name),
                body.content_type
            );
            code(out, &body.content_type, &body.example);
            let _ = writeln!(out, "</details>\n");
        }
    }
}

fn table(out: &mut String, title: &str, params: &[Param]) {
    if params.is_empty() {
        return;
    }
    let _ = writeln!(out, "**{title}**\n\n| Name | Example |\n| --- | --- |");
    for p in params {
        let example = if p.example.is_empty() {
            String::new()
        } else {
            format!("`{}`", p.example.replace('|', "\\|"))
        };
        let _ = writeln!(out, "| `{}` | {} |", p.name, example);
    }
    out.push('\n');
}

fn code(out: &mut String, content_type: &str, text: &str) {
    let lang = if content_type.contains("json") {
        "json"
    } else if content_type.contains("xml") {
        "xml"
    } else if content_type.contains("html") {
        "html"
    } else {
        ""
    };
    // A fence longer than any run of backticks in the text.
    let fence = "`".repeat(3.max(text.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1));
    let _ = writeln!(out, "{fence}{lang}\n{}\n{fence}\n", text.trim_end());
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
