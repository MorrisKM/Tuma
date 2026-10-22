//! Collection runs from the app: progress goes to the UI as `run-step` events, and each finished run is saved in the
//! data folder so past runs can be opened and exported.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager};
use tuma_core::history::History;
use tuma_core::runner::{Detail, Summary};
use tuma_core::runs::{self, Report, RunOptions, Step};

use crate::workspace::{Open, secrets};

/// How many past runs to keep per workspace.
const KEEP: usize = 50;

/// Runs in progress, by the id the UI gave them, so they can be stopped.
#[derive(Default)]
pub struct Running(Mutex<HashMap<String, Arc<AtomicBool>>>);

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StepEvent<'a> {
    run_id: &'a str,
    step: &'a Step,
}

fn runs_dir(app: &tauri::AppHandle, open: &Open) -> Result<PathBuf> {
    let guard = open.0.lock().unwrap();
    let ws = guard.as_ref().ok_or("No workspace is open")?;
    let id: String = ws
        .id()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("runs").join(id))
}

/// A saved run's file name, refusing anything that is not one.
fn run_file(dir: &std::path::Path, name: &str) -> Result<PathBuf> {
    let valid = name
        .strip_suffix(".json")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err(format!("not a saved run: {name}"));
    }
    Ok(dir.join(name))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    /// The saved run's file name, for run_read and run_export.
    file: Option<String>,
    report: Report,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn run_collection(
    app: tauri::AppHandle,
    run_id: String,
    path: String,
    environment: Option<String>,
    iterations: usize,
    delay_ms: u64,
    data_file: Option<PathBuf>,
    only_tests: bool,
    open: tauri::State<'_, Open>,
    running: tauri::State<'_, Running>,
    history: tauri::State<'_, History>,
) -> Result<Finished> {
    let ws = open
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("No workspace is open")?;
    let data = match &data_file {
        Some(file) => runs::read_data_file(file)?,
        None => vec![],
    };
    let network = tuma_core::settings::network_for(Some(&ws), secrets())?;
    let stop = Arc::new(AtomicBool::new(false));
    running
        .0
        .lock()
        .unwrap()
        .insert(run_id.clone(), stop.clone());
    let emitter = app.clone();
    let id = run_id.clone();
    let report = runs::run(
        RunOptions {
            workspace: &ws,
            path: &path,
            environment: environment.as_deref(),
            overrides: &[],
            secrets: secrets(),
            history: Some(&history),
            source: "",
            allow_host: None,
            network: Some(network.clone()),
            iterations,
            data,
            delay: Duration::from_millis(delay_ms),
            only_tests,
            fail_fast: false,
            scripts: true,
            cancel: Some(&stop),
            detail: Detail {
                max_body: Some(20_000),
                ..Default::default()
            },
        },
        &mut move |step| {
            let _ = emitter.emit("run-step", StepEvent { run_id: &id, step });
        },
    )
    .await;
    running.0.lock().unwrap().remove(&run_id);
    let _ = tuma_core::settings::keep_cookies(&network, Some(&ws));
    let report = report.map_err(|e| e.to_string())?;

    // Saving is a convenience; a run that could not be saved still shows its results.
    let file = runs_dir(&app, &open).ok().and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        let name = format!("{}.json", report.started_at);
        std::fs::write(dir.join(&name), serde_json::to_string(&report).ok()?).ok()?;
        prune(&dir);
        Some(name)
    });
    Ok(Finished { file, report })
}

/// Keeps the newest runs only.
fn prune(dir: &std::path::Path) {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(KEEP);
    for name in &names[..excess] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

#[tauri::command]
pub fn stop_run(run_id: String, running: tauri::State<'_, Running>) {
    if let Some(stop) = running.0.lock().unwrap().get(&run_id) {
        stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastRun {
    file: String,
    path: String,
    environment: Option<String>,
    iterations: usize,
    started_at: u64,
    duration_ms: u64,
    summary: Summary,
    stopped: Option<String>,
}

/// The saved runs of the open workspace, newest first.
#[tauri::command]
pub fn runs_list(app: tauri::AppHandle, open: tauri::State<'_, Open>) -> Result<Vec<PastRun>> {
    let dir = runs_dir(&app, &open)?;
    let mut out: Vec<PastRun> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path()).ok()?;
            let report: Report = serde_json::from_str(&text).ok()?;
            Some(PastRun {
                file,
                path: report.path,
                environment: report.environment,
                iterations: report.iterations,
                started_at: report.started_at,
                duration_ms: report.duration_ms,
                summary: report.summary,
                stopped: report.stopped,
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    Ok(out)
}

#[tauri::command]
pub fn run_read(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    file: String,
) -> Result<Report> {
    let path = run_file(&runs_dir(&app, &open)?, &file)?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Writes a saved run to `target` as JUnit XML (`junit`) or JSON (`json`).
#[tauri::command]
pub fn run_export(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    file: String,
    target: PathBuf,
    format: String,
) -> Result<()> {
    let report = run_read(app, open, file)?;
    let name = if report.path.is_empty() {
        "Tuma run"
    } else {
        &report.path
    };
    let contents = match format.as_str() {
        "junit" => runs::junit(&report, name),
        "json" => serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n",
        other => return Err(format!("unknown format {other}")),
    };
    std::fs::write(&target, contents).map_err(|e| format!("{}: {e}", target.display()))
}
