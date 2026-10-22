//! Importers from other API clients.

pub mod bruno;
pub mod har;
pub mod insomnia;
pub mod postman;

use crate::secrets::SecretStore;
use crate::workspace::{VariableValue, Workspace};

/// Things an import could not carry over, for the user to review.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Warnings(pub Vec<String>);

impl Warnings {
    pub fn add(&mut self, warning: String) {
        self.0.push(warning);
    }
}

/// What an import created.
#[derive(Debug, Clone, PartialEq)]
pub enum Imported {
    /// The new collection's path.
    Collection(String),
    /// The new environment's name.
    Environment(String),
    Globals,
    /// An export with collections and environments together, such as Insomnia's or Bruno's.
    Several(Vec<Imported>),
}

/// Imports a Postman collection, environment or globals export into `workspace`. Nothing is overwritten: a name that
/// is taken gets a number. Imported globals are added to the workspace's globals, replacing ones with the same name.
pub fn postman(
    workspace: &mut Workspace,
    json: &str,
    secrets: &dyn SecretStore,
) -> Result<(Imported, Warnings), String> {
    let mut warnings = Warnings::default();
    let imported = match postman::parse(json, &mut warnings)? {
        postman::Export::Collection(tree) => {
            Imported::Collection(workspace.add_collection(&tree).map_err(|e| e.to_string())?)
        }
        postman::Export::Environment {
            name,
            globals,
            variables,
        } => {
            let values: Vec<VariableValue> = variables
                .into_iter()
                .map(|(v, secret)| VariableValue {
                    name: v.name,
                    value: v.value,
                    secret,
                    enabled: v.enabled,
                })
                .collect();
            if globals {
                let mut all = workspace.read_globals(secrets).map_err(|e| e.to_string())?;
                all.retain(|g| !values.iter().any(|v| v.name == g.name));
                all.extend(values);
                workspace
                    .write_globals(&all, secrets)
                    .map_err(|e| e.to_string())?;
                Imported::Globals
            } else {
                let name = workspace
                    .create_environment(if name.is_empty() { "Imported" } else { &name })
                    .map_err(|e| e.to_string())?;
                workspace
                    .write_environment(&name, &values, secrets)
                    .map_err(|e| e.to_string())?;
                Imported::Environment(name)
            }
        }
    };
    Ok((imported, warnings))
}

/// Adds collections and environments read from another tool's export. A single one is returned as itself.
fn add_all(
    workspace: &mut Workspace,
    collections: &[crate::workspace::FolderTree],
    environments: Vec<(String, Vec<crate::workspace::files::Variable>)>,
    secrets: &dyn SecretStore,
) -> Result<Imported, String> {
    let mut done = Vec::new();
    for tree in collections {
        done.push(Imported::Collection(
            workspace.add_collection(tree).map_err(|e| e.to_string())?,
        ));
    }
    for (name, variables) in environments {
        done.push(Imported::Environment(add_environment(
            workspace, &name, variables, secrets,
        )?));
    }
    match done.len() {
        0 => Err("the export has no collections or environments in it".into()),
        1 => Ok(done.remove(0)),
        _ => Ok(Imported::Several(done)),
    }
}

fn add_environment(
    workspace: &mut Workspace,
    name: &str,
    variables: Vec<crate::workspace::files::Variable>,
    secrets: &dyn SecretStore,
) -> Result<String, String> {
    let values: Vec<VariableValue> = variables
        .into_iter()
        .map(|v| VariableValue {
            name: v.name,
            value: v.value,
            secret: v.secret,
            enabled: v.enabled,
        })
        .collect();
    let name = workspace
        .create_environment(if name.trim().is_empty() {
            "Imported"
        } else {
            name
        })
        .map_err(|e| e.to_string())?;
    workspace
        .write_environment(&name, &values, secrets)
        .map_err(|e| e.to_string())?;
    Ok(name)
}

/// A JSON file that is only names and values, such as `{"baseUrl": "https://…", "token": "…"}` or
/// `[{"key": "baseUrl", "value": "…"}]`, read as an environment's variables.
fn plain_variables(json: &serde_json::Value) -> Option<Vec<crate::workspace::files::Variable>> {
    use crate::workspace::files::Variable;
    use serde_json::Value;
    let text = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(v.to_string()),
        Value::Null => Some(String::new()),
        _ => None,
    };
    match json {
        Value::Object(map) if !map.is_empty() => map
            .iter()
            .map(|(k, v)| text(v).map(|value| Variable::new(k, value)))
            .collect(),
        Value::Array(list) if !list.is_empty() => list
            .iter()
            .map(|item| {
                let name = item.get("key").or_else(|| item.get("name"))?.as_str()?;
                let mut variable =
                    Variable::new(name, text(item.get("value").unwrap_or(&Value::Null))?);
                variable.enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                Some(variable)
            })
            .collect(),
        _ => None,
    }
}

/// Imports any file Tuma understands, telling them apart by content: an OpenAPI or Swagger spec (JSON or YAML), a
/// Postman collection, environment or globals export, an Insomnia or Bruno export, a HAR recording, or a JSON file of
/// names and values (an environment named after the file). `source` is the file's location as the
/// workspace should remember it (for syncing a spec later); its last part names a HAR collection.
pub fn any(
    workspace: &mut Workspace,
    text: &str,
    source: &str,
    secrets: &dyn SecretStore,
) -> Result<(Imported, Warnings), String> {
    let json: Option<serde_json::Value> = serde_json::from_str(text).ok();
    if json
        .as_ref()
        .is_some_and(|j| j.pointer("/log/entries").is_some())
    {
        let name = std::path::Path::new(source)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Recording".into());
        let mut warnings = Warnings::default();
        let tree = har::parse(text, &name, &mut warnings)?;
        return Ok((
            Imported::Collection(workspace.add_collection(&tree).map_err(|e| e.to_string())?),
            warnings,
        ));
    }
    if json.as_ref().is_some_and(|j| {
        (j.get("info").is_some() && j.get("item").is_some()) || j.get("values").is_some()
    }) {
        return postman(workspace, text, secrets);
    }
    if let Some(j) = json.as_ref().filter(|j| insomnia::detect(j)) {
        let mut warnings = Warnings::default();
        let parsed = insomnia::parse(j, &mut warnings);
        return Ok((
            add_all(workspace, &parsed.collections, parsed.environments, secrets)?,
            warnings,
        ));
    }
    if let Some(j) = json.as_ref().filter(|j| bruno::detect(j)) {
        let mut warnings = Warnings::default();
        let parsed = bruno::parse(j, &mut warnings);
        let collections = [parsed.collection];
        return Ok((
            add_all(workspace, &collections, parsed.environments, secrets)?,
            warnings,
        ));
    }
    // A JSON object of plain values is an environment; OpenAPI specs (checked below) have "openapi" or "swagger".
    if let Some(variables) = json
        .as_ref()
        .filter(|j| j.get("openapi").is_none() && j.get("swagger").is_none())
        .and_then(plain_variables)
    {
        let name = std::path::Path::new(source)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported".into());
        let name = add_environment(workspace, &name, variables, secrets)?;
        return Ok((Imported::Environment(name), Warnings::default()));
    }
    match crate::openapi::Spec::parse(text) {
        Ok(spec) => {
            let (path, warnings) = crate::openapi::import(workspace, &spec, Some(source))
                .map_err(|e| e.to_string())?;
            Ok((Imported::Collection(path), warnings))
        }
        Err(e) => Err(format!(
            "not a file Tuma can import ({e}). Tuma reads OpenAPI and Swagger specs, Postman, Insomnia and Bruno \
             exports, HAR files, and JSON files of names and values (as an environment)"
        )),
    }
}
