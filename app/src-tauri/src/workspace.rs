//! Commands for the open workspace: the tree, requests, folders, environments and imports. Each one locks the
//! workspace for its duration; they are all quick file operations.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tuma_core::import::{self, Imported};
use tuma_core::secrets::{Keychain, SecretStore};
use tuma_core::workspace::files::{FolderFile, RequestFile};
use tuma_core::workspace::{Node, VariableValue, Workspace};

#[derive(Default)]
pub struct Open(pub Mutex<Option<Workspace>>);

pub fn secrets() -> &'static dyn SecretStore {
    &Keychain
}

type Result<T> = std::result::Result<T, String>;

/// Runs `f` on the open workspace, or fails if none is open.
fn with<T>(
    open: &Open,
    f: impl FnOnce(&mut Workspace) -> std::result::Result<T, tuma_core::workspace::Error>,
) -> Result<T> {
    let mut guard = open.0.lock().unwrap();
    let workspace = guard.as_mut().ok_or("No workspace is open")?;
    f(workspace).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    name: String,
    root: String,
}

fn info(workspace: &Workspace) -> Info {
    Info {
        name: workspace.name().to_string(),
        root: workspace.root().to_string_lossy().into_owned(),
    }
}

/// Remembers the last workspace so the app reopens it.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    last_workspace: Option<PathBuf>,
}

fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

fn remember(app: &tauri::AppHandle, root: Option<PathBuf>) {
    let Some(path) = settings_path(app) else {
        return;
    };
    let settings = Settings {
        last_workspace: root,
    };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Err(e) = std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    ) {
        eprintln!("could not save settings: {e}");
    }
}

/// Reopens the last workspace at startup, if there was one and it still exists.
pub fn restore(app: &tauri::AppHandle) {
    let Some(path) = settings_path(app) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let settings: Settings = serde_json::from_str(&text).unwrap_or_default();
    if let Some(root) = settings.last_workspace
        && let Ok(workspace) = Workspace::open(root)
    {
        *app.state::<Open>().0.lock().unwrap() = Some(workspace);
    }
}

#[tauri::command]
pub fn workspace_current(open: tauri::State<'_, Open>) -> Option<Info> {
    open.0.lock().unwrap().as_ref().map(info)
}

#[tauri::command]
pub fn workspace_open(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    root: PathBuf,
) -> Result<Info> {
    // The folder itself, a project with a .tuma/ workspace, or any folder inside a workspace, as the CLI finds them.
    let root = tuma_core::runner::find_workspace(&root).unwrap_or(root);
    let workspace = Workspace::open(&root).map_err(|e| e.to_string())?;
    let result = info(&workspace);
    *open.0.lock().unwrap() = Some(workspace);
    remember(&app, Some(root));
    Ok(result)
}

#[tauri::command]
pub fn workspace_create(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    root: PathBuf,
    name: String,
) -> Result<Info> {
    let workspace = Workspace::create(&root, &name).map_err(|e| e.to_string())?;
    let result = info(&workspace);
    *open.0.lock().unwrap() = Some(workspace);
    remember(&app, Some(root));
    Ok(result)
}

#[tauri::command]
pub fn workspace_close(app: tauri::AppHandle, open: tauri::State<'_, Open>) {
    *open.0.lock().unwrap() = None;
    remember(&app, None);
}

#[tauri::command]
pub fn tree(open: tauri::State<'_, Open>) -> Result<Vec<Node>> {
    with(&open, |w| w.tree())
}

#[tauri::command]
pub fn request_read(open: tauri::State<'_, Open>, path: String) -> Result<RequestFile> {
    with(&open, |w| w.read_request(&path))
}

#[tauri::command]
pub fn request_write(
    open: tauri::State<'_, Open>,
    path: String,
    request: RequestFile,
) -> Result<()> {
    with(&open, |w| w.write_request(&path, &request))
}

#[tauri::command]
pub fn request_create(
    open: tauri::State<'_, Open>,
    parent: String,
    name: String,
    request: RequestFile,
) -> Result<String> {
    with(&open, |w| w.create_request(&parent, &name, &request))
}

#[tauri::command]
pub fn folder_read(open: tauri::State<'_, Open>, path: String) -> Result<FolderFile> {
    with(&open, |w| w.read_folder(&path))
}

#[tauri::command]
pub fn folder_write(open: tauri::State<'_, Open>, path: String, folder: FolderFile) -> Result<()> {
    with(&open, |w| w.write_folder(&path, &folder))
}

/// Creates a folder in `parent`, or a collection when `parent` is null.
#[tauri::command]
pub fn folder_create(
    open: tauri::State<'_, Open>,
    parent: Option<String>,
    name: String,
) -> Result<String> {
    with(&open, |w| w.create_folder(parent.as_deref(), &name))
}

#[tauri::command]
pub fn item_rename(open: tauri::State<'_, Open>, path: String, name: String) -> Result<String> {
    with(&open, |w| w.rename(&path, &name))
}

#[tauri::command]
pub fn item_delete(open: tauri::State<'_, Open>, path: String) -> Result<()> {
    with(&open, |w| w.delete(&path))
}

#[tauri::command]
pub fn environments(open: tauri::State<'_, Open>) -> Result<Vec<String>> {
    with(&open, |w| w.environments())
}

#[tauri::command]
pub fn environment_read(open: tauri::State<'_, Open>, name: String) -> Result<Vec<VariableValue>> {
    with(&open, |w| w.read_environment(&name, secrets()))
}

#[tauri::command]
pub fn environment_write(
    open: tauri::State<'_, Open>,
    name: String,
    variables: Vec<VariableValue>,
) -> Result<()> {
    with(&open, |w| w.write_environment(&name, &variables, secrets()))
}

#[tauri::command]
pub fn environment_create(open: tauri::State<'_, Open>, name: String) -> Result<String> {
    with(&open, |w| w.create_environment(&name))
}

#[tauri::command]
pub fn environment_rename(
    open: tauri::State<'_, Open>,
    name: String,
    new_name: String,
) -> Result<String> {
    with(&open, |w| w.rename_environment(&name, &new_name, secrets()))
}

#[tauri::command]
pub fn environment_delete(open: tauri::State<'_, Open>, name: String) -> Result<()> {
    with(&open, |w| w.delete_environment(&name, secrets()))
}

#[tauri::command]
pub fn globals_read(open: tauri::State<'_, Open>) -> Result<Vec<VariableValue>> {
    with(&open, |w| w.read_globals(secrets()))
}

#[tauri::command]
pub fn globals_write(open: tauri::State<'_, Open>, variables: Vec<VariableValue>) -> Result<()> {
    with(&open, |w| w.write_globals(&variables, secrets()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImportResult {
    Collection { path: String, warnings: Vec<String> },
    Environment { name: String, warnings: Vec<String> },
    Globals { warnings: Vec<String> },
}

/// Imports a file: an OpenAPI or Swagger spec, a Postman collection, environment or globals export, or a HAR file.
#[tauri::command]
pub fn import_file(open: tauri::State<'_, Open>, file: PathBuf) -> Result<ImportResult> {
    let json = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut guard = open.0.lock().unwrap();
    let workspace = guard.as_mut().ok_or("Open a workspace first")?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let source = tuma_core::openapi::source_for(&file.to_string_lossy(), &cwd, workspace.root());
    let (imported, warnings) = import::any(workspace, &json, &source, secrets())?;
    let warnings = warnings.0;
    Ok(match imported {
        Imported::Collection(path) => ImportResult::Collection { path, warnings },
        Imported::Environment(name) => ImportResult::Environment { name, warnings },
        Imported::Globals => ImportResult::Globals { warnings },
    })
}

/// What the "Connect an agent" view needs: whether the `tuma` command is installed, and where.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSetup {
    /// The `tuma` executable found on PATH, if any.
    cli: Option<String>,
    /// The open workspace's project folder, where agents should run `tuma mcp`.
    project: Option<String>,
}

#[tauri::command]
pub fn agent_setup(open: tauri::State<'_, Open>) -> AgentSetup {
    let exe = if cfg!(windows) { "tuma.exe" } else { "tuma" };
    let cli = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(exe))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    });
    let project = open.0.lock().unwrap().as_ref().map(|w| {
        let root = w.root();
        let project = if root.ends_with(tuma_core::runner::HIDDEN_WORKSPACE) {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        project.to_string_lossy().into_owned()
    });
    AgentSetup { cli, project }
}
