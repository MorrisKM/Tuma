//! Automatic updates, from the GitHub release's `latest.json`. Only release builds have them: the update signing key's
//! public half is put in their config (`plugins.updater.pubkey`, see app/scripts/updater-config.mjs), so a build
//! without it (dev, or from source) never updates itself and doesn't offer to.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Whether this build has an update key to check updates against.
fn enabled<R: Runtime>(app: &impl Manager<R>) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}

/// The update found by the last check, ready to install.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Whether this build can update itself.
    enabled: bool,
    current: &'static str,
}

#[tauri::command]
pub fn update_status(app: tauri::AppHandle) -> Status {
    Status {
        enabled: enabled(&app),
        current: tuma_core::VERSION,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    version: String,
    notes: Option<String>,
    date: Option<String>,
}

/// Looks for a newer version. `None` when this is the newest, or the build doesn't update itself.
#[tauri::command]
pub async fn update_check(
    app: tauri::AppHandle,
    pending: tauri::State<'_, Pending>,
) -> Result<Option<Available>, String> {
    if !enabled(&app) {
        return Ok(None);
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("could not check for updates: {e}"))?;
    let available = update.as_ref().map(|u| Available {
        version: u.version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    *pending.0.lock().unwrap() = update;
    Ok(available)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: usize,
    total: Option<u64>,
}

/// Downloads and installs the update the last check found, then restarts into it. Progress goes to the UI as
/// `update-progress`.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    pending: tauri::State<'_, Pending>,
) -> Result<(), String> {
    let update = pending
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or("Check for updates first")?;
    let mut downloaded = 0;
    let emitter = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk;
                let _ = emitter.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("the update could not be installed: {e}"))?;
    app.restart();
}
