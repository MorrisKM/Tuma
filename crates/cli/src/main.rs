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
    /// Create a workspace: .tuma/ at the root of the current project.
    Init {
        /// The workspace's name. Defaults to the project folder's name.
        #[arg(long)]
        name: Option<String>,
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
