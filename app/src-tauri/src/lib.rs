//! The desktop shell: Tauri commands that call into `tuma_core`. Keep logic out of here.

mod workspace;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;
use tauri::Manager;
use tokio::sync::oneshot;
use tuma_core::history::{Entry, History};
use tuma_core::http::{self, Header, Redirect, SentRequest, Timing};
use tuma_core::workspace::files::RequestFile;
use tuma_core::workspace::prepare_standalone;

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
    /// `{{variables}}` that had no value and were sent as written.
    unresolved: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorView {
    kind: &'static str,
    message: String,
    unresolved: Vec<String>,
}

impl ErrorView {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            unresolved: vec![],
        }
    }
}

/// Sends a request. `path` is where it is saved, so it inherits from its folders; `environment` supplies variables.
#[tauri::command]
async fn send_request(
    id: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    in_flight: tauri::State<'_, InFlight>,
    history: tauri::State<'_, History>,
    open: tauri::State<'_, workspace::Open>,
) -> Result<ResponseView, ErrorView> {
    let sent_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let prepared = match open.0.lock().unwrap().as_ref() {
        Some(ws) => ws
            .prepare(
                &request,
                path.as_deref(),
                environment.as_deref(),
                workspace::secrets(),
            )
            .map_err(|e| ErrorView::new("workspace", e.to_string()))?,
        None => prepare_standalone(&request),
    };
    let unresolved: Vec<String> = prepared.unresolved.into_iter().collect();

    let (cancel, cancelled) = oneshot::channel();
    in_flight.0.lock().unwrap().insert(id.clone(), cancel);
    let result = tokio::select! {
        result = http::send(&prepared.request) => Some(result),
        _ = cancelled => None,
    };
    in_flight.0.lock().unwrap().remove(&id);

    // A cancelled request is not worth remembering.
    let Some(result) = result else {
        return Err(ErrorView::new("cancelled", "Request cancelled"));
    };
    let entry = Entry {
        id,
        sent_at,
        request,
        path,
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

    let response = result.map_err(|e| ErrorView {
        kind: e.kind(),
        message: e.to_string(),
        unresolved: unresolved.clone(),
    })?;
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
        unresolved,
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
        .manage(workspace::Open::default())
        .setup(|app| {
            app.manage(History::new(app.path().app_data_dir()?));
            workspace::restore(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            send_request,
            cancel_request,
            history_list,
            history_clear,
            workspace::workspace_current,
            workspace::workspace_open,
            workspace::workspace_create,
            workspace::workspace_close,
            workspace::tree,
            workspace::request_read,
            workspace::request_write,
            workspace::request_create,
            workspace::folder_read,
            workspace::folder_write,
            workspace::folder_create,
            workspace::item_rename,
            workspace::item_delete,
            workspace::environments,
            workspace::environment_read,
            workspace::environment_write,
            workspace::environment_create,
            workspace::environment_rename,
            workspace::environment_delete,
            workspace::globals_read,
            workspace::globals_write,
            workspace::import_postman,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
