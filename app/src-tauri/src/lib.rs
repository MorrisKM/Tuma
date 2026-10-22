//! The desktop shell: Tauri commands that call into `tuma_core`. Keep logic out of here.

mod grpc;
mod mocks;
mod oauth;
mod perf;
mod runs;
mod settings;
mod sockets;
mod updates;
mod visualizer;
mod workspace;

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::sync::oneshot;
use tuma_core::expect::CheckResult;
use tuma_core::history::{Entry, History};
use tuma_core::http::{Header, Redirect, SentRequest, Timing};
use tuma_core::runner::{self, Job};
use tuma_core::script::{LogLine, ScriptError, TestResult};
use tuma_core::workspace::files::RequestFile;

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
    /// The results of the request's `expect:` checks.
    checks: Vec<CheckResult>,
    /// For a `text/event-stream` response: its events.
    events: Option<Vec<tuma_core::sse::Event>>,
    /// With decryption settings: what was decrypted and the body as it came, or why it could not be decrypted.
    decryption: Option<DecryptionView>,
    #[serde(flatten)]
    scripts: ScriptsView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecryptionView {
    original: Option<String>,
    fields: Vec<(String, usize)>,
    error: Option<String>,
}

/// A server-sent event as it arrives, for the request with this id.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LiveEvent<'a> {
    id: &'a str,
    /// `None` when the stream opened.
    event: Option<tuma_core::sse::Event>,
}

/// What scripts did, for the Tests, Console and Visualize tabs.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ScriptsView {
    tests: Vec<TestResult>,
    script_errors: Vec<ScriptError>,
    console: Vec<LogLine>,
    /// The URL of the rendered `pm.visualizer` template, or why it could not be rendered.
    visualizer: Option<visualizer::Rendered>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorView {
    kind: String,
    message: String,
    unresolved: Vec<String>,
    #[serde(flatten)]
    scripts: ScriptsView,
}

impl ErrorView {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            unresolved: vec![],
            scripts: ScriptsView::default(),
        }
    }
}

/// Sends a request with its scripts. `path` is where it is saved, so it inherits from its folders; `environment`
/// supplies variables.
#[tauri::command]
// Tauri serializes the error straight away, so its size does not matter.
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
async fn send_request(
    app: tauri::AppHandle,
    id: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    in_flight: tauri::State<'_, InFlight>,
    history: tauri::State<'_, History>,
    open: tauri::State<'_, workspace::Open>,
    visualizations: tauri::State<'_, visualizer::Store>,
) -> Result<ResponseView, ErrorView> {
    // A copy, so the workspace is not locked while the request is out.
    let ws = open.0.lock().unwrap().clone();
    let network = tuma_core::settings::network_for(ws.as_ref(), workspace::secrets())
        .map_err(|e| ErrorView::new("settings", format!("network settings: {e}")))?;
    let (cancel, cancelled) = oneshot::channel();
    in_flight.0.lock().unwrap().insert(id.clone(), cancel);
    // Server-sent events go to the UI as they arrive.
    let parser = Mutex::new(tuma_core::sse::Parser::default());
    let observer = |event: tuma_core::http::StreamEvent<'_>| match event {
        tuma_core::http::StreamEvent::Open { .. } => {
            let _ = app.emit(
                "sse-event",
                LiveEvent {
                    id: &id,
                    event: None,
                },
            );
        }
        tuma_core::http::StreamEvent::Data(data) => {
            for event in parser.lock().unwrap().feed(data) {
                let _ = app.emit(
                    "sse-event",
                    LiveEvent {
                        id: &id,
                        event: Some(event),
                    },
                );
            }
        }
    };
    let job = Job {
        workspace: ws.as_ref(),
        request: &request,
        path: path.as_deref(),
        context: None,
        environment: environment.as_deref(),
        overrides: &[],
        extra_checks: None,
        secrets: workspace::secrets(),
        history: Some(&history),
        source: "",
        allow_host: None,
        scripting: Default::default(),
        network: Some(network.clone()),
        observer: Some(&observer),
    };
    let execution = tokio::select! {
        execution = runner::execute(job) => Some(execution),
        _ = cancelled => None,
    };
    in_flight.0.lock().unwrap().remove(&id);
    if let Err(e) = tuma_core::settings::keep_cookies(&network, ws.as_ref()) {
        eprintln!("could not save cookies: {e}");
    }
    // A cancelled request is not worth remembering.
    let Some(execution) = execution else {
        return Err(ErrorView::new("cancelled", "Request cancelled"));
    };

    let scripts = ScriptsView {
        visualizer: execution
            .visualizer
            .as_ref()
            .map(|v| visualizations.render(&id, v)),
        tests: execution.tests,
        script_errors: execution.script_errors,
        console: execution.console,
    };
    let unresolved: Vec<String> = execution.unresolved.into_iter().collect();
    let decryption = execution.decryption.map(|d| match d {
        Ok(d) => DecryptionView {
            original: Some(d.original),
            fields: d.fields,
            error: None,
        },
        Err(e) => DecryptionView {
            original: None,
            fields: vec![],
            error: Some(e),
        },
    });
    let response = match execution.response {
        Ok(response) => response,
        Err(failure) => {
            return Err(ErrorView {
                kind: failure.kind,
                message: failure.message,
                unresolved,
                scripts,
            });
        }
    };
    let events = response
        .headers
        .iter()
        .any(|h| {
            h.name.eq_ignore_ascii_case("content-type")
                && h.value.trim_start().starts_with("text/event-stream")
        })
        .then(|| tuma_core::sse::parse_all(&response.body));
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
        checks: execution.checks,
        events,
        decryption,
        scripts,
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
        .plugin(updates::plugin())
        .manage(InFlight::default())
        .manage(workspace::Open::default())
        .manage(visualizer::Store::default())
        .manage(runs::Running::default())
        .manage(sockets::Sockets::default())
        .manage(grpc::Calls::default())
        .manage(mocks::Mocks::default())
        .manage(perf::Running::default())
        .manage(updates::Pending::default())
        .register_uri_scheme_protocol(visualizer::SCHEME, visualizer::serve)
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
            mocks::mock_start,
            perf::perf_start,
            perf::perf_stop,
            perf::perf_export,
            mocks::mock_stop,
            mocks::mock_list,
            mocks::mock_routes,
            workspace::docs_generate,
            workspace::workspace_list,
            workspace::workspaces_dir_set,
            workspace::workspace_new,
            workspace::workspace_import,
            workspace::workspace_export,
            workspace::item_duplicate,
            workspace::item_delete,
            workspace::environments,
            workspace::environment_read,
            workspace::environment_write,
            workspace::environment_create,
            workspace::environment_rename,
            workspace::environment_delete,
            workspace::globals_read,
            workspace::globals_write,
            workspace::import_file,
            workspace::agent_setup,
            workspace::variables_in_scope,
            workspace::snippet,
            workspace::parse_curl,
            workspace::export_postman,
            workspace::current_values,
            workspace::reset_current_values,
            runs::run_collection,
            oauth::oauth2_status,
            oauth::oauth2_authorize,
            oauth::oauth2_tokens,
            oauth::oauth2_forget,
            oauth::graphql_schema,
            updates::update_status,
            updates::update_check,
            updates::update_install,
            grpc::grpc_describe,
            grpc::grpc_start,
            grpc::grpc_send,
            grpc::grpc_end,
            grpc::grpc_cancel,
            sockets::ws_connect,
            sockets::ws_send,
            sockets::ws_close,
            settings::settings_read,
            settings::settings_write,
            settings::cookies_list,
            settings::cookies_remove,
            settings::cookies_set,
            runs::stop_run,
            runs::runs_list,
            runs::run_read,
            runs::run_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
