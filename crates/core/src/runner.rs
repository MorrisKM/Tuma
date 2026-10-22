//! What the CLI and the MCP server share: finding the workspace, sending a request with its checks, running folders,
//! and a compact, secret-free summary of each result.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::expect::{self, CheckResult, Expectations};
use crate::history::{Entry, History};
use crate::http::{self, Header};
use crate::openapi;
use crate::secrets::SecretStore;
use crate::workspace::files::RequestFile;
use crate::workspace::{self, Node, NodeKind, Workspace};

/// The folder name for a workspace created inside a project.
pub const HIDDEN_WORKSPACE: &str = ".tuma";

/// Finds the workspace for `start`: a folder with `workspace.yaml`, or a `.tuma/` folder, in `start` or above it.
pub fn find_workspace(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        if dir.join("workspace.yaml").is_file() {
            Some(dir.to_path_buf())
        } else if dir.join(HIDDEN_WORKSPACE).join("workspace.yaml").is_file() {
            Some(dir.join(HIDDEN_WORKSPACE))
        } else {
            None
        }
    })
}

/// Opens the workspace for `start`, or creates `.tuma/` at the root of the project (the nearest folder with `.git`,
/// else `start`), named `name` or after the project folder.
pub fn open_or_create(start: &Path, name: Option<&str>) -> Result<Workspace, workspace::Error> {
    if let Some(root) = find_workspace(start) {
        return Workspace::open(root);
    }
    let project = start
        .ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(start);
    let folder = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let name = name
        .map(String::from)
        .or(folder)
        .unwrap_or_else(|| "Workspace".into());
    Workspace::create(project.join(HIDDEN_WORKSPACE), &name)
}

/// A request URL as agents write it: a path such as `/users` is relative to the `baseUrl` variable.
pub fn absolute_url(url: &str) -> String {
    if url.starts_with('/') {
        format!("{{{{baseUrl}}}}{url}")
    } else {
        url.to_string()
    }
}

/// How much of a response to include in a summary.
#[derive(Debug, Clone, Default)]
pub struct Detail {
    /// Cut the body after this many characters. `None` keeps all of it.
    pub max_body: Option<usize>,
    /// Return only these parts of a JSON body (a JSON path).
    pub select: Option<String>,
    /// Include every response header instead of a few useful ones.
    pub all_headers: bool,
}

/// One request's result, safe to show to an agent: secret values are masked everywhere.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The saved request, when it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub method: String,
    /// The URL that was sent, after variables.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Header>,
    /// The body as text, or the selected JSON values. Absent for empty bodies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub body_truncated: bool,
    /// Why the request could not be sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckResult>,
    /// `{{variables}}` that had no value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    /// True when the request was sent and every check passed.
    pub passed: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Everything needed to send one request.
pub struct Job<'a> {
    pub workspace: Option<&'a Workspace>,
    pub request: &'a RequestFile,
    /// Where the request is saved, for inheritance.
    pub path: Option<&'a str>,
    /// For a request that is not saved: a collection or folder to send it as if it were saved in, so it gets that
    /// folder's variables and auth.
    pub context: Option<&'a str>,
    pub environment: Option<&'a str>,
    /// One-off variable values that win over every scope.
    pub overrides: &'a [(String, String)],
    /// Checks on top of the request's own `expect:` block.
    pub extra_checks: Option<&'a Expectations>,
    pub secrets: &'a dyn SecretStore,
    pub history: Option<&'a History>,
    /// Recorded in history: `cli` or `mcp`.
    pub source: &'a str,
    /// When set, only hosts it accepts may be called. Checked after variables are resolved.
    pub allow_host: Option<&'a (dyn Fn(&str) -> bool + Sync)>,
}

/// The host of a URL as Tuma sends it (`http://` is assumed), lower case.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let url = if url.contains("://") {
        url.to_string()
    } else {
        format!("http://{url}")
    };
    url::Url::parse(&url)
        .ok()?
        .host_str()
        .map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Sends a request and checks the response. Failures to send become an [`Outcome`] with `error` set.
pub async fn send(job: Job<'_>, detail: &Detail) -> Outcome {
    let sent_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let mut file = job.request.clone();
    file.url = absolute_url(&file.url);
    // Inheritance works from a request file's location, so a folder context becomes a file inside it.
    let inherit_from = match (job.path, job.context, job.workspace) {
        (Some(path), _, _) => Some(path.to_string()),
        (None, Some(folder), _) => Some(format!("{}/request.yaml", folder.trim_matches('/'))),
        (None, None, Some(ws)) => implicit_location(ws, &job.request.url),
        _ => None,
    };

    let (prepared, secrets) = match job.workspace {
        Some(ws) => {
            match ws.prepare(
                &file,
                inherit_from.as_deref(),
                job.environment,
                job.overrides,
                job.secrets,
            ) {
                Ok(prepared) => (
                    prepared,
                    ws.secret_values(job.environment, job.secrets)
                        .unwrap_or_default(),
                ),
                Err(e) => return failed(job.path, &file, e.to_string(), BTreeSet::new(), &[]),
            }
        }
        None => (workspace::prepare_standalone(&file, job.overrides), vec![]),
    };
    let mask = |text: &str| mask(text, &secrets);
    let unresolved = prepared.unresolved;
    if unresolved.contains("baseUrl") && job.request.url.starts_with('/') {
        let message = format!(
            "\"{}\" is a path, so it needs a baseUrl variable: set one in an environment, pass one for this call, send it \
             in a collection that has one, or use a full URL",
            job.request.url
        );
        return failed(job.path, &file, message, unresolved, &secrets);
    }

    if let Some(allow) = job.allow_host {
        let host = host_of(&prepared.request.url).unwrap_or_default();
        if !allow(&host) {
            let message = format!(
                "{host} is not an allowed host. Agents may call localhost and the hosts in the selected environment; \
                 the user can allow more with `tuma mcp --allow-host {host}`"
            );
            return failed(job.path, &file, message, unresolved, &secrets);
        }
    }

    let result = http::send(&prepared.request).await;
    if let Some(history) = job.history {
        let entry = Entry {
            id: uuid::Uuid::new_v4().to_string(),
            sent_at,
            request: job.request.clone(),
            path: job.path.map(String::from),
            status: result.as_ref().ok().map(|r| r.status),
            error: result.as_ref().err().map(|e| e.to_string()),
            total_ms: result
                .as_ref()
                .ok()
                .map(|r| r.timing.total.as_secs_f64() * 1000.0),
            size: result.as_ref().ok().map(|r| r.body.len()),
            source: Some(job.source.to_string()),
        };
        // History is a convenience; failing to write it must not fail the request.
        let _ = history.append(&entry);
    }

    let response = match result {
        Ok(response) => response,
        Err(e) => {
            let mut outcome = failed(job.path, &file, e.to_string(), unresolved, &secrets);
            outcome.url = mask(&prepared.request.url);
            return outcome;
        }
    };

    let mut expect = file.expect.clone();
    if let Some(extra) = job.extra_checks {
        expect.merge(extra.clone());
    }
    let mut checks = expect::evaluate(&expect, &response);
    if expect.schema {
        checks.push(contract_check(
            job.workspace,
            inherit_from.as_deref(),
            &file,
            &response,
        ));
    }
    let checks: Vec<CheckResult> = checks
        .into_iter()
        .map(|c| CheckResult {
            actual: mask(&c.actual),
            ..c
        })
        .collect();
    let (body, body_truncated) = summarize_body(&response.body, detail);
    Outcome {
        path: job.path.map(String::from),
        method: prepared.request.method.to_uppercase(),
        url: mask(&response.url),
        status: Some(response.status),
        reason: response.reason.clone(),
        time_ms: Some(response.timing.total.as_millis() as u64),
        size: Some(response.body.len()),
        headers: pick_headers(&response.headers, detail.all_headers)
            .into_iter()
            .map(|h| Header {
                value: mask(&h.value),
                ..h
            })
            .collect(),
        body: body.map(|b| mask(&b)),
        body_truncated,
        error: None,
        passed: checks.iter().all(|c| c.passed),
        checks,
        unresolved: unresolved.into_iter().collect(),
    }
}

/// Where an unsaved request is treated as saved, for its variables and auth: in the workspace's only collection, when
/// its URL is relative to a variable (`/users`, `{{baseUrl}}/users`). A full URL inherits nothing, so a collection's
/// credentials never go to a host typed into a new request.
pub fn implicit_location(workspace: &Workspace, url: &str) -> Option<String> {
    let url = url.trim();
    if url.starts_with('/') || url.starts_with("{{") {
        sole_collection(workspace).map(|c| format!("{c}/request.yaml"))
    } else {
        None
    }
}

/// The workspace's only collection, if it has exactly one: a path such as `/users` then means "in that API".
pub fn sole_collection(workspace: &Workspace) -> Option<String> {
    match workspace.tree().ok()?.as_slice() {
        [only] => Some(only.path.clone()),
        _ => None,
    }
}

/// Checks the response against the OpenAPI spec of the request's collection, or for an unsaved request, against any
/// spec in the workspace that has a matching operation.
pub fn contract_check(
    workspace: Option<&Workspace>,
    path: Option<&str>,
    file: &RequestFile,
    response: &http::Response,
) -> CheckResult {
    let missing = |why: &str| CheckResult {
        check: "response matches the spec".into(),
        passed: false,
        expected: "an OpenAPI spec for this request".into(),
        actual: why.into(),
    };
    let Some(workspace) = workspace else {
        return missing("no workspace");
    };
    let url_path = url::Url::parse(&response.url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let specs = match path.and_then(|p| openapi::snapshot_for(workspace, p)) {
        Some(spec) => vec![spec],
        None => openapi::snapshots(workspace),
    };
    let found = specs.iter().find_map(|spec| {
        let operation = file
            .operation
            .clone()
            .or_else(|| spec.find_operation(&file.method, &url_path))?;
        Some((spec, operation))
    });
    let Some((spec, operation)) = found else {
        return missing(
            "no imported spec has this operation; import one with `tuma openapi import`",
        );
    };
    let content_type = response
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str())
        .unwrap_or_default();
    spec.check_response(&operation, response.status, content_type, &response.body)
}

fn failed(
    path: Option<&str>,
    file: &RequestFile,
    error: String,
    unresolved: BTreeSet<String>,
    secrets: &[String],
) -> Outcome {
    Outcome {
        path: path.map(String::from),
        method: file.method.to_uppercase(),
        url: mask(&file.url, secrets),
        status: None,
        reason: String::new(),
        time_ms: None,
        size: None,
        headers: vec![],
        body: None,
        body_truncated: false,
        error: Some(mask(&error, secrets)),
        checks: vec![],
        unresolved: unresolved.into_iter().collect(),
        passed: false,
    }
}

/// Replaces every secret value with `••••`.
fn mask(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |text, secret| {
        text.replace(secret.as_str(), "••••")
    })
}

/// Headers worth showing by default: the ones that say what the body is, where to go next, and how to retry.
fn pick_headers(headers: &[Header], all: bool) -> Vec<Header> {
    const USEFUL: [&str; 6] = [
        "content-type",
        "location",
        "www-authenticate",
        "retry-after",
        "set-cookie",
        "allow",
    ];
    headers
        .iter()
        .filter(|h| all || USEFUL.contains(&h.name.to_ascii_lowercase().as_str()))
        .cloned()
        .collect()
}

fn summarize_body(body: &[u8], detail: &Detail) -> (Option<String>, bool) {
    if body.is_empty() {
        return (None, false);
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return (
            Some(format!("({} bytes of binary data)", body.len())),
            false,
        );
    };
    let text = match (&detail.select, serde_json::from_str::<Value>(text)) {
        (Some(path), Ok(json)) => match expect::select(&json, path) {
            Ok(values) if values.len() == 1 => pretty(values[0]),
            Ok(values) => pretty(&Value::Array(values.into_iter().cloned().collect())),
            Err(e) => format!("(could not select {path}: {e})"),
        },
        (Some(_), Err(_)) => "(the body is not JSON, so nothing could be selected)".to_string(),
        (None, Ok(json)) => pretty(&json),
        (None, Err(_)) => text.to_string(),
    };
    match detail.max_body.and_then(|max| text.char_indices().nth(max)) {
        Some((cut, _)) => (Some(text[..cut].to_string()), true),
        None => (Some(text), false),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The saved requests under `path` (a request, folder or collection; empty for the whole workspace), in tree order.
/// A request can be named with or without `.yaml`.
pub fn requests_under(workspace: &Workspace, path: &str) -> Result<Vec<String>, workspace::Error> {
    fn collect(node: &Node, out: &mut Vec<String>) {
        match node.kind {
            NodeKind::Request => out.push(node.path.clone()),
            _ => node.children.iter().for_each(|c| collect(c, out)),
        }
    }
    fn find<'a>(nodes: &'a [Node], path: &str) -> Option<&'a Node> {
        nodes.iter().find_map(|n| {
            if n.path == path || n.path.strip_suffix(".yaml") == Some(path) {
                Some(n)
            } else {
                find(&n.children, path)
            }
        })
    }

    let tree = workspace.tree()?;
    let path = path.trim_matches('/');
    let mut out = Vec::new();
    if path.is_empty() {
        tree.iter().for_each(|n| collect(n, &mut out));
    } else {
        let node =
            find(&tree, path).ok_or_else(|| workspace::Error::InvalidPath(path.to_string()))?;
        collect(node, &mut out);
    }
    Ok(out)
}

/// The totals of a run.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub requests: usize,
    pub passed: usize,
    pub failed: usize,
    pub checks: usize,
    pub failed_checks: usize,
}

impl Summary {
    pub fn of(outcomes: &[Outcome]) -> Self {
        let mut summary = Summary {
            requests: outcomes.len(),
            ..Default::default()
        };
        for outcome in outcomes {
            if outcome.passed {
                summary.passed += 1;
            } else {
                summary.failed += 1;
            }
            summary.checks += outcome.checks.len();
            summary.failed_checks += outcome.checks.iter().filter(|c| !c.passed).count();
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secrets() {
        assert_eq!(
            mask("Bearer abc123 abc123", &["abc123".into()]),
            "Bearer •••• ••••"
        );
    }

    #[test]
    fn paths_get_the_base_url() {
        assert_eq!(absolute_url("/users"), "{{baseUrl}}/users");
        assert_eq!(absolute_url("localhost:3000/users"), "localhost:3000/users");
    }

    #[test]
    fn finds_hidden_workspaces_above() {
        let root = std::env::temp_dir().join(format!("tuma-find-{}", std::process::id()));
        let deep = root.join("src/api");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(find_workspace(&deep), None);
        let ws = open_or_create(&deep, None).unwrap();
        assert_eq!(ws.root(), root.join(".tuma"));
        assert_eq!(find_workspace(&deep), Some(root.join(".tuma")));
        std::fs::remove_dir_all(root).unwrap();
    }
}
