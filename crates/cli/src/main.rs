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
use tuma_core::runner::{self, Detail, Job, Outcome, Summary};
use tuma_core::secrets::EnvThenKeychain;
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
    /// Check every file in the workspace: YAML syntax, unknown keys (typos), and checks that can't work.
    Check {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Import an OpenAPI or Swagger spec as a collection, find one, or sync a collection with its spec.
    #[command(subcommand)]
    Openapi(OpenapiCommand),
    /// Create a workspace: .tuma/ at the root of the current project.
    Init {
        /// The workspace's name. Defaults to the project folder's name.
        #[arg(long)]
        name: Option<String>,
    },
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
            match imported {
                tuma_core::import::Imported::Collection(path) => {
                    println!("Imported collection \"{path}\"")
                }
                tuma_core::import::Imported::Environment(name) => {
                    println!("Imported environment \"{name}\"")
                }
                tuma_core::import::Imported::Globals => println!("Imported global variables"),
            }
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
        },
        &detail,
    )
    .await;

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
    let history = History::default_location();
    let detail = args.common.detail();
    let mut outcomes: Vec<Outcome> = Vec::new();
    for path in runner::requests_under(&workspace, args.path.as_deref().unwrap_or(""))? {
        let request = workspace.read_request(&path)?;
        if only_tests && request.expect.is_empty() {
            continue;
        }
        let outcome = runner::send(
            Job {
                workspace: Some(&workspace),
                request: &request,
                path: Some(&path),
                context: None,
                environment: args.common.environment.as_deref(),
                overrides: &args.common.vars,
                extra_checks: None,
                secrets: &EnvThenKeychain,
                history: history.as_ref(),
                source: "cli",
                allow_host: None,
            },
            &detail,
        )
        .await;
        if !args.common.json {
            output::outcome(&outcome, false);
        }
        let stop = args.fail_fast && !outcome.passed;
        outcomes.push(outcome);
        if stop {
            break;
        }
    }

    let summary = Summary::of(&outcomes);
    if args.common.json {
        // Passing requests are listed briefly; failures in full.
        let results: Vec<serde_json::Value> = outcomes
            .iter()
            .map(|o| {
                if o.passed {
                    serde_json::json!({ "path": o.path, "status": o.status, "timeMs": o.time_ms, "passed": true })
                } else {
                    serde_json::to_value(o).unwrap_or_default()
                }
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "summary": summary, "results": results })
            )?
        );
    } else {
        output::summary(&summary);
    }
    if outcomes.is_empty() && only_tests {
        eprintln!(
            "No saved requests with checks. Add an expect: block, or save one with `tuma send ... --save`."
        );
    }
    Ok(summary.failed == 0)
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
