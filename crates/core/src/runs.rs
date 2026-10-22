//! Collection runs: every request under a collection or folder, in order, for a number of iterations or once per row
//! of a data file, following `pm.execution.setNextRequest`. Results can be written as JUnit XML for CI.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::history::History;
use crate::runner::{self, Detail, Job, Outcome, Scripting, Summary};
use crate::script::AllowHost;
use crate::secrets::SecretStore;
use crate::workspace::{self, Workspace};

/// What to run, and how.
pub struct RunOptions<'a> {
    pub workspace: &'a Workspace,
    /// A collection, folder or request; empty for the whole workspace.
    pub path: &'a str,
    pub environment: Option<&'a str>,
    pub overrides: &'a [(String, String)],
    pub secrets: &'a dyn SecretStore,
    pub history: Option<&'a History>,
    pub source: &'a str,
    pub allow_host: Option<AllowHost>,
    /// How many times to run everything. With a data file, at least once per row.
    pub iterations: usize,
    /// Rows of a data file (see [`read_data_file`]): row N is `pm.iterationData` in iteration N.
    pub data: Vec<BTreeMap<String, String>>,
    /// A pause between requests.
    pub delay: Duration,
    /// Only requests that test something: an `expect:` block, or a test script of their own or inherited.
    pub only_tests: bool,
    /// Stop at the first request that fails.
    pub fail_fast: bool,
    /// Run pre-request and test scripts.
    pub scripts: bool,
    /// Set from elsewhere to stop the run after the current request.
    pub cancel: Option<&'a AtomicBool>,
    pub detail: Detail,
}

/// One request in a run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// Counting from 0.
    pub iteration: usize,
    pub name: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// What was run: a collection, folder or request, or empty for the whole workspace.
    pub path: String,
    pub environment: Option<String>,
    pub iterations: usize,
    /// Unix time in milliseconds.
    pub started_at: u64,
    pub duration_ms: u64,
    pub summary: Summary,
    pub steps: Vec<Step>,
    /// Why the run ended early, if it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

/// Runs the requests. `progress` is called after each one, for live output.
pub async fn run(
    options: RunOptions<'_>,
    progress: &mut (dyn FnMut(&Step) + Send),
) -> Result<Report, workspace::Error> {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let started = Instant::now();
    let ws = options.workspace;
    let mut requests = Vec::new();
    for path in runner::requests_under(ws, options.path)? {
        let file = ws.read_request(&path)?;
        if options.only_tests && !tests_something(ws, &path, &file) {
            continue;
        }
        requests.push((path, file));
    }
    let iterations = options.iterations.max(options.data.len()).max(1);
    // setNextRequest can loop on purpose (polling until something is ready), but not forever.
    let max_steps = (requests.len() * 100).max(1000);
    let name_of = |path: &str| {
        path.rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".yaml")
            .to_string()
    };

    let mut steps: Vec<Step> = Vec::new();
    let mut stopped = None;
    'run: for iteration in 0..iterations {
        let data = options.data.get(iteration);
        let mut locals = BTreeMap::new();
        let mut index = 0;
        let mut taken = 0;
        while index < requests.len() {
            if options.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                stopped = Some("stopped".into());
                break 'run;
            }
            taken += 1;
            if taken > max_steps {
                stopped = Some(format!(
                    "stopped after {max_steps} requests: setNextRequest seems to loop forever"
                ));
                break 'run;
            }
            if !(steps.is_empty() || options.delay.is_zero()) {
                tokio::time::sleep(options.delay).await;
            }
            let (path, file) = &requests[index];
            let execution = runner::execute(Job {
                workspace: Some(ws),
                request: file,
                path: Some(path),
                context: None,
                environment: options.environment,
                overrides: options.overrides,
                extra_checks: None,
                secrets: options.secrets,
                history: options.history,
                source: options.source,
                allow_host: options.allow_host.clone(),
                scripting: Scripting {
                    enabled: options.scripts,
                    iteration,
                    iteration_count: iterations,
                    data,
                    locals: Some(&locals),
                    ..Default::default()
                },
            })
            .await;
            locals = execution.locals.clone();
            let next = execution.next_request.clone();
            let step = Step {
                iteration,
                name: name_of(path),
                outcome: runner::summarize(Some(path), execution, &options.detail),
            };
            progress(&step);
            let failed = !step.outcome.passed && !step.outcome.skipped;
            steps.push(step);
            if failed && options.fail_fast {
                stopped = Some("stopped at the first failure (--fail-fast)".into());
                break 'run;
            }
            index = match next {
                None => index + 1,
                // setNextRequest(null) ends this iteration.
                Some(None) => break,
                Some(Some(name)) => match requests
                    .iter()
                    .position(|(p, _)| name_of(p) == name || p == &name)
                {
                    Some(i) => i,
                    None => {
                        stopped = Some(format!(
                            "setNextRequest(\"{name}\"): no request with that name in this run"
                        ));
                        break 'run;
                    }
                },
            };
        }
    }
    let outcomes: Vec<Outcome> = steps.iter().map(|s| s.outcome.clone()).collect();
    Ok(Report {
        path: options.path.to_string(),
        environment: options.environment.map(String::from),
        iterations,
        started_at,
        duration_ms: started.elapsed().as_millis() as u64,
        summary: Summary::of(&outcomes),
        steps,
        stopped,
    })
}

/// Whether a request tests anything: checks, or a test script of its own or from its folders.
pub fn tests_something(
    ws: &Workspace,
    path: &str,
    file: &crate::workspace::files::RequestFile,
) -> bool {
    !file.expect.is_empty()
        || !file.scripts.test.trim().is_empty()
        || ws
            .ancestors(path)
            .unwrap_or_default()
            .iter()
            .any(|a| !a.folder.scripts.test.trim().is_empty())
}

/// Reads a data file for a run: a JSON array of objects, or CSV with a header row. Values become strings.
pub fn read_data_file(path: &Path) -> Result<Vec<BTreeMap<String, String>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let is_json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        || text.trim_start().starts_with('[');
    if is_json {
        parse_json_data(&text).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        parse_csv(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn parse_json_data(text: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let rows = value
        .as_array()
        .ok_or("a JSON data file is an array of objects")?;
    rows.iter()
        .map(|row| {
            let row = row
                .as_object()
                .ok_or("every row of a JSON data file is an object")?;
            Ok(row
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), v)
                })
                .collect())
        })
        .collect()
}

/// CSV as spreadsheets write it: commas, double quotes around fields with commas, quotes or line breaks, `""` for a
/// quote inside one.
fn parse_csv(text: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => record.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (c, _) => field.push(c),
        }
    }
    if quoted {
        return Err("a quoted field is not closed".into());
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    let mut records = records.into_iter();
    let Some(header) = records.next() else {
        return Ok(vec![]);
    };
    let header: Vec<String> = header.into_iter().map(|h| h.trim().to_string()).collect();
    records
        .enumerate()
        .map(|(i, r)| {
            if r.len() > header.len() {
                return Err(format!(
                    "row {} has {} fields but the header has {}",
                    i + 2,
                    r.len(),
                    header.len()
                ));
            }
            Ok(header
                .iter()
                .cloned()
                .zip(r.into_iter().chain(std::iter::repeat(String::new())))
                .collect())
        })
        .collect()
}

fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A run as JUnit XML, which CI systems show as test results: a test suite per request (and iteration), a test case
/// per check and test.
pub fn junit(report: &Report, name: &str) -> String {
    let seconds = |ms: Option<u64>| format!("{:.3}", ms.unwrap_or(0) as f64 / 1000.0);
    let mut suites = String::new();
    let mut total_tests = 0;
    let mut total_failures = 0;
    let mut total_errors = 0;
    for step in &report.steps {
        let o = &step.outcome;
        let suite = if report.iterations > 1 {
            format!(
                "{} (iteration {})",
                o.path.as_deref().unwrap_or(&step.name),
                step.iteration + 1
            )
        } else {
            o.path.clone().unwrap_or_else(|| step.name.clone())
        };
        let mut cases = String::new();
        let (mut tests, mut failures, mut errors, mut skipped) = (0, 0, 0, 0);
        let class = xml(o
            .path
            .as_deref()
            .unwrap_or(&step.name)
            .trim_end_matches(".yaml"));
        if let Some(error) = &o.error {
            tests += 1;
            if o.skipped {
                skipped += 1;
                cases.push_str(&format!("    <testcase name=\"request\" classname=\"{class}\"><skipped message=\"{}\"/></testcase>\n", xml(error)));
            } else {
                errors += 1;
                cases.push_str(&format!(
                    "    <testcase name=\"{} {}\" classname=\"{class}\"><error message=\"{}\"/></testcase>\n",
                    xml(&o.method),
                    xml(&o.url),
                    xml(error)
                ));
            }
        }
        for c in &o.checks {
            tests += 1;
            cases.push_str(&format!(
                "    <testcase name=\"{}\" classname=\"{class}\"",
                xml(&c.check)
            ));
            if c.passed {
                cases.push_str("/>\n");
            } else {
                failures += 1;
                cases.push_str(&format!(
                    "><failure message=\"expected {}, got {}\"/></testcase>\n",
                    xml(&c.expected),
                    xml(&c.actual)
                ));
            }
        }
        for t in &o.tests {
            tests += 1;
            cases.push_str(&format!(
                "    <testcase name=\"{}\" classname=\"{class}\"",
                xml(&t.name)
            ));
            if t.skipped {
                skipped += 1;
                cases.push_str("><skipped/></testcase>\n");
            } else if t.passed {
                cases.push_str("/>\n");
            } else {
                failures += 1;
                cases.push_str(&format!(
                    "><failure message=\"{}\"/></testcase>\n",
                    xml(t.error.as_deref().unwrap_or("failed"))
                ));
            }
        }
        for e in &o.script_errors {
            tests += 1;
            errors += 1;
            cases.push_str(&format!(
                "    <testcase name=\"{} script\" classname=\"{class}\"><error message=\"{}\"/></testcase>\n",
                xml(&e.script),
                xml(&e.message)
            ));
        }
        if tests == 0 {
            // A request without checks still shows up, passing if it was sent.
            tests = 1;
            cases.push_str(&format!(
                "    <testcase name=\"{} {}\" classname=\"{class}\"/>\n",
                xml(&o.method),
                xml(&o.url)
            ));
        }
        total_tests += tests;
        total_failures += failures;
        total_errors += errors;
        suites.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\" skipped=\"{skipped}\" time=\"{}\">\n{cases}  </testsuite>\n",
            xml(&suite),
            seconds(o.time_ms)
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"{}\" tests=\"{total_tests}\" failures=\"{total_failures}\" errors=\"{total_errors}\" time=\"{}\">\n{suites}</testsuites>\n",
        xml(name),
        seconds(Some(report.duration_ms))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_data_files() {
        let rows = parse_csv("name,note\r\nAnn,\"says \"\"hi\"\", twice\"\nBob,\n\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["note"], "says \"hi\", twice");
        assert_eq!(rows[1]["name"], "Bob");
        assert_eq!(rows[1]["note"], "");
        assert!(parse_csv("a\n\"open").is_err());
        assert!(parse_csv("a\n1,2").is_err());
    }

    #[test]
    fn json_data_files() {
        let rows = parse_json_data(r#"[{"id": 1, "name": "Ann", "tags": ["a"]}]"#).unwrap();
        assert_eq!(rows[0]["id"], "1");
        assert_eq!(rows[0]["tags"], r#"["a"]"#);
        assert!(parse_json_data(r#"{"id": 1}"#).is_err());
    }
}
