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

pub mod archive;
pub mod current;
pub mod files;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::http;
use crate::secrets::SecretStore;
use crate::vars::Scopes;
pub use current::{Changes, CurrentValues, Scope};
use files::{
    Auth, EnvironmentFile, FolderFile, KeyValue, RequestFile, Variable, WorkspaceFile, to_yaml,
};

const WORKSPACE_FILE: &str = "workspace.yaml";
const FOLDER_FILE: &str = "_folder.yaml";
const ENVIRONMENTS: &str = "environments";
/// Not a collection either: the Scripts page's scripts.
const SCRIPTS: &str = crate::scripts::DIR;
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
// Trees are built only for imports and exports, so the size of a request in one doesn't matter.
#[allow(clippy::large_enum_variant)]
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

/// A variable as a request sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedVariable {
    pub name: String,
    pub value: String,
    pub secret: bool,
    /// Such as "environment Staging", "folder Admin", "collection Users API" or "globals".
    pub source: String,
}

/// A request ready to send, and the variables that had no value.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub request: http::Request,
    pub unresolved: BTreeSet<String>,
    /// The request's encryption settings with the key and IV's variables filled in.
    pub crypto: Option<PreparedCrypto>,
}

#[derive(Debug, Clone)]
pub struct PreparedCrypto {
    pub settings: crate::crypto::CryptoSettings,
    pub key: String,
    pub iv: String,
    pub private_key: String,
}

impl PreparedCrypto {
    pub fn cipher(&self) -> std::result::Result<crate::crypto::Cipher, String> {
        crate::crypto::Cipher::with_private_key(
            &self.settings,
            &self.key,
            &self.iv,
            &self.private_key,
        )
    }
}

/// The key and IV of `request`'s encryption settings, resolved with `scopes`.
fn prepare_crypto(
    request: &RequestFile,
    scopes: &Scopes,
    unresolved: &mut BTreeSet<String>,
) -> Option<PreparedCrypto> {
    let settings = request.crypto.as_ref().filter(|c| !c.is_empty())?;
    Some(PreparedCrypto {
        key: scopes.resolve(&settings.key, unresolved),
        iv: scopes.resolve(&settings.iv, unresolved),
        private_key: scopes.resolve(&settings.private_key, unresolved),
        settings: settings.clone(),
    })
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    file: WorkspaceFile,
    /// Where this machine's current values are kept; see [`current`].
    local_dir: PathBuf,
}

/// A collection or folder a request is in, for its scripts.
#[derive(Debug, Clone)]
pub struct Ancestor {
    pub name: String,
    /// True for the collection, false for a folder in it.
    pub collection: bool,
    pub folder: FolderFile,
}

/// Variable scopes that exist only while sending: they win over the environment, in this order.
#[derive(Debug, Clone, Copy, Default)]
pub struct Layers<'a> {
    /// One-off values, such as `tuma send --var`.
    pub overrides: &'a [(String, String)],
    /// `pm.variables.set` values.
    pub local: Option<&'a BTreeMap<String, String>>,
    /// The current row of a collection run's data file.
    pub data: Option<&'a BTreeMap<String, String>>,
}

impl Workspace {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let path = root.join(WORKSPACE_FILE);
        if !path.is_file() {
            return Err(Error::NotAWorkspace(root));
        }
        let file = read_yaml(&path)?;
        let local_dir = current::default_local_dir(&root);
        Ok(Self {
            root,
            file,
            local_dir,
        })
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
            order: vec![],
        };
        write_yaml(&root.join(WORKSPACE_FILE), &file)?;
        let local_dir = current::default_local_dir(&root);
        Ok(Self {
            root,
            file,
            local_dir,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.file.name
    }

    /// The workspace's id, the same in every clone of it.
    pub fn id(&self) -> &str {
        &self.file.id
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
            if entry.is_dir() && !name.starts_with('.') && name != ENVIRONMENTS && name != SCRIPTS {
                collections.push(self.folder_node(&entry, NodeKind::Collection)?);
            }
        }
        let order = self.order_of(&self.root)?;
        collections.sort_by_key(|n| {
            let position = order
                .iter()
                .position(|o| *o == n.name)
                .unwrap_or(usize::MAX);
            (position, n.name.to_lowercase())
        });
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

    /// Reads a file that is not a request or folder file, such as a collection's spec copy.
    pub fn read_file(&self, path: &str) -> Result<String> {
        let abs = self.abs(path)?;
        fs::read_to_string(&abs).map_err(io_err(&abs))
    }

    /// Writes a file that is not a request or folder file. It must not end in `.yaml`, which would make it a request.
    pub fn write_file(&self, path: &str, contents: &str) -> Result<()> {
        if path.ends_with(".yaml") {
            return Err(Error::InvalidPath(path.to_string()));
        }
        let abs = self.abs(path)?;
        let tmp = abs.with_extension("tmp");
        fs::write(&tmp, contents).map_err(io_err(&tmp))?;
        fs::rename(&tmp, &abs).map_err(io_err(&abs))
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
        if parent.is_none() && (name == ENVIRONMENTS || name == SCRIPTS) {
            name = unique_name(&dir, &format!("{name} collection"))?;
        }
        let path = dir.join(&name);
        fs::create_dir(&path).map_err(io_err(&path))?;
        // Collections stay alphabetical until they are arranged; then a new one goes last.
        if parent.is_some() || !self.order_of(&dir)?.is_empty() {
            self.append_to_order(&dir, &name)?;
        }
        Ok(self.rel(&path))
    }

    /// Makes sure a collection and folder path such as `Users API/Admin` exists, creating what is missing, and
    /// returns it with names made safe.
    pub fn ensure_folder(&self, path: &str) -> Result<String> {
        let mut current: Option<String> = None;
        for segment in path.split('/').map(str::trim).filter(|s| !s.is_empty()) {
            let name = file_name(segment);
            let candidate = match &current {
                Some(parent) => format!("{parent}/{name}"),
                None => name.clone(),
            };
            if !self.abs(&candidate)?.is_dir() {
                self.create_folder(current.as_deref(), &name)?;
            }
            current = Some(candidate);
        }
        current.ok_or_else(|| Error::InvalidPath(path.to_string()))
    }

    /// Saves a request at `Collection/Folder/Name`, creating folders as needed. An existing request with that name is
    /// replaced only when `overwrite` is set. Returns the request's path.
    pub fn save_request_at(
        &self,
        path: &str,
        request: &RequestFile,
        overwrite: bool,
    ) -> Result<String> {
        let path = path.trim_matches('/').trim_end_matches(".yaml");
        let (folder, name) = path.rsplit_once('/').ok_or_else(|| {
            Error::InvalidPath(format!(
                "{path} (give a collection and a name, such as \"Users API/create user\")"
            ))
        })?;
        let folder = self.ensure_folder(folder)?;
        let file = format!("{folder}/{}.{EXTENSION}", file_name(name));
        if self.abs(&file)?.is_file() {
            if !overwrite {
                return Err(Error::Exists(file));
            }
            self.write_request(&file, request)?;
            return Ok(file);
        }
        self.create_request(&folder, name, request)
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
        let mut order = self.order_of(&dir)?;
        if order.contains(&old_name) {
            for entry in order.iter_mut().filter(|o| **o == old_name) {
                *entry = new_name.clone();
            }
            self.set_order(&dir, order)?;
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
        let mut order = self.order_of(&dir)?;
        if order.contains(&name) {
            order.retain(|o| *o != name);
            self.set_order(&dir, order)?;
        }
        Ok(())
    }

    /// Copies a request, or a folder or collection with everything in it, next to the original as "<name> copy"
    /// (numbered if that is taken), and returns the copy's path. The copy comes right after the original.
    pub fn duplicate(&self, path: &str) -> Result<String> {
        let source = self.abs(path)?;
        let dir = source.parent().unwrap_or(&self.root).to_path_buf();
        let is_request = source.is_file();
        let original = if is_request {
            stem(&source)
        } else {
            dir_name(&source)
        };
        let name = unique_name(&dir, &format!("{original} copy"))?;
        let copy = if is_request {
            let file = self.read_request(path)?;
            let copy = dir.join(format!("{name}.{EXTENSION}"));
            write_yaml(&copy, &file)?;
            copy
        } else {
            let tree = self.read_tree(path)?;
            let copy = dir.join(&name);
            fs::create_dir(&copy).map_err(io_err(&copy))?;
            self.write_tree(&copy, &tree)?;
            copy
        };
        // Where there is an order, the copy goes right after the original; otherwise it sorts by name.
        let mut order = self.order_of(&dir)?;
        if let Some(i) = order.iter().position(|o| *o == original) {
            order.insert(i + 1, name);
            self.set_order(&dir, order)?;
        }
        Ok(self.rel(&copy))
    }

    fn append_to_order(&self, dir: &Path, name: &str) -> Result<()> {
        let mut order = self.order_of(dir)?;
        // An empty order means "alphabetical"; start it from the current display order so nothing moves.
        if order.is_empty() {
            order = self.display_order(dir)?;
            order.retain(|n| n != name);
        }
        order.push(name.to_string());
        self.set_order(dir, order)
    }

    /// The names of what is in `dir` as they are shown: the collections for the workspace root.
    fn display_order(&self, dir: &Path) -> Result<Vec<String>> {
        let nodes = if dir == self.root {
            self.tree()?
        } else {
            self.folder_node(dir, NodeKind::Folder)?.children
        };
        Ok(nodes.into_iter().map(|n| n.name).collect())
    }

    /// The saved order of what is in `dir`: `_folder.yaml`'s, or `workspace.yaml`'s for the collections. Empty means
    /// alphabetical.
    fn order_of(&self, dir: &Path) -> Result<Vec<String>> {
        Ok(if dir == self.root {
            read_yaml_or_default::<WorkspaceFile>(&self.root.join(WORKSPACE_FILE))?.order
        } else {
            read_yaml_or_default::<FolderFile>(&dir.join(FOLDER_FILE))?.order
        })
    }

    fn set_order(&self, dir: &Path, order: Vec<String>) -> Result<()> {
        if dir == self.root {
            let file = self.root.join(WORKSPACE_FILE);
            let mut workspace: WorkspaceFile = read_yaml(&file)?;
            workspace.order = order;
            write_yaml(&file, &workspace)
        } else {
            let rel = self.rel(dir);
            let mut folder = self.read_folder(&rel)?;
            folder.order = order;
            self.write_folder(&rel, &folder)
        }
    }

    /// Moves a request, folder or collection into the folder or collection `parent` (`None` for the top level, where
    /// a folder becomes a collection), before the item named `before` there, or last. Returns its new path, which has
    /// a number added if the name is taken in `parent`.
    pub fn move_item(
        &self,
        path: &str,
        parent: Option<&str>,
        before: Option<&str>,
    ) -> Result<String> {
        let source = self.abs(path)?;
        let is_request = source.is_file();
        let from = source.parent().unwrap_or(&self.root).to_path_buf();
        let to = match parent {
            Some(parent) => self.abs(parent)?,
            None if is_request => {
                return Err(Error::InvalidPath(format!(
                    "{path} (a request goes in a collection or folder)"
                )));
            }
            None => self.root.clone(),
        };
        if !to.is_dir() || to.starts_with(&source) {
            return Err(Error::InvalidPath(format!(
                "{path} can't move into {}",
                parent.unwrap_or("the top level")
            )));
        }
        let old_name = if is_request {
            stem(&source)
        } else {
            dir_name(&source)
        };
        let name = if to == from {
            old_name.clone()
        } else {
            let mut name = unique_name(&to, &old_name)?;
            if to == self.root && (name == ENVIRONMENTS || name == SCRIPTS) {
                name = unique_name(&to, &format!("{name} collection"))?;
            }
            name
        };
        if name == before.unwrap_or_default() {
            return Ok(path.to_string());
        }

        // Where it goes, in the order the destination is shown, before it changes.
        let mut order = self.display_order(&to)?;
        order.retain(|n| *n != old_name || to != from);
        let at = before
            .and_then(|b| order.iter().position(|n| n == b))
            .unwrap_or(order.len());
        order.insert(at, name.clone());

        let target = if is_request {
            to.join(format!("{name}.{EXTENSION}"))
        } else {
            to.join(&name)
        };
        if target != source {
            fs::rename(&source, &target).map_err(io_err(&source))?;
            let mut left = self.order_of(&from)?;
            if left.contains(&old_name) {
                left.retain(|n| *n != old_name);
                self.set_order(&from, left)?;
            }
        }
        self.set_order(&to, order)?;
        Ok(self.rel(&target))
    }

    /// Reads a collection or folder and everything in it, in display order. The opposite of
    /// [`Workspace::add_collection`]; exporters use it.
    pub fn read_tree(&self, path: &str) -> Result<FolderTree> {
        let dir = self.abs(path)?;
        let node = self.folder_node(&dir, NodeKind::Folder)?;
        self.tree_from(&node)
    }

    fn tree_from(&self, node: &Node) -> Result<FolderTree> {
        let mut items = Vec::new();
        for child in &node.children {
            items.push(match child.kind {
                NodeKind::Request => TreeItem::Request {
                    name: child.name.clone(),
                    file: self.read_request(&child.path)?,
                },
                _ => TreeItem::Folder(self.tree_from(child)?),
            });
        }
        Ok(FolderTree {
            name: node.name.clone(),
            folder: self.read_folder(&node.path)?,
            items,
        })
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

    /// Every variable a request at `path` (or an unsaved one) can use, highest priority first, each name once:
    /// what `{{name}}` resolves to and where it comes from. Secret values are included; callers decide whether to
    /// show them.
    pub fn variables_in_scope(
        &self,
        path: Option<&str>,
        environment: Option<&str>,
        secrets: &dyn SecretStore,
    ) -> Result<Vec<ScopedVariable>> {
        let mut out: Vec<ScopedVariable> = Vec::new();
        let mut add = |vars: Vec<VariableValue>, source: String| {
            for v in vars.into_iter().filter(|v| v.enabled) {
                if !out.iter().any(|o| o.name == v.name) {
                    out.push(ScopedVariable {
                        name: v.name,
                        value: v.value,
                        secret: v.secret,
                        source: source.clone(),
                    });
                }
            }
        };
        if let Some(environment) = environment {
            add(
                self.effective_environment(environment, secrets)?,
                format!("environment {environment}"),
            );
        }
        if let Some(path) = path {
            let mut dir = self.abs(path)?.parent().map(Path::to_path_buf);
            while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
                let folder: FolderFile = read_yaml_or_default(&d.join(FOLDER_FILE))?;
                let kind = if d.parent() == Some(self.root.as_path()) {
                    "collection"
                } else {
                    "folder"
                };
                add(
                    self.effective_folder_vars(&d, &folder),
                    format!("{kind} {}", dir_name(&d)),
                );
                dir = d.parent().map(Path::to_path_buf);
            }
        }
        add(self.effective_globals(secrets)?, "globals".into());
        Ok(out)
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
            // Arranging collections writes the order straight to the file; keep it.
            order: self.order_of(&self.root)?,
            ..self.file.clone()
        };
        write_yaml(&self.root.join(WORKSPACE_FILE), &file)?;
        self.file = file;
        Ok(())
    }

    /// The values of the secret variables a request could use, so output can mask them. Very short values are left
    /// out: masking every "1" would make output unreadable and protect nothing.
    pub fn secret_values(
        &self,
        environment: Option<&str>,
        secrets: &dyn SecretStore,
    ) -> Result<Vec<String>> {
        let mut vars = self.read_globals(secrets)?;
        if let Some(environment) = environment {
            vars.extend(self.read_environment(environment, secrets)?);
        }
        Ok(vars
            .into_iter()
            .filter(|v| v.secret && v.value.len() >= 4)
            .map(|v| v.value)
            .collect())
    }

    // Sending

    /// Builds the request to send: auth and headers inherited from folders, then `{{variables}}` resolved from the
    /// environment, the folders (nearest first), the collection and the globals, in that order of priority.
    ///
    /// `path` is where the request is saved, if it is; an unsaved request only sees the environment and globals.
    /// `overrides` are one-off values, such as `tuma send --var`, that win over every scope.
    pub fn prepare(
        &self,
        request: &RequestFile,
        path: Option<&str>,
        environment: Option<&str>,
        overrides: &[(String, String)],
        secrets: &dyn SecretStore,
    ) -> Result<Prepared> {
        let layers = Layers {
            overrides,
            ..Default::default()
        };
        self.prepare_with(request, path, environment, &layers, secrets)
    }

    /// The collection and folders a request at `path` is in, collection first.
    pub fn ancestors(&self, path: &str) -> Result<Vec<Ancestor>> {
        let mut out = Vec::new();
        let mut dir = self.abs(path)?.parent().map(Path::to_path_buf);
        while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
            out.push(Ancestor {
                name: dir_name(&d),
                collection: d.parent() == Some(self.root.as_path()),
                folder: read_yaml_or_default(&d.join(FOLDER_FILE))?,
            });
            dir = d.parent().map(Path::to_path_buf);
        }
        out.reverse();
        Ok(out)
    }

    /// The variables scripts see for a request at `path` (or an unsaved one), with current values applied, and the
    /// secret ones by name for `pm.vault`.
    pub fn script_vars(
        &self,
        path: Option<&str>,
        environment: Option<&str>,
        secrets: &dyn SecretStore,
    ) -> Result<(crate::script::Vars, BTreeMap<String, String>)> {
        let map = |vars: &[VariableValue]| -> BTreeMap<String, String> {
            vars.iter()
                .filter(|v| v.enabled)
                .map(|v| (v.name.clone(), v.value.clone()))
                .collect()
        };
        let mut vault = BTreeMap::new();
        let mut vars = crate::script::Vars::default();
        let globals = self.effective_globals(secrets)?;
        vars.globals = map(&globals);
        vault.extend(
            globals
                .iter()
                .filter(|v| v.secret)
                .map(|v| (v.name.clone(), v.value.clone())),
        );
        if let Some(environment) = environment {
            let env = self.effective_environment(environment, secrets)?;
            vars.environment.name = Some(environment.to_string());
            vars.environment.values = map(&env);
            vault.extend(
                env.iter()
                    .filter(|v| v.secret)
                    .map(|v| (v.name.clone(), v.value.clone())),
            );
        }
        if let Some(path) = path {
            let mut dir = self.abs(path)?.parent().map(Path::to_path_buf);
            while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
                let folder: FolderFile = read_yaml_or_default(&d.join(FOLDER_FILE))?;
                let values = map(&self.effective_folder_vars(&d, &folder));
                if d.parent() == Some(self.root.as_path()) {
                    vars.collection = values;
                } else {
                    vars.folders.push(values);
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
        Ok((vars, vault))
    }

    /// [`Workspace::prepare`] with the scopes a script run adds.
    pub fn prepare_with(
        &self,
        request: &RequestFile,
        path: Option<&str>,
        environment: Option<&str>,
        layers: &Layers,
        secrets: &dyn SecretStore,
    ) -> Result<Prepared> {
        // The request's folders, nearest first, ending with the collection.
        let mut folders = Vec::new();
        if let Some(path) = path {
            let mut dir = self.abs(path)?.parent().map(Path::to_path_buf);
            while let Some(d) = dir.filter(|d| *d != self.root && d.starts_with(&self.root)) {
                let folder = read_yaml_or_default::<FolderFile>(&d.join(FOLDER_FILE))?;
                let vars = self.effective_folder_vars(&d, &folder);
                folders.push((folder, vars));
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
        scopes.push(layers.overrides.iter().cloned());
        for layer in [layers.local, layers.data].into_iter().flatten() {
            scopes.push(layer.clone());
        }
        if let Some(environment) = environment {
            scopes.push(enabled(self.effective_environment(environment, secrets)?));
        }
        for (_, vars) in &folders {
            scopes.push(enabled(vars.clone()));
        }
        scopes.push(enabled(self.effective_globals(secrets)?));
        let folders: Vec<FolderFile> = folders.into_iter().map(|(f, _)| f).collect();

        let auth = request
            .auth
            .as_ref()
            .or_else(|| folders.iter().find_map(|f| f.auth.as_ref()));
        let inherited: Vec<&KeyValue> = folders.iter().rev().flat_map(|f| &f.headers).collect();
        let (mut http_request, mut unresolved) =
            scopes.resolve_request(&to_http(request, auth, &inherited));
        anchor_files(&mut http_request.body, &self.root);
        let crypto = prepare_crypto(request, &scopes, &mut unresolved);
        Ok(Prepared {
            request: http_request,
            unresolved,
            crypto,
        })
    }
}

/// Makes the relative paths of files sent in a body relative to the workspace folder, as `.proto` paths are, rather
/// than to wherever Tuma was started, so a workspace in git sends the same files on every computer.
fn anchor_files(body: &mut http::Body, root: &Path) {
    let anchor = |path: &str| -> Option<String> {
        let p = Path::new(path);
        (!path.is_empty() && p.is_relative()).then(|| root.join(p).to_string_lossy().into_owned())
    };
    match body {
        http::Body::Multipart { parts } => {
            for part in parts.iter_mut().filter(|p| p.file) {
                if let Some(path) = anchor(&part.value) {
                    part.value = path;
                }
            }
        }
        http::Body::File { path, .. } => {
            if let Some(anchored) = anchor(&path.to_string_lossy()) {
                *path = anchored.into();
            }
        }
        _ => {}
    }
}

/// Builds the request to send when no workspace is open: no inheritance, and only `overrides` and dynamic variables
/// such as `{{$guid}}`.
pub fn prepare_standalone(request: &RequestFile, overrides: &[(String, String)]) -> Prepared {
    let mut scopes = Scopes::default();
    scopes.push(overrides.iter().cloned());
    let (http_request, mut unresolved) =
        scopes.resolve_request(&to_http(request, request.auth.as_ref(), &[]));
    let crypto = prepare_crypto(request, &scopes, &mut unresolved);
    Prepared {
        request: http_request,
        unresolved,
        crypto,
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
