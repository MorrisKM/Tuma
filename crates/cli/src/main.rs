//! `tuma`: send requests and run saved tests from a terminal, CI or an AI agent, and serve MCP.

mod mcp;
mod output;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use tuma_core::expect::{self, Expectations};
use tuma_core::history::History;
use tuma_core::http::Settings;
use tuma_core::runner::{self, Detail, Job};
use tuma_core::runs;
use tuma_core::secrets::EnvThenKeychain;
use tuma_core::settings;
use tuma_core::workspace::Workspace;
use tuma_core::workspace::files::{Auth, Body, KeyValue, RequestFile};

#[derive(Parser)]
#[command(
    name = "tuma",
    version,
    about = "A fast, local-first API client, for terminals, CI and AI agents"
)]
struct Cli {
    /// The workspace folder. By default, the nearest folder with workspace.yaml or .tuma/ from the current folder up.
    #[arg(long, global = true, env = "TUMA_WORKSPACE")]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a request and check the response.
    ///
    /// The URL can be a path such as /users, which uses the baseUrl variable of the environment.
    ///
    /// Checks (--expect), one per flag:
    ///   status=201  status=2xx  time<500
    ///   header content-type~json  header x-id  header x-id=abc  header !x-debug
    ///   $.id exists  $.id !exists  $.name == "Ann"  $.count == 3  $.email ~ @  $.email =~ ^.+@
    ///   $.items length 3  $.id type integer
    #[command(verbatim_doc_comment)]
    Send(Box<SendArgs>),
    /// Run every saved request under a collection or folder (or the whole workspace), with their checks.
    Run(RunArgs),
    /// Run the saved requests that have checks (an expect: block), and fail if any check fails.
    Test(RunArgs),
    /// Serve the Model Context Protocol over stdio, so coding agents can send requests and run tests.
    ///
    /// Add it to your agent, for example: claude mcp add tuma -- tuma mcp
    Mcp {
        /// The environment to use when a tool call does not name one.
        #[arg(short, long = "env", env = "TUMA_ENV")]
        environment: Option<String>,
        /// Also let agents call this host (repeat for more; *.example.com and * work). localhost and the hosts in the
        /// selected environment are always allowed.
        #[arg(long = "allow-host", value_name = "HOST")]
        allow_hosts: Vec<String>,
        /// Don't let agents save requests or change variables.
        #[arg(long)]
        read_only: bool,
    },
    /// Import a file into the workspace: an OpenAPI/Swagger spec, a Postman collection or environment, or a HAR file.
    Import { file: PathBuf },
    /// Export a collection as Postman Collection v2.1 JSON, which Postman, Insomnia and Bruno can import.
    Export {
        /// The collection to export.
        collection: String,
        /// Write to this file instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate API docs from the saved requests: OpenAPI 3.1, Markdown or a standalone HTML page.
    ///
    /// For one request ("Shop/Orders/Get order"), a folder or collection ("Shop"), or the whole workspace. Request
    /// descriptions and saved examples make them complete; secret values are never included.
    Docs {
        /// A request, folder or collection; the whole workspace when left out.
        scope: Option<String>,
        /// openapi (YAML), openapi-json, markdown or html. Guessed from --output's extension when not given.
        #[arg(short, long)]
        format: Option<String>,
        /// The environment whose base URL the docs show.
        #[arg(short, long)]
        env: Option<String>,
        /// Write to this file instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run a mock server that answers with the saved examples of a collection's or folder's requests.
    ///
    /// Requests are matched by method and path ({{name}} and :name segments match anything). Choose an example with
    /// an x-mock-response-code or x-mock-response-name header; otherwise the first successful one is sent. Edits to
    /// examples apply at once. Stop it with Ctrl+C.
    Mock {
        /// A collection or folder; the whole workspace when left out.
        scope: Option<String>,
        #[arg(short, long, default_value_t = 4010)]
        port: u16,
        /// Wait this long before every answer, in milliseconds.
        #[arg(long, default_value_t = 0)]
        delay: u64,
    },
    /// Load-test a request, folder or collection: virtual users send its requests over and over.
    ///
    /// Reports throughput, error rate and latency percentiles per request, with passive security checks of the
    /// responses (headers, cookies, CORS, HTTPS, leaks). With --max-p95 or --max-errors, exits with 1 when they fail.
    Perf {
        /// A request ("Shop/Orders/List"), folder or collection.
        scope: String,
        /// Virtual users sending at the same time (at most 200).
        #[arg(short, long, default_value_t = 10)]
        users: u32,
        /// How long to run, in seconds. 0 runs --iterations times instead.
        #[arg(short, long, default_value_t = 30)]
        duration: u64,
        #[arg(short, long, default_value_t = 1)]
        iterations: u32,
        /// Start the users over this many seconds.
        #[arg(long, default_value_t = 0)]
        ramp_up: u64,
        /// Think time between a user's requests, in milliseconds.
        #[arg(long, default_value_t = 0)]
        delay: u64,
        #[arg(short, long)]
        env: Option<String>,
        /// Fail when the 95th percentile latency is above this many milliseconds.
        #[arg(long)]
        max_p95: Option<f64>,
        /// Fail when more than this percentage of requests fail.
        #[arg(long)]
        max_errors: Option<f64>,
        /// Write the report: .html or .json.
        #[arg(short, long)]
        report: Option<PathBuf>,
    },
    /// Check every file in the workspace: YAML syntax, unknown keys (typos), and checks that can't work.
    Check {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Import an OpenAPI or Swagger spec as a collection, find one, or sync a collection with its spec.
    #[command(subcommand)]
    Openapi(OpenapiCommand),
    /// Call a gRPC method, or list a server's methods. Methods come from server reflection, or from --proto files.
    ///
    ///   tuma grpc grpc://localhost:50051 --list
    ///   tuma grpc grpc://localhost:50051 shop.v1.Shop/GetItem -d '{"id": 7}'
    #[command(verbatim_doc_comment)]
    Grpc(GrpcArgs),
    /// Open a WebSocket, send messages, and print what comes back.
    ///
    ///   tuma ws wss://echo.example.com -m '{"type":"ping"}' --wait 3
    #[command(verbatim_doc_comment)]
    Ws(WsArgs),
    /// OAuth 2.0 tokens: log in for a request, folder or collection, list tokens, or forget them.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Create a workspace: .tuma/ at the root of the current project.
    Init {
        /// The workspace's name. Defaults to the project folder's name.
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Args)]
struct GrpcArgs {
    /// A grpc:// or grpcs:// address, or a saved gRPC request.
    target: String,
    /// The method, as package.Service/Method. A saved request has its own.
    method: Option<String>,
    /// A JSON message. Repeat for client-streaming calls; {{variables}} work.
    #[arg(short = 'd', long = "data")]
    messages: Vec<String>,
    /// List the services and methods instead of calling one.
    #[arg(long)]
    list: bool,
    /// .proto files describing the service, when the server has no reflection.
    #[arg(long = "proto")]
    protos: Vec<String>,
    /// Folders imports are found in.
    #[arg(short = 'I', long = "import")]
    imports: Vec<String>,
    /// Metadata, as "name: value". Repeat for more.
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// Send "authorization: Bearer <token>".
    #[arg(long)]
    bearer: Option<String>,
    /// Don't verify the server's certificate (grpcs://).
    #[arg(short = 'k', long)]
    insecure: bool,
    #[command(flatten)]
    common: Common,
}

#[derive(Args)]
struct WsArgs {
    /// A ws:// or wss:// URL, or a saved WebSocket request.
    target: String,
    /// A header for the handshake, as "Name: value". Repeat for more.
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// A message to send once connected. Repeat for more; they are sent in order. {{variables}} work.
    #[arg(short = 'm', long = "message")]
    messages: Vec<String>,
    /// Also send these saved messages of a saved request, by name.
    #[arg(long = "saved")]
    saved: Vec<String>,
    /// How long to keep listening after the last message, in seconds; 0 waits until the server closes.
    #[arg(long, default_value_t = 2.0)]
    wait: f64,
    /// Send "Authorization: Bearer <token>".
    #[arg(long)]
    bearer: Option<String>,
    #[arg(long, value_name = "FOLDER")]
    r#in: Option<String>,
    #[command(flatten)]
    common: Common,
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Get a token with the OAuth 2.0 settings of a request, folder or collection; browser grants open a browser.
    Login {
        /// Such as "Users API" or "Users API/Admin/list users".
        path: String,
        #[arg(short, long = "env", env = "TUMA_ENV")]
        environment: Option<String>,
    },
    /// List the workspace's stored tokens.
    Tokens,
    /// Forget a stored token (by its key, from `tuma auth tokens`), or all of them.
    Logout { key: Option<String> },
}

#[derive(Subcommand)]
enum OpenapiCommand {
    /// Import a spec (a file or URL, JSON or YAML) as a new collection.
    Import { source: String },
    /// Look for a spec: spec files in the project, and the usual URLs on a running server.
    Discover {
        /// A running server to ask, such as http://localhost:8000.
        #[arg(long)]
        server: Option<String>,
        /// Import the first spec found.
        #[arg(long)]
        import: bool,
    },
    /// Update a collection from its spec: add new operations, update changed ones, list removed ones.
    Sync {
        /// The collection. Defaults to every collection made from a spec.
        collection: Option<String>,
    },
}

#[derive(Args)]
struct SendArgs {
    /// GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS or any other method.
    method: String,
    url: String,
    /// A header, as "Name: value". Repeat for more.
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// The body. JSON is detected; @path sends a file's contents.
    #[arg(short = 'd', long)]
    body: Option<String>,
    /// The body's content type, when it is not JSON or plain text.
    #[arg(long)]
    content_type: Option<String>,
    /// Send "Authorization: Bearer <token>". Use {{variables}} for secrets.
    #[arg(long, conflicts_with = "basic")]
    bearer: Option<String>,
    /// Send basic auth, as user:password.
    #[arg(long)]
    basic: Option<String>,
    /// A check on the response. Repeat for more; see above for the forms.
    #[arg(short = 'x', long = "expect")]
    checks: Vec<String>,
    /// Send as if saved in this collection or folder, to use its variables (such as baseUrl) and auth. A path URL
    /// in a workspace with a single collection uses that collection.
    #[arg(long = "in", value_name = "FOLDER")]
    context: Option<String>,
    /// Save the request with its checks as a test, at "Collection/Folder/Name", if every check passes.
    #[arg(long, value_name = "PATH")]
    save: Option<String>,
    /// With --save, replace a saved request with the same name.
    #[arg(long, requires = "save")]
    overwrite: bool,
    /// Give up after this many milliseconds.
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
    /// Accept any TLS certificate, for self-signed development servers.
    #[arg(short = 'k', long)]
    insecure: bool,
    #[command(flatten)]
    common: Common,
}

#[derive(Args)]
struct RunArgs {
    /// A collection, folder or request, such as "Users API/Admin". Defaults to the whole workspace.
    path: Option<String>,
    /// Stop at the first failure.
    #[arg(long)]
    fail_fast: bool,
    /// Run everything this many times. With --data, at least once per row.
    #[arg(short = 'n', long, default_value_t = 1)]
    iterations: usize,
    /// A CSV (with a header row) or JSON file: row N is pm.iterationData, and {{column}} variables, in iteration N.
    #[arg(long, value_name = "FILE")]
    data: Option<PathBuf>,
    /// Wait this many milliseconds between requests.
    #[arg(long, value_name = "MS", default_value_t = 0)]
    delay: u64,
    /// Also write the results as JUnit XML, which CI systems show as test results.
    #[arg(long, value_name = "FILE")]
    junit: Option<PathBuf>,
    /// Don't run pre-request and test scripts.
    #[arg(long)]
    no_scripts: bool,
    #[command(flatten)]
    common: Common,
}

#[derive(Args)]
struct Common {
    /// The environment whose variables to use.
    #[arg(short, long = "env", env = "TUMA_ENV")]
    environment: Option<String>,
    /// A variable for this run only, as NAME=VALUE, such as baseUrl=http://localhost:3000. Wins over every scope.
    #[arg(long = "var", value_name = "NAME=VALUE", value_parser = parse_var)]
    vars: Vec<(String, String)>,
    /// Print JSON instead of text, for scripts and agents.
    #[arg(long)]
    json: bool,
    /// Show only these parts of a JSON body, as a JSON path such as $.items[*].id.
    #[arg(long, value_name = "PATH")]
    select: Option<String>,
    /// Cut bodies after this many characters. Defaults to everything for text output and 8000 for --json.
    #[arg(long, value_name = "CHARS")]
    max_body: Option<usize>,
    /// Show every response header, not only the useful ones.
    #[arg(long)]
    all_headers: bool,
}

fn parse_var(text: &str) -> Result<(String, String), String> {
    let (name, value) = text.split_once('=').ok_or("expected NAME=VALUE")?;
    Ok((name.trim().to_string(), value.to_string()))
}

impl Common {
    fn detail(&self) -> Detail {
        Detail {
            max_body: self.max_body.or(if self.json { Some(8000) } else { None }),
            select: self.select.clone(),
            all_headers: self.all_headers,
        }
    }
}

fn print_imported(imported: &tuma_core::import::Imported) {
    use tuma_core::import::Imported;
    match imported {
        Imported::Collection(path) => println!("Imported collection \"{path}\""),
        Imported::Environment(name) => println!("Imported environment \"{name}\""),
        Imported::Globals => println!("Imported global variables"),
        Imported::Several(all) => all.iter().for_each(print_imported),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("tuma: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether everything passed.
async fn run(cli: Cli) -> anyhow::Result<bool> {
    let cwd = std::env::current_dir()?;
    let start = cli.workspace.clone().unwrap_or(cwd);
    match cli.command {
        Command::Send(args) => send(&start, *args).await,
        Command::Run(args) => run_saved(&start, args, false).await,
        Command::Test(args) => run_saved(&start, args, true).await,
        Command::Openapi(command) => openapi(&start, command).await,
        Command::Import { file } => {
            let mut workspace = runner::open_or_create(&start, None)?;
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("could not read {}", file.display()))?;
            let source =
                tuma_core::openapi::source_for(&file.to_string_lossy(), &start, workspace.root());
            let (imported, warnings) =
                tuma_core::import::any(&mut workspace, &text, &source, &EnvThenKeychain)
                    .map_err(anyhow::Error::msg)?;
            print_imported(&imported);
            for warning in warnings.0 {
                println!("  ! {warning}");
            }
            Ok(true)
        }
        Command::Export { collection, output } => {
            let Some(workspace) = find_workspace(&start)? else {
                bail!("no workspace here or above {}", start.display());
            };
            let tree = workspace.read_tree(&collection)?;
            let json =
                serde_json::to_string_pretty(&tuma_core::export::postman::collection(&tree))?;
            match output {
                Some(path) => {
                    std::fs::write(&path, json + "\n")?;
                    eprintln!("Wrote {}", path.display());
                }
                None => println!("{json}"),
            }
            Ok(true)
        }
        Command::Docs {
            scope,
            format,
            env,
            output,
        } => {
            let Some(workspace) = find_workspace(&start)? else {
                bail!("no workspace here or above {}", start.display());
            };
            let guessed = output
                .as_ref()
                .and_then(|o| o.extension())
                .map(|e| e.to_string_lossy().to_string());
            let format = match format.as_deref().or(guessed.as_deref()) {
                None => tuma_core::docs::Format::Markdown,
                Some(name) => tuma_core::docs::Format::parse(name).with_context(|| {
                    format!("unknown format {name}; use openapi, openapi-json, markdown or html")
                })?,
            };
            let scope = scope.map(|s| {
                let s = s.trim_matches('/').to_string();
                if !s.ends_with(".yaml") && workspace.root().join(format!("{s}.yaml")).is_file() {
                    format!("{s}.yaml")
                } else {
                    s
                }
            });
            let docs = tuma_core::docs::generate(
                &workspace,
                scope.as_deref(),
                env.as_deref(),
                format,
                &EnvThenKeychain,
            )
            .map_err(anyhow::Error::msg)?;
            for warning in &docs.warnings {
                eprintln!("  ! {warning}");
            }
            match output {
                Some(path) => {
                    std::fs::write(&path, &docs.text)?;
                    eprintln!("Wrote {}", path.display());
                }
                None => print!("{}", docs.text),
            }
            Ok(true)
        }
        Command::Mock { scope, port, delay } => {
            let Some(workspace) = find_workspace(&start)? else {
                bail!("no workspace here or above {}", start.display());
            };
            let routes = tuma_core::mock::routes(&workspace, scope.as_deref())
                .map_err(anyhow::Error::msg)?;
            let server = tuma_core::mock::start(
                tuma_core::mock::Options {
                    root: workspace.root().to_path_buf(),
                    scope: scope.clone(),
                    port,
                    delay_ms: delay,
                },
                std::sync::Arc::new(|e: tuma_core::mock::MockEvent| {
                    let by = e.request.map(|r| {
                        format!(
                            "  {r}{}",
                            e.example.map(|x| format!(" › {x}")).unwrap_or_default()
                        )
                    });
                    eprintln!(
                        "{} {} {}{}",
                        e.status,
                        e.method,
                        e.path,
                        by.unwrap_or_default()
                    );
                }),
            )
            .await
            .map_err(anyhow::Error::msg)?;
            eprintln!(
                "Mock server for {} at {}",
                scope.as_deref().unwrap_or(workspace.name()),
                server.url()
            );
            for r in &routes {
                let examples = if r.examples.is_empty() {
                    "no examples: answers 501".to_string()
                } else {
                    r.examples
                        .iter()
                        .map(|e| format!("{} {}", e.status, e.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                eprintln!(
                    "  {:<7} {:<32} {} ({examples})",
                    r.method,
                    r.pattern(),
                    r.name
                );
            }
            eprintln!("Press Ctrl+C to stop.");
            tokio::signal::ctrl_c().await?;
            server.stop();
            Ok(true)
        }
        Command::Perf {
            scope,
            users,
            duration,
            iterations,
            ramp_up,
            delay,
            env,
            max_p95,
            max_errors,
            report,
        } => {
            let Some(workspace) = find_workspace(&start)? else {
                bail!("no workspace here or above {}", start.display());
            };
            let scope = {
                let s = scope.trim_matches('/').to_string();
                if !s.ends_with(".yaml") && workspace.root().join(format!("{s}.yaml")).is_file() {
                    format!("{s}.yaml")
                } else {
                    s
                }
            };
            let network = tuma_core::settings::network_for(Some(&workspace), &EnvThenKeychain)
                .map_err(anyhow::Error::msg)?;
            let (targets, notes) = tuma_core::perf::prepare(
                &workspace,
                Some(&scope),
                env.as_deref(),
                &EnvThenKeychain,
                Some(&network),
            )
            .await
            .map_err(anyhow::Error::msg)?;
            for note in &notes {
                eprintln!("  ! {note}");
            }
            let options = tuma_core::perf::Options {
                users,
                duration_s: duration,
                iterations,
                ramp_up_s: ramp_up,
                delay_ms: delay,
                max_p95_ms: max_p95,
                max_error_rate: max_errors,
            };
            let (stop, stopped) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = stop.send(true);
                }
            });
            let last_line = std::sync::Mutex::new(std::time::Instant::now());
            let name = scope.trim_end_matches(".yaml").to_string();
            eprintln!(
                "Load testing {name}: {} request(s), {users} users. Ctrl+C stops.",
                targets.len()
            );
            let result = tuma_core::perf::run(
                &name,
                targets,
                options,
                env.clone(),
                notes,
                network,
                std::sync::Arc::new(move |r: tuma_core::perf::Report| {
                    let mut last = last_line.lock().unwrap();
                    if !r.finished && last.elapsed().as_secs() >= 2 {
                        *last = std::time::Instant::now();
                        eprintln!(
                            "  {:>4}s  {:>7} requests  {:>7.1}/s  p95 {:>6.0} ms  errors {:.2}%",
                            r.elapsed_ms / 1000,
                            r.requests,
                            r.throughput,
                            r.latency.p95,
                            r.error_rate
                        );
                    }
                }),
                stopped,
            )
            .await;
            println!(
                "\n{} requests in {:.1} s: {:.1}/s, {:.2}% errors, avg {:.0} ms, p50 {:.0}, p95 {:.0}, p99 {:.0} ms{}",
                result.requests,
                result.elapsed_ms as f64 / 1000.0,
                result.throughput,
                result.error_rate,
                result.latency.avg,
                result.latency.p50,
                result.latency.p95,
                result.latency.p99,
                if result.stopped {
                    " (stopped early)"
                } else {
                    ""
                }
            );
            for r in &result.per_request {
                println!(
                    "  {:<6} {:<32} {:>7} req  {:>5} err  p95 {:>6.0} ms",
                    r.method, r.name, r.requests, r.errors, r.latency.p95
                );
            }
            if !result.findings.is_empty() {
                println!("\nSecurity checks (passive):");
                for f in &result.findings {
                    println!(
                        "  {:<6} {}: {} ({})",
                        format!("{:?}", f.severity),
                        f.check,
                        f.request,
                        f.detail
                    );
                }
            }
            for failure in &result.failures {
                println!("FAILED: {failure}");
            }
            if let Some(path) = report {
                let text = if path.extension().is_some_and(|e| e == "json") {
                    tuma_core::perf::report::json(&result)
                } else {
                    tuma_core::perf::report::html(&result)
                };
                std::fs::write(&path, text)?;
                eprintln!("Wrote {}", path.display());
            }
            Ok(result.passed != Some(false))
        }
        Command::Check { json } => {
            let Some(root) = runner::find_workspace(&start) else {
                bail!(
                    "no workspace here or above {}; create one with `tuma init`",
                    start.display()
                );
            };
            let problems = tuma_core::check::check_workspace(&root);
            let errors = problems.iter().filter(|p| !p.warning).count();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "problems": problems }))?
                );
            } else if problems.is_empty() {
                println!("{}: no problems", root.display());
            } else {
                for p in &problems {
                    let at = p.line.map(|l| format!(":{l}")).unwrap_or_default();
                    let kind = if p.warning { "warning" } else { "error" };
                    println!("{}{at}: {kind}: {}", p.file, p.message);
                }
                let warnings = problems.len() - errors;
                let plural =
                    |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
                println!(
                    "\n{}, {}",
                    plural(errors, "error"),
                    plural(warnings, "warning")
                );
            }
            Ok(errors == 0)
        }
        Command::Mcp {
            environment,
            allow_hosts,
            read_only,
        } => {
            mcp::serve(mcp::Options {
                start,
                environment,
                allow_hosts,
                read_only,
            })
            .await?;
            Ok(true)
        }
        Command::Auth(command) => auth(&start, command).await,
        Command::Ws(args) => websocket(&start, args).await,
        Command::Grpc(args) => grpc_command(&start, args).await,
        Command::Init { name } => {
            if let Some(root) = runner::find_workspace(&start) {
                bail!("there is already a workspace at {}", root.display());
            }
            let workspace = runner::open_or_create(&start, name.as_deref())?;
            println!("Created a workspace in {}", workspace.root().display());
            Ok(true)
        }
    }
}

/// The workspace for `start`, if there is one. `send` works without one.
fn find_workspace(start: &Path) -> anyhow::Result<Option<Workspace>> {
    runner::find_workspace(start)
        .map(Workspace::open)
        .transpose()
        .context("could not open the workspace")
}

async fn send(start: &Path, args: SendArgs) -> anyhow::Result<bool> {
    let mut checks = Expectations::default();
    for check in &args.checks {
        checks.merge(expect::parse_short(check).map_err(anyhow::Error::msg)?);
    }
    let request = request_from_args(&args)?;
    let workspace = match &args.save {
        Some(_) => Some(runner::open_or_create(start, None)?),
        None => find_workspace(start)?,
    };
    let history = History::default_location();
    let detail = args.common.detail();
    let network =
        settings::network_for(workspace.as_ref(), &EnvThenKeychain).map_err(anyhow::Error::msg)?;
    // Server-sent events are printed as they arrive; the summary follows when the stream ends.
    let parser = std::sync::Mutex::new(tuma_core::sse::Parser::default());
    let print_events = |event: tuma_core::http::StreamEvent<'_>| match event {
        tuma_core::http::StreamEvent::Open { status } => {
            eprintln!("Connected ({status}); events follow. Ctrl+C stops.")
        }
        tuma_core::http::StreamEvent::Data(data) => {
            for e in parser.lock().unwrap().feed(data) {
                let id = e.id.map(|id| format!(" #{id}")).unwrap_or_default();
                println!("{}{id}: {}", e.event, e.data);
            }
        }
    };
    let outcome = runner::send(
        Job {
            workspace: workspace.as_ref(),
            request: &request,
            path: None,
            context: args.context.as_deref(),
            environment: args.common.environment.as_deref(),
            overrides: &args.common.vars,
            extra_checks: Some(&checks),
            secrets: &EnvThenKeychain,
            history: history.as_ref(),
            source: "cli",
            allow_host: None,
            scripting: Default::default(),
            network: Some(network.clone()),
            observer: (!args.common.json).then_some(&print_events as &tuma_core::http::Observer),
        },
        &detail,
    )
    .await;
    settings::keep_cookies(&network, workspace.as_ref()).map_err(anyhow::Error::msg)?;

    let mut saved = None;
    if let (Some(path), Some(workspace)) = (&args.save, &workspace)
        && outcome.passed
    {
        let file = RequestFile {
            expect: checks,
            ..request
        };
        saved = Some(workspace.save_request_at(path, &file, args.overwrite)?);
    }

    if args.common.json {
        let mut value = serde_json::to_value(&outcome)?;
        if let Some(saved) = &saved {
            value["saved"] = saved.clone().into();
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        output::outcome(&outcome, true);
        match (&saved, &args.save) {
            (Some(path), _) => println!("\nSaved as {path}"),
            (None, Some(_)) => {
                println!("\nNot saved: a check failed or the request could not be sent.")
            }
            _ => {}
        }
    }
    Ok(outcome.passed)
}

fn request_from_args(args: &SendArgs) -> anyhow::Result<RequestFile> {
    let headers = args
        .headers
        .iter()
        .map(|h| {
            let (name, value) = h
                .split_once(':')
                .with_context(|| format!("header \"{h}\" should be \"Name: value\""))?;
            Ok(KeyValue {
                name: name.trim().into(),
                value: value.trim().into(),
                enabled: true,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let body = match &args.body {
        None => Body::None,
        Some(body) => {
            let content = match body.strip_prefix('@') {
                Some(path) => std::fs::read_to_string(path)
                    .with_context(|| format!("could not read {path}"))?,
                None => body.clone(),
            };
            let content_type = match &args.content_type {
                Some(ct) => ct.clone(),
                None if serde_json::from_str::<serde_json::Value>(&content).is_ok() => {
                    "application/json".into()
                }
                None => "text/plain".into(),
            };
            Body::text(content, &content_type)
        }
    };

    let auth = match (&args.bearer, &args.basic) {
        (Some(token), _) => Some(Auth::Bearer {
            token: token.clone(),
        }),
        (_, Some(basic)) => {
            let (username, password) = basic.split_once(':').unwrap_or((basic, ""));
            Some(Auth::Basic {
                username: username.into(),
                password: password.into(),
            })
        }
        _ => None,
    };

    Ok(RequestFile {
        method: args.method.to_uppercase(),
        url: args.url.clone(),
        headers,
        body,
        auth,
        settings: Settings {
            timeout_ms: args.timeout,
            verify_tls: !args.insecure,
            ..Settings::default()
        },
        ..RequestFile::default()
    })
}

async fn run_saved(start: &Path, args: RunArgs, only_tests: bool) -> anyhow::Result<bool> {
    let Some(workspace) = find_workspace(start)? else {
        bail!(
            "no workspace here or above {}; create one with `tuma init`",
            start.display()
        );
    };
    let data = match &args.data {
        Some(path) => runs::read_data_file(path).map_err(anyhow::Error::msg)?,
        None => vec![],
    };
    let history = History::default_location();
    let network =
        settings::network_for(Some(&workspace), &EnvThenKeychain).map_err(anyhow::Error::msg)?;
    let json = args.common.json;
    let several = args.iterations.max(data.len()) > 1;
    let mut printed_iteration = None;
    let report = runs::run(
        runs::RunOptions {
            workspace: &workspace,
            path: args.path.as_deref().unwrap_or(""),
            environment: args.common.environment.as_deref(),
            overrides: &args.common.vars,
            secrets: &EnvThenKeychain,
            history: history.as_ref(),
            source: "cli",
            allow_host: None,
            network: Some(network.clone()),
            iterations: args.iterations,
            data,
            delay: std::time::Duration::from_millis(args.delay),
            only_tests,
            fail_fast: args.fail_fast,
            scripts: !args.no_scripts,
            cancel: None,
            detail: args.common.detail(),
        },
        &mut |step| {
            if json {
                return;
            }
            if several && printed_iteration != Some(step.iteration) {
                let gap = if printed_iteration.is_some() {
                    "\n"
                } else {
                    ""
                };
                printed_iteration = Some(step.iteration);
                println!("{gap}Iteration {}", step.iteration + 1);
            }
            output::outcome(&step.outcome, false);
        },
    )
    .await?;

    settings::keep_cookies(&network, Some(&workspace)).map_err(anyhow::Error::msg)?;
    if let Some(path) = &args.junit {
        let name = args.path.as_deref().unwrap_or(workspace.name());
        std::fs::write(path, runs::junit(&report, name))
            .with_context(|| format!("could not write {}", path.display()))?;
    }
    if json {
        // Passing requests are listed briefly; failures in full.
        let results: Vec<serde_json::Value> = report
            .steps
            .iter()
            .map(|s| {
                let o = &s.outcome;
                let mut value = if o.passed {
                    serde_json::json!({ "path": o.path, "status": o.status, "timeMs": o.time_ms, "passed": true })
                } else {
                    serde_json::to_value(o).unwrap_or_default()
                };
                if report.iterations > 1 {
                    value["iteration"] = (s.iteration + 1).into();
                }
                value
            })
            .collect();
        let mut value = serde_json::json!({ "summary": report.summary, "results": results });
        if let Some(stopped) = &report.stopped {
            value["stopped"] = stopped.clone().into();
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        if let Some(stopped) = &report.stopped {
            println!("\n{stopped}");
        }
        output::summary(&report.summary);
    }
    if report.steps.is_empty() && only_tests {
        eprintln!(
            "No saved requests with checks or test scripts. Add an expect: block or pm.test(), or save one with \
             `tuma send ... --save`."
        );
    }
    Ok(report.summary.failed == 0
        && report
            .stopped
            .as_deref()
            .is_none_or(|s| !s.contains("loop")))
}

async fn grpc_command(start: &Path, args: GrpcArgs) -> anyhow::Result<bool> {
    use tuma_core::grpc::{self, GrpcEvent};
    let workspace = find_workspace(start)?;
    let saved = workspace.as_ref().and_then(|ws| {
        let path = args.target.trim_matches('/');
        let path = if path.ends_with(".yaml") {
            path.to_string()
        } else {
            format!("{path}.yaml")
        };
        ws.read_request(&path).ok().map(|r| (path, r))
    });
    let (path, mut request) = match saved {
        Some((path, request)) => (Some(path), request),
        None => (
            None,
            RequestFile {
                url: args.target.clone(),
                ..RequestFile::default()
            },
        ),
    };
    if !request.is_grpc() {
        bail!(
            "\"{}\" is not a gRPC address: use grpc:// or grpcs://",
            request.url
        );
    }
    let mut settings = request.grpc.clone().unwrap_or_default();
    if let Some(method) = &args.method {
        settings.method = method.clone();
    }
    if !args.protos.is_empty() {
        settings.protos = args.protos.clone();
        settings.imports = args.imports.clone();
    }
    for h in &args.headers {
        let (name, value) = h
            .split_once(':')
            .with_context(|| format!("metadata \"{h}\" should be \"name: value\""))?;
        request.headers.push(KeyValue {
            name: name.trim().into(),
            value: value.trim().into(),
            enabled: true,
        });
    }
    if let Some(token) = &args.bearer {
        request.auth = Some(Auth::Bearer {
            token: token.clone(),
        });
    }
    if args.insecure {
        request.settings.verify_tls = false;
    }
    let network =
        settings::network_for(workspace.as_ref(), &EnvThenKeychain).map_err(anyhow::Error::msg)?;
    let job = Job {
        workspace: workspace.as_ref(),
        request: &request,
        path: path.as_deref(),
        context: None,
        environment: args.common.environment.as_deref(),
        overrides: &args.common.vars,
        extra_checks: None,
        secrets: &EnvThenKeychain,
        history: None,
        source: "cli",
        allow_host: None,
        scripting: Default::default(),
        network: Some(network.clone()),
        observer: None,
    };
    let resolved = workspace
        .as_ref()
        .map(|ws| {
            ws.prepare(
                &request,
                path.as_deref(),
                job.environment,
                job.overrides,
                job.secrets,
            )
        })
        .transpose()?
        .map(|p| p.request)
        .unwrap_or_else(|| {
            tuma_core::workspace::prepare_standalone(&request, job.overrides).request
        });
    let target = grpc::Target::from_request(&resolved, network.clone())
        .await
        .map_err(anyhow::Error::msg)?;
    // Relative .proto paths are from the workspace for a saved request, else from here.
    let root = match (&path, &workspace) {
        (Some(_), Some(ws)) => ws.root().to_path_buf(),
        _ => start.to_path_buf(),
    };
    let pool = grpc::describe(&target, &settings, Some(&root))
        .await
        .map_err(anyhow::Error::msg)?;
    if args.list || settings.method.is_empty() {
        let services = grpc::services(&pool);
        if args.common.json {
            println!("{}", serde_json::to_string_pretty(&services)?);
        } else {
            for s in &services {
                println!("{}", s.name);
                for m in &s.methods {
                    let kind = match (m.client_streaming, m.server_streaming) {
                        (false, false) => "",
                        (false, true) => "  (server streaming)",
                        (true, false) => "  (client streaming)",
                        (true, true) => "  (bidirectional)",
                    };
                    println!("  {}({}) returns {}{kind}", m.name, m.input, m.output);
                }
            }
            if settings.method.is_empty() && !args.list {
                eprintln!(
                    "\nName a method to call it, such as: tuma grpc {} <Service/Method> -d '{{...}}'",
                    args.target
                );
            }
        }
        return Ok(true);
    }
    let mut messages = Vec::new();
    for message in if args.messages.is_empty() {
        match &request.body {
            Body::Json { content } => vec![content.clone()],
            _ => request.messages.iter().map(|m| m.content.clone()).collect(),
        }
    } else {
        args.messages.clone()
    } {
        messages
            .push(runner::resolve_text(&job, &message).map_err(|f| anyhow::anyhow!(f.message))?);
    }
    let json = args.common.json;
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
    let sink: grpc::EventSink = std::sync::Arc::new(move |event: GrpcEvent| {
        if json {
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
        } else {
            match &event {
                GrpcEvent::Headers { .. } => {}
                GrpcEvent::Message {
                    incoming: true,
                    json,
                    ..
                } => println!("{json}"),
                GrpcEvent::Message { .. } => {}
                GrpcEvent::Done {
                    status,
                    message,
                    elapsed_ms,
                    ..
                } => eprintln!(
                    "{status}{} ({elapsed_ms} ms)",
                    if message.is_empty() {
                        String::new()
                    } else {
                        format!(": {message}")
                    }
                ),
            }
        }
        if let GrpcEvent::Done { code, .. } = event {
            let _ = done_tx.send(code == 0);
        }
    });
    let mut call = grpc::start(&target, &pool, &settings.method, sink)
        .await
        .map_err(anyhow::Error::msg)?;
    if messages.is_empty() {
        messages.push("{}".into());
    }
    for message in &messages {
        call.send(message).map_err(anyhow::Error::msg)?;
    }
    call.end();
    let ok = done_rx.recv().await.unwrap_or(false);
    settings::keep_cookies(&network, workspace.as_ref()).map_err(anyhow::Error::msg)?;
    Ok(ok)
}

async fn websocket(start: &Path, args: WsArgs) -> anyhow::Result<bool> {
    use tuma_core::http::websocket::{Outgoing, WsEvent};
    let workspace = find_workspace(start)?;
    // A saved request, or a URL.
    let saved = workspace.as_ref().and_then(|ws| {
        let path = args.target.trim_matches('/');
        let path = if path.ends_with(".yaml") {
            path.to_string()
        } else {
            format!("{path}.yaml")
        };
        ws.read_request(&path).ok().map(|r| (path, r))
    });
    let (path, mut request) = match saved {
        Some((path, request)) => (Some(path), request),
        None => (
            None,
            RequestFile {
                url: args.target.clone(),
                ..RequestFile::default()
            },
        ),
    };
    if !request.is_websocket() {
        bail!(
            "\"{}\" is not a WebSocket: use a ws:// or wss:// URL",
            request.url
        );
    }
    for h in &args.headers {
        let (name, value) = h
            .split_once(':')
            .with_context(|| format!("header \"{h}\" should be \"Name: value\""))?;
        request.headers.push(KeyValue {
            name: name.trim().into(),
            value: value.trim().into(),
            enabled: true,
        });
    }
    if let Some(token) = &args.bearer {
        request.auth = Some(Auth::Bearer {
            token: token.clone(),
        });
    }
    let mut messages = Vec::new();
    for name in &args.saved {
        let m = request
            .messages
            .iter()
            .find(|m| &m.name == name)
            .with_context(|| format!("no saved message \"{name}\""))?;
        messages.push(m.content.clone());
    }
    messages.extend(args.messages.iter().cloned());

    let network =
        settings::network_for(workspace.as_ref(), &EnvThenKeychain).map_err(anyhow::Error::msg)?;
    let job = || Job {
        workspace: workspace.as_ref(),
        request: &request,
        path: path.as_deref(),
        context: args.r#in.as_deref(),
        environment: args.common.environment.as_deref(),
        overrides: &args.common.vars,
        extra_checks: None,
        secrets: &EnvThenKeychain,
        history: None,
        source: "cli",
        allow_host: None,
        scripting: Default::default(),
        network: Some(network.clone()),
        observer: None,
    };
    let json = args.common.json;
    let (closed_tx, mut closed_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
    let sink: tuma_core::http::websocket::EventSink = std::sync::Arc::new(move |event: WsEvent| {
        if json {
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
        } else {
            match &event {
                WsEvent::Open { url, status, .. } => eprintln!("Connected to {url} ({status})"),
                WsEvent::Message {
                    incoming,
                    text,
                    binary,
                    size,
                    ..
                } => {
                    let arrow = if *incoming { "<" } else { ">" };
                    match (text, binary) {
                        (Some(text), _) => println!("{arrow} {text}"),
                        (_, Some(_)) => println!("{arrow} ({size} bytes of binary data)"),
                        _ => {}
                    }
                }
                WsEvent::Closed {
                    code,
                    reason,
                    by_server,
                } => eprintln!(
                    "Closed{} {}{}",
                    if *by_server { " by the server" } else { "" },
                    code.map(|c| c.to_string()).unwrap_or_default(),
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!(": {reason}")
                    }
                ),
                WsEvent::Error { message } => eprintln!("Error: {message}"),
            }
        }
        if let WsEvent::Closed { by_server, .. } = event {
            let _ = closed_tx.send(by_server);
        }
    });
    let socket = runner::open_websocket(job(), sink)
        .await
        .map_err(|f| anyhow::anyhow!(f.message))?;
    for message in &messages {
        let text = runner::resolve_text(&job(), message).map_err(|f| anyhow::anyhow!(f.message))?;
        socket
            .send(Outgoing::Text(text))
            .map_err(anyhow::Error::msg)?;
    }
    let waited = if args.wait > 0.0 {
        tokio::time::timeout(
            std::time::Duration::from_secs_f64(args.wait),
            closed_rx.recv(),
        )
        .await
        .ok()
    } else {
        Some(closed_rx.recv().await)
    };
    if waited.is_none() {
        socket.close();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), closed_rx.recv()).await;
    }
    settings::keep_cookies(&network, workspace.as_ref()).map_err(anyhow::Error::msg)?;
    Ok(true)
}

async fn auth(start: &Path, command: AuthCommand) -> anyhow::Result<bool> {
    use tuma_core::oauth2;
    let Some(workspace) = find_workspace(start)? else {
        bail!("no workspace here or above {}", start.display());
    };
    match command {
        AuthCommand::Login { path, environment } => {
            let path = path.trim_matches('/');
            // A request is read as is; a folder or collection is logged in for as if a request were in it.
            let (request, location) = match workspace
                .read_request(path)
                .or_else(|_| workspace.read_request(&format!("{path}.yaml")))
            {
                Ok(request) => (
                    request,
                    if path.ends_with(".yaml") {
                        path.to_string()
                    } else {
                        format!("{path}.yaml")
                    },
                ),
                Err(_) => (RequestFile::default(), format!("{path}/request.yaml")),
            };
            let Some(settings) = oauth2::effective_settings(
                &workspace,
                &request,
                Some(&location),
                environment.as_deref(),
                &EnvThenKeychain,
            )
            .map_err(anyhow::Error::msg)?
            else {
                bail!("\"{path}\" doesn't use OAuth 2.0 auth (its own or inherited)");
            };
            let network = settings::network_for(Some(&workspace), &EnvThenKeychain)
                .map_err(anyhow::Error::msg)?;
            let open = |url: &str| {
                println!("Log in in your browser. If it didn't open, go to:\n  {url}\n");
                let _ = oauth2::open_in_browser(url);
            };
            let token = oauth2::authorize(
                &settings,
                Some(workspace.id()),
                &EnvThenKeychain,
                Some(&network),
                &open,
            )
            .await
            .map_err(anyhow::Error::msg)?;
            let expiry = match token.expires_at {
                Some(at) => format!(
                    ", valid for {} minutes",
                    at.saturating_sub(token.obtained_at) / 60
                ),
                None => String::new(),
            };
            println!(
                "Got a token{expiry}{}. Requests using these settings now send it.",
                if token.refresh_token.is_some() {
                    "; it will be refreshed when it expires"
                } else {
                    ""
                }
            );
            Ok(true)
        }
        AuthCommand::Tokens => {
            let tokens = oauth2::list(workspace.id());
            if tokens.is_empty() {
                println!("No stored tokens. Get one with `tuma auth login <request or folder>`.");
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for t in tokens {
                let state = match t.expires_at {
                    Some(at) if at <= now => "expired".to_string(),
                    Some(at) => format!("{} min left", (at - now) / 60),
                    None => "no expiry".to_string(),
                };
                println!(
                    "{}  {} at {}  ({state}{})",
                    t.key,
                    t.client_id,
                    t.token_url,
                    if t.refreshable { ", refreshable" } else { "" }
                );
            }
            Ok(true)
        }
        AuthCommand::Logout { key } => {
            let keys: Vec<String> = match key {
                Some(key) => vec![key],
                None => oauth2::list(workspace.id())
                    .into_iter()
                    .map(|t| t.key)
                    .collect(),
            };
            for key in &keys {
                oauth2::forget(workspace.id(), key, &EnvThenKeychain)
                    .map_err(anyhow::Error::msg)?;
            }
            println!(
                "Forgot {} token{}",
                keys.len(),
                if keys.len() == 1 { "" } else { "s" }
            );
            Ok(true)
        }
    }
}

async fn openapi(start: &Path, command: OpenapiCommand) -> anyhow::Result<bool> {
    use tuma_core::openapi::{self, Spec};

    let import = |workspace: &Workspace, spec: &Spec, source: &str| -> anyhow::Result<()> {
        let (path, warnings) = openapi::import(workspace, spec, Some(source))?;
        let requests = runner::requests_under(workspace, &path)?.len();
        println!("Imported \"{path}\": {requests} requests from {source}");
        for warning in warnings.0 {
            println!("  ! {warning}");
        }
        Ok(())
    };

    match command {
        OpenapiCommand::Import { source } => {
            let workspace = runner::open_or_create(start, None)?;
            let source = openapi::source_for(&source, start, workspace.root());
            let text = openapi::load(&source, workspace.root())
                .await
                .map_err(anyhow::Error::msg)?;
            let spec = Spec::parse(&text).map_err(anyhow::Error::msg)?;
            import(&workspace, &spec, &source)?;
            Ok(true)
        }
        OpenapiCommand::Discover {
            server,
            import: do_import,
        } => {
            let project = runner::find_workspace(start)
                .and_then(|w| {
                    if w.ends_with(runner::HIDDEN_WORKSPACE) {
                        w.parent().map(Path::to_path_buf)
                    } else {
                        Some(w)
                    }
                })
                .unwrap_or_else(|| start.to_path_buf());
            let mut found: Vec<(String, Spec)> = Vec::new();
            for file in openapi::discover_files(&project) {
                if let Ok(spec) = std::fs::read_to_string(&file)
                    .map_err(|e| e.to_string())
                    .and_then(|t| Spec::parse(&t))
                {
                    found.push((file.to_string_lossy().into_owned(), spec));
                }
            }
            if let Some(server) = &server {
                if let Some((url, spec)) = openapi::discover_on_server(server).await {
                    found.push((url, spec));
                } else {
                    println!("No spec at the usual paths on {server}.");
                }
            }
            if found.is_empty() {
                println!(
                    "No OpenAPI spec found. Try --server http://localhost:<port> with your API running."
                );
                return Ok(false);
            }
            for (source, spec) in &found {
                println!("{}  ({source})", spec.title());
            }
            if do_import {
                let workspace = runner::open_or_create(start, None)?;
                let (source, spec) = &found[0];
                let source = openapi::source_for(source, start, workspace.root());
                import(&workspace, spec, &source)?;
            }
            Ok(true)
        }
        OpenapiCommand::Sync { collection } => {
            let Some(workspace) = find_workspace(start)? else {
                bail!("no workspace here or above {}", start.display());
            };
            let collections: Vec<String> = match collection {
                Some(c) => vec![c],
                None => workspace.tree()?.into_iter().map(|n| n.path).collect(),
            };
            let mut synced = 0;
            for collection in collections {
                let Some(source) = workspace.read_folder(&collection)?.openapi else {
                    continue;
                };
                let text = openapi::load(&source, workspace.root())
                    .await
                    .map_err(anyhow::Error::msg)?;
                let spec = Spec::parse(&text).map_err(anyhow::Error::msg)?;
                let report = openapi::sync(&workspace, &collection, &spec)?;
                synced += 1;
                println!(
                    "{collection}: {} added, {} updated, {} no longer in the spec",
                    report.added.len(),
                    report.updated.len(),
                    report.removed.len()
                );
                for path in &report.added {
                    println!("  + {path}");
                }
                for path in &report.updated {
                    println!("  ~ {path}");
                }
                for path in &report.removed {
                    println!("  - {path} (kept; delete it if the endpoint is gone)");
                }
            }
            if synced == 0 {
                bail!(
                    "no collection here was imported from a spec; use `tuma openapi import` first"
                );
            }
            Ok(true)
        }
    }
}
