//! Current values: what scripts set with `pm.environment.set` and friends. They override the values in the files
//! (Postman's "initial values") on this machine only, in Tuma's data folder, so a token a login script stores never
//! ends up in git. A secret variable's current value goes to the OS keychain instead, like any secret.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::files::{EnvironmentFile, FolderFile, Variable};
use super::{
    ENVIRONMENTS, EXTENSION, Error, FOLDER_FILE, Result, VariableValue, Workspace, file_name,
    io_err, read_yaml_or_default,
};
use crate::secrets::SecretStore;

/// Changes by variable name: a new value, or `None` for unset.
pub type Changes = BTreeMap<String, Option<String>>;

/// Current values by scope.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CurrentValues {
    /// By environment name.
    pub environments: BTreeMap<String, Changes>,
    /// By collection (its folder name).
    pub collections: BTreeMap<String, Changes>,
    pub globals: Changes,
}

impl CurrentValues {
    pub fn is_empty(&self) -> bool {
        self.environments.values().all(|c| c.is_empty())
            && self.collections.values().all(|c| c.is_empty())
            && self.globals.is_empty()
    }
}

/// A scope scripts can write to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    Environment(&'a str),
    /// A collection's variables, by its folder name.
    Collection(&'a str),
    Globals,
}

/// The folder current values are kept in: Tuma's data folder (shared by the app, the CLI and the MCP server).
pub(super) fn default_local_dir(root: &Path) -> PathBuf {
    crate::history::data_dir().unwrap_or_else(|| root.join(".tuma-local"))
}

/// Applies current values to a scope's variables: changed values replace file values, unset ones disappear, and new
/// ones are added.
pub(super) fn overlay(
    mut vars: Vec<VariableValue>,
    changes: Option<&Changes>,
) -> Vec<VariableValue> {
    let Some(changes) = changes else { return vars };
    vars.retain(|v| !matches!(changes.get(&v.name), Some(None)));
    for v in &mut vars {
        if let Some(Some(value)) = changes.get(&v.name)
            && !v.secret
        {
            v.value = value.clone();
            v.enabled = true;
        }
    }
    for (name, value) in changes {
        if let Some(value) = value
            && !vars.iter().any(|v| &v.name == name)
        {
            vars.push(VariableValue {
                name: name.clone(),
                value: value.clone(),
                secret: false,
                enabled: true,
            });
        }
    }
    vars
}

impl Workspace {
    /// Keeps current values under `dir` instead of Tuma's data folder; for tests.
    pub fn set_local_dir(&mut self, dir: impl Into<PathBuf>) {
        self.local_dir = dir.into();
    }

    fn values_path(&self) -> PathBuf {
        self.local_dir
            .join("values")
            .join(format!("{}.json", file_name(&self.file.id)))
    }

    /// The current values set on this machine. A missing or unreadable file means none.
    pub fn current_values(&self) -> CurrentValues {
        fs::read_to_string(self.values_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write_current_values(&self, values: &CurrentValues) -> Result<()> {
        let path = self.values_path();
        if values.is_empty() {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err(&path)(e)),
                _ => Ok(()),
            };
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        let json =
            serde_json::to_string_pretty(values).map_err(|e| Error::Serialize(e.to_string()))?;
        fs::write(&path, json).map_err(io_err(&path))
    }

    /// Forgets current values, going back to the values in the files: for one scope, or all of them.
    pub fn reset_current_values(&self, scope: Option<Scope>) -> Result<()> {
        let mut values = self.current_values();
        match scope {
            None => values = CurrentValues::default(),
            Some(Scope::Environment(name)) => {
                values.environments.remove(name);
            }
            Some(Scope::Collection(name)) => {
                values.collections.remove(name);
            }
            Some(Scope::Globals) => values.globals.clear(),
        }
        self.write_current_values(&values)
    }

    /// An environment's variables with current values applied.
    pub fn effective_environment(
        &self,
        name: &str,
        secrets: &dyn SecretStore,
    ) -> Result<Vec<VariableValue>> {
        Ok(overlay(
            self.read_environment(name, secrets)?,
            self.current_values().environments.get(name),
        ))
    }

    /// The global variables with current values applied.
    pub fn effective_globals(&self, secrets: &dyn SecretStore) -> Result<Vec<VariableValue>> {
        Ok(overlay(
            self.read_globals(secrets)?,
            Some(&self.current_values().globals),
        ))
    }

    /// A folder's variables; for a collection, with current values applied.
    pub(super) fn effective_folder_vars(
        &self,
        dir: &Path,
        folder: &FolderFile,
    ) -> Vec<VariableValue> {
        let vars = folder
            .variables
            .iter()
            .map(|v| VariableValue {
                name: v.name.clone(),
                value: v.value.clone(),
                secret: false,
                enabled: v.enabled,
            })
            .collect();
        if dir.parent() != Some(self.root.as_path()) {
            return vars;
        }
        let name = super::dir_name(dir);
        overlay(vars, self.current_values().collections.get(&name))
    }

    /// Stores what scripts changed in a scope. A value equal to the file's value is no longer a change; a secret
    /// variable's new value goes to the keychain.
    pub fn store_current_values(
        &self,
        scope: Scope,
        changes: &Changes,
        secrets: &dyn SecretStore,
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let (file_vars, secret_scope): (Vec<Variable>, Option<String>) = match scope {
            Scope::Environment(name) => {
                let path = self
                    .root
                    .join(ENVIRONMENTS)
                    .join(format!("{}.{EXTENSION}", file_name(name)));
                let file: EnvironmentFile = read_yaml_or_default(&path)?;
                (file.variables, Some(format!("environments/{name}")))
            }
            Scope::Collection(name) => {
                let file: FolderFile =
                    read_yaml_or_default(&self.root.join(file_name(name)).join(FOLDER_FILE))?;
                (file.variables, None)
            }
            Scope::Globals => (self.file.variables.clone(), Some("globals".into())),
        };
        let mut values = self.current_values();
        let stored = match scope {
            Scope::Environment(name) => values.environments.entry(name.to_string()).or_default(),
            Scope::Collection(name) => values.collections.entry(name.to_string()).or_default(),
            Scope::Globals => &mut values.globals,
        };
        for (name, value) in changes {
            let in_file = file_vars.iter().find(|v| &v.name == name);
            match (in_file, value, &secret_scope) {
                (Some(v), Some(value), Some(secret_scope)) if v.secret => {
                    secrets
                        .set(&self.secret_key(secret_scope, name), value)
                        .map_err(Error::Secrets)?;
                }
                (Some(v), Some(value), _) if v.enabled && &v.value == value => {
                    stored.remove(name);
                }
                (None, None, _) => {
                    stored.remove(name);
                }
                _ => {
                    stored.insert(name.clone(), value.clone());
                }
            }
        }
        values.environments.retain(|_, c| !c.is_empty());
        values.collections.retain(|_, c| !c.is_empty());
        self.write_current_values(&values)
    }
}
