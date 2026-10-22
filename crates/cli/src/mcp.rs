//! `tuma mcp`: a Model Context Protocol server over stdio, so coding agents can test the API they are building.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{
    Peer, RoleServer, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tuma_core::expect::{self, Expectations};
use tuma_core::history::History;
use tuma_core::http::Settings;
use tuma_core::runner::{self, Detail, Job, Scripting, Summary};
use tuma_core::script::AllowHost;
use tuma_core::secrets::EnvThenKeychain;
use tuma_core::workspace::files::{Auth, Body, KeyValue, RequestFile};
use tuma_core::workspace::{VariableValue, Workspace};

const INSTRUCTIONS: &str = "\
Tuma sends HTTP requests for you and checks the responses, so you can test the API you are building.

Typical loop: change the code, restart the server, call wait_for, then send_request with `checks`. Read the failed \
checks, fix, repeat. When everything passes, send again with `save_as` to keep the request as a regression test \
(stored as YAML in the project's .tuma/ folder); run_saved reruns them.

URLs can be paths such as /users when a baseUrl variable is set: pass vars {\"baseUrl\": \"http://localhost:3000\"} \
or set it once with set_variables. Write secrets as {{variables}}; their values never reach you.

Check forms: \"status=201\", \"status=2xx\", \"time<500\", \"header content-type~json\", \"$.id exists\", \
\"$.id !exists\", \"$.name == \\\"Ann\\\"\", \"$.count == 3\", \"$.email =~ ^.+@\", \"$.tags ~ admin\", \
\"$.items length 3\", \"$.id type integer\", and \"schema\" (the response matches the OpenAPI spec; import \
one with import_openapi, which can find it on the running server).";

/// Options from the command line.
pub struct Options {
    pub start: PathBuf,
    pub environment: Option<String>,
    pub allow_hosts: Vec<String>,
    pub read_only: bool,
}

#[derive(Clone)]
pub struct Server {
    start: PathBuf,
    environment: Option<String>,
    allow_hosts: Vec<String>,
    read_only: bool,
    tool_router: ToolRouter<Self>,
}

pub async fn serve(options: Options) -> anyhow::Result<()> {
    let server = Server {
        start: options.start,
        environment: options.environment,
        allow_hosts: options
            .allow_hosts
            .into_iter()
            .map(|h| h.to_ascii_lowercase())
            .collect(),
        read_only: options.read_only,
        tool_router: Server::tool_router(),
    };
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

// Tool parameters. Doc comments become the descriptions agents see.

#[derive(Deserialize, schemars::JsonSchema)]
struct SendParams {
    /// GET, POST, PUT, PATCH, DELETE, or any other method.
    method: String,
    /// A full URL, or a path such as /users when a baseUrl variable is set.
    url: String,
    /// Request headers, by name.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// The body. A JSON value is sent as JSON; a string is sent as is (JSON text is detected).
    body: Option<Value>,
    /// The body's content type, when it is neither JSON nor plain text.
    content_type: Option<String>,
    /// Sends "Authorization: Bearer <token>". Use a {{variable}} for real tokens.
    bearer_token: Option<String>,
    /// Checks on the response, such as "status=201" or "$.id exists".
    #[serde(default)]
    checks: Vec<String>,
    /// The environment to use, instead of the server's default.
    environment: Option<String>,
    /// Variables for this call only, such as {"baseUrl": "http://localhost:3000"}.
    #[serde(default)]
    vars: BTreeMap<String, String>,
    /// Return only part of a JSON body, as a JSON path such as $.items[*].id.
    select: Option<String>,
    /// Cut the body after this many characters (default 4000).
    max_body: Option<usize>,
    /// Send as if saved in this collection or folder, to use its variables (such as baseUrl) and auth. Paths in a
    /// workspace with a single collection use that collection.
    collection: Option<String>,
    /// Save the request and its checks as a test at "Collection/Folder/Name", but only if every check passes.
    save_as: Option<String>,
    /// With save_as, replace a saved request with the same name.
    #[serde(default)]
    overwrite: bool,
    /// Milliseconds to wait for a response (default 30000).
    timeout_ms: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RunParams {
    /// A collection, folder or request path such as "Users API/Admin". Empty runs the whole workspace.
    #[serde(default)]
    path: String,
    /// Only run requests that test something: checks, or a test script of their own or inherited.
    #[serde(default)]
    only_tests: bool,
    environment: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PathParams {
    /// A saved request's path, as list_requests shows it.
    path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SaveParams {
    /// Where to save, as "Collection/Folder/Name". Folders are created as needed.
    path: String,
    method: String,
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: Option<Value>,
    content_type: Option<String>,
    bearer_token: Option<String>,
    /// Checks to store with the request, such as "status=200".
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WaitParams {
    /// A URL or /path to poll until the server answers with any HTTP response.
    url: String,
    /// Give up after this many milliseconds (default 30000).
    timeout_ms: Option<u64>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
    environment: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ImportParams {
    /// A spec file (relative to the project) or URL. Leave out to look for one: spec files in the project, then the
    /// usual URLs (/openapi.json, /v3/api-docs, /swagger.json, ...) on `server`.
    source: Option<String>,
    /// A running server to look for a spec on, such as http://localhost:8000.
    server: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SyncParams {
    /// The collection to sync. Leave out to sync every collection made from a spec.
    collection: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct VariablesParams {
    /// The environment to change; it is created if missing. Leave out to change the workspace's global variables.
    environment: Option<String>,
    /// Variables to set. Existing variables with other names are kept. Secrets cannot be set here.
    vars: BTreeMap<String, String>,
}

#[tool_router]
impl Server {
    #[tool(
        description = "Send an HTTP request, check the response and return a short summary. Secret values are \
        masked. Optionally save the request as a test when every check passes."
    )]
    async fn send_request(
        &self,
        Parameters(p): Parameters<SendParams>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        if p.save_as.is_some() && self.read_only {
            return error("Saving is turned off: this server was started with --read-only.");
        }
        let checks = match parse_checks(&p.checks) {
            Ok(checks) => checks,
            Err(e) => return error(&e),
        };
        let request = request_file(RequestParts {
            method: &p.method,
            url: &p.url,
            headers: &p.headers,
            body: p.body.as_ref(),
            content_type: p.content_type.as_deref(),
            bearer_token: p.bearer_token.as_deref(),
            timeout_ms: Some(p.timeout_ms.unwrap_or(30_000)),
        });
        let workspace = match &p.save_as {
            Some(_) => runner::open_or_create(&self.start, None)
                .map(Some)
                .map_err(|e| e.to_string()),
            None => self.workspace(),
        };
        let workspace = match workspace {
            Ok(w) => w,
            Err(e) => return error(&e),
        };
        let environment = p.environment.as_deref().or(self.environment.as_deref());
        let vars: Vec<(String, String)> = p.vars.into_iter().collect();
        let detail = Detail {
            max_body: Some(p.max_body.unwrap_or(4000)),
            select: p.select,
            all_headers: false,
        };
        let outcome = self
            .send(
                workspace.as_ref(),
                &request,
                None,
                p.collection.as_deref(),
                environment,
                &vars,
                Some(&checks),
                &detail,
                &peer,
            )
            .await;

        let mut result = serde_json::to_value(&outcome).unwrap_or_default();
        if let (Some(path), Some(workspace)) = (&p.save_as, &workspace) {
            result["saved"] = if outcome.passed {
                // The 30 s default is for this call; a saved test keeps only a timeout the agent asked for.
                let settings = Settings {
                    timeout_ms: p.timeout_ms,
                    ..request.settings
                };
                let file = RequestFile {
                    expect: checks,
                    settings,
                    ..request
                };
                match workspace.save_request_at(path, &file, p.overwrite) {
                    Ok(saved) => json!(saved),
                    Err(e) => json!(format!("not saved: {e}")),
                }
            } else {
                json!("not saved, because the request failed or a check did not pass")
            };
        }
        ok(&result)
    }

    #[tool(
        description = "Run saved requests under a collection or folder (or all of them) with their checks and \
        test scripts, in order (scripts can change it with pm.execution.setNextRequest). Returns totals, and details \
        only for failures, including test results and script errors."
    )]
    async fn run_saved(
        &self,
        Parameters(p): Parameters<RunParams>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        let workspace = match self.workspace() {
            Ok(Some(w)) => w,
            Ok(None) => {
                return error(
                    "There is no workspace yet. Save a request with send_request's save_as first.",
                );
            }
            Err(e) => return error(&e),
        };
        let environment = p.environment.as_deref().or(self.environment.as_deref());
        let vars: Vec<(String, String)> = p.vars.into_iter().collect();
        let detail = Detail {
            max_body: Some(1500),
            select: None,
            all_headers: false,
        };
        let history = History::default_location();
        let network = match tuma_core::settings::network_for(Some(&workspace), &EnvThenKeychain) {
            Ok(network) => Some(network),
            Err(e) => return error(&format!("network settings: {e}")),
        };
        let report = tuma_core::runs::run(
            tuma_core::runs::RunOptions {
                workspace: &workspace,
                path: &p.path,
                environment,
                overrides: &vars,
                secrets: &EnvThenKeychain,
                history: history.as_ref(),
                source: &source_name(&peer),
                allow_host: Some(self.host_policy(Some(&workspace), environment)),
                network: network.clone(),
                iterations: 1,
                data: vec![],
                delay: Duration::ZERO,
                only_tests: p.only_tests,
                fail_fast: false,
                scripts: true,
                cancel: None,
                detail,
            },
            &mut |_| {},
        )
        .await;
        let report = match report {
            Ok(report) => report,
            Err(e) => return error(&e.to_string()),
        };
        if let Some(network) = &network {
            let _ = tuma_core::settings::keep_cookies(network, Some(&workspace));
        }
        let outcomes: Vec<runner::Outcome> = report.steps.into_iter().map(|s| s.outcome).collect();
        let summary = Summary::of(&outcomes);
        let failures: Vec<_> = outcomes.iter().filter(|o| !o.passed).collect();
        let passed: Vec<_> = outcomes
            .iter()
            .filter(|o| o.passed)
            .map(|o| o.path.clone())
            .collect();
        ok(&json!({ "summary": summary, "failures": failures, "passed": passed }))
    }

    #[tool(
        description = "List the saved requests (with their methods) and the environments in the workspace."
    )]
    async fn list_requests(&self) -> CallToolResult {
        let workspace = match self.workspace() {
            Ok(Some(w)) => w,
            Ok(None) => {
                return ok(&json!({ "workspace": null, "requests": [], "environments": [] }));
            }
            Err(e) => return error(&e),
        };
        let mut requests = Vec::new();
        let tree = workspace.tree().unwrap_or_default();
        fn walk(nodes: &[tuma_core::workspace::Node], out: &mut Vec<Value>) {
            for node in nodes {
                match &node.method {
                    Some(method) => out.push(json!({ "path": node.path, "method": method })),
                    None => walk(&node.children, out),
                }
            }
        }
        walk(&tree, &mut requests);
        ok(&json!({
            "workspace": workspace.root(),
            "requests": requests,
            "environments": workspace.environments().unwrap_or_default(),
            "defaultEnvironment": self.environment,
        }))
    }

    #[tool(
        description = "Check every workspace file after editing YAML by hand: syntax, unknown keys (typos that \
        would be ignored), invalid JSON paths and regular expressions. The JSON Schemas of the files are in \
        Tuma's repository under schemas/."
    )]
    async fn check_workspace(&self) -> CallToolResult {
        match runner::find_workspace(&self.start) {
            Some(root) => ok(&json!({ "problems": tuma_core::check::check_workspace(&root) })),
            None => error("There is no workspace yet."),
        }
    }

    #[tool(description = "Show a saved request's file (YAML), including its checks.")]
    async fn read_request(&self, Parameters(p): Parameters<PathParams>) -> CallToolResult {
        let workspace = match self.workspace() {
            Ok(Some(w)) => w,
            Ok(None) => return error("There is no workspace yet."),
            Err(e) => return error(&e),
        };
        let path = if p.path.ends_with(".yaml") {
            p.path
        } else {
            format!("{}.yaml", p.path)
        };
        match workspace
            .read_request(&path)
            .map(|r| tuma_core::workspace::files::to_yaml(&r))
        {
            Ok(Ok(yaml)) => {
                CallToolResult::success(vec![ContentBlock::text(format!("# {path}\n{yaml}"))])
            }
            Ok(Err(e)) => error(&e.to_string()),
            Err(e) => error(&e.to_string()),
        }
    }

    #[tool(
        description = "Save a request with optional checks without sending it, at \"Collection/Folder/Name\". \
        Creates the workspace (.tuma/) and folders as needed."
    )]
    async fn save_request(&self, Parameters(p): Parameters<SaveParams>) -> CallToolResult {
        if self.read_only {
            return error("Saving is turned off: this server was started with --read-only.");
        }
        let checks = match parse_checks(&p.checks) {
            Ok(checks) => checks,
            Err(e) => return error(&e),
        };
        let mut request = request_file(RequestParts {
            method: &p.method,
            url: &p.url,
            headers: &p.headers,
            body: p.body.as_ref(),
            content_type: p.content_type.as_deref(),
            bearer_token: p.bearer_token.as_deref(),
            timeout_ms: None,
        });
        request.expect = checks;
        let result = runner::open_or_create(&self.start, None)
            .and_then(|w| w.save_request_at(&p.path, &request, p.overwrite));
        match result {
            Ok(saved) => ok(&json!({ "saved": saved })),
            Err(e) => error(&e.to_string()),
        }
    }

    #[tool(
        description = "Wait until a server answers (with any HTTP status), for example after restarting it. \
        Returns how long it took, or an error when it times out."
    )]
    async fn wait_for(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        let workspace = self.workspace().ok().flatten();
        let environment = p.environment.as_deref().or(self.environment.as_deref());
        let vars: Vec<(String, String)> = p.vars.into_iter().collect();
        let request = RequestFile {
            url: p.url.clone(),
            settings: Settings {
                timeout_ms: Some(2000),
                ..Settings::default()
            },
            ..RequestFile::default()
        };
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(30_000));
        let started = Instant::now();
        let allow = self.host_policy(workspace.as_ref(), environment);
        let mut last_error = String::new();
        while started.elapsed() < timeout {
            let outcome = runner::send(
                Job {
                    workspace: workspace.as_ref(),
                    request: &request,
                    path: None,
                    context: None,
                    environment,
                    overrides: &vars,
                    extra_checks: None,
                    secrets: &EnvThenKeychain,
                    // Polling would flood history.
                    history: None,
                    source: "mcp",
                    allow_host: Some(allow.clone()),
                    // Polling runs no scripts: they would run, and change variables, on every attempt.
                    scripting: Scripting {
                        enabled: false,
                        ..Default::default()
                    },
                    network: tuma_core::settings::network_for(workspace.as_ref(), &EnvThenKeychain)
                        .ok(),
                    observer: None,
                },
                &Detail::default(),
            )
            .await;
            match (outcome.status, outcome.error) {
                (Some(status), _) => {
                    return ok(
                        &json!({ "ready": true, "status": status, "waitedMs": started.elapsed().as_millis() as u64 }),
                    );
                }
                (None, Some(e))
                    if e.contains("not an allowed host") || e.contains("needs a baseUrl") =>
                {
                    return error(&e);
                }
                (None, Some(e)) => last_error = e,
                (None, None) => {}
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        error(&format!(
            "No response after {} ms. Last error: {last_error}",
            timeout.as_millis()
        ))
    }

    #[tool(
        description = "Import an OpenAPI/Swagger spec as a collection: a request per operation with example bodies, \
        auth from the security schemes, and baseUrl from the servers. Finds the spec if no source is given. Then \
        \"schema\" checks compare responses with the spec."
    )]
    async fn import_openapi(&self, Parameters(p): Parameters<ImportParams>) -> CallToolResult {
        use tuma_core::openapi::{self, Spec};
        if self.read_only {
            return error("Importing is turned off: this server was started with --read-only.");
        }
        let workspace = match runner::open_or_create(&self.start, None) {
            Ok(w) => w,
            Err(e) => return error(&e.to_string()),
        };
        let found: Result<(String, Spec), String> = match (&p.source, &p.server) {
            (Some(source), _) => {
                let source = tuma_core::openapi::source_for(source, &self.start, workspace.root());
                match openapi::load(&source, workspace.root())
                    .await
                    .and_then(|t| Spec::parse(&t))
                {
                    Ok(spec) => Ok((source, spec)),
                    Err(e) => Err(e),
                }
            }
            (None, server) => {
                let project = workspace
                    .root()
                    .parent()
                    .unwrap_or(workspace.root())
                    .to_path_buf();
                let from_files = openapi::discover_files(&project).into_iter().find_map(|f| {
                    let spec = std::fs::read_to_string(&f)
                        .ok()
                        .and_then(|t| Spec::parse(&t).ok())?;
                    Some((
                        tuma_core::openapi::source_for(
                            &f.to_string_lossy(),
                            &self.start,
                            workspace.root(),
                        ),
                        spec,
                    ))
                });
                match (from_files, server) {
                    (Some(found), _) => Ok(found),
                    (None, Some(server)) => openapi::discover_on_server(server)
                        .await
                        .ok_or_else(|| format!("No spec file in the project and none at the usual paths on {server}.")),
                    (None, None) => Err("No spec file in the project. Pass `server` (where the API runs) or `source`.".into()),
                }
            }
        };
        let (source, spec) = match found {
            Ok(found) => found,
            Err(e) => return error(&e),
        };
        match openapi::import(&workspace, &spec, Some(&source)) {
            Ok((path, warnings)) => {
                let requests = runner::requests_under(&workspace, &path).unwrap_or_default();
                ok(
                    &json!({ "collection": path, "source": source, "requests": requests, "warnings": warnings.0 }),
                )
            }
            Err(e) => error(&e.to_string()),
        }
    }

    #[tool(
        description = "Update collections made from a spec after the spec changed: adds new operations, updates \
        changed URLs, and lists requests whose operation is gone. Saved bodies, auth and checks are kept."
    )]
    async fn sync_openapi(&self, Parameters(p): Parameters<SyncParams>) -> CallToolResult {
        use tuma_core::openapi::{self, Spec};
        if self.read_only {
            return error("Syncing is turned off: this server was started with --read-only.");
        }
        let workspace = match self.workspace() {
            Ok(Some(w)) => w,
            Ok(None) => {
                return error(
                    "There is no workspace yet. Import a spec with import_openapi first.",
                );
            }
            Err(e) => return error(&e),
        };
        let collections: Vec<String> = match p.collection {
            Some(c) => vec![c],
            None => workspace
                .tree()
                .unwrap_or_default()
                .into_iter()
                .map(|n| n.path)
                .collect(),
        };
        let mut reports = serde_json::Map::new();
        for collection in collections {
            let Ok(Some(source)) = workspace.read_folder(&collection).map(|f| f.openapi) else {
                continue;
            };
            let result = match openapi::load(&source, workspace.root())
                .await
                .and_then(|t| Spec::parse(&t))
            {
                Ok(spec) => openapi::sync(&workspace, &collection, &spec)
                    .map(|r| json!(r))
                    .unwrap_or_else(|e| json!(e.to_string())),
                Err(e) => json!(format!("could not read {source}: {e}")),
            };
            reports.insert(collection, result);
        }
        if reports.is_empty() {
            return error("No collection here was imported from a spec.");
        }
        ok(&Value::Object(reports))
    }

    #[tool(
        description = "Set variables (such as baseUrl) in an environment, or in the workspace's globals. Not for \
        secrets: the user sets those in the Tuma app, and you refer to them as {{name}}."
    )]
    async fn set_variables(&self, Parameters(p): Parameters<VariablesParams>) -> CallToolResult {
        if self.read_only {
            return error(
                "Changing variables is turned off: this server was started with --read-only.",
            );
        }
        let result = (|| -> Result<Value, tuma_core::workspace::Error> {
            let mut workspace = runner::open_or_create(&self.start, None)?;
            let secrets = EnvThenKeychain;
            let mut current = match &p.environment {
                Some(env) if workspace.environments()?.contains(env) => {
                    workspace.read_environment(env, &secrets)?
                }
                Some(env) => {
                    workspace.create_environment(env)?;
                    vec![]
                }
                None => workspace.read_globals(&secrets)?,
            };
            for (name, value) in &p.vars {
                if current.iter().any(|v| v.name == *name && v.secret) {
                    continue;
                }
                match current.iter_mut().find(|v| v.name == *name) {
                    Some(v) => v.value = value.clone(),
                    None => current.push(VariableValue {
                        name: name.clone(),
                        value: value.clone(),
                        secret: false,
                        enabled: true,
                    }),
                }
            }
            match &p.environment {
                Some(env) => workspace.write_environment(env, &current, &secrets)?,
                None => workspace.write_globals(&current, &secrets)?,
            }
            let names: Vec<&String> = current.iter().map(|v| &v.name).collect();
            Ok(json!({ "environment": p.environment, "variables": names }))
        })();
        match result {
            Ok(v) => ok(&v),
            Err(e) => error(&e.to_string()),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tuma", tuma_core::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

impl Server {
    /// The workspace, re-read on every call so edits made meanwhile are seen. `None` until one exists.
    fn workspace(&self) -> Result<Option<Workspace>, String> {
        runner::find_workspace(&self.start)
            .map(Workspace::open)
            .transpose()
            .map_err(|e| e.to_string())
    }

    /// Agents may call local servers, the hosts the environment points at, and hosts the user allowed. Applies to
    /// requests and to `pm.sendRequest` in their scripts.
    fn host_policy(&self, workspace: Option<&Workspace>, environment: Option<&str>) -> AllowHost {
        let allowed = self.allow_hosts.clone();
        let environment_hosts: Vec<String> = match (workspace, environment) {
            (Some(workspace), Some(environment)) => workspace
                .read_environment(environment, &EnvThenKeychain)
                .unwrap_or_default()
                .iter()
                .filter(|v| !v.secret)
                .filter_map(|v| runner::host_of(&v.value).filter(|_| v.value.contains("://")))
                .collect(),
            _ => vec![],
        };
        Arc::new(move |host: &str| {
            let local = matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
                || host.ends_with(".localhost");
            local
                || allowed.iter().any(|a| {
                    a == "*"
                        || a == host
                        || host.ends_with(&format!(".{}", a.trim_start_matches("*.")))
                })
                || environment_hosts.iter().any(|h| h == host)
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn send(
        &self,
        workspace: Option<&Workspace>,
        request: &RequestFile,
        path: Option<&str>,
        context: Option<&str>,
        environment: Option<&str>,
        vars: &[(String, String)],
        checks: Option<&Expectations>,
        detail: &Detail,
        peer: &Peer<RoleServer>,
    ) -> runner::Outcome {
        let source = source_name(peer);
        let history = History::default_location();
        let network = match tuma_core::settings::network_for(workspace, &EnvThenKeychain) {
            Ok(network) => network,
            Err(e) => {
                return runner::Outcome {
                    method: request.method.clone(),
                    url: request.url.clone(),
                    error: Some(format!("network settings: {e}")),
                    ..Default::default()
                };
            }
        };
        let outcome = runner::send(
            Job {
                workspace,
                request,
                path,
                context,
                environment,
                overrides: vars,
                extra_checks: checks,
                secrets: &EnvThenKeychain,
                history: history.as_ref(),
                source: &source,
                allow_host: Some(self.host_policy(workspace, environment)),
                scripting: Default::default(),
                network: Some(network.clone()),
                observer: None,
            },
            detail,
        )
        .await;
        let _ = tuma_core::settings::keep_cookies(&network, workspace);
        outcome
    }
}

/// How history names an agent: `mcp:<client name>`.
fn source_name(peer: &Peer<RoleServer>) -> String {
    match peer.peer_info() {
        Some(info) => format!("mcp:{}", info.client_info.name),
        None => "mcp".to_string(),
    }
}

struct RequestParts<'a> {
    method: &'a str,
    url: &'a str,
    headers: &'a BTreeMap<String, String>,
    body: Option<&'a Value>,
    content_type: Option<&'a str>,
    bearer_token: Option<&'a str>,
    timeout_ms: Option<u64>,
}

fn request_file(p: RequestParts<'_>) -> RequestFile {
    let body = match p.body {
        None | Some(Value::Null) => Body::None,
        Some(value) => {
            let (content, json) = match value {
                Value::String(s) => (s.clone(), serde_json::from_str::<Value>(s).is_ok()),
                other => (
                    serde_json::to_string_pretty(other).unwrap_or_default(),
                    true,
                ),
            };
            let content_type = p.content_type.unwrap_or(if json {
                "application/json"
            } else {
                "text/plain"
            });
            Body::text(content, content_type)
        }
    };
    RequestFile {
        method: p.method.to_uppercase(),
        url: p.url.to_string(),
        headers: p
            .headers
            .iter()
            .map(|(name, value)| KeyValue {
                name: name.clone(),
                value: value.clone(),
                enabled: true,
            })
            .collect(),
        body,
        auth: p.bearer_token.map(|token| Auth::Bearer {
            token: token.to_string(),
        }),
        settings: Settings {
            timeout_ms: p.timeout_ms,
            ..Settings::default()
        },
        ..RequestFile::default()
    }
}

fn parse_checks(checks: &[String]) -> Result<Expectations, String> {
    let mut expect = Expectations::default();
    for check in checks {
        expect.merge(expect::parse_short(check)?);
    }
    Ok(expect)
}

fn ok(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )])
}

fn error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_string())])
}
