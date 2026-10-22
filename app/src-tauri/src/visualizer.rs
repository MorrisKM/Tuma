//! `pm.visualizer.set(template, data)`: Handlebars templates rendered to HTML, as in Postman, and served from their
//! own URL scheme so they get their own content security policy. The UI shows them in a sandboxed iframe: they can
//! run scripts and load libraries from CDNs, but can't reach the app, its IPC or the network with fetch.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::Manager;
use tauri::http::{Request, Response};

pub const SCHEME: &str = "tumaviz";
/// How many visualizations to keep: one per recent response.
const KEEP: usize = 20;

const CSP: &str = "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' https:; style-src 'unsafe-inline' https:; \
                   img-src data: blob: https:; font-src data: https:; connect-src 'none'; form-action 'none'";

#[derive(Default)]
pub struct Store(Mutex<VecDeque<(String, String)>>);

/// A rendered visualization's URL, or why the template could not be rendered.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rendered {
    url: Option<String>,
    error: Option<String>,
}

impl Store {
    /// Renders `{template, data}` from a script for the response with this id.
    pub fn render(&self, id: &str, visualizer: &Value) -> Rendered {
        let template = visualizer["template"].as_str().unwrap_or_default();
        let data = &visualizer["data"];
        let html = match handlebars::Handlebars::new().render_template(template, data) {
            Ok(html) => html,
            Err(e) => {
                return Rendered {
                    url: None,
                    error: Some(format!(
                        "the visualizer template could not be rendered: {e}"
                    )),
                };
            }
        };
        // pm.getData(callback), which templates use to get the data in their own scripts.
        let json = serde_json::to_string(data)
            .unwrap_or_else(|_| "null".into())
            .replace('<', "\\u003c");
        let html = format!(
            "<!doctype html><meta charset=\"utf-8\"><script>window.pm = {{ getData: (cb) => cb(null, {json}) }};</script>\n{html}"
        );
        let id = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>();
        let mut store = self.0.lock().unwrap();
        store.retain(|(i, _)| *i != id);
        store.push_back((id.clone(), html));
        while store.len() > KEEP {
            store.pop_front();
        }
        Rendered {
            url: Some(url(&id)),
            error: None,
        }
    }
}

/// How the webview spells a custom scheme's URL: Windows (WebView2) maps it to http://<scheme>.localhost.
fn url(id: &str) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{id}")
    } else {
        format!("{SCHEME}://localhost/{id}")
    }
}

pub fn serve<R: tauri::Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let id = request.uri().path().trim_start_matches('/');
    let store = ctx.app_handle().state::<Store>();
    let html = store
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|(i, _)| i == id)
        .map(|(_, html)| html.clone());
    let builder = Response::builder()
        .header("Content-Security-Policy", CSP)
        .header("X-Content-Type-Options", "nosniff");
    match html {
        Some(html) => builder
            .header("Content-Type", "text/html; charset=utf-8")
            .body(html.into_bytes()),
        None => builder
            .status(404)
            .header("Content-Type", "text/plain")
            .body(b"This visualization is gone; send the request again.".to_vec()),
    }
    .unwrap_or_default()
}
