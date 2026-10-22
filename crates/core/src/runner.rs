//! What the CLI and the MCP server share: finding the workspace, sending a request with its checks, running folders,
//! and a compact, secret-free summary of each result.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::expect::{self, CheckResult, Expectations};
use crate::history::{Entry, History};
use crate::http::{self, Header};
use crate::openapi;
use crate::script::{self, AllowHost, Event, LogLine, Script, ScriptError, TestResult};
use crate::secrets::SecretStore;
use crate::workspace::files::{self, RequestFile};
use crate::workspace::{self, Changes, Layers, Node, NodeKind, Scope, Workspace};

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    /// True when a pre-request script skipped the request.
    #[serde(skip_serializing_if = "is_false")]
    pub skipped: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckResult>,
    /// Results of `pm.test` in the collection's, folders' and request's scripts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestResult>,
    /// Scripts that threw.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub script_errors: Vec<ScriptError>,
    /// What scripts logged with `console.log`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub console: Vec<LogLine>,
    /// `{{variables}}` that had no value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    /// True when the request was sent, every check and test passed, and no script failed.
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
    /// Recorded in history: `cli`, `mcp` or `mcp:<agent>`; empty for the app.
    pub source: &'a str,
    /// When set, only hosts it accepts may be called, by the request and by `pm.sendRequest`. Checked after
    /// variables are resolved.
    pub allow_host: Option<AllowHost>,
    pub scripting: Scripting<'a>,
    /// Proxy, certificates and cookie jar, from [`crate::settings::MachineSettings::network`]. `None` sends
    /// directly, with no cookies.
    pub network: Option<Arc<http::Network>>,
}

/// How scripts run for a job.
#[derive(Debug, Clone, Copy)]
pub struct Scripting<'a> {
    /// Run the collection's, folders' and request's pre-request and test scripts.
    pub enabled: bool,
    /// For `pm.info` in a collection run: which iteration, counting from 0, of how many.
    pub iteration: usize,
    pub iteration_count: usize,
    /// The current row of a collection run's data file (`pm.iterationData`).
    pub data: Option<&'a BTreeMap<String, String>>,
    /// `pm.variables.set` values from earlier requests in a collection run.
    pub locals: Option<&'a BTreeMap<String, String>>,
    pub timeout: Duration,
}

impl Default for Scripting<'_> {
    fn default() -> Self {
        Self {
            enabled: true,
            iteration: 0,
            iteration_count: 1,
            data: None,
            locals: None,
            timeout: script::DEFAULT_TIMEOUT,
        }
    }
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

/// Why a request was not sent, or failed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Failure {
    /// The HTTP engine's error kind (`connect`, `timeout`, ...), or `workspace`, `script`, `blocked`, `skipped`.
    pub kind: String,
    pub message: String,
}

impl Failure {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// Everything that happened when sending a request: what the app shows, and what [`send`] summarizes.
#[derive(Debug)]
pub struct Execution {
    /// The request after pre-request scripts, before variables.
    pub file: RequestFile,
    /// Where the request was treated as saved, for inheritance.
    pub location: Option<String>,
    /// The request as sent, after variables.
    pub sent: Option<http::Request>,
    pub unresolved: BTreeSet<String>,
    pub response: Result<http::Response, Failure>,
    pub checks: Vec<CheckResult>,
    pub tests: Vec<TestResult>,
    pub script_errors: Vec<ScriptError>,
    pub console: Vec<LogLine>,
    pub visualizer: Option<Value>,
    /// `pm.execution.setNextRequest`, for collection runs.
    pub next_request: Option<Option<String>>,
    /// `pm.variables` after the scripts, for the next request of a collection run.
    pub locals: BTreeMap<String, String>,
    /// Secret values the request could use, so output can mask them.
    pub secrets: Vec<String>,
}

impl Execution {
    /// The response arrived, every check and test passed, and no script failed.
    pub fn passed(&self) -> bool {
        self.response.is_ok()
            && self.checks.iter().all(|c| c.passed)
            && self.tests.iter().all(|t| t.passed || t.skipped)
            && self.script_errors.is_empty()
    }
}

/// Runs scripts without stalling the async runtime more than needed.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// What changed between two variable maps.
fn changes(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Changes {
    let mut out = Changes::new();
    for (k, v) in after {
        if before.get(k) != Some(v) {
            out.insert(k.clone(), Some(v.clone()));
        }
    }
    for k in before.keys().filter(|k| !after.contains_key(*k)) {
        out.insert(k.clone(), None);
    }
    out
}

/// Stores what scripts did to variables. Environment changes without a selected environment are dropped, as in
/// Postman.
fn store_changes(
    ws: &Workspace,
    input: &script::Vars,
    output: &script::VarsOut,
    environment: Option<&str>,
    collection: Option<&str>,
    secrets: &dyn SecretStore,
) -> Result<(), workspace::Error> {
    if let Some(environment) = environment {
        ws.store_current_values(
            Scope::Environment(environment),
            &changes(&input.environment.values, &output.environment),
            secrets,
        )?;
    }
    if let Some(collection) = collection {
        ws.store_current_values(
            Scope::Collection(collection),
            &changes(&input.collection, &output.collection),
            secrets,
        )?;
    }
    ws.store_current_values(
        Scope::Globals,
        &changes(&input.globals, &output.globals),
        secrets,
    )
}

/// Sends a request: pre-request scripts, variables, the request, checks, test scripts. Variables scripts set are
/// stored as current values.
pub async fn execute(job: Job<'_>) -> Execution {
    let sent_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let mut file = job.request.clone();
    file.url = absolute_url(&file.url);
    // Inheritance works from a request file's location, so a folder context becomes a file inside it.
    let location = match (job.path, job.context, job.workspace) {
        (Some(path), _, _) => Some(path.to_string()),
        (None, Some(folder), _) => Some(format!("{}/request.yaml", folder.trim_matches('/'))),
        (None, None, Some(ws)) => implicit_location(ws, &job.request.url),
        _ => None,
    };
    let collection = location
        .as_deref()
        .and_then(|l| l.split('/').next())
        .filter(|_| location.as_deref().is_some_and(|l| l.contains('/')))
        .map(String::from);
    let ancestors = match (job.workspace, &location) {
        (Some(ws), Some(location)) => ws.ancestors(location).unwrap_or_default(),
        _ => vec![],
    };
    let secrets = job
        .workspace
        .and_then(|ws| ws.secret_values(job.environment, job.secrets).ok())
        .unwrap_or_default();
    let mut execution = Execution {
        file: file.clone(),
        location: location.clone(),
        sent: None,
        unresolved: BTreeSet::new(),
        response: Err(Failure::new("skipped", "not sent")),
        checks: vec![],
        tests: vec![],
        script_errors: vec![],
        console: vec![],
        visualizer: None,
        next_request: None,
        locals: job.scripting.locals.cloned().unwrap_or_default(),
        secrets,
    };
    let request_name = job
        .path
        .map(|p| {
            p.rsplit('/')
                .next()
                .unwrap_or(p)
                .trim_end_matches(".yaml")
                .to_string()
        })
        .unwrap_or_else(|| "Untitled".into());
    let scripts_of = |pick: fn(&files::Scripts) -> &String, file: &RequestFile| -> Vec<Script> {
        ancestors
            .iter()
            .map(|a| Script {
                label: format!(
                    "{} {}",
                    if a.collection { "collection" } else { "folder" },
                    a.name
                ),
                source: pick(&a.folder.scripts).clone(),
            })
            .chain([Script {
                label: "request".into(),
                source: pick(&file.scripts).clone(),
            }])
            .filter(|s| !s.source.trim().is_empty())
            .collect()
    };
    let script_input = |execution: &Execution| -> Result<script::Input, workspace::Error> {
        let (vars, vault) = match job.workspace {
            Some(ws) => ws.script_vars(location.as_deref(), job.environment, job.secrets)?,
            None => Default::default(),
        };
        let mut vars = vars;
        vars.data = job.scripting.data.cloned().unwrap_or_default();
        vars.local = execution.locals.clone();
        Ok(script::Input {
            info: script::Info {
                event_name: String::new(),
                request_name: request_name.clone(),
                request_id: job.path.unwrap_or_default().to_string(),
                iteration: job.scripting.iteration,
                iteration_count: job.scripting.iteration_count,
                location: ancestors.iter().map(|a| a.name.clone()).collect(),
            },
            request: Value::Null,
            response: None,
            vars,
            vault,
        })
    };
    let run_scripts = |event: Event,
                       scripts: &[Script],
                       input: script::Input,
                       execution: &mut Execution|
     -> script::Output {
        let output = blocking(|| {
            script::run(
                event,
                scripts,
                &input,
                job.allow_host.clone(),
                job.network.clone(),
                job.scripting.timeout,
            )
        });
        if let Some(ws) = job.workspace
            && let Err(e) = store_changes(
                ws,
                &input.vars,
                &output.vars,
                job.environment,
                collection.as_deref(),
                job.secrets,
            )
        {
            execution.script_errors.push(ScriptError {
                script: "variables".into(),
                message: format!("could not store variables: {e}"),
            });
        }
        execution.locals = output.vars.local.clone();
        execution.tests.extend(output.tests.iter().cloned());
        execution
            .script_errors
            .extend(output.errors.iter().cloned());
        execution.console.extend(output.console.iter().cloned());
        if output.next_request.is_some() {
            execution.next_request = output.next_request.clone();
        }
        if output.visualizer.is_some() {
            execution.visualizer = output.visualizer.clone();
        }
        output
    };

    // Pre-request scripts
    let pre = if job.scripting.enabled {
        scripts_of(|s| &s.pre_request, &file)
    } else {
        vec![]
    };
    if !pre.is_empty() {
        let mut input = match script_input(&execution) {
            Ok(input) => input,
            Err(e) => {
                execution.response = Err(Failure::new("workspace", e.to_string()));
                return execution;
            }
        };
        input.request = script::request_value(&file, &request_name, job.path.unwrap_or_default());
        let before = input.request.clone();
        let output = run_scripts(Event::Prerequest, &pre, input, &mut execution);
        script::apply_request(&mut file, &before, &output.request);
        file.url = absolute_url(&file.url);
        execution.file = file.clone();
        if !execution.script_errors.is_empty() {
            let error = &execution.script_errors[0];
            execution.response = Err(Failure::new(
                "script",
                format!(
                    "the {} pre-request script failed: {}",
                    error.script, error.message
                ),
            ));
            return execution;
        }
        if output.skip_request {
            execution.response = Err(Failure::new(
                "skipped",
                "skipped by pm.execution.skipRequest()",
            ));
            return execution;
        }
    }

    // Variables
    let layers = Layers {
        overrides: job.overrides,
        local: Some(&execution.locals),
        data: job.scripting.data,
    };
    let prepared = match job.workspace {
        Some(ws) => match ws.prepare_with(
            &file,
            location.as_deref(),
            job.environment,
            &layers,
            job.secrets,
        ) {
            Ok(prepared) => prepared,
            Err(e) => {
                execution.response = Err(Failure::new("workspace", e.to_string()));
                return execution;
            }
        },
        None => {
            let mut overrides: Vec<(String, String)> = job.overrides.to_vec();
            overrides.extend(execution.locals.iter().map(|(k, v)| (k.clone(), v.clone())));
            workspace::prepare_standalone(&file, &overrides)
        }
    };
    let mut prepared = prepared;
    execution.unresolved = prepared.unresolved.clone();
    // OAuth 2.0: a stored, refreshed or new token, with the settings' variables already resolved.
    if let http::Auth::OAuth2(oauth) = &mut prepared.request.auth {
        match crate::oauth2::token_for(
            oauth,
            job.workspace.map(|w| w.id()),
            job.secrets,
            job.network.as_deref(),
        )
        .await
        {
            Ok(token) => oauth.access_token = Some(token.access_token),
            Err(e) => {
                execution.response = Err(Failure::new("auth", e));
                return execution;
            }
        }
    }
    if prepared.unresolved.contains("baseUrl")
        && file.url.starts_with("{{baseUrl}}")
        && job.request.url.starts_with('/')
    {
        execution.response = Err(Failure::new(
            "baseUrl",
            format!(
                "\"{}\" is a path, so it needs a baseUrl variable: set one in an environment, pass one for this call, send \
                 it in a collection that has one, or use a full URL",
                job.request.url
            ),
        ));
        return execution;
    }
    if let Some(allow) = &job.allow_host {
        let host = host_of(&prepared.request.url).unwrap_or_default();
        if !allow(&host) {
            execution.response = Err(Failure::new(
                "blocked",
                format!(
                    "{host} is not an allowed host. Agents may call localhost and the hosts in the selected environment; \
                     the user can allow more with `tuma mcp --allow-host {host}`"
                ),
            ));
            return execution;
        }
    }

    // The request
    let result = match &job.network {
        Some(network) => http::send_with(&prepared.request, network).await,
        None => http::send(&prepared.request).await,
    };
    execution.sent = Some(prepared.request.clone());
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
            source: (!job.source.is_empty()).then(|| job.source.to_string()),
        };
        // History is a convenience; failing to write it must not fail the request.
        let _ = history.append(&entry);
    }
    let response = match result {
        Ok(response) => response,
        Err(e) => {
            execution.response = Err(Failure::new(e.kind(), e.to_string()));
            return execution;
        }
    };

    // Checks
    let mut expect = file.expect.clone();
    if let Some(extra) = job.extra_checks {
        expect.merge(extra.clone());
    }
    execution.checks = expect::evaluate(&expect, &response);
    if expect.schema {
        execution.checks.push(contract_check(
            job.workspace,
            location.as_deref(),
            &file,
            &response,
        ));
    }

    // Test scripts
    let post = if job.scripting.enabled {
        scripts_of(|s| &s.test, &file)
    } else {
        vec![]
    };
    if !post.is_empty() {
        match script_input(&execution) {
            Ok(mut input) => {
                input.request = script::sent_request_value(
                    &prepared.request,
                    &response.sent,
                    &request_name,
                    job.path.unwrap_or_default(),
                );
                input.response = Some(script::response_value(&response));
                run_scripts(Event::Test, &post, input, &mut execution);
            }
            Err(e) => execution.script_errors.push(ScriptError {
                script: "variables".into(),
                message: e.to_string(),
            }),
        }
    }
    execution.response = Ok(response);
    execution
}

/// Sends a request and checks the response, returning a summary with secrets masked.
pub async fn send(job: Job<'_>, detail: &Detail) -> Outcome {
    let path = job.path.map(String::from);
    summarize(path.as_deref(), execute(job).await, detail)
}

/// A secret-free summary of an [`Execution`].
pub fn summarize(path: Option<&str>, execution: Execution, detail: &Detail) -> Outcome {
    let secrets = &execution.secrets;
    let mask = |text: &str| mask(text, secrets);
    let passed = execution.passed();
    let method = execution
        .sent
        .as_ref()
        .map(|r| r.method.clone())
        .unwrap_or_else(|| execution.file.method.clone())
        .to_uppercase();
    let tests: Vec<TestResult> = execution
        .tests
        .into_iter()
        .map(|t| TestResult {
            error: t.error.map(|e| mask(&e)),
            ..t
        })
        .collect();
    let script_errors: Vec<ScriptError> = execution
        .script_errors
        .into_iter()
        .map(|e| ScriptError {
            message: mask(&e.message),
            ..e
        })
        .collect();
    let console: Vec<LogLine> = execution
        .console
        .into_iter()
        .map(|l| LogLine {
            text: mask(&l.text),
            ..l
        })
        .collect();
    let unresolved: Vec<String> = execution.unresolved.into_iter().collect();
    let response = match execution.response {
        Ok(response) => response,
        Err(failure) => {
            let url = execution
                .sent
                .as_ref()
                .map(|r| r.url.clone())
                .unwrap_or_else(|| execution.file.url.clone());
            return Outcome {
                path: path.map(String::from),
                method,
                url: mask(&url),
                status: None,
                reason: String::new(),
                time_ms: None,
                size: None,
                headers: vec![],
                body: None,
                body_truncated: false,
                error: Some(mask(&failure.message)),
                skipped: failure.kind == "skipped",
                checks: vec![],
                tests,
                script_errors,
                console,
                unresolved,
                passed: false,
            };
        }
    };
    let checks: Vec<CheckResult> = execution
        .checks
        .into_iter()
        .map(|c| CheckResult {
            actual: mask(&c.actual),
            ..c
        })
        .collect();
    let (body, body_truncated) = summarize_body(&response.body, detail);
    Outcome {
        path: path.map(String::from),
        method,
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
        skipped: false,
        passed,
        checks,
        tests,
        script_errors,
        console,
        unresolved,
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
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
            summary.checks += outcome.checks.len() + outcome.tests.len();
            summary.failed_checks += outcome.checks.iter().filter(|c| !c.passed).count()
                + outcome
                    .tests
                    .iter()
                    .filter(|t| !t.passed && !t.skipped)
                    .count();
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
