//! Mock servers started from the app: each serves a collection, folder or the whole workspace from its saved
//! examples, and reports the requests it answers to the UI as `mock-event`. They stop when the workspace changes.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};
use tuma_core::mock::{self, MockEvent, MockServer};

use crate::workspace::Open;

struct Running {
    info: MockInfo,
    server: MockServer,
}

#[derive(Default)]
pub struct Mocks(Mutex<Vec<Running>>);

impl Mocks {
    /// Stops every mock server, as when another workspace opens.
    pub fn stop_all(&self) {
        for running in self.0.lock().unwrap().drain(..) {
            running.server.stop();
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MockInfo {
    id: String,
    /// A collection's or folder's path; null for the whole workspace.
    scope: Option<String>,
    url: String,
    port: u16,
    delay_ms: u64,
}

#[derive(Serialize, Clone)]
struct EventView<'a> {
    id: &'a str,
    event: &'a MockEvent,
}

type Result<T> = std::result::Result<T, String>;

#[tauri::command]
pub async fn mock_start(
    app: tauri::AppHandle,
    scope: Option<String>,
    port: u16,
    delay_ms: u64,
) -> Result<MockInfo> {
    let root = {
        let open = app.state::<Open>();
        let guard = open.0.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open a workspace first")?
            .root()
            .to_path_buf()
    };
    let mocks = app.state::<Mocks>();
    if let Some(running) = mocks
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.info.scope == scope)
    {
        return Err(format!(
            "A mock server for this is already running at {}",
            running.info.url
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let events_app = app.clone();
    let events_id = id.clone();
    let server = mock::start(
        mock::Options {
            root,
            scope: scope.clone(),
            port,
            delay_ms,
        },
        Arc::new(move |event| {
            let _ = events_app.emit(
                "mock-event",
                EventView {
                    id: &events_id,
                    event: &event,
                },
            );
        }),
    )
    .await?;
    let info = MockInfo {
        id,
        scope,
        url: server.url(),
        port: server.addr.port(),
        delay_ms,
    };
    mocks.0.lock().unwrap().push(Running {
        info: info.clone(),
        server,
    });
    Ok(info)
}

#[tauri::command]
pub fn mock_stop(mocks: tauri::State<'_, Mocks>, id: String) {
    let mut running = mocks.0.lock().unwrap();
    if let Some(i) = running.iter().position(|r| r.info.id == id) {
        running.remove(i).server.stop();
    }
}

#[tauri::command]
pub fn mock_list(mocks: tauri::State<'_, Mocks>) -> Vec<MockInfo> {
    mocks
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.info.clone())
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    name: String,
    path: String,
    method: String,
    pattern: String,
    examples: Vec<String>,
}

/// What a mock server for `scope` answers: each request, its path pattern and its examples.
#[tauri::command]
pub fn mock_routes(open: tauri::State<'_, Open>, scope: Option<String>) -> Result<Vec<RouteView>> {
    let guard = open.0.lock().unwrap();
    let workspace = guard.as_ref().ok_or("Open a workspace first")?;
    Ok(mock::routes(workspace, scope.as_deref())?
        .into_iter()
        .map(|r| RouteView {
            pattern: r.pattern(),
            examples: r
                .examples
                .iter()
                .map(|e| format!("{} {}", e.status, e.name))
                .collect(),
            name: r.name,
            path: r.path,
            method: r.method,
        })
        .collect())
}
