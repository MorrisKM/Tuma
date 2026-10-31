//! Performance tests: virtual users send a collection's (or folder's, or one request's) requests over and over, for a
//! time or a number of iterations, with an optional ramp-up, as Postman's performance runner does. The result is
//! throughput, error rates and latency percentiles per request and over time, passive security checks of the
//! responses, and a report to export.
//!
//! Requests are prepared once (variables, auth, encryption), so the load measures the API rather than Tuma; scripts
//! don't run during a performance test.

pub mod report;
pub mod security;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::http::{self, Network};
use crate::secrets::SecretStore;
use crate::workspace::{Layers, Node, NodeKind, Workspace};

/// The most virtual users a run may have: enough for a laptop to load a test server, not enough to flood anything.
pub const MAX_USERS: u32 = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Users sending at the same time.
    pub users: u32,
    /// How long the test runs, in seconds; when 0, each user goes through the requests `iterations` times.
    pub duration_s: u64,
    pub iterations: u32,
    /// Users start one after another over this many seconds.
    pub ramp_up_s: u64,
    /// Each user waits this long between requests ("think time").
    pub delay_ms: u64,
    /// Fail the test when the 95th percentile latency is above this.
    pub max_p95_ms: Option<f64>,
    /// Fail the test when more than this percentage of requests fail.
    pub max_error_rate: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            users: 10,
            duration_s: 30,
            iterations: 1,
            ramp_up_s: 0,
            delay_ms: 0,
            max_p95_ms: None,
            max_error_rate: None,
        }
    }
}

/// A request ready to send many times.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub path: String,
    pub request: http::Request,
    /// Sent with credentials, for the security checks.
    pub authenticated: bool,
}

/// Prepares the HTTP requests in `scope` (a request's, folder's or collection's path; the workspace when `None`) with
/// `environment`. WebSocket and gRPC requests are left out.
pub async fn prepare(
    workspace: &Workspace,
    scope: Option<&str>,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
) -> Result<(Vec<Target>, Vec<String>), String> {
    let mut paths = Vec::new();
    fn walk(nodes: &[Node], scope: Option<&str>, out: &mut Vec<(String, String)>) {
        for n in nodes {
            let inside = scope.is_none_or(|s| n.path == s || n.path.starts_with(&format!("{s}/")));
            let above = scope.is_some_and(|s| s.starts_with(&format!("{}/", n.path)));
            if n.kind == NodeKind::Request && inside {
                out.push((n.name.clone(), n.path.clone()));
            } else if n.kind != NodeKind::Request && (inside || above) {
                walk(&n.children, scope, out);
            }
        }
    }
    walk(
        &workspace.tree().map_err(|e| e.to_string())?,
        scope,
        &mut paths,
    );
    let mut targets = Vec::new();
    let mut notes = Vec::new();
    for (name, path) in paths {
        let file = workspace.read_request(&path).map_err(|e| e.to_string())?;
        if file.is_websocket() || file.is_grpc() {
            notes.push(format!(
                "{name}: WebSocket and gRPC requests are not part of performance tests"
            ));
            continue;
        }
        if !file.scripts.is_empty() {
            notes.push(format!(
                "{name}: its scripts don't run during a performance test"
            ));
        }
        let mut prepared = workspace
            .prepare_with(&file, Some(&path), environment, &Layers::default(), secrets)
            .map_err(|e| e.to_string())?;
        if !prepared.unresolved.is_empty() {
            notes.push(format!(
                "{name}: no value for {}",
                prepared
                    .unresolved
                    .iter()
                    .map(|v| format!("{{{{{v}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        crate::request_token::resolve(
            &mut prepared.request.auth,
            Some(workspace),
            environment,
            secrets,
            network,
        )
        .await?;
        if let http::Auth::OAuth2(oauth) = &mut prepared.request.auth {
            let token =
                crate::oauth2::token_for(oauth, Some(workspace.id()), secrets, network).await?;
            oauth.access_token = Some(token.access_token);
        }
        if let Some(crypto) = &prepared.crypto
            && !crypto.settings.encrypt.is_empty()
            && let http::Body::Raw { content, .. } = &mut prepared.request.body
        {
            *content =
                crate::crypto::encrypt_body(content, &crypto.settings.encrypt, &crypto.cipher()?)?;
        }
        let authenticated = !matches!(prepared.request.auth, http::Auth::None)
            || prepared
                .request
                .headers
                .iter()
                .any(|h| h.enabled && h.name.eq_ignore_ascii_case("authorization"));
        targets.push(Target {
            name,
            path,
            request: prepared.request,
            authenticated,
        });
    }
    if targets.is_empty() {
        return Err("there are no HTTP requests to test here".into());
    }
    Ok((targets, notes))
}

/// Latency figures, in milliseconds.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Latency {
    pub min: f64,
    pub avg: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Latency {
    fn of(samples: &mut [f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_by(f64::total_cmp);
        let at = |p: f64| samples[((p / 100.0) * (samples.len() - 1) as f64).round() as usize];
        Self {
            min: samples[0],
            avg: samples.iter().sum::<f64>() / samples.len() as f64,
            p50: at(50.0),
            p90: at(90.0),
            p95: at(95.0),
            p99: at(99.0),
            max: samples[samples.len() - 1],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestStats {
    pub name: String,
    pub path: String,
    pub method: String,
    pub requests: u64,
    /// Failed to send (connection refused, timeout) or answered 4xx/5xx.
    pub errors: u64,
    /// Counts by status code, and "error" for requests that got no response.
    pub statuses: BTreeMap<String, u64>,
    pub latency: Latency,
    pub throughput: f64,
}

/// One second of the run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Second {
    pub second: u64,
    pub requests: u64,
    pub errors: u64,
    pub avg_ms: f64,
    pub users: u32,
}

/// A run, as it goes and when it ends.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub name: String,
    /// Unix ms.
    pub started_at: u64,
    pub elapsed_ms: u64,
    pub finished: bool,
    pub stopped: bool,
    pub options: Options,
    pub environment: Option<String>,
    pub requests: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub throughput: f64,
    pub latency: Latency,
    pub per_request: Vec<RequestStats>,
    pub timeline: Vec<Second>,
    pub findings: Vec<security::Finding>,
    pub notes: Vec<String>,
    /// Whether the thresholds held; `None` without thresholds.
    pub passed: Option<bool>,
    /// Which thresholds failed.
    pub failures: Vec<String>,
}

#[derive(Default)]
struct Shared {
    samples: Vec<Vec<f64>>,
    errors: Vec<u64>,
    statuses: Vec<BTreeMap<String, u64>>,
    /// Per second: requests, errors, total latency, most users active.
    timeline: BTreeMap<u64, (u64, u64, f64, u32)>,
    active: u32,
    /// The first response of each request, for the security checks.
    sampled: Vec<bool>,
    findings: Vec<security::Finding>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Runs a performance test. `progress` gets a report about twice a second; setting `stop` to true ends the run early.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    name: &str,
    targets: Vec<Target>,
    options: Options,
    environment: Option<String>,
    notes: Vec<String>,
    network: Arc<Network>,
    progress: Arc<dyn Fn(Report) + Send + Sync>,
    stop: watch::Receiver<bool>,
) -> Report {
    let mut options = options;
    options.users = options.users.clamp(1, MAX_USERS);
    options.iterations = options.iterations.max(1);
    let targets = Arc::new(targets);
    let shared = Arc::new(Mutex::new(Shared {
        samples: vec![Vec::new(); targets.len()],
        errors: vec![0; targets.len()],
        statuses: vec![BTreeMap::new(); targets.len()],
        sampled: vec![false; targets.len()],
        ..Shared::default()
    }));
    let started = Instant::now();
    let started_at = now_ms();
    let deadline =
        (options.duration_s > 0).then(|| started + Duration::from_secs(options.duration_s));

    let mut users = Vec::new();
    for user in 0..options.users {
        let targets = targets.clone();
        let shared = shared.clone();
        let network = network.clone();
        let mut stop = stop.clone();
        let options = options.clone();
        // Ramp-up: user i starts i/users of the way through it.
        let start_after = Duration::from_millis(
            options.ramp_up_s * 1000 * u64::from(user) / u64::from(options.users),
        );
        users.push(tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(start_after) => {}
                _ = stop.wait_for(|s| *s) => return,
            }
            shared.lock().unwrap().active += 1;
            let mut iteration = 0;
            'run: loop {
                if deadline.is_none() && iteration >= options.iterations {
                    break;
                }
                iteration += 1;
                for (i, target) in targets.iter().enumerate() {
                    if *stop.borrow() || deadline.is_some_and(|d| Instant::now() >= d) {
                        break 'run;
                    }
                    let sent = Instant::now();
                    let result = tokio::select! {
                        r = http::send_with(&target.request, &network) => r,
                        _ = stop.wait_for(|s| *s) => break 'run,
                    };
                    let ms = sent.elapsed().as_secs_f64() * 1000.0;
                    let second = started.elapsed().as_secs();
                    // Recorded in a block of its own, so the lock is never held across an await.
                    {
                        let mut s = shared.lock().unwrap();
                        let failed = match &result {
                            Ok(r) => r.status >= 400,
                            Err(_) => true,
                        };
                        s.samples[i].push(ms);
                        if failed {
                            s.errors[i] += 1;
                        }
                        let status = match &result {
                            Ok(r) => r.status.to_string(),
                            Err(_) => "error".to_string(),
                        };
                        *s.statuses[i].entry(status).or_default() += 1;
                        let active = s.active;
                        let slot = s.timeline.entry(second).or_default();
                        slot.0 += 1;
                        slot.1 += u64::from(failed);
                        slot.2 += ms;
                        slot.3 = slot.3.max(active);
                        if let Ok(response) = &result
                            && !s.sampled[i]
                        {
                            s.sampled[i] = true;
                            let found = security::check(
                                &target.name,
                                &target.request.url,
                                response,
                                target.authenticated,
                            );
                            s.findings.extend(found);
                        }
                    }
                    if options.delay_ms > 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(options.delay_ms)) => {}
                            _ = stop.wait_for(|s| *s) => break 'run,
                        }
                    }
                }
            }
            shared.lock().unwrap().active -= 1;
        }));
    }

    let snapshot = |finished: bool, stopped: bool| -> Report {
        let s = shared.lock().unwrap();
        let elapsed = started.elapsed();
        let secs = elapsed.as_secs_f64().max(0.001);
        let mut all: Vec<f64> = s.samples.iter().flatten().copied().collect();
        let per_request: Vec<RequestStats> = targets
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut samples = s.samples[i].clone();
                RequestStats {
                    name: t.name.clone(),
                    path: t.path.clone(),
                    method: t.request.method.to_uppercase(),
                    requests: samples.len() as u64,
                    errors: s.errors[i],
                    statuses: s.statuses[i].clone(),
                    throughput: samples.len() as f64 / secs,
                    latency: Latency::of(&mut samples),
                }
            })
            .collect();
        let requests = all.len() as u64;
        let errors: u64 = s.errors.iter().sum();
        let timeline: Vec<Second> = s
            .timeline
            .iter()
            .map(|(second, (n, e, total, users))| Second {
                second: *second,
                requests: *n,
                errors: *e,
                avg_ms: if *n > 0 { total / *n as f64 } else { 0.0 },
                users: *users,
            })
            .collect();
        let mut findings = s.findings.clone();
        findings.sort_by_key(|f| f.severity);
        let error_rate = if requests > 0 {
            errors as f64 * 100.0 / requests as f64
        } else {
            0.0
        };
        let latency = Latency::of(&mut all);
        let mut failures = Vec::new();
        if let Some(max) = options.max_p95_ms
            && latency.p95 > max
        {
            failures.push(format!(
                "p95 latency {:.0} ms is above {max:.0} ms",
                latency.p95
            ));
        }
        if let Some(max) = options.max_error_rate
            && error_rate > max
        {
            failures.push(format!("error rate {error_rate:.2}% is above {max}%"));
        }
        let has_thresholds = options.max_p95_ms.is_some() || options.max_error_rate.is_some();
        Report {
            name: name.to_string(),
            started_at,
            elapsed_ms: elapsed.as_millis() as u64,
            finished,
            stopped,
            options: options.clone(),
            environment: environment.clone(),
            requests,
            errors,
            error_rate,
            throughput: requests as f64 / secs,
            latency,
            per_request,
            timeline,
            findings,
            notes: notes.clone(),
            passed: (finished && has_thresholds).then_some(failures.is_empty()),
            failures,
        }
    };

    let all_done = async {
        for user in users {
            let _ = user.await;
        }
    };
    tokio::pin!(all_done);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = &mut all_done => break,
            _ = tick.tick() => progress(snapshot(false, false)),
        }
    }
    let report = snapshot(true, *stop.borrow());
    progress(report.clone());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let mut samples: Vec<f64> = (1..=100).map(f64::from).collect();
        let l = Latency::of(&mut samples);
        assert_eq!((l.min, l.max), (1.0, 100.0));
        assert_eq!(l.p50, 51.0);
        assert_eq!(l.p95, 95.0);
        assert!((l.avg - 50.5).abs() < 1e-9);
        assert_eq!(Latency::of(&mut []), Latency::default());
    }
}
