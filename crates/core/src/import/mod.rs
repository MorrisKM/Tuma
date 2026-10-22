//! Importers from other API clients.

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
