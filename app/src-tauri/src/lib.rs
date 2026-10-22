//! The desktop shell: Tauri commands that call into `tuma_core`. Keep logic out of here.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;
use tauri::Manager;
use tokio::sync::oneshot;
use tuma_core::history::{Entry, History};
use tuma_core::http::{self, Header, Redirect, Request, SentRequest, Timing};

/// How many history entries the sidebar shows.
const HISTORY_LIMIT: usize = 500;

#[tauri::command]
fn app_version() -> &'static str {
    tuma_core::VERSION
}

/// Requests in flight, by the id the UI gave them, so they can be cancelled.
#[derive(Default)]
struct InFlight(Mutex<HashMap<String, oneshot::Sender<()>>>);

/// A response as the UI needs it. The body is text when it is valid UTF-8, base64 otherwise.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseView {
    status: u16,
    reason: String,
    version: String,
    url: String,
    remote_addr: Option<String>,
    headers: Vec<Header>,
    body: String,
    body_base64: bool,
    size: usize,
    encoded_size: usize,
    timing: Timing,
    redirects: Vec<Redirect>,
    sent: SentRequest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorView {
    kind: &'static str,
    message: String,
}

impl From<http::Error> for ErrorView {
    fn from(error: http::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

#[tauri::command]
async fn send_request(
    id: String,
    request: Request,
    in_flight: tauri::State<'_, InFlight>,
    history: tauri::State<'_, History>,
) -> Result<ResponseView, ErrorView> {
    let sent_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let (cancel, cancelled) = oneshot::channel();
    in_flight.0.lock().unwrap().insert(id.clone(), cancel);

    let result = tokio::select! {
        result = http::send(&request) => Some(result),
        _ = cancelled => None,
    };
    in_flight.0.lock().unwrap().remove(&id);

    // A cancelled request is not worth remembering.
    let Some(result) = result else {
        return Err(ErrorView {
            kind: "cancelled",
            message: "Request cancelled".into(),
        });
    };
    let entry = Entry {
        id,
        sent_at,
        request,
        status: result.as_ref().ok().map(|r| r.status),
        error: result.as_ref().err().map(|e| e.to_string()),
        total_ms: result
            .as_ref()
            .ok()
            .map(|r| r.timing.total.as_secs_f64() * 1000.0),
        size: result.as_ref().ok().map(|r| r.body.len()),
    };
    // Failing to record history must not fail the request.
    if let Err(error) = history.append(&entry) {
        eprintln!("could not save history: {error}");
    }

    let response = result?;
    let (body, body_base64) = match std::str::from_utf8(&response.body) {
        Ok(text) => (text.to_string(), false),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(&response.body),
            true,
        ),
    };
    Ok(ResponseView {
        status: response.status,
        reason: response.reason,
        version: response.version,
        url: response.url,
        remote_addr: response.remote_addr.map(|a| a.to_string()),
        headers: response.headers,
        size: response.body.len(),
        body,
        body_base64,
        encoded_size: response.encoded_size,
        timing: response.timing,
        redirects: response.redirects,
        sent: response.sent,
    })
}

#[tauri::command]
fn cancel_request(id: String, in_flight: tauri::State<'_, InFlight>) {
    if let Some(cancel) = in_flight.0.lock().unwrap().remove(&id) {
        let _ = cancel.send(());
    }
}

#[tauri::command]
fn history_list(history: tauri::State<'_, History>) -> Result<Vec<Entry>, String> {
    history.recent(HISTORY_LIMIT).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_clear(history: tauri::State<'_, History>) -> Result<(), String> {
    history.clear().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(InFlight::default())
        .setup(|app| {
            app.manage(History::new(app.path().app_data_dir()?));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            send_request,
            cancel_request,
            history_list,
            history_clear
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
