//! Performance tests from the app: progress goes to the UI as `perf-progress` events, about twice a second, and the
//! finished report can be written as HTML or JSON.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::watch;
use tuma_core::perf::{self, Options, Report};

use crate::workspace::{Open, secrets};

/// Tests in progress, by the id the UI gave them, so they can be stopped.
#[derive(Default)]
pub struct Running(Mutex<HashMap<String, watch::Sender<bool>>>);

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    run_id: &'a str,
    report: &'a Report,
}

#[tauri::command]
pub async fn perf_start(
    app: tauri::AppHandle,
    run_id: String,
    scope: String,
    environment: Option<String>,
    options: Options,
    open: tauri::State<'_, Open>,
    running: tauri::State<'_, Running>,
) -> Result<Report> {
    let ws = open
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("Open a workspace first")?;
    let network = tuma_core::settings::network_for(Some(&ws), secrets())?;
    let (targets, notes) = perf::prepare(
        &ws,
        Some(&scope),
        environment.as_deref(),
        secrets(),
        Some(&network),
    )
    .await?;
    let (stop, stopped) = watch::channel(false);
    running.0.lock().unwrap().insert(run_id.clone(), stop);
    let events_app = app.clone();
    let events_id = run_id.clone();
    let name = scope.trim_end_matches(".yaml").to_string();
    let report = perf::run(
        &name,
        targets,
        options,
        environment,
        notes,
        network.clone(),
        Arc::new(move |report| {
            let _ = events_app.emit(
                "perf-progress",
                Progress {
                    run_id: &events_id,
                    report: &report,
                },
            );
        }),
        stopped,
    )
    .await;
    running.0.lock().unwrap().remove(&run_id);
    if let Err(e) = tuma_core::settings::keep_cookies(&network, Some(&ws)) {
        eprintln!("could not save cookies: {e}");
    }
    Ok(report)
}

#[tauri::command]
pub fn perf_stop(running: tauri::State<'_, Running>, run_id: String) {
    if let Some(stop) = running.0.lock().unwrap().get(&run_id) {
        let _ = stop.send(true);
    }
}

/// Writes a finished test's report as HTML, or as JSON when the file ends in .json.
#[tauri::command]
pub fn perf_export(report: Report, file: PathBuf) -> Result<()> {
    let text = if file.extension().is_some_and(|e| e == "json") {
        perf::report::json(&report)
    } else {
        perf::report::html(&report)
    };
    std::fs::write(&file, text).map_err(|e| format!("could not write {}: {e}", file.display()))
}
