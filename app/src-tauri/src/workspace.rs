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

/// What the app keeps about workspaces on this machine, in `settings.json`.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    /// Reopened at startup.
    last_workspace: Option<PathBuf>,
    /// Where new and imported workspaces go; the default when absent.
    workspaces_dir: Option<PathBuf>,
    /// Opened lately, newest first, for the workspace menu.
    recent: Vec<PathBuf>,
}

const RECENT: usize = 10;

fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

fn load(app: &tauri::AppHandle) -> Settings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store(app: &tauri::AppHandle, settings: &Settings) {
    let Some(path) = settings_path(app) else {
        return;
    };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Err(e) = std::fs::write(
        &path,
        serde_json::to_string_pretty(settings).unwrap_or_default(),
    ) {
        eprintln!("could not save settings: {e}");
    }
}

/// Remembers the open workspace, so the app reopens it, and puts it first among the recent ones.
fn remember(app: &tauri::AppHandle, root: Option<PathBuf>) {
    let mut settings = load(app);
    if let Some(root) = &root {
        settings.recent.retain(|r| r != root);
        settings.recent.insert(0, root.clone());
        settings.recent.truncate(RECENT);
    }
    settings.last_workspace = root;
    store(app, &settings);
}

/// `Documents/Tuma Workspaces`, or the home folder's if there is no Documents folder.
fn default_workspaces_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Tuma Workspaces")
}

fn workspaces_dir_of(app: &tauri::AppHandle) -> PathBuf {
    load(app)
        .workspaces_dir
        .unwrap_or_else(default_workspaces_dir)
}

/// Reopens the last workspace at startup, if there was one and it still exists.
pub fn restore(app: &tauri::AppHandle) {
    if let Some(root) = load(app).last_workspace
        && let Ok(workspace) = Workspace::open(root)
    {
        *app.state::<Open>().0.lock().unwrap() = Some(workspace);
    }
}

/// Opens the workspace at `root` and remembers it.
fn open_at(app: &tauri::AppHandle, open: &Open, root: PathBuf) -> Result<Info> {
    let workspace = Workspace::open(&root).map_err(|e| e.to_string())?;
    // Mock servers serve the workspace they were started in.
    app.state::<crate::mocks::Mocks>().stop_all();
    let result = info(&workspace);
    *open.0.lock().unwrap() = Some(workspace);
    remember(app, Some(root));
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    name: String,
    root: String,
    /// In the workspaces folder, rather than only opened from elsewhere.
    in_folder: bool,
}

/// The workspaces folder and the workspaces in it, then recent ones from elsewhere that still exist.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspaces {
    folder: String,
    workspaces: Vec<Listed>,
}

#[tauri::command]
pub fn workspace_list(app: tauri::AppHandle) -> Workspaces {
    let folder = workspaces_dir_of(&app);
    let mut workspaces = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&folder) {
        let mut found: Vec<Listed> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| Workspace::open(e.path()).ok())
            .map(|w| Listed {
                name: w.name().to_string(),
                root: w.root().to_string_lossy().into_owned(),
                in_folder: true,
            })
            .collect();
        found.sort_by_key(|w| w.name.to_lowercase());
        workspaces.extend(found);
    }
    for root in load(&app).recent {
        let root_text = root.to_string_lossy().into_owned();
        if workspaces.iter().any(|w| w.root == root_text) {
            continue;
        }
        if let Ok(w) = Workspace::open(&root) {
            workspaces.push(Listed {
                name: w.name().to_string(),
                root: root_text,
                in_folder: false,
            });
        }
    }
    Workspaces {
        folder: folder.to_string_lossy().into_owned(),
        workspaces,
    }
}

#[tauri::command]
pub fn workspaces_dir_set(app: tauri::AppHandle, folder: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let mut settings = load(&app);
    settings.workspaces_dir = (folder != default_workspaces_dir()).then_some(folder);
    store(&app, &settings);
    Ok(())
}

/// Creates a workspace called `name` in the workspaces folder and opens it.
#[tauri::command]
pub fn workspace_new(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    name: String,
) -> Result<Info> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the workspace a name".into());
    }
    let folder = workspaces_dir_of(&app);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let root = tuma_core::workspace::archive::free_folder(&folder, name);
    Workspace::create(&root, name).map_err(|e| e.to_string())?;
    open_at(&app, &open, root)
}

/// Copies a workspace from a folder or a `.zip` into the workspaces folder and opens the copy.
#[tauri::command]
pub fn workspace_import(
    app: tauri::AppHandle,
    open: tauri::State<'_, Open>,
    source: PathBuf,
) -> Result<Info> {
    let root = tuma_core::workspace::archive::import(&source, &workspaces_dir_of(&app))?;
    open_at(&app, &open, root)
}

/// Writes the open workspace to a `.zip` and returns how many files it holds.
#[tauri::command]
pub fn workspace_export(open: tauri::State<'_, Open>, file: PathBuf) -> Result<usize> {
    let guard = open.0.lock().unwrap();
    let workspace = guard.as_ref().ok_or("No workspace is open")?;
    tuma_core::workspace::archive::export(workspace, &file)
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
    open_at(&app, &open, root)
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
    app.state::<crate::mocks::Mocks>().stop_all();
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

/// Moves a request, folder or collection into `parent` (none: the top level), before the item named `before`.
#[tauri::command]
pub fn item_move(
    open: tauri::State<'_, Open>,
    path: String,
    parent: Option<String>,
    before: Option<String>,
) -> Result<String> {
    with(&open, |w| {
        w.move_item(&path, parent.as_deref(), before.as_deref())
    })
}

#[tauri::command]
pub fn item_duplicate(open: tauri::State<'_, Open>, path: String) -> Result<String> {
    with(&open, |w| w.duplicate(&path))
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
    Collection {
        path: String,
        warnings: Vec<String>,
    },
    Environment {
        name: String,
        warnings: Vec<String>,
    },
    Globals {
        warnings: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Several {
        collections: Vec<String>,
        environments: Vec<String>,
        warnings: Vec<String>,
    },
}

/// Imports a file: an OpenAPI or Swagger spec, a Postman, Insomnia or Bruno export, a HAR file, or a JSON file of
/// names and values as an environment.
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
        Imported::Several(all) => {
            let (mut collections, mut environments) = (Vec::new(), Vec::new());
            for item in all {
                match item {
                    Imported::Collection(path) => collections.push(path),
                    Imported::Environment(name) => environments.push(name),
                    _ => {}
                }
            }
            ImportResult::Several {
                collections,
                environments,
                warnings,
            }
        }
    })
}

#[derive(Serialize)]
pub struct Docs {
    text: String,
    warnings: Vec<String>,
}

/// API docs for a request, folder or collection (`scope`), or the whole workspace; written to `file` when given.
#[tauri::command]
pub fn docs_generate(
    open: tauri::State<'_, Open>,
    scope: Option<String>,
    format: String,
    environment: Option<String>,
    file: Option<PathBuf>,
) -> Result<Docs> {
    let guard = open.0.lock().unwrap();
    let workspace = guard.as_ref().ok_or("Open a workspace first")?;
    let format = tuma_core::docs::Format::parse(&format).ok_or("Unknown format")?;
    let docs = tuma_core::docs::generate(
        workspace,
        scope.as_deref(),
        environment.as_deref(),
        format,
        secrets(),
    )?;
    if let Some(file) = file {
        std::fs::write(&file, &docs.text)
            .map_err(|e| format!("could not write {}: {e}", file.display()))?;
    }
    Ok(Docs {
        text: docs.text,
        warnings: docs.warnings,
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
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(exe))
            .find(|p| p.is_file())
    });
    // Release packages ship the CLI next to the app (on Linux that is /usr/bin, on the PATH already).
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|app| app.parent().map(|dir| dir.join(exe)))
        .filter(|p| p.is_file());
    let cli = on_path
        .or(bundled)
        .map(|p| p.to_string_lossy().into_owned());
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

/// A variable for the URL bar's hints. Secret values are never sent to the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableHint {
    name: String,
    /// Empty for secrets.
    value: String,
    secret: bool,
    source: String,
}

/// The variables a request at `path` (or an unsaved one) can use, highest priority first.
#[tauri::command]
pub fn variables_in_scope(
    open: tauri::State<'_, Open>,
    path: Option<String>,
    environment: Option<String>,
) -> Result<Vec<VariableHint>> {
    let guard = open.0.lock().unwrap();
    let Some(workspace) = guard.as_ref() else {
        return Ok(vec![]);
    };
    // An unsaved request is offered its only collection's variables too, as it gets them once its URL is relative.
    let path = path.or_else(|| {
        tuma_core::runner::sole_collection(workspace).map(|c| format!("{c}/request.yaml"))
    });
    let vars = workspace
        .variables_in_scope(path.as_deref(), environment.as_deref(), secrets())
        .map_err(|e| e.to_string())?;
    Ok(vars
        .into_iter()
        .map(|v| VariableHint {
            value: if v.secret { String::new() } else { v.value },
            name: v.name,
            secret: v.secret,
            source: v.source,
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    code: String,
    /// The code contains the value of a secret variable, so be careful where it is pasted.
    contains_secrets: bool,
    unresolved: Vec<String>,
}

/// The request as code, with variables and inherited auth resolved.
#[tauri::command]
pub fn snippet(
    open: tauri::State<'_, Open>,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
    language: tuma_core::codegen::Language,
) -> Result<Snippet> {
    let file = RequestFile {
        url: tuma_core::runner::absolute_url(&request.url),
        ..request
    };
    let guard = open.0.lock().unwrap();
    let (prepared, secret_values) = match guard.as_ref() {
        Some(ws) => (
            ws.prepare(
                &file,
                path.clone()
                    .or_else(|| tuma_core::runner::implicit_location(ws, &file.url))
                    .as_deref(),
                environment.as_deref(),
                &[],
                secrets(),
            )
            .map_err(|e| e.to_string())?,
            ws.secret_values(environment.as_deref(), secrets())
                .unwrap_or_default(),
        ),
        None => (tuma_core::workspace::prepare_standalone(&file, &[]), vec![]),
    };
    let code = tuma_core::codegen::generate(&prepared.request, language);
    Ok(Snippet {
        contains_secrets: secret_values.iter().any(|s| code.contains(s.as_str())),
        code,
        unresolved: prepared.unresolved.into_iter().collect(),
    })
}

/// Reads a cURL command into a request. Options Tuma ignores are returned so the UI can mention them.
#[tauri::command]
pub fn parse_curl(command: String) -> Result<(RequestFile, Vec<String>)> {
    tuma_core::curl::parse(&command)
}

/// Writes a collection as Postman v2.1 JSON to `file`.
#[tauri::command]
pub fn export_postman(
    open: tauri::State<'_, Open>,
    collection: String,
    file: PathBuf,
) -> Result<()> {
    let tree = with(&open, |w| w.read_tree(&collection))?;
    let json = serde_json::to_string_pretty(&tuma_core::export::postman::collection(&tree))
        .map_err(|e| e.to_string())?;
    std::fs::write(&file, json + "\n").map_err(|e| format!("{}: {e}", file.display()))
}

/// What scripts have set on this machine, by scope.
#[tauri::command]
pub fn current_values(open: tauri::State<'_, Open>) -> Result<tuma_core::workspace::CurrentValues> {
    with(&open, |w| Ok(w.current_values()))
}

/// A scope to reset: `environment` or `collection` with a name, or `globals`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeArg {
    kind: String,
    name: Option<String>,
}

/// Forgets current values, going back to the values in the files: for one scope, or all of them.
#[tauri::command]
pub fn reset_current_values(open: tauri::State<'_, Open>, scope: Option<ScopeArg>) -> Result<()> {
    use tuma_core::workspace::Scope;
    with(&open, |w| {
        let scope = scope
            .as_ref()
            .map(|s| match (s.kind.as_str(), s.name.as_deref()) {
                ("environment", Some(name)) => Scope::Environment(name),
                ("collection", Some(name)) => Scope::Collection(name),
                _ => Scope::Globals,
            });
        w.reset_current_values(scope)
    })
}
