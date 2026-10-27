//! Scripts that are not part of a request, from the Scripts page and `tuma script run`: JavaScript run in the script
//! sandbox (`tuma.*` on top of `pm.*`), or commands run on this computer, such as `python flow.py --count 5`.
//!
//! They live in the workspace's `scripts` folder, so they are shared with it in git: `<name>.js` is a JavaScript
//! script's source and `<name>.yaml` its settings (description and inputs), optional; a command has only
//! `<name>.yaml`, with `command:` set.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::http;
use crate::script::{self, LogLine, SavedFile};
use crate::secrets::SecretStore;
use crate::vars::Scopes;
use crate::workspace::files::{KeyValue, RequestFile};
use crate::workspace::{Workspace, file_name};

/// The scripts folder, in the workspace folder.
pub const DIR: &str = "scripts";

/// `scripts/<name>.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptFile {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Values asked for before each run: `tuma.inputs` in JavaScript, arguments and `TUMA_INPUT_<NAME>` variables
    /// for a command.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ScriptInput>,
    /// For a command: what to run, as on a command line, such as `env/bin/python flow.py --verbose`. `{{variables}}`
    /// from the environment and inputs are filled in. Empty for a JavaScript script.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// The folder a command runs in: absolute, from `~`, or relative to the workspace folder (for a `.tuma` workspace,
    /// the project it is in). Empty: that folder.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// Environment variables for a command, with `{{variables}}`, such as `API_TOKEN: {{token}}`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<KeyValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptInput {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "is_text")]
    pub kind: InputKind,
    /// Shown instead of the name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The value it starts with; `true` or `false` for a checkbox.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub default: String,
    /// The values to choose from, for `type: choice`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// For a command: how the value is passed, such as `--count` (giving `--count 5`) or `--count=` (giving
    /// `--count=5`). A checkbox passes the flag alone when ticked. Empty values are left out.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub arg: String,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    #[default]
    Text,
    Number,
    Boolean,
    Choice,
}

fn is_text(kind: &InputKind) -> bool {
    *kind == InputKind::Text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Javascript,
    Command,
}

/// A script in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub name: String,
    pub kind: Kind,
    pub description: String,
}

/// A script as the editor works on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub name: String,
    pub kind: Kind,
    pub settings: ScriptFile,
    /// A JavaScript script's source; empty for a command.
    #[serde(default)]
    pub source: String,
}

pub type Result<T> = std::result::Result<T, String>;

fn dir(ws: &Workspace) -> PathBuf {
    ws.root().join(DIR)
}

/// A script's name as given, refusing anything that is not a plain name in the scripts folder.
fn checked(name: &str) -> Result<&str> {
    let valid = !name.is_empty() && name == file_name(name) && !name.starts_with('.');
    if valid {
        Ok(name)
    } else {
        Err(format!("\"{name}\" is not a valid script name"))
    }
}

fn paths(ws: &Workspace, name: &str) -> Result<(PathBuf, PathBuf)> {
    let name = checked(name)?;
    let dir = dir(ws);
    Ok((
        dir.join(format!("{name}.yaml")),
        dir.join(format!("{name}.js")),
    ))
}

fn read_settings(path: &Path) -> Result<ScriptFile> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(ScriptFile::default()),
        Ok(text) => serde_saphyr::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ScriptFile::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn kind_of(settings: &ScriptFile, js: &Path) -> Kind {
    if settings.command.trim().is_empty() || js.is_file() {
        Kind::Javascript
    } else {
        Kind::Command
    }
}

/// The workspace's scripts, by name.
pub fn list(ws: &Workspace) -> Result<Vec<Summary>> {
    let Ok(entries) = fs::read_dir(dir(ws)) else {
        return Ok(vec![]);
    };
    let mut names = BTreeSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let script = path.extension().is_some_and(|e| e == "js" || e == "yaml");
        if path.is_file() && script && checked(&stem).is_ok() {
            names.insert(stem);
        }
    }
    let mut out: Vec<Summary> = names
        .into_iter()
        .map(|name| {
            let (yaml, js) = paths(ws, &name)?;
            // A broken settings file still lists the script, so it can be opened and fixed.
            let settings = read_settings(&yaml).unwrap_or_default();
            Ok(Summary {
                kind: kind_of(&settings, &js),
                description: settings.description,
                name,
            })
        })
        .collect::<Result<_>>()?;
    out.sort_by_key(|s| s.name.to_lowercase());
    Ok(out)
}

pub fn read(ws: &Workspace, name: &str) -> Result<Script> {
    let (yaml, js) = paths(ws, name)?;
    if !yaml.is_file() && !js.is_file() {
        return Err(format!("there is no script called \"{name}\""));
    }
    let settings = read_settings(&yaml)?;
    let kind = kind_of(&settings, &js);
    let source = match kind {
        Kind::Javascript => fs::read_to_string(&js).unwrap_or_default(),
        Kind::Command => String::new(),
    };
    Ok(Script {
        name: name.to_string(),
        kind,
        settings,
        source,
    })
}

/// Saves a script. A JavaScript script without settings has no `.yaml` file.
pub fn write(ws: &Workspace, script: &Script) -> Result<()> {
    let (yaml, js) = paths(ws, &script.name)?;
    let dir = dir(ws);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut settings = script.settings.clone();
    if script.kind == Kind::Javascript {
        settings.command.clear();
        settings.cwd.clear();
        settings.env.clear();
        fs::write(&js, &script.source).map_err(|e| format!("{}: {e}", js.display()))?;
    } else if settings.command.trim().is_empty() {
        return Err("A command script needs a command to run".into());
    }
    if settings == ScriptFile::default() {
        if yaml.exists() {
            fs::remove_file(&yaml).map_err(|e| format!("{}: {e}", yaml.display()))?;
        }
        return Ok(());
    }
    let text = serde_saphyr::to_string(&settings).map_err(|e| e.to_string())?;
    fs::write(&yaml, text).map_err(|e| format!("{}: {e}", yaml.display()))
}

const JS_TEMPLATE: &str = r#"// Runs on its own, not as part of a request. Inputs are in tuma.inputs, environment variables in
// pm.environment, and tuma.get / tuma.post / tuma.request send requests. tuma.saveFile offers a file to save.
const baseUrl = pm.environment.get("baseUrl") || "https://httpbin.org";

const response = await tuma.get(`${baseUrl}/get`, { query: { name: tuma.inputs.name } });
console.log(response.code, response.json().args);

return { status: response.code };
"#;

/// Creates a script with a starting point to edit, and returns its name, which is made unique.
pub fn create(ws: &Workspace, name: &str, kind: Kind) -> Result<String> {
    let base = file_name(name);
    let mut name = base.clone();
    let mut n = 2;
    while list(ws)?.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
        name = format!("{base} {n}");
        n += 1;
    }
    let script = match kind {
        Kind::Javascript => Script {
            name: name.clone(),
            kind,
            settings: ScriptFile {
                inputs: vec![ScriptInput {
                    name: "name".into(),
                    default: "Tuma".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            source: JS_TEMPLATE.into(),
        },
        Kind::Command => Script {
            name: name.clone(),
            kind,
            settings: ScriptFile {
                command: if cfg!(windows) {
                    "cmd /c echo Hello from {{name}}".into()
                } else {
                    "echo Hello from {{name}}".into()
                },
                inputs: vec![ScriptInput {
                    name: "name".into(),
                    default: "Tuma".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            source: String::new(),
        },
    };
    write(ws, &script)?;
    Ok(name)
}

pub fn rename(ws: &Workspace, name: &str, new_name: &str) -> Result<String> {
    let new_name = file_name(new_name);
    if new_name == name {
        return Ok(new_name);
    }
    let (yaml, js) = paths(ws, name)?;
    let (new_yaml, new_js) = paths(ws, &new_name)?;
    // Compared without case: on Windows and macOS, renaming to another case is the same file.
    let taken = list(ws)?
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(&new_name) && !s.name.eq_ignore_ascii_case(name));
    if taken {
        return Err(format!("\"{new_name}\" already exists"));
    }
    for (from, to) in [(yaml, new_yaml), (js, new_js)] {
        if from.exists() {
            fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(new_name)
}

pub fn delete(ws: &Workspace, name: &str) -> Result<()> {
    let (yaml, js) = paths(ws, name)?;
    for path in [yaml, js] {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

// Running

/// What a run reports while it goes.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum Event {
    /// The command line being run, with `{{variables}}` as written so secrets are not shown.
    Command(String),
    /// A line a command wrote to its output.
    Stdout(String),
    /// A line a command wrote to its error output.
    Stderr(String),
    /// A JavaScript script's `console.log` (`info`, `warn`, `error`, `debug`).
    Console(LogLine),
}

/// How a run ended.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub ok: bool,
    /// A command's exit code; None when it was stopped or could not start, and for JavaScript.
    pub exit_code: Option<i32>,
    /// Why it failed: the script's error, or why the command could not run.
    pub error: Option<String>,
    pub stopped: bool,
    pub duration_ms: u64,
    /// Files a JavaScript script offered with `tuma.saveFile`.
    pub files: Vec<SavedFile>,
    /// What a JavaScript script returned.
    pub result: Option<Value>,
}

pub struct RunOptions<'a> {
    pub workspace: &'a Workspace,
    pub name: &'a str,
    pub environment: Option<&'a str>,
    /// Input values by name; inputs left out get their defaults.
    pub inputs: &'a BTreeMap<String, String>,
    /// For a command: more arguments, as on a command line.
    pub extra_args: &'a str,
    pub secrets: &'a dyn SecretStore,
    pub network: Option<Arc<http::Network>>,
    /// Set to stop the run.
    pub stop: Arc<AtomicBool>,
}

/// Runs a saved script. Events go to `on_event` as they happen, from any thread.
pub async fn run(
    options: RunOptions<'_>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) -> Result<Outcome> {
    let script = read(options.workspace, options.name)?;
    let inputs = input_values(&script.settings.inputs, options.inputs);
    let started = Instant::now();
    let mut outcome = match script.kind {
        Kind::Javascript => run_js(&script, &inputs, &options, on_event).await?,
        Kind::Command => run_command(&script, &inputs, &options, on_event).await?,
    };
    outcome.duration_ms = started.elapsed().as_millis() as u64;
    Ok(outcome)
}

/// The declared inputs with their defaults, then any others that were passed.
fn input_values(
    declared: &[ScriptInput],
    given: &BTreeMap<String, String>,
) -> Vec<(ScriptInput, String)> {
    let mut out: Vec<(ScriptInput, String)> = declared
        .iter()
        .filter(|i| !i.name.is_empty())
        .map(|i| {
            let value = given.get(&i.name).unwrap_or(&i.default).clone();
            (i.clone(), value)
        })
        .collect();
    for (name, value) in given {
        if !declared.iter().any(|i| i.name == *name) {
            let input = ScriptInput {
                name: name.clone(),
                ..Default::default()
            };
            out.push((input, value.clone()));
        }
    }
    out
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on"
    )
}

/// Inputs as JavaScript sees them: numbers and booleans as such.
fn typed_inputs(inputs: &[(ScriptInput, String)]) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for (input, value) in inputs {
        let typed = match input.kind {
            InputKind::Number if value.trim().is_empty() => Value::Null,
            InputKind::Number => {
                let n: f64 = value.trim().parse().map_err(|_| {
                    format!("The input {} needs a number, not \"{value}\"", input.name)
                })?;
                json!(n)
            }
            InputKind::Boolean => json!(truthy(value)),
            InputKind::Text | InputKind::Choice => json!(value),
        };
        out.insert(input.name.clone(), typed);
    }
    Ok(Value::Object(out))
}

async fn run_js(
    script: &Script,
    inputs: &[(ScriptInput, String)],
    options: &RunOptions<'_>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) -> Result<Outcome> {
    let ws = options.workspace;
    let (vars, vault) = ws
        .script_vars(None, options.environment, options.secrets)
        .map_err(|e| e.to_string())?;
    let input = script::Input {
        info: script::Info {
            request_name: script.name.clone(),
            ..Default::default()
        },
        request: script::request_value(&RequestFile::default(), &script.name, ""),
        response: None,
        vars: vars.clone(),
        vault,
        inputs: Some(typed_inputs(inputs)?),
    };
    let source = script::Script {
        label: format!("script {}", script.name),
        source: script.source.clone(),
    };
    let live = script::Live {
        stop: options.stop.clone(),
        on_log: Arc::new(move |line: &LogLine| on_event(Event::Console(line.clone()))),
    };
    let network = options.network.clone();
    let output =
        tokio::task::spawn_blocking(move || script::run_standalone(&source, &input, network, live))
            .await
            .map_err(|e| format!("the script engine crashed: {e}"))?;
    let stopped = options.stop.load(Ordering::Relaxed);
    let mut errors: Vec<String> = output.errors.iter().map(|e| e.message.clone()).collect();
    // What it set in the environment and globals is kept, as for a request's scripts.
    if let Err(e) = crate::runner::store_changes(
        ws,
        &vars,
        &output.vars,
        options.environment,
        None,
        options.secrets,
    ) {
        errors.push(format!("could not store variables: {e}"));
    }
    let failed_tests: Vec<&str> = output
        .tests
        .iter()
        .filter(|t| !t.passed && !t.skipped)
        .map(|t| t.name.as_str())
        .collect();
    if !failed_tests.is_empty() {
        errors.push(format!("tests failed: {}", failed_tests.join(", ")));
    }
    Ok(Outcome {
        ok: errors.is_empty() && !stopped,
        exit_code: None,
        error: (!errors.is_empty() && !stopped).then(|| errors.join("\n")),
        stopped,
        duration_ms: 0,
        files: output.files,
        result: output.result,
    })
}

/// `~/x` as a path in the home folder.
fn expand_home(text: &str) -> PathBuf {
    match text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| text.into()),
        None if text == "~" => dirs::home_dir().unwrap_or_else(|| text.into()),
        None => text.into(),
    }
}

/// Where commands run and relative paths start: the workspace folder, or for a project's `.tuma` workspace, the
/// project.
fn base_dir(ws: &Workspace) -> &Path {
    let root = ws.root();
    match root.parent() {
        Some(project) if root.file_name().is_some_and(|n| n == ".tuma") => project,
        _ => root,
    }
}

/// A command ready to start, and how to show it.
#[derive(Debug, PartialEq)]
struct Prepared {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    shown: String,
}

fn prepare_command(
    ws: &Workspace,
    settings: &ScriptFile,
    inputs: &[(ScriptInput, String)],
    environment: Option<&str>,
    extra_args: &str,
    secrets: &dyn SecretStore,
) -> Result<Prepared> {
    let mut scopes = Scopes::default();
    scopes.push(inputs.iter().map(|(i, v)| (i.name.clone(), v.clone())));
    let enabled = |vars: Vec<crate::workspace::VariableValue>| {
        vars.into_iter()
            .filter(|v| v.enabled)
            .map(|v| (v.name, v.value))
            .collect::<Vec<_>>()
    };
    if let Some(environment) = environment {
        scopes.push(enabled(
            ws.effective_environment(environment, secrets)
                .map_err(|e| e.to_string())?,
        ));
    }
    scopes.push(enabled(
        ws.effective_globals(secrets).map_err(|e| e.to_string())?,
    ));
    let mut unresolved = BTreeSet::new();
    let split = |text: &str, what: &str| -> Result<Vec<String>> {
        shlex::split(text).ok_or_else(|| format!("The {what} has an unclosed quote"))
    };

    let mut written = split(&settings.command, "command")?;
    if written.is_empty() {
        return Err("The script has no command to run".into());
    }
    for (input, value) in inputs {
        if input.arg.is_empty() {
            continue;
        }
        match input.kind {
            InputKind::Boolean if truthy(value) => written.push(input.arg.clone()),
            InputKind::Boolean => {}
            _ if value.is_empty() => {}
            _ if input.arg.ends_with('=') => written.push(format!("{}{value}", input.arg)),
            _ => written.extend([input.arg.clone(), value.clone()]),
        }
    }
    written.extend(split(extra_args, "arguments")?);
    let shown =
        shlex::try_join(written.iter().map(String::as_str)).unwrap_or_else(|_| written.join(" "));

    let mut words: Vec<String> = written
        .iter()
        .map(|w| scopes.resolve(w, &mut unresolved))
        .collect();
    let cwd_text = scopes.resolve(&settings.cwd, &mut unresolved);
    let mut env: Vec<(String, String)> = settings
        .env
        .iter()
        .filter(|kv| kv.enabled && !kv.name.trim().is_empty())
        .map(|kv| {
            (
                kv.name.trim().to_string(),
                scopes.resolve(&kv.value, &mut unresolved),
            )
        })
        .collect();
    if !unresolved.is_empty() {
        let names: Vec<String> = unresolved.iter().map(|n| format!("{{{{{n}}}}}")).collect();
        let hint = if environment.is_some() {
            "Add them to the environment or the inputs"
        } else {
            "Choose an environment that has them, or add them to the inputs"
        };
        return Err(format!("No value for {}. {hint}.", names.join(", ")));
    }

    let base = base_dir(ws);
    let cwd = match cwd_text.trim() {
        "" => base.to_path_buf(),
        text => {
            let path = expand_home(text);
            if path.is_absolute() {
                path
            } else {
                base.join(path)
            }
        }
    };
    if !cwd.is_dir() {
        return Err(format!("The folder {} does not exist", cwd.display()));
    }
    let program_text = words.remove(0);
    let mut program = expand_home(&program_text);
    // A program given by a path, such as env/bin/python, is found from the folder it runs in.
    if program.is_relative() && program.components().count() > 1 {
        program = cwd.join(program);
    }
    for (input, value) in inputs {
        let name: String = input
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        env.push((format!("TUMA_INPUT_{name}"), value.clone()));
    }
    if let Some(environment) = environment {
        env.push(("TUMA_ENVIRONMENT".into(), environment.to_string()));
    }
    // So Python prints as it goes rather than when it ends, unless the script says otherwise.
    if !env.iter().any(|(k, _)| k == "PYTHONUNBUFFERED") {
        env.push(("PYTHONUNBUFFERED".into(), "1".into()));
    }
    Ok(Prepared {
        program,
        args: words,
        cwd,
        env,
        shown,
    })
}

async fn run_command(
    script: &Script,
    inputs: &[(ScriptInput, String)],
    options: &RunOptions<'_>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) -> Result<Outcome> {
    let prepared = match prepare_command(
        options.workspace,
        &script.settings,
        inputs,
        options.environment,
        options.extra_args,
        options.secrets,
    ) {
        Ok(p) => p,
        Err(error) => {
            return Ok(Outcome {
                error: Some(error),
                ..Default::default()
            });
        }
    };
    on_event(Event::Command(prepared.shown.clone()));
    let mut command = tokio::process::Command::new(&prepared.program);
    command
        .args(&prepared.args)
        .current_dir(&prepared.cwd)
        .envs(prepared.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // No console window flashing up for each run.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let error = if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "{} was not found: check the command, or give the program's full path",
                    prepared.program.display()
                )
            } else {
                format!("could not start {}: {e}", prepared.program.display())
            };
            return Ok(Outcome {
                error: Some(error),
                ..Default::default()
            });
        }
    };
    let out = child.stdout.take().map(|o| {
        let on_event = on_event.clone();
        tokio::spawn(lines(o, move |l| on_event(Event::Stdout(l))))
    });
    let err = child.stderr.take().map(|e| {
        let on_event = on_event.clone();
        tokio::spawn(lines(e, move |l| on_event(Event::Stderr(l))))
    });

    let mut stopped = false;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if options.stop.load(Ordering::Relaxed) {
                    stopped = true;
                    let _ = child.kill().await;
                    break child.wait().await;
                }
            }
        }
    };
    for task in [out, err].into_iter().flatten() {
        let _ = task.await;
    }
    let status = status.map_err(|e| e.to_string())?;
    let exit_code = if stopped { None } else { status.code() };
    Ok(Outcome {
        ok: !stopped && status.success(),
        exit_code,
        error: (!stopped && !status.success()).then(|| match status.code() {
            Some(code) => format!("exited with code {code}"),
            None => "was ended by a signal".to_string(),
        }),
        stopped,
        ..Default::default()
    })
}

/// Passes on each line of `reader`, without its line ending; text that is not UTF-8 is shown as best it can be.
async fn lines(reader: impl AsyncRead + Unpin, mut each: impl FnMut(String)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                    buf.pop();
                }
                each(String::from_utf8_lossy(&buf).into_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    fn workspace() -> (tempdir::Dir, Workspace) {
        let dir = tempdir::Dir::new();
        let ws = Workspace::create(dir.path(), "Test").unwrap();
        (dir, ws)
    }

    mod tempdir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let path =
                    std::env::temp_dir().join(format!("tuma-scripts-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn scripts_are_created_listed_renamed_and_deleted() {
        let (_dir, ws) = workspace();
        assert_eq!(
            create(&ws, "Seed data", Kind::Javascript).unwrap(),
            "Seed data"
        );
        assert_eq!(
            create(&ws, "Seed data", Kind::Javascript).unwrap(),
            "Seed data 2"
        );
        assert_eq!(create(&ws, "Flow", Kind::Command).unwrap(), "Flow");
        let names: Vec<(String, Kind)> = list(&ws)
            .unwrap()
            .into_iter()
            .map(|s| (s.name, s.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("Flow".to_string(), Kind::Command),
                ("Seed data".to_string(), Kind::Javascript),
                ("Seed data 2".to_string(), Kind::Javascript),
            ]
        );
        assert!(ws.root().join("scripts/Seed data.js").is_file());
        assert!(!ws.root().join("scripts/Flow.js").exists());
        // The scripts folder is not a collection.
        assert!(ws.tree().unwrap().is_empty());

        assert_eq!(rename(&ws, "Seed data 2", "Other").unwrap(), "Other");
        assert!(rename(&ws, "Other", "Flow").is_err());
        assert!(read(&ws, "Other").unwrap().source.contains("tuma.get"));
        delete(&ws, "Other").unwrap();
        assert!(read(&ws, "Other").is_err());
        assert!(read(&ws, "../workspace").is_err());
    }

    #[test]
    fn a_javascript_script_without_settings_has_no_yaml_file() {
        let (_dir, ws) = workspace();
        let script = Script {
            name: "Plain".into(),
            kind: Kind::Javascript,
            settings: ScriptFile::default(),
            source: "console.log(1)".into(),
        };
        write(&ws, &script).unwrap();
        assert!(!ws.root().join("scripts/Plain.yaml").exists());
        assert_eq!(read(&ws, "Plain").unwrap(), script);
    }

    fn input(name: &str, kind: InputKind, arg: &str, default: &str) -> ScriptInput {
        ScriptInput {
            name: name.into(),
            kind,
            arg: arg.into(),
            default: default.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_command_gets_its_inputs_as_arguments_and_variables() {
        let (_dir, ws) = workspace();
        let secrets = MemoryStore::default();
        ws.create_environment("UAT").unwrap();
        ws.write_environment(
            "UAT",
            &[crate::workspace::VariableValue {
                name: "token".into(),
                value: "key:secret".into(),
                enabled: true,
                secret: false,
            }],
            &secrets,
        )
        .unwrap();
        let settings = ScriptFile {
            command: "env/bin/python 'flow.py' --who {{who}}".into(),
            env: vec![KeyValue {
                name: "CR_TOKEN".into(),
                value: "{{token}}".into(),
                enabled: true,
            }],
            ..Default::default()
        };
        let declared = [
            input("count", InputKind::Number, "--count", "1"),
            input("no_facility", InputKind::Boolean, "--no-facility", "false"),
            input("csv", InputKind::Text, "--csv=", ""),
            input("who", InputKind::Text, "", "me"),
        ];
        let given = BTreeMap::from([
            ("count".to_string(), "5".to_string()),
            ("no_facility".to_string(), "true".to_string()),
        ]);
        let inputs = input_values(&declared, &given);
        let p = prepare_command(
            &ws,
            &settings,
            &inputs,
            Some("UAT"),
            "--owner-type 'Sole Proprietor'",
            &secrets,
        )
        .unwrap();
        assert_eq!(p.program, ws.root().join("env/bin/python"));
        assert_eq!(
            p.args,
            [
                "flow.py",
                "--who",
                "me",
                "--count",
                "5",
                "--no-facility",
                "--owner-type",
                "Sole Proprietor"
            ]
        );
        assert_eq!(p.cwd, ws.root());
        assert!(p.env.contains(&("CR_TOKEN".into(), "key:secret".into())));
        assert!(p.env.contains(&("TUMA_INPUT_COUNT".into(), "5".into())));
        assert!(p.env.contains(&("PYTHONUNBUFFERED".into(), "1".into())));
        // Shown as written, so the token is not.
        assert!(
            p.shown
                .starts_with("env/bin/python flow.py --who '{{who}}' --count 5"),
            "{}",
            p.shown
        );

        let err = prepare_command(&ws, &settings, &inputs, None, "", &secrets).unwrap_err();
        assert!(err.contains("{{token}}"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_runs_and_its_output_is_passed_on() {
        let (_dir, ws) = workspace();
        let script = Script {
            name: "Echo".into(),
            kind: Kind::Command,
            settings: ScriptFile {
                command: "sh -c 'echo out $TUMA_INPUT_WHO; echo err >&2; exit 3'".into(),
                inputs: vec![input("who", InputKind::Text, "", "Tuma")],
                ..Default::default()
            },
            source: String::new(),
        };
        write(&ws, &script).unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = events.clone();
        let outcome = run(
            RunOptions {
                workspace: &ws,
                name: "Echo",
                environment: None,
                inputs: &BTreeMap::new(),
                extra_args: "",
                secrets: &MemoryStore::default(),
                network: None,
                stop: Arc::default(),
            },
            Arc::new(move |e| seen.lock().unwrap().push(e)),
        )
        .await
        .unwrap();
        assert!(!outcome.ok);
        assert_eq!(outcome.exit_code, Some(3));
        let events = events.lock().unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Stdout(t) if t == "out Tuma"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Stderr(t) if t == "err"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_can_be_stopped() {
        let (_dir, ws) = workspace();
        let script = Script {
            name: "Sleep".into(),
            kind: Kind::Command,
            settings: ScriptFile {
                command: "sleep 30".into(),
                ..Default::default()
            },
            source: String::new(),
        };
        write(&ws, &script).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let outcome = run(
            RunOptions {
                workspace: &ws,
                name: "Sleep",
                environment: None,
                inputs: &BTreeMap::new(),
                extra_args: "",
                secrets: &MemoryStore::default(),
                network: None,
                stop,
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        assert!(outcome.stopped && !outcome.ok);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_javascript_script_gets_its_inputs_and_can_save_files() {
        let (_dir, ws) = workspace();
        let script = Script {
            name: "Csv".into(),
            kind: Kind::Javascript,
            settings: ScriptFile {
                inputs: vec![
                    input("count", InputKind::Number, "", "2"),
                    input("upper", InputKind::Boolean, "", "false"),
                ],
                ..Default::default()
            },
            source: r#"
                const rows = [];
                for (let i = 0; i < tuma.inputs.count; i++) rows.push(tuma.inputs.upper ? `ROW ${i}` : `row ${i}`);
                console.log("made", rows.length);
                await tuma.sleep(10);
                pm.globals.set("made", String(rows.length));
                tuma.saveFile("rows.csv", rows.join("\n"));
                return { rows: rows.length };
            "#
            .into(),
        };
        write(&ws, &script).unwrap();
        let logged = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = logged.clone();
        let secrets = MemoryStore::default();
        let outcome = run(
            RunOptions {
                workspace: &ws,
                name: "Csv",
                environment: None,
                inputs: &BTreeMap::from([("upper".to_string(), "true".to_string())]),
                extra_args: "",
                secrets: &secrets,
                network: None,
                stop: Arc::default(),
            },
            Arc::new(move |e| seen.lock().unwrap().push(e)),
        )
        .await
        .unwrap();
        assert!(outcome.ok, "{:?}", outcome.error);
        assert_eq!(outcome.result, Some(json!({ "rows": 2 })));
        assert_eq!(
            outcome.files,
            [SavedFile {
                name: "rows.csv".into(),
                content: "ROW 0\nROW 1".into()
            }]
        );
        assert!(
            logged
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, Event::Console(l) if l.text == "made 2"))
        );
        let globals = ws.effective_globals(&secrets).unwrap();
        assert!(globals.iter().any(|v| v.name == "made" && v.value == "2"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_javascript_script_can_be_stopped() {
        let (_dir, ws) = workspace();
        let script = Script {
            name: "Forever".into(),
            kind: Kind::Javascript,
            settings: ScriptFile::default(),
            source: "for (;;) { await tuma.sleep(50); }".into(),
        };
        write(&ws, &script).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.store(true, Ordering::Relaxed);
        });
        let outcome = run(
            RunOptions {
                workspace: &ws,
                name: "Forever",
                environment: None,
                inputs: &BTreeMap::new(),
                extra_args: "",
                secrets: &MemoryStore::default(),
                network: None,
                stop,
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        assert!(outcome.stopped && !outcome.ok && outcome.error.is_none());
    }
}
