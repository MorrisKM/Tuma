//! WebSocket connections from the app, one per tab: events go to the UI as `ws-event`, and messages are sent with
//! their `{{variables}}` resolved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};
use tuma_core::http::websocket::{Outgoing, Socket, WsEvent};
use tuma_core::runner::{self, Job};
use tuma_core::workspace::files::RequestFile;

use crate::workspace::{Open, secrets};

/// Open connections, by the id the UI gave them.
#[derive(Default)]
pub struct Sockets(Mutex<HashMap<String, Socket>>);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EventView<'a> {
    id: &'a str,
    event: &'a WsEvent,
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

#[tauri::command]
pub async fn ws_connect(
    app: tauri::AppHandle,
    id: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    open: tauri::State<'_, Open>,
    sockets: tauri::State<'_, Sockets>,
) -> Result<()> {
    if let Some(old) = sockets.0.lock().unwrap().remove(&id) {
        old.close();
    }
    let ws = open.0.lock().unwrap().clone();
    let network = tuma_core::settings::network_for(ws.as_ref(), secrets())?;
    let emitter = app.clone();
    let event_id = id.clone();
    let sink: tuma_core::http::websocket::EventSink = Arc::new(move |event: WsEvent| {
        let _ = emitter.emit(
            "ws-event",
            EventView {
                id: &event_id,
                event: &event,
            },
        );
        if matches!(event, WsEvent::Closed { .. }) {
            emitter
                .state::<Sockets>()
                .0
                .lock()
                .unwrap()
                .remove(&event_id);
        }
    });
    let socket = runner::open_websocket(
        job(
            ws.as_ref(),
            &request,
            path.as_deref(),
            environment.as_deref(),
            Some(network.clone()),
        ),
        sink,
    )
    .await
    .map_err(|f| f.message)?;
    let _ = tuma_core::settings::keep_cookies(&network, ws.as_ref());
    sockets.0.lock().unwrap().insert(id, socket);
    Ok(())
}

/// Sends a text message, with `{{variables}}` resolved as in the request.
#[tauri::command]
pub fn ws_send(
    id: String,
    text: String,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    open: tauri::State<'_, Open>,
    sockets: tauri::State<'_, Sockets>,
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
        &text,
    )
    .map_err(|f| f.message)?;
    let guard = sockets.0.lock().unwrap();
    let socket = guard.get(&id).ok_or("Not connected")?;
    socket.send(Outgoing::Text(text))
}

#[tauri::command]
pub fn ws_close(id: String, sockets: tauri::State<'_, Sockets>) {
    if let Some(socket) = sockets.0.lock().unwrap().get(&id) {
        socket.close();
    }
}
