//! A workspace is a folder of YAML files (see docs/PLAN.md, "On-disk format"):
//!
//! ```text
//! my-workspace/
//! ├── workspace.yaml            name, id, global variables
//! ├── environments/local.yaml   variables; secret values are in the OS keychain
//! └── Users API/                a collection: a top-level folder
//!     ├── _folder.yaml          optional: order, auth, headers, variables, description
//!     ├── Admin/                a folder, which can have its own _folder.yaml
//!     └── create user.yaml      a request
//! ```
//!
//! Paths passed in and out are relative to the workspace root and use `/`, such as `Users API/create user.yaml`.

pub mod files;

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::http;
use crate::secrets::SecretStore;
use crate::vars::Scopes;
use files::{
    Auth, EnvironmentFile, FolderFile, KeyValue, RequestFile, Variable, WorkspaceFile, to_yaml,
};

const WORKSPACE_FILE: &str = "workspace.yaml";
const FOLDER_FILE: &str = "_folder.yaml";
const ENVIRONMENTS: &str = "environments";
const EXTENSION: &str = "yaml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} is not a Tuma workspace (no workspace.yaml)", .0.display())]
    NotAWorkspace(PathBuf),
    #[error("{} already contains a workspace", .0.display())]
    AlreadyAWorkspace(PathBuf),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error("invalid path \"{0}\"")]
    InvalidPath(String),
    #[error("\"{0}\" already exists")]
    Exists(String),
    #[error("could not write YAML: {0}")]
    Serialize(String),
    #[error("keychain: {0}")]
    Secrets(String),
}

type Result<T> = std::result::Result<T, Error>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// An item in the sidebar tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub kind: NodeKind,
    pub name: String,
    pub path: String,
    /// Requests only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Collection,
    Folder,
    Request,
}

/// A folder and everything in it, held in memory. Importers build these and [`Workspace::add_collection`] writes them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderTree {
    pub name: String,
    pub folder: FolderFile,
    pub items: Vec<TreeItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeItem {
    Folder(FolderTree),
    Request { name: String, file: RequestFile },
}

/// A variable as the UI edits it: secret values are filled in from the keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableValue {
    pub name: String,
    pub value: String,
    pub secret: bool,
    pub enabled: bool,
}

/// A request ready to send, and the variables that had no value.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub request: http::Request,
    pub unresolved: BTreeSet<String>,
}

pub struct Workspace {
    root: PathBuf,
    file: WorkspaceFile,
}

impl Workspace {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let path = root.join(WORKSPACE_FILE);
        if !path.is_file() {
            return Err(Error::NotAWorkspace(root));
        }
        let file = read_yaml(&path)?;
        Ok(Self { root, file })
    }

    /// Makes `root` a workspace. The folder may already exist and hold other files, such as a code repository.
    pub fn create(root: impl Into<PathBuf>, name: &str) -> Result<Self> {
        let root = root.into();
        if root.join(WORKSPACE_FILE).exists() {
            return Err(Error::AlreadyAWorkspace(root));
        }
        fs::create_dir_all(&root).map_err(io_err(&root))?;
        let file = WorkspaceFile {
            name: name.to_string(),
            id: uuid::Uuid::new_v4().to_string(),
            variables: vec![],
        };
        write_yaml(&root.join(WORKSPACE_FILE), &file)?;
        Ok(Self { root, file })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.file.name
    }

    /// Turns a relative path from the UI into an absolute one, refusing anything that would leave the workspace.
    fn abs(&self, rel: &str) -> Result<PathBuf> {
        let path = Path::new(rel);
        if rel.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(Error::InvalidPath(rel.to_string()));
        }
        Ok(self.root.join(path))
    }

    fn rel(&self, abs: &Path) -> String {
        let rel = abs.strip_prefix(&self.root).unwrap_or(abs);
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    // Tree

    /// The collections, with their folders and requests in display order.
    pub fn tree(&self) -> Result<Vec<Node>> {
        let mut collections = Vec::new();
        for entry in list_dir(&self.root)? {
            let name = dir_name(&entry);
            if entry.is_dir() && !name.starts_with('.') && name != ENVIRONMENTS {
                collections.push(self.folder_node(&entry, NodeKind::Collection)?);
            }
        }
        collections.sort_by_key(|n| n.name.to_lowercase());
        Ok(collections)
    }

    fn folder_node(&self, dir: &Path, kind: NodeKind) -> Result<Node> {
        let folder: FolderFile = read_yaml_or_default(&dir.join(FOLDER_FILE))?;
        let mut children = Vec::new();
        for entry in list_dir(dir)? {
            let file_name = dir_name(&entry);
            if file_name.starts_with('.') {
                continue;
            }
            if entry.is_dir() {
                children.push(self.folder_node(&entry, NodeKind::Folder)?);
            } else if file_name != FOLDER_FILE && entry.extension().is_some_and(|e| e == EXTENSION)
            {
                let request: RequestFile = read_yaml(&entry)?;
                children.push(Node {
                    kind: NodeKind::Request,
                    name: stem(&entry),
                    path: self.rel(&entry),
                    method: Some(request.method),
                    children: vec![],
                });
            }
        }
        children.sort_by_key(|n| {
            let position = folder
                .order
                .iter()
                .position(|o| *o == n.name)
                .unwrap_or(usize::MAX);
            (position, n.name.to_lowercase())
        });
        Ok(Node {
            kind,
            name: dir_name(dir),
            path: self.rel(dir),
            method: None,
            children,
        })
    }

    // Requests and folders

    pub fn read_request(&self, path: &str) -> Result<RequestFile> {
        read_yaml(&self.abs(path)?)
    }

    pub fn write_request(&self, path: &str, request: &RequestFile) -> Result<()> {
        write_yaml(&self.abs(path)?, request)
    }

    /// The settings of a collection or folder. A missing `_folder.yaml` reads as empty.
    pub fn read_folder(&self, path: &str) -> Result<FolderFile> {
        read_yaml_or_default(&self.abs(path)?.join(FOLDER_FILE))
    }

    pub fn write_folder(&self, path: &str, folder: &FolderFile) -> Result<()> {
        let file = self.abs(path)?.join(FOLDER_FILE);
        if *folder == FolderFile::default() {
            return remove_if_exists(&file);
        }
        write_yaml(&file, folder)
    }

    /// Creates a request in the folder at `parent` and returns its path. The name gets a number if it is taken.
    pub fn create_request(
        &self,
        parent: &str,
        name: &str,
        request: &RequestFile,
    ) -> Result<String> {
        let dir = self.abs(parent)?;
        let name = unique_name(&dir, &file_name(name))?;
        let path = dir.join(format!("{name}.{EXTENSION}"));
        write_yaml(&path, request)?;
        self.append_to_order(&dir, &name)?;
        Ok(self.rel(&path))
    }

    /// Creates a folder inside `parent`, or a collection when `parent` is `None`, and returns its path.
    pub fn create_folder(&self, parent: Option<&str>, name: &str) -> Result<String> {
        let dir = match parent {
            Some(parent) => self.abs(parent)?,
            None => self.root.clone(),
        };
        let mut name = unique_name(&dir, &file_name(name))?;
        if parent.is_none() && name == ENVIRONMENTS {
            name = unique_name(&dir, &format!("{name} collection"))?;
        }
        let path = dir.join(&name);
        fs::create_dir(&path).map_err(io_err(&path))?;
        if parent.is_some() {
            self.append_to_order(&dir, &name)?;
        }
        Ok(self.rel(&path))
    }

    /// Renames a request, folder or collection and returns its new path. The position in its folder is kept.
    pub fn rename(&self, path: &str, new_name: &str) -> Result<String> {
        let old = self.abs(path)?;
        let dir = old.parent().unwrap_or(&self.root).to_path_buf();
        let is_request = old.is_file();
        let old_name = if is_request {
            stem(&old)
        } else {
            dir_name(&old)
        };
        let new_name = file_name(new_name);
        if new_name == old_name {
            return Ok(path.to_string());
        }
        if name_taken(&dir, &new_name)? && !new_name.eq_ignore_ascii_case(&old_name) {
            return Err(Error::Exists(new_name));
        }
        let new = if is_request {
            dir.join(format!("{new_name}.{EXTENSION}"))
        } else {
            dir.join(&new_name)
        };
        fs::rename(&old, &new).map_err(io_err(&old))?;
        if dir != self.root {
            let dir_rel = self.rel(&dir);
            let mut folder = self.read_folder(&dir_rel)?;
            for entry in folder.order.iter_mut().filter(|o| **o == old_name) {
                *entry = new_name.clone();
            }
            self.write_folder(&dir_rel, &folder)?;
        }
        Ok(self.rel(&new))
    }

    /// Deletes a request, or a folder or collection with everything in it.
    pub fn delete(&self, path: &str) -> Result<()> {
        let target = self.abs(path)?;
        let dir = target.parent().unwrap_or(&self.root).to_path_buf();
        let name = if target.is_file() {
            fs::remove_file(&target).map_err(io_err(&target))?;
            stem(&target)
        } else {
            fs::remove_dir_all(&target).map_err(io_err(&target))?;
            dir_name(&target)
        };
        if dir != self.root {
            let dir_rel = self.rel(&dir);
            let mut folder = self.read_folder(&dir_rel)?;
            folder.order.retain(|o| *o != name);
            self.write_folder(&dir_rel, &folder)?;
        }
        Ok(())
    }

    fn append_to_order(&self, dir: &Path, name: &str) -> Result<()> {
        let file = dir.join(FOLDER_FILE);
        let mut folder: FolderFile = read_yaml_or_default(&file)?;
        // An empty order means "alphabetical"; start it from the current display order so nothing moves.
        if folder.order.is_empty() {
            folder.order = self
                .folder_node(dir, NodeKind::Folder)?
                .children
                .into_iter()
                .map(|n| n.name)
                .filter(|n| n != name)
                .collect();
        }
        folder.order.push(name.to_string());
        write_yaml(&file, &folder)
    }

    /// Writes an in-memory collection, such as an import, and returns its path.
    pub fn add_collection(&self, tree: &FolderTree) -> Result<String> {
        let path = self.create_folder(None, &tree.name)?;
        self.write_tree(&self.abs(&path)?, tree)?;
        Ok(path)
    }

    fn write_tree(&self, dir: &Path, tree: &FolderTree) -> Result<()> {
        let mut folder = tree.folder.clone();
        folder.order.clear();
        for item in &tree.items {
            match item {
                TreeItem::Folder(child) => {
                    let name = unique_name(dir, &file_name(&child.name))?;
                    let path = dir.join(&name);
                    fs::create_dir(&path).map_err(io_err(&path))?;
                    self.write_tree(&path, child)?;
                    folder.order.push(name);
                }
                TreeItem::Request { name, file } => {
                    let name = unique_name(dir, &file_name(name))?;
                    write_yaml(&dir.join(format!("{name}.{EXTENSION}")), file)?;
                    folder.order.push(name);
                }
            }
        }
        if folder != FolderFile::default() {
            write_yaml(&dir.join(FOLDER_FILE), &folder)?;
        }
        Ok(())
    }

    // Environments and globals

    pub fn environments(&self) -> Result<Vec<String>> {
        let dir = self.root.join(ENVIRONMENTS);
        if !dir.is_dir() {
            return Ok(vec![]);
        }
        let mut names: Vec<String> = list_dir(&dir)?
            .into_iter()
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == EXTENSION))
            .map(|p| stem(&p))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    fn environment_path(&self, name: &str) -> Result<PathBuf> {
        let file = format!("{}.{EXTENSION}", file_name(name));
        if file_name(name) != name {
            return Err(Error::InvalidPath(name.to_string()));
        }
        Ok(self.root.join(ENVIRONMENTS).join(file))
    }

    fn secret_key(&self, scope: &str, name: &str) -> String {
        format!("{}/{scope}/{name}", self.file.id)
    }

    fn with_secrets(
        &self,
        scope: &str,
        vars: &[Variable],
        secrets: &dyn SecretStore,
    ) -> Result<Vec<VariableValue>> {
        vars.iter()
            .map(|v| {
                let value = if v.secret {
                    secrets
                        .get(&self.secret_key(scope, &v.name))
                        .map_err(Error::Secrets)?
                        .unwrap_or_default()
                } else {
                    v.value.clone()
                };
                Ok(VariableValue {
                    name: v.name.clone(),
                    value,
                    secret: v.secret,
                    enabled: v.enabled,
                })
            })
            .collect()
    }

    /// Splits variables into what goes in the file and what goes in the keychain, and removes keychain entries that
    /// are no longer secret variables.
    fn store_secrets(
        &self,
        scope: &str,
        old: &[Variable],
        new: &[VariableValue],
        secrets: &dyn SecretStore,
    ) -> Result<Vec<Variable>> {
        for v in new.iter().filter(|v| v.secret) {
            secrets
                .set(&self.secret_key(scope, &v.name), &v.value)
                .map_err(Error::Secrets)?;
        }
        for v in old.iter().filter(|v| v.secret) {
            if !new.iter().any(|n| n.secret && n.name == v.name) {
                secrets
                    .delete(&self.secret_key(scope, &v.name))
                    .map_err(Error::Secrets)?;
            }
        }
        Ok(new
            .iter()
            .map(|v| Variable {
                name: v.name.clone(),
                value: if v.secret {
                    String::new()
                } else {
                    v.value.clone()
                },
                secret: v.secret,
                enabled: v.enabled,
            })
            .collect())
    }

    pub fn read_environment(
        &self,
        name: &str,
        secrets: &dyn SecretStore,
    ) -> Result<Vec<VariableValue>> {
        let file: EnvironmentFile = read_yaml(&self.environment_path(name)?)?;
        self.with_secrets(&format!("environments/{name}"), &file.variables, secrets)
    }

    /// Creates or replaces an environment.
    pub fn write_environment(
        &self,
        name: &str,
        vars: &[VariableValue],
        secrets: &dyn SecretStore,
    ) -> Result<()> {
        let path = self.environment_path(name)?;
        let old: EnvironmentFile = read_yaml_or_default(&path)?;
        let variables = self.store_secrets(
            &format!("environments/{name}"),
            &old.variables,
            vars,
            secrets,
        )?;
        write_yaml(&path, &EnvironmentFile { variables })
    }

    pub fn create_environment(&self, name: &str) -> Result<String> {
        let dir = self.root.join(ENVIRONMENTS);
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        let name = unique_name(&dir, &file_name(name))?;
        write_yaml(
            &dir.join(format!("{name}.{EXTENSION}")),
            &EnvironmentFile::default(),
        )?;
        Ok(name)
    }

    pub fn rename_environment(
        &self,
        name: &str,
        new_name: &str,
        secrets: &dyn SecretStore,
    ) -> Result<String> {
        let new_name = file_name(new_name);
        if new_name == name {
            return Ok(new_name);
        }
        let new_path = self.environment_path(&new_name)?;
        if new_path.exists() {
            return Err(Error::Exists(new_name));
        }
        let vars = self.read_environment(name, secrets)?;
        self.write_environment(&new_name, &vars, secrets)?;
        self.delete_environment(name, secrets)?;
        Ok(new_name)
    }

    pub fn delete_environment(&self, name: &str, secrets: &dyn SecretStore) -> Result<()> {
        let path = self.environment_path(name)?;
        let old: EnvironmentFile = read_yaml_or_default(&path)?;
        self.store_secrets(
            &format!("environments/{name}"),
            &old.variables,
            &[],
            secrets,
        )?;
        remove_if_exists(&path)
    }

    pub fn read_globals(&self, secrets: &dyn SecretStore) -> Result<Vec<VariableValue>> {
        self.with_secrets("globals", &self.file.variables, secrets)
    }

    pub fn write_globals(
        &mut self,
        vars: &[VariableValue],
        secrets: &dyn SecretStore,
    ) -> Result<()> {
        let variables = self.store_secrets("globals", &self.file.variables, vars, secrets)?;
        let file = WorkspaceFile {
            variables,
            ..self.file.clone()
        };
        write_yaml(&self.root.join(WORKSPACE_FILE), &file)?;
        self.file = file;
        Ok(())
    }

    // Sending

    /// Builds the request to send: auth and headers inherited from folders, then `{{variables}}` resolved from the
    /// environment, the folders (nearest first), the collection and the globals, in that order of priority.
    ///
    /// `path` is where the request is saved, if it is; an unsaved request only sees the environment and globals.
    pub fn prepare(
        &self,
        request: &RequestFile,
        path: Option<&str>,
        environment: Option<&str>,
        secrets: &dyn SecretStore,
    ) -> Result<Prepared> {
        // The request's folders, nearest first, ending with the collection.
        let mut folders = Vec::new();
        if let Some(path) = path {
            let mut dir = self.abs(path)?.parent().map(Path::to_path_buf);
            while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
                folders.push(read_yaml_or_default::<FolderFile>(&d.join(FOLDER_FILE))?);
                dir = d.parent().map(Path::to_path_buf);
            }
        }

        let enabled = |vars: Vec<VariableValue>| {
            vars.into_iter()
                .filter(|v| v.enabled)
                .map(|v| (v.name, v.value))
                .collect::<Vec<_>>()
        };
        let mut scopes = Scopes::default();
        if let Some(environment) = environment {
            scopes.push(enabled(self.read_environment(environment, secrets)?));
        }
        for folder in &folders {
            scopes.push(
                folder
                    .variables
                    .iter()
                    .filter(|v| v.enabled)
                    .map(|v| (v.name.clone(), v.value.clone())),
            );
        }
        scopes.push(enabled(self.read_globals(secrets)?));

        let auth = request
            .auth
            .as_ref()
            .or_else(|| folders.iter().find_map(|f| f.auth.as_ref()));
        let inherited: Vec<&KeyValue> = folders.iter().rev().flat_map(|f| &f.headers).collect();
        let (request, unresolved) = scopes.resolve_request(&to_http(request, auth, &inherited));
        Ok(Prepared {
            request,
            unresolved,
        })
    }
}

/// Builds the request to send when no workspace is open: no inheritance, and only dynamic variables such as
/// `{{$guid}}`.
pub fn prepare_standalone(request: &RequestFile) -> Prepared {
    let (request, unresolved) =
        Scopes::default().resolve_request(&to_http(request, request.auth.as_ref(), &[]));
    Prepared {
        request,
        unresolved,
    }
}

/// Converts a request file to an engine request, given the auth and headers it inherits.
pub fn to_http(
    request: &RequestFile,
    auth: Option<&Auth>,
    inherited_headers: &[&KeyValue],
) -> http::Request {
    // A header set on the request replaces an inherited one with the same name.
    let own = |name: &str| {
        request
            .headers
            .iter()
            .any(|h| h.enabled && h.name.eq_ignore_ascii_case(name))
    };
    let headers = inherited_headers
        .iter()
        .filter(|h| !own(&h.name))
        .map(|h| (*h).into())
        .chain(request.headers.iter().map(Into::into))
        .collect();
    http::Request {
        method: request.method.clone(),
        url: request.url.clone(),
        headers,
        body: (&request.body).into(),
        auth: auth.map(Into::into).unwrap_or_default(),
        settings: request.settings.clone(),
    }
}

/// Makes a name safe as a file or folder name on every OS.
pub fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    // Windows does not allow names ending in a dot or space.
    let cleaned = cleaned.trim().trim_end_matches('.').trim_end().to_string();
    if cleaned.is_empty() {
        return "Untitled".to_string();
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if RESERVED.contains(&base.as_str()) || cleaned.eq_ignore_ascii_case("_folder") {
        return format!("{cleaned} (1)");
    }
    cleaned
}

/// Whether a folder or request called `name` exists in `dir`. Case-insensitive, for macOS and Windows file systems.
fn name_taken(dir: &Path, name: &str) -> Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    Ok(list_dir(dir)?.iter().any(|p| {
        let existing = if p.is_dir() { dir_name(p) } else { stem(p) };
        existing.eq_ignore_ascii_case(name)
    }))
}

fn unique_name(dir: &Path, name: &str) -> Result<String> {
    if !name_taken(dir, name)? {
        return Ok(name.to_string());
    }
    for n in 2.. {
        let candidate = format!("{name} ({n})");
        if !name_taken(dir, &candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn list_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    fs::read_dir(dir)
        .map_err(io_err(dir))?
        .map(|e| e.map(|e| e.path()).map_err(io_err(dir)))
        .collect()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(io_err(path))?;
    serde_saphyr::from_str(&text).map_err(|e| Error::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

fn read_yaml_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if path.exists() {
        read_yaml(path)
    } else {
        Ok(T::default())
    }
}

/// Writes through a temporary file and a rename, so nothing ever sees a half-written file.
fn write_yaml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = to_yaml(value).map_err(|e| Error::Serialize(e.to_string()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let tmp = path.with_extension("yaml.tmp");
    fs::write(&tmp, text).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(io_err(path))
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(Error::Io {
            path: path.to_path_buf(),
            source: e,
        }),
        _ => Ok(()),
    }
}
