//! Pre-request and test scripts: Postman's `pm.*` API in an embedded JavaScript engine (QuickJS), so scripts from
//! imported collections run unchanged.
//!
//! Scripts are sandboxed: no files, no network except `pm.sendRequest` (which follows the same host rules as the
//! request), a memory limit and a time limit. The API itself is written in JavaScript (`pm.js`); this module runs it
//! and converts requests, responses and variables to and from what it expects.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Function, Object, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http;
use crate::workspace::files::{Body, KeyValue, Part, RequestFile};

const PRELUDE: &str = include_str!("pm.js");

/// How long the scripts of one request may run, timers and `pm.sendRequest` included.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MEMORY_LIMIT: usize = 256 << 20;
/// Scripts run on their own thread with this much stack: libraries such as lodash need more than a default thread has.
const THREAD_STACK: usize = 32 << 20;
const JS_STACK: usize = 16 << 20;

/// Libraries scripts can `require`, as Postman's sandbox has them. Licenses are in `vendor/licenses/`.
fn library(name: &str) -> Option<&'static str> {
    Some(match name {
        "lodash" => include_str!("vendor/lodash.js"),
        "moment" => include_str!("vendor/moment.js"),
        "crypto-js" => include_str!("vendor/crypto-js.js"),
        "chai" => include_str!("vendor/chai.js"),
        "ajv" => include_str!("vendor/ajv.js"),
        "tv4" => include_str!("vendor/tv4.js"),
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    /// Before the request is sent: scripts can change it and set variables.
    Prerequest,
    /// After the response arrives (Postman's "post-response" or "tests").
    Test,
}

/// One script to run, with a label for errors and the console, such as `collection Users API` or `request`.
#[derive(Debug, Clone)]
pub struct Script {
    pub label: String,
    pub source: String,
}

/// What scripts see.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub info: Info,
    /// See [`request_value`] and [`sent_request_value`].
    pub request: Value,
    /// See [`response_value`]. Only for test scripts.
    pub response: Option<Value>,
    pub vars: Vars,
    /// Secret variables, for `pm.vault.get`.
    pub vault: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub event_name: String,
    pub request_name: String,
    /// The saved request's path, or empty.
    pub request_id: String,
    pub iteration: usize,
    pub iteration_count: usize,
    /// The collection and folders the request is in.
    pub location: Vec<String>,
}

/// Variable values by scope, as scripts read and write them.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vars {
    /// `pm.variables.set` values from earlier scripts in the same request or collection run.
    pub local: BTreeMap<String, String>,
    /// The current iteration's row of a data file.
    pub data: BTreeMap<String, String>,
    pub environment: EnvironmentVars,
    /// Tuma's folder variables, nearest first. Read-only to scripts (`pm.variables.get` sees them).
    pub folders: Vec<BTreeMap<String, String>>,
    pub collection: BTreeMap<String, String>,
    pub globals: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EnvironmentVars {
    pub name: Option<String>,
    pub values: BTreeMap<String, String>,
}

/// What scripts did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Output {
    pub tests: Vec<TestResult>,
    /// Errors that stopped a script. A pre-request script error stops the request from being sent.
    pub errors: Vec<ScriptError>,
    #[serde(skip_deserializing)]
    pub console: Vec<LogLine>,
    /// The request after pre-request scripts, in the form of [`request_value`].
    #[serde(skip_serializing)]
    pub request: Value,
    #[serde(skip_serializing)]
    pub vars: VarsOut,
    pub skip_request: bool,
    /// `pm.execution.setNextRequest`: absent when not called, `None` to stop a collection run.
    #[serde(skip_serializing_if = "Option::is_none", with = "double_option")]
    pub next_request: Option<Option<String>>,
    /// `pm.visualizer.set`: `{template, data, options}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visualizer: Option<Value>,
}

/// Variables after the scripts ran. The caller compares them with what it passed in to see what changed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VarsOut {
    /// `pm.variables.set`: for the rest of this request (or run) only.
    pub local: BTreeMap<String, String>,
    pub environment: BTreeMap<String, String>,
    pub collection: BTreeMap<String, String>,
    pub globals: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for TestResult {
    fn default() -> Self {
        Self {
            name: String::new(),
            passed: true,
            skipped: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptError {
    /// Which script: such as `collection Users API`, `folder Admin` or `request`.
    pub script: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    /// `log`, `info`, `warn`, `error` or `debug`.
    pub level: String,
    pub text: String,
}

mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Option<String>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_ref().map(|v| v.as_deref()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(d).map(Some)
    }
}

/// Decides whether `pm.sendRequest` may call a host.
pub type AllowHost = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Runs `scripts` in order in one sandbox, so variables they set are seen by the next. Empty scripts are skipped.
pub fn run(
    event: Event,
    scripts: &[Script],
    input: &Input,
    allow: Option<AllowHost>,
    timeout: Duration,
) -> Output {
    let scripts: Vec<&Script> = scripts
        .iter()
        .filter(|s| !s.source.trim().is_empty())
        .collect();
    if scripts.is_empty() {
        return Output {
            request: input.request.clone(),
            vars: VarsOut {
                local: input.vars.local.clone(),
                environment: input.vars.environment.values.clone(),
                collection: input.vars.collection.clone(),
                globals: input.vars.globals.clone(),
            },
            ..Default::default()
        };
    }
    let mut input = input.clone();
    input.info.event_name = match event {
        Event::Prerequest => "prerequest",
        Event::Test => "test",
    }
    .into();
    let result = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("tuma-script".into())
            .stack_size(THREAD_STACK)
            .spawn_scoped(scope, || run_scripts(&scripts, &input, allow, timeout))
            .map_err(|e| e.to_string())
            .and_then(|t| {
                t.join()
                    .unwrap_or_else(|_| Err("the script engine crashed".into()))
            })
    });
    match result {
        Ok(output) => output,
        Err(message) => Output {
            errors: vec![ScriptError {
                script: scripts.last().map(|s| s.label.clone()).unwrap_or_default(),
                message,
            }],
            request: input.request.clone(),
            vars: VarsOut {
                local: input.vars.local.clone(),
                environment: input.vars.environment.values.clone(),
                collection: input.vars.collection.clone(),
                globals: input.vars.globals.clone(),
            },
            ..Default::default()
        },
    }
}

fn run_scripts(
    scripts: &[&Script],
    input: &Input,
    allow: Option<AllowHost>,
    timeout: Duration,
) -> Result<Output, String> {
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(JS_STACK);
    let deadline = Instant::now() + timeout;
    // Off while Tuma's own code collects the results, so a script that used up its time still reports what it did.
    let enforce = Arc::new(AtomicBool::new(true));
    let handler_enforce = enforce.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        handler_enforce.load(Ordering::Relaxed) && Instant::now() > deadline
    })));
    let context = Context::full(&runtime).map_err(|e| e.to_string())?;
    let console: Rc<RefCell<Vec<LogLine>>> = Rc::default();
    let input_json = serde_json::to_string(input).map_err(|e| e.to_string())?;

    let output = context.with(|ctx| -> Result<String, String> {
        let js = |r: rquickjs::Result<()>, ctx: &Ctx| r.map_err(|e| exception(ctx, e));
        let native = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
        let log = console.clone();
        js(
            native.set(
                "log",
                Function::new(ctx.clone(), move |level: String, text: String| {
                    log.borrow_mut().push(LogLine { level, text });
                }),
            ),
            &ctx,
        )?;
        js(
            native.set(
                "lib",
                Function::new(ctx.clone(), |name: String| library(&name).map(String::from)),
            ),
            &ctx,
        )?;
        js(
            native.set(
                "send",
                Function::new(ctx.clone(), move |spec: String| {
                    send_blocking(&spec, allow.as_ref(), deadline).to_string()
                }),
            ),
            &ctx,
        )?;
        let globals = ctx.globals();
        js(globals.set("__native", native), &ctx)?;
        js(globals.set("__inputJson", input_json), &ctx)?;
        ctx.eval::<(), _>(PRELUDE)
            .map_err(|e| format!("internal error starting scripts: {}", exception(&ctx, e)))?;

        let call = |name: &str, args: (Option<String>, Option<String>)| -> Result<(), String> {
            let f: Function = globals.get(name).map_err(|e| exception(&ctx, e))?;
            let result = match args {
                (Some(a), Some(b)) => f.call::<_, ()>((a, b)),
                _ => f.call::<_, ()>(()),
            };
            result.map_err(|e| exception(&ctx, e))
        };
        let mut failure = None;
        for script in scripts {
            let step = call(
                "__run",
                (Some(script.label.clone()), Some(script.source.clone())),
            )
            .and_then(|_| drain(&ctx, &globals, deadline))
            .and_then(|_| call("__collectTests", (None, None)));
            if let Err(e) = step {
                failure = Some(ScriptError {
                    script: script.label.clone(),
                    message: if e == "timeout" || Instant::now() > deadline {
                        format!(
                            "the scripts took longer than {}s and were stopped",
                            timeout.as_secs()
                        )
                    } else {
                        e
                    },
                });
                break;
            }
        }
        enforce.store(false, Ordering::Relaxed);
        let out: Function = globals.get("__output").map_err(|e| exception(&ctx, e))?;
        let mut text: String = out.call(()).map_err(|e| exception(&ctx, e))?;
        if let Some(failure) = failure {
            let mut value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if let Some(errors) = value["errors"].as_array_mut() {
                errors.push(serde_json::to_value(failure).unwrap_or_default());
            }
            text = value.to_string();
        }
        Ok(text)
    })?;
    let mut output: Output = serde_json::from_str(&output)
        .map_err(|e| format!("internal error reading script results: {e}"))?;
    output.console = console.take();
    Ok(output)
}

/// Runs promise jobs and timers until there is nothing left to do.
fn drain<'js>(ctx: &Ctx<'js>, globals: &Object<'js>, deadline: Instant) -> Result<(), String> {
    loop {
        while ctx.execute_pending_job() {}
        if ctx.has_exception() {
            return Err(exception(ctx, rquickjs::Error::Exception));
        }
        let next: f64 = globals
            .get::<_, Function>("__nextTimer")
            .and_then(|f| f.call(()))
            .map_err(|e| exception(ctx, e))?;
        if next < 0.0 {
            return Ok(());
        }
        let wait = Duration::from_millis(next as u64);
        if Instant::now() + wait > deadline {
            return Err("timeout".into());
        }
        std::thread::sleep(wait);
        globals
            .get::<_, Function>("__fireTimer")
            .and_then(|f| f.call::<_, ()>(()))
            .map_err(|e| exception(ctx, e))?;
    }
}

/// The message of a JavaScript exception, or of an engine error.
fn exception(ctx: &Ctx, error: rquickjs::Error) -> String {
    if !matches!(error, rquickjs::Error::Exception) {
        return error.to_string();
    }
    let value = ctx.catch();
    if let Some(e) = value.as_exception() {
        let message = e.message().unwrap_or_default();
        let name: Option<String> = e.get("name").ok();
        return match name {
            Some(name) if name != "Error" && !name.is_empty() => format!("{name}: {message}"),
            _ => message,
        };
    }
    value
        .as_string()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| format!("{value:?}"))
}

/// A script's syntax error, with its line (from 1) when the engine says, or `None` if it parses.
pub fn syntax_error(source: &str) -> Option<(String, Option<usize>)> {
    if source.trim().is_empty() {
        return None;
    }
    let runtime = Runtime::new().ok()?;
    let context = Context::full(&runtime).ok()?;
    context.with(|ctx| {
        // Compiled, not run: the same wrapper the sandbox uses, one line above the script.
        let wrapped = format!("(async function () {{\n{source}\n}})");
        ctx.eval::<(), _>(wrapped).err()?;
        let value = ctx.catch();
        let e = value.as_exception()?;
        let name: String = e.get("name").unwrap_or_default();
        let message = format!("{name}: {}", e.message().unwrap_or_default());
        // The stack ends with "<input>:LINE:COLUMN"; the wrapper adds one line.
        let line = e
            .stack()
            .and_then(|stack| {
                let digits: Vec<&str> = stack
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|d| !d.is_empty())
                    .collect();
                digits
                    .iter()
                    .rev()
                    .nth(1)
                    .and_then(|l| l.parse::<usize>().ok())
            })
            // An unclosed block is reported at the end of input, past the script.
            .map(|l| l.saturating_sub(1).clamp(1, source.lines().count().max(1)));
        Some((message, line))
    })
}

/// `pm.sendRequest`: sends a request described by the sandbox and returns `{response}` or `{error}`. It runs on its own
/// thread with its own async runtime, so it works whatever the calling thread is doing.
fn send_blocking(spec: &str, allow: Option<&AllowHost>, deadline: Instant) -> Value {
    let spec: Value = match serde_json::from_str(spec) {
        Ok(spec) => spec,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    let mut request = spec_request(&spec);
    let host = crate::runner::host_of(&request.url).unwrap_or_default();
    if let Some(allow) = allow
        && !allow(&host)
    {
        return json!({ "error": format!("pm.sendRequest: {host} is not an allowed host") });
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let limit = request
        .settings
        .timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(remaining)
        .min(remaining);
    request.settings.timeout_ms = Some(limit.as_millis().max(1) as u64);
    let result = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(http::send(&request))
            .map_err(|e| e.to_string())
    })
    .join()
    .unwrap_or_else(|_| Err("pm.sendRequest failed".into()));
    match result {
        Ok(response) => json!({ "response": response_value(&response) }),
        Err(e) => json!({ "error": e }),
    }
}

/// The request `pm.sendRequest` describes.
fn spec_request(spec: &Value) -> http::Request {
    let pairs = |v: &Value| -> Vec<(String, String, bool)> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    p["key"].as_str().unwrap_or_default().to_string(),
                    p["value"].as_str().unwrap_or_default().to_string(),
                    !p["disabled"].as_bool().unwrap_or(false),
                )
            })
            .collect()
    };
    let headers: Vec<http::KeyValue> = pairs(&spec["headers"])
        .into_iter()
        .map(|(name, value, enabled)| http::KeyValue {
            name,
            value,
            enabled,
        })
        .collect();
    let body = &spec["body"];
    let body = match body["mode"].as_str() {
        Some("raw") => {
            let content = body["raw"].as_str().unwrap_or_default().to_string();
            let content_type = if serde_json::from_str::<Value>(&content)
                .is_ok_and(|v| v.is_object() || v.is_array())
            {
                "application/json"
            } else {
                "text/plain"
            };
            http::Body::Raw {
                content,
                content_type: content_type.into(),
            }
        }
        Some("urlencoded") => http::Body::FormUrlEncoded {
            fields: pairs(&body["urlencoded"])
                .into_iter()
                .map(|(name, value, enabled)| http::KeyValue {
                    name,
                    value,
                    enabled,
                })
                .collect(),
        },
        Some("formdata") => http::Body::Multipart {
            parts: pairs(&body["formdata"])
                .into_iter()
                .map(|(name, value, enabled)| http::Part {
                    name,
                    value,
                    enabled,
                    ..Default::default()
                })
                .collect(),
        },
        _ => http::Body::None,
    };
    let mut settings = http::Settings::default();
    if let Some(ms) = spec["timeout"].as_u64() {
        settings.timeout_ms = Some(ms);
    }
    http::Request {
        method: spec["method"].as_str().unwrap_or("GET").to_string(),
        url: spec["url"].as_str().unwrap_or_default().to_string(),
        headers,
        body,
        auth: http::Auth::None,
        settings,
    }
}

// Conversions between Tuma's types and the shapes pm.js expects (Postman's).

fn pair(name: &str, value: &str, enabled: bool) -> Value {
    let mut out = json!({ "key": name, "value": value });
    if !enabled {
        out["disabled"] = json!(true);
    }
    out
}

/// A request as pre-request scripts see it: as saved, before variables and inherited auth.
pub fn request_value(file: &RequestFile, name: &str, id: &str) -> Value {
    let raw = |content: &str, language: &str| json!({ "mode": "raw", "raw": content, "options": { "raw": { "language": language } } });
    let body = match &file.body {
        Body::None => Value::Null,
        Body::Json { content } => raw(content, "json"),
        Body::Xml { content } => raw(content, "xml"),
        Body::Html { content } => raw(content, "html"),
        Body::Javascript { content } => raw(content, "javascript"),
        Body::Text { content } | Body::Raw { content, .. } => raw(content, "text"),
        Body::Form { fields } => json!({
            "mode": "urlencoded",
            "urlencoded": fields.iter().map(|f| pair(&f.name, &f.value, f.enabled)).collect::<Vec<_>>(),
        }),
        Body::Multipart { parts } => json!({
            "mode": "formdata",
            "formdata": parts.iter().map(|p| {
                let mut out = pair(&p.name, &p.value, p.enabled);
                out["type"] = json!(if p.file { "file" } else { "text" });
                out
            }).collect::<Vec<_>>(),
        }),
        Body::File { path, .. } => json!({ "mode": "file", "file": { "src": path } }),
    };
    json!({
        "id": id,
        "name": name,
        "method": file.method,
        "url": file.url,
        "headers": file.headers.iter().map(|h| pair(&h.name, &h.value, h.enabled)).collect::<Vec<_>>(),
        "body": body,
    })
}

/// Applies what pre-request scripts changed (`after`, compared with `before` from [`request_value`]) to `file`.
pub fn apply_request(file: &mut RequestFile, before: &Value, after: &Value) {
    if after["method"] != before["method"]
        && let Some(method) = after["method"].as_str()
    {
        file.method = method.to_uppercase();
    }
    if after["url"] != before["url"]
        && let Some(url) = after["url"].as_str()
    {
        file.url = url.to_string();
    }
    let key_values = |v: &Value| -> Vec<KeyValue> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|p| KeyValue {
                name: p["key"].as_str().unwrap_or_default().to_string(),
                value: p["value"].as_str().unwrap_or_default().to_string(),
                enabled: !p["disabled"].as_bool().unwrap_or(false),
            })
            .collect()
    };
    if after["headers"] != before["headers"] {
        file.headers = key_values(&after["headers"]);
    }
    // The sandbox writes every body field back; compare only the ones the mode uses.
    let essential = |b: &Value| match b["mode"].as_str() {
        Some("raw") => json!(["raw", b["raw"]]),
        Some("urlencoded") => json!(["urlencoded", b["urlencoded"]]),
        Some("formdata") => json!(["formdata", b["formdata"]]),
        Some("file") => json!(["file", b["file"]]),
        _ => Value::Null,
    };
    let (old, new) = (essential(&before["body"]), essential(&after["body"]));
    if old == new {
        return;
    }
    let body = &after["body"];
    file.body = match body["mode"].as_str() {
        Some("raw") => {
            let content = body["raw"].as_str().unwrap_or_default().to_string();
            match &file.body {
                Body::Json { .. } => Body::Json { content },
                Body::Xml { .. } => Body::Xml { content },
                Body::Html { .. } => Body::Html { content },
                Body::Javascript { .. } => Body::Javascript { content },
                Body::Text { .. } => Body::Text { content },
                Body::Raw { content_type, .. } => Body::Raw {
                    content,
                    content_type: content_type.clone(),
                },
                _ if serde_json::from_str::<Value>(&content).is_ok() => Body::Json { content },
                _ => Body::Text { content },
            }
        }
        Some("urlencoded") => Body::Form {
            fields: key_values(&body["urlencoded"]),
        },
        Some("formdata") => Body::Multipart {
            parts: body["formdata"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| Part {
                    name: p["key"].as_str().unwrap_or_default().to_string(),
                    value: p["value"]
                        .as_str()
                        .or(p["src"].as_str())
                        .unwrap_or_default()
                        .to_string(),
                    file: p["type"] == "file",
                    enabled: !p["disabled"].as_bool().unwrap_or(false),
                    ..Default::default()
                })
                .collect(),
        },
        Some("file") => Body::File {
            path: PathBuf::from(body["file"]["src"].as_str().unwrap_or_default()),
            content_type: None,
        },
        _ => Body::None,
    };
}

/// A request as test scripts see it: as sent, with variables resolved and auth added.
pub fn sent_request_value(
    request: &http::Request,
    sent: &http::SentRequest,
    name: &str,
    id: &str,
) -> Value {
    let body = match &request.body {
        http::Body::None => Value::Null,
        http::Body::Raw { content, .. } => json!({ "mode": "raw", "raw": content }),
        http::Body::FormUrlEncoded { fields } => json!({
            "mode": "urlencoded",
            "urlencoded": fields.iter().map(|f| pair(&f.name, &f.value, f.enabled)).collect::<Vec<_>>(),
        }),
        http::Body::Multipart { parts } => json!({
            "mode": "formdata",
            "formdata": parts.iter().map(|p| pair(&p.name, &p.value, p.enabled)).collect::<Vec<_>>(),
        }),
        http::Body::File { path, .. } => json!({ "mode": "file", "file": { "src": path } }),
    };
    json!({
        "id": id,
        "name": name,
        "method": sent.method,
        "url": sent.url,
        "headers": sent.headers.iter().map(|h| pair(&h.name, &h.value, true)).collect::<Vec<_>>(),
        "body": body,
    })
}

/// A response as test scripts see it.
pub fn response_value(response: &http::Response) -> Value {
    json!({
        "code": response.status,
        "status": response.reason,
        "headers": response.headers.iter().map(|h| pair(&h.name, &h.value, true)).collect::<Vec<_>>(),
        "body": String::from_utf8_lossy(&response.body),
        "responseTime": response.timing.total.as_millis() as u64,
        "responseSize": response.body.len(),
    })
}

#[cfg(test)]
mod tests;
