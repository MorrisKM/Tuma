//! `tuma check`: validates every file in a workspace, for people and agents who edit the YAML by hand.
//!
//! Reading a workspace is lenient (unknown keys are ignored, so newer files open in older versions). Checking is
//! strict: an unknown key is almost always a typo, such as `expct:`, that would silently do nothing.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::expect;
use crate::workspace::files::{EnvironmentFile, FolderFile, RequestFile, WorkspaceFile};

/// The JSON Schemas of the workspace files, by name, as shipped in the repository's `schemas/` folder.
pub fn schemas() -> Vec<(&'static str, Value)> {
    vec![
        ("workspace", schema::<WorkspaceFile>("Tuma workspace.yaml")),
        (
            "folder",
            schema::<FolderFile>("Tuma collection or folder settings (_folder.yaml)"),
        ),
        ("request", schema::<RequestFile>("Tuma request")),
        (
            "environment",
            schema::<EnvironmentFile>("Tuma environment (environments/<name>.yaml)"),
        ),
    ]
}

fn schema<T: schemars::JsonSchema>(title: &str) -> Value {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default();
    value["title"] = Value::String(title.to_string());
    value
}

/// The schema with unknown keys forbidden, for checking.
fn strict(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            if map.contains_key("properties") && !map.contains_key("additionalProperties") {
                map.insert("additionalProperties".into(), Value::Bool(false));
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, strict(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(strict).collect()),
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// Relative to the workspace.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
    /// Warnings don't stop anything from working, but are probably mistakes.
    pub warning: bool,
}

/// Checks every file in the workspace at `root`.
pub fn check_workspace(root: &Path) -> Vec<Problem> {
    let validators: Vec<(&str, jsonschema::Validator)> = schemas()
        .into_iter()
        .filter_map(|(name, schema)| {
            jsonschema::validator_for(&strict(schema))
                .ok()
                .map(|v| (name, v))
        })
        .collect();
    let validator = |name: &str| validators.iter().find(|(n, _)| *n == name).map(|(_, v)| v);
    let mut problems = Vec::new();
    let check = |path: &Path, kind: &str| -> Vec<Problem> {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        validator(kind)
            .map(|v| check_file(path, &rel, kind, v))
            .unwrap_or_default()
    };

    problems.extend(check(&root.join("workspace.yaml"), "workspace"));
    for file in yaml_files(&root.join("environments")) {
        problems.extend(check(&file, "environment"));
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return problems;
    };
    let mut collections: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            !name.starts_with('.') && name != "environments"
        })
        .collect();
    collections.sort();
    let mut folders = Vec::new();
    for collection in collections {
        walk(&collection, &mut folders);
    }
    for folder in folders {
        let settings = folder.join("_folder.yaml");
        if settings.exists() {
            problems.extend(check(&settings, "folder"));
            problems.extend(check_order(root, &folder));
        }
        for file in yaml_files(&folder) {
            if file.file_name().is_some_and(|n| n != "_folder.yaml") {
                problems.extend(check(&file, "request"));
            }
        }
    }
    problems
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    out.push(dir.to_path_buf());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with('.')
        })
        .collect();
    dirs.sort();
    for d in dirs {
        walk(&d, out);
    }
}

fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    files.sort();
    files
}

fn check_file(
    path: &Path,
    rel: &str,
    kind: &str,
    validator: &jsonschema::Validator,
) -> Vec<Problem> {
    let problem = |line, message: String| Problem {
        file: rel.to_string(),
        line,
        message,
        warning: false,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => return vec![problem(None, e.to_string())],
    };
    let value: Value = match serde_saphyr::from_str(&text) {
        Ok(Value::Null) => Value::Object(Default::default()),
        Ok(value) => value,
        Err(e) => {
            return vec![problem(
                yaml_error_line(&e.to_string()),
                format!(
                    "not valid YAML: {}",
                    e.to_string().lines().next().unwrap_or_default()
                ),
            )];
        }
    };
    let mut problems: Vec<Problem> = validator
        .iter_errors(&value)
        .map(|e| {
            let at = e.instance_path().to_string();
            let error = e.to_string();
            let prefix = if at.is_empty() { String::new() } else { format!("{at}: ") };
            if error.contains("Additional properties") {
                // Point at the unknown key itself.
                let key = error.split('\'').nth(1).unwrap_or_default();
                let line = line_of(&text, &format!("{at}/{key}")).or_else(|| line_of(&text, &at));
                return problem(line, format!("{prefix}{}", unknown_key_message(&error)));
            }
            let line = line_of(&text, &at);
            if error.contains("anyOf") || error.contains("oneOf") {
                // A tagged value (auth, body) whose type or fields don't fit any variant.
                let kind = value.pointer(&format!("{at}/type")).and_then(Value::as_str);
                let message = match kind {
                    Some(kind) => format!(
                        "{prefix}\"type: {kind}\" is not supported here, or one of its fields is missing or misspelled"
                    ),
                    None => format!("{prefix}does not match any allowed form"),
                };
                return problem(line, message);
            }
            problem(line, format!("{prefix}{error}"))
        })
        .collect();
    if !problems.is_empty() {
        return problems;
    }
    // The schema passed; now what it can't express.
    if kind == "request"
        && let Some(request) = parse::<RequestFile>(&text)
    {
        for check in &request.expect.body {
            if let Some(path) = &check.path
                && let Err(e) = expect::select(&Value::Null, path)
            {
                problems.push(problem(
                    line_of_text(&text, path),
                    format!("invalid JSON path: {e}"),
                ));
            }
            if let Some(pattern) = &check.matches
                && let Err(e) = regex::Regex::new(pattern)
            {
                problems.push(problem(
                    line_of_text(&text, pattern),
                    format!("invalid regular expression: {}", last_line(&e.to_string())),
                ));
            }
        }
    }
    problems
}

fn parse<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_saphyr::from_str(text).ok()
}

/// Order entries that name nothing in the folder: harmless, but usually left over from a rename outside Tuma.
fn check_order(root: &Path, folder: &Path) -> Vec<Problem> {
    let Ok(text) = std::fs::read_to_string(folder.join("_folder.yaml")) else {
        return vec![];
    };
    let Some(settings) = parse::<FolderFile>(&text) else {
        return vec![];
    };
    let rel = folder.join("_folder.yaml");
    let rel = rel
        .strip_prefix(root)
        .unwrap_or(&rel)
        .to_string_lossy()
        .replace('\\', "/");
    settings
        .order
        .iter()
        .filter(|name| {
            !folder.join(format!("{name}.yaml")).is_file() && !folder.join(name).is_dir()
        })
        .map(|name| Problem {
            file: rel.clone(),
            line: line_of_text(&text, name),
            message: format!(
                "order lists \"{name}\", but there is no request or folder with that name"
            ),
            warning: true,
        })
        .collect()
}

/// "Additional properties are not allowed ('expct' was unexpected)" → a message that names the key.
fn unknown_key_message(error: &str) -> String {
    let keys: Vec<&str> = error.split('\'').skip(1).step_by(2).collect();
    match keys.as_slice() {
        [] => error.to_string(),
        [key] => format!("unknown key \"{key}\" (a typo? it is ignored)"),
        keys => format!(
            "unknown keys {} (typos? they are ignored)",
            keys.iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The line of the value at a JSON pointer such as `/expect/body/0/path`, found by looking for its keys in order.
fn line_of(text: &str, pointer: &str) -> Option<usize> {
    let keys: Vec<&str> = pointer
        .split('/')
        .filter(|s| !s.is_empty() && s.parse::<usize>().is_err())
        .collect();
    let mut from = 0;
    let lines: Vec<&str> = text.lines().collect();
    for key in &keys {
        let found = lines.iter().enumerate().skip(from).find(|(_, l)| {
            let l = l.trim_start().trim_start_matches("- ");
            l.starts_with(&format!("{key}:"))
        })?;
        from = found.0;
    }
    Some(if keys.is_empty() { 1 } else { from + 1 })
}

fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
}

fn line_of_text(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// serde-saphyr errors mention "line N"; this pulls it out.
fn yaml_error_line(error: &str) -> Option<usize> {
    let rest = &error[error.find("line ")? + 5..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(files: &[(&str, &str)]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("tuma-check-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    #[test]
    fn clean_workspace() {
        let root = workspace(&[
            ("workspace.yaml", "name: Test\nid: x\n"),
            (
                "environments/local.yaml",
                "variables:\n- name: baseUrl\n  value: http://localhost\n",
            ),
            (
                "API/_folder.yaml",
                "order:\n- get\nauth:\n  type: bearer\n  token: \"{{t}}\"\n",
            ),
            (
                "API/get.yaml",
                "method: GET\nurl: /x\nexpect:\n  status: 2xx\n  schema: true\n  body:\n  - path: $.id\n    exists: true\n",
            ),
        ]);
        assert_eq!(check_workspace(&root), vec![]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_typos_bad_values_and_stale_order() {
        let root = workspace(&[
            ("workspace.yaml", "name: Test\nid: x\n"),
            ("API/_folder.yaml", "order:\n- gone\n"),
            (
                "API/a.yaml",
                "method: GET\nurl: /x\nexpct:\n  status: 200\n",
            ),
            (
                "API/b.yaml",
                "method: GET\nurl: /x\nexpect:\n  body:\n  - path: $.items[x]\n  - path: $.name\n    matches: \"(\"\n",
            ),
            ("API/c.yaml", "method: GET\nauth:\n  type: oauth9\n"),
            ("API/d.yaml", "method: [GET\n"),
        ]);
        let problems = check_workspace(&root);
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{}:{} {}", p.file, p.line.unwrap_or(0), p.message))
            .collect();
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("API/_folder.yaml:2 order lists \"gone\"")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("API/a.yaml:3 unknown key \"expct\"")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("API/b.yaml:5 invalid JSON path")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "API/b.yaml:7 invalid regular expression: error: unclosed group"),
            "{lines:#?}"
        );
        assert!(
            lines.iter().any(|l| l == "API/c.yaml:2 /auth: \"type: oauth9\" is not supported here, or one of its fields is missing or misspelled"),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("API/d.yaml:1 not valid YAML") && !l.contains('\n')),
            "{lines:#?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
