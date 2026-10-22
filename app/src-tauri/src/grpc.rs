//! gRPC from the app: the methods of a request's server, and calls, one per tab, whose events go to the UI as
//! `grpc-event`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};
use tuma_core::grpc::{self, Call, GrpcEvent, ServiceInfo};
use tuma_core::runner::{self, Job};
use tuma_core::workspace::files::RequestFile;

use crate::workspace::{Open, secrets};

/// Calls in progress, by the id the UI gave them.
#[derive(Default)]
pub struct Calls(Mutex<HashMap<String, Call>>);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EventView<'a> {
    id: &'a str,
    event: &'a GrpcEvent,
}

type Result<T> = std::result::Result<T, String>;

fn job<'a>(
    ws: Option<&'a tuma_core::workspace::Workspace>,
    request: &'a RequestFile,
    path: Option<&'a str>,
    environment: Option<&'a str>,
    network: Option<Arc<tuma_core::http::Network>>,
) -> Job<'a> {
    Job {
        workspace: ws,
        request,
        path,
        context: None,
        environment,
        overrides: &[],
        extra_checks: None,
        secrets: secrets(),
        history: None,
        source: "",
        allow_host: None,
        scripting: Default::default(),
        network,
        observer: None,
    }
}

/// The services and methods of the request's server (reflection) or `.proto` files.
#[tauri::command]
pub async fn grpc_describe(
    open: tauri::State<'_, Open>,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
) -> Result<Vec<ServiceInfo>> {
    let ws = open.0.lock().unwrap().clone();
    let network = tuma_core::settings::network_for(ws.as_ref(), secrets())?;
    let (_, pool, _) = runner::grpc_connection(&job(
        ws.as_ref(),
        &request,
        path.as_deref(),
        environment.as_deref(),
        Some(network),
    ))
    .await
    .map_err(|f| f.message)?;
    Ok(grpc::services(&pool))
}

/// Starts the request's call. Send messages with `grpc_send`, then `grpc_end`.
#[tauri::command]
pub async fn grpc_start(
    app: tauri::AppHandle,
    id: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    open: tauri::State<'_, Open>,
    calls: tauri::State<'_, Calls>,
) -> Result<()> {
    if let Some(mut old) = calls.0.lock().unwrap().remove(&id) {
        old.cancel();
    }
    let ws = open.0.lock().unwrap().clone();
    let network = tuma_core::settings::network_for(ws.as_ref(), secrets())?;
    let (target, pool, settings) = runner::grpc_connection(&job(
        ws.as_ref(),
        &request,
        path.as_deref(),
        environment.as_deref(),
        Some(network),
    ))
    .await
    .map_err(|f| f.message)?;
    let emitter = app.clone();
    let event_id = id.clone();
    let sink: grpc::EventSink = Arc::new(move |event: GrpcEvent| {
        let _ = emitter.emit(
            "grpc-event",
            EventView {
                id: &event_id,
                event: &event,
            },
        );
        if matches!(event, GrpcEvent::Done { .. }) {
            emitter.state::<Calls>().0.lock().unwrap().remove(&event_id);
        }
    });
    let call = grpc::start(&target, &pool, &settings.method, sink).await?;
    calls.0.lock().unwrap().insert(id, call);
    Ok(())
}

/// Sends a JSON message, with `{{variables}}` resolved.
#[tauri::command]
pub fn grpc_send(
    id: String,
    json: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    open: tauri::State<'_, Open>,
    calls: tauri::State<'_, Calls>,
) -> Result<()> {
    let ws = open.0.lock().unwrap().clone();
    let text = runner::resolve_text(
        &job(
            ws.as_ref(),
            &request,
            path.as_deref(),
            environment.as_deref(),
            None,
        ),
        &json,
    )
    .map_err(|f| f.message)?;
    let guard = calls.0.lock().unwrap();
    guard.get(&id).ok_or("The call has ended")?.send(&text)
}

/// Ends the request stream; the server answers and the call finishes.
#[tauri::command]
pub fn grpc_end(id: String, calls: tauri::State<'_, Calls>) {
    if let Some(call) = calls.0.lock().unwrap().get_mut(&id) {
        call.end();
    }
}

#[tauri::command]
pub fn grpc_cancel(id: String, calls: tauri::State<'_, Calls>) {
    let call = calls.0.lock().unwrap().remove(&id);
    if let Some(mut call) = call {
        call.cancel();
    }
}
