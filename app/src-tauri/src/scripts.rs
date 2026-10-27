//! The Scripts and Shell pages: scripts in the workspace's `scripts` folder, run with their output sent to the UI as
//! `script-event` events.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::Emitter;
use tuma_core::scripts::{self, Event, Kind, Outcome, Script, Summary};

use crate::workspace::{Open, secrets};

/// Runs in progress, by the id the UI gave them, so they can be stopped.
#[derive(Default)]
pub struct Running(Mutex<HashMap<String, Arc<AtomicBool>>>);

type Result<T> = std::result::Result<T, String>;

fn with<T>(
    open: &Open,
    f: impl FnOnce(&tuma_core::workspace::Workspace) -> Result<T>,
) -> Result<T> {
    let guard = open.0.lock().unwrap();
    f(guard.as_ref().ok_or("No workspace is open")?)
}

#[tauri::command]
pub fn scripts_list(open: tauri::State<'_, Open>) -> Result<Vec<Summary>> {
    with(&open, scripts::list)
}

#[tauri::command]
pub fn script_read(open: tauri::State<'_, Open>, name: String) -> Result<Script> {
    with(&open, |w| scripts::read(w, &name))
}

#[tauri::command]
pub fn script_write(open: tauri::State<'_, Open>, script: Script) -> Result<()> {
    with(&open, |w| scripts::write(w, &script))
}

#[tauri::command]
pub fn script_create(open: tauri::State<'_, Open>, name: String, kind: Kind) -> Result<String> {
    with(&open, |w| scripts::create(w, &name, kind))
}

#[tauri::command]
pub fn script_rename(
    open: tauri::State<'_, Open>,
    name: String,
    new_name: String,
) -> Result<String> {
    with(&open, |w| scripts::rename(w, &name, &new_name))
}

#[tauri::command]
pub fn script_delete(open: tauri::State<'_, Open>, name: String) -> Result<()> {
    with(&open, |w| scripts::delete(w, &name))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RunEvent<'a> {
    run_id: &'a str,
    event: &'a Event,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn script_run(
    app: tauri::AppHandle,
    run_id: String,
    name: String,
    environment: Option<String>,
    inputs: BTreeMap<String, String>,
    extra_args: String,
    open: tauri::State<'_, Open>,
    running: tauri::State<'_, Running>,
) -> Result<Outcome> {
    let ws = open
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("No workspace is open")?;
    let network = tuma_core::settings::network_for(Some(&ws), secrets())?;
    let stop = Arc::new(AtomicBool::new(false));
    running
        .0
        .lock()
        .unwrap()
        .insert(run_id.clone(), stop.clone());
    let id = run_id.clone();
    let outcome = scripts::run(
        scripts::RunOptions {
            workspace: &ws,
            name: &name,
            environment: environment.as_deref(),
            inputs: &inputs,
            extra_args: &extra_args,
            secrets: secrets(),
            network: Some(network.clone()),
            stop,
        },
        Arc::new(move |event| {
            let _ = app.emit(
                "script-event",
                RunEvent {
                    run_id: &id,
                    event: &event,
                },
            );
        }),
    )
    .await;
    running.0.lock().unwrap().remove(&run_id);
    let _ = tuma_core::settings::keep_cookies(&network, Some(&ws));
    outcome
}

#[tauri::command]
pub fn script_stop(run_id: String, running: tauri::State<'_, Running>) {
    if let Some(stop) = running.0.lock().unwrap().get(&run_id) {
        stop.store(true, Ordering::Relaxed);
    }
}

/// Writes a file a script offered, where the person chose to save it.
#[tauri::command]
pub fn script_save_file(file: PathBuf, content: String) -> Result<()> {
    std::fs::write(&file, content).map_err(|e| format!("could not write {}: {e}", file.display()))
}
