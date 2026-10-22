//! A finished performance test as a file to keep or share: one standalone HTML page (styles and charts inline), or
//! JSON for tools.

use std::fmt::Write;

use super::security::Severity;
use super::{Report, Second};

pub fn json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_default() + "\n"
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn ms(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2} s", v / 1000.0)
    } else {
        format!("{v:.0} ms")
    }
}

/// A line chart of `value` per second, as inline SVG.
fn chart(timeline: &[Second], value: impl Fn(&Second) -> f64, unit: &str, color: &str) -> String {
    if timeline.is_empty() {
        return String::from("<p class=\"none\">No data.</p>");
    }
    let (w, h, pad) = (640.0, 160.0, 28.0);
    let max = timeline
        .iter()
        .map(&value)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    let last = timeline.last().map(|s| s.second).unwrap_or(0).max(1) as f64;
    let points: Vec<String> = timeline
        .iter()
        .map(|s| {
            let x = pad + (s.second as f64 / last) * (w - 2.0 * pad);
            let y = h - pad - (value(s) / max) * (h - 2.0 * pad);
            format!("{x:.1},{y:.1}")
        })
        .collect();
    format!(
        r#"<svg viewBox="0 0 {w} {h}" class="chart" role="img"><line x1="{pad}" y1="{bottom}" x2="{right}" y2="{bottom}" class="axis"/><text x="{pad}" y="14" class="label">{max_label} {unit}</text><text x="{right}" y="{h_label}" class="label end">{last}s</text><polyline points="{points}" fill="none" stroke="{color}" stroke-width="2" stroke-linejoin="round"/></svg>"#,
        bottom = h - pad,
        right = w - pad,
        h_label = h - 8.0,
        max_label = if max >= 100.0 {
            format!("{max:.0}")
        } else {
            format!("{max:.1}")
        },
        points = points.join(" "),
    )
}

pub fn html(report: &Report) -> String {
    let mut out = String::new();
    let o = &report.options;
    let load = if o.duration_s > 0 {
        format!("{} users for {} s", o.users, o.duration_s)
    } else {
        format!("{} users × {} iterations", o.users, o.iterations)
    };
    let ramp = if o.ramp_up_s > 0 {
        format!(", ramp-up {} s", o.ramp_up_s)
    } else {
        String::new()
    };
    let verdict = match report.passed {
        Some(true) => "<span class=\"verdict ok\">Passed</span>".to_string(),
        Some(false) => format!(
            "<span class=\"verdict fail\">Failed</span> <span class=\"muted\">{}</span>",
            esc(&report.failures.join("; "))
        ),
        None => String::new(),
    };
    let started = report.started_at / 1000;
    let _ = write!(
        out,
        r#"<header><div class="eyebrow">Performance test</div><h1>{name}</h1><p class="muted">{load}{ramp}{env} · started at Unix time {started} · {elapsed}{stopped}</p>{verdict}</header>"#,
        name = esc(&report.name),
        env = report
            .environment
            .as_ref()
            .map(|e| format!(" · environment {}", esc(e)))
            .unwrap_or_default(),
        elapsed = ms(report.elapsed_ms as f64),
        stopped = if report.stopped {
            " · stopped early"
        } else {
            ""
        },
    );
    let cards = [
        ("Requests", report.requests.to_string()),
        ("Throughput", format!("{:.1}/s", report.throughput)),
        ("Error rate", format!("{:.2}%", report.error_rate)),
        ("Average", ms(report.latency.avg)),
        ("p95", ms(report.latency.p95)),
        ("p99", ms(report.latency.p99)),
    ];
    out.push_str("<section class=\"cards\">");
    for (label, value) in cards {
        let _ = write!(
            out,
            "<div class=\"card\"><div class=\"label\">{label}</div><div class=\"value\">{value}</div></div>"
        );
    }
    out.push_str("</section>");
    let _ = write!(
        out,
        "<section class=\"charts\"><div><h2>Requests per second</h2>{}</div><div><h2>Average response time</h2>{}</div></section>",
        chart(&report.timeline, |s| s.requests as f64, "req/s", "#0c9a6e"),
        chart(&report.timeline, |s| s.avg_ms, "ms", "#2759b8"),
    );
    out.push_str("<h2>Requests</h2><table><thead><tr><th>Request</th><th>Count</th><th>Errors</th><th>Avg</th><th>p50</th><th>p90</th><th>p95</th><th>p99</th><th>Max</th><th>Statuses</th></tr></thead><tbody>");
    for r in &report.per_request {
        let statuses: Vec<String> = r
            .statuses
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        let _ = write!(
            out,
            "<tr><td><span class=\"m\">{}</span> {}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"muted\">{}</td></tr>",
            esc(&r.method),
            esc(&r.name),
            r.requests,
            if r.errors > 0 { "fail" } else { "" },
            r.errors,
            ms(r.latency.avg),
            ms(r.latency.p50),
            ms(r.latency.p90),
            ms(r.latency.p95),
            ms(r.latency.p99),
            ms(r.latency.max),
            esc(&statuses.join(", ")),
        );
    }
    out.push_str("</tbody></table>");
    out.push_str("<h2>Security checks</h2><p class=\"muted\">Passive checks of the responses received: nothing extra was sent. They point out configuration to review; they are not a penetration test.</p>");
    if report.findings.is_empty() {
        out.push_str("<p class=\"ok\">No findings.</p>");
    } else {
        out.push_str("<table><thead><tr><th>Severity</th><th>Check</th><th>Request</th><th>Detail</th><th>What to do</th></tr></thead><tbody>");
        for f in &report.findings {
            let (class, label) = match f.severity {
                Severity::High => ("high", "High"),
                Severity::Medium => ("medium", "Medium"),
                Severity::Low => ("low", "Low"),
                Severity::Info => ("info", "Info"),
            };
            let _ = write!(
                out,
                "<tr><td><span class=\"sev {class}\">{label}</span></td><td>{}</td><td>{}</td><td>{}</td><td class=\"muted\">{}</td></tr>",
                esc(&f.check),
                esc(&f.request),
                esc(&f.detail),
                esc(&f.advice)
            );
        }
        out.push_str("</tbody></table>");
    }
    if !report.notes.is_empty() {
        out.push_str("<h2>Notes</h2><ul>");
        for n in &report.notes {
            let _ = write!(out, "<li>{}</li>", esc(n));
        }
        out.push_str("</ul>");
    }
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · performance test</title><style>{STYLE}</style></head>
<body><main>{out}<footer class="muted">Generated by Tuma</footer></main></body></html>
"#,
        title = esc(&report.name)
    )
}

const STYLE: &str = r#"
:root{--bg:#fff;--text:#171717;--muted:#737373;--border:#e6e6e3;--card:#f7f7f6;--ok:#0f8a5f;--fail:#d23c3c;--warn:#b26b00;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--bg:#121212;--text:#ececec;--muted:#8f8f8f;--border:#262626;--card:#1a1a1a;--ok:#4ad295;--fail:#f26d6d;--warn:#ebb54d}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:1060px;margin:0 auto;padding:40px 28px 60px}h1{margin:2px 0 6px;font-size:26px;letter-spacing:-.02em}
h2{font-size:14px;margin:30px 0 10px}.eyebrow{font-size:11px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);font-weight:600}
.muted{color:var(--muted)}.verdict{display:inline-block;padding:3px 10px;border-radius:99px;font-weight:700;font-size:12px}
.verdict.ok{color:var(--ok);background:color-mix(in srgb,var(--ok) 14%,transparent)}.verdict.fail{color:var(--fail);background:color-mix(in srgb,var(--fail) 14%,transparent)}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:10px;margin-top:22px}
.card{background:var(--card);border:1px solid var(--border);border-radius:10px;padding:12px 14px}.card .label{font-size:12px;color:var(--muted)}
.card .value{font-size:22px;font-weight:650;font-variant-numeric:tabular-nums}.charts{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:16px}
.chart{width:100%;height:auto;border:1px solid var(--border);border-radius:10px;background:var(--card)}.chart .axis{stroke:var(--border)}
.chart .label{fill:var(--muted);font-size:11px}.chart .end{text-anchor:end}table{width:100%;border-collapse:collapse;font-size:13px}
th,td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--border);vertical-align:top}th{font-size:11.5px;color:var(--muted);font-weight:600}
td{font-variant-numeric:tabular-nums}.m{font-family:ui-monospace,Menlo,monospace;font-size:11px;font-weight:700;color:var(--muted)}
.fail{color:var(--fail)}.ok{color:var(--ok)}.sev{font-size:11px;font-weight:700;padding:2px 8px;border-radius:99px}
.sev.high{color:#fff;background:var(--fail)}.sev.medium{color:var(--warn);background:color-mix(in srgb,var(--warn) 16%,transparent)}
.sev.low{color:var(--muted);background:var(--card);border:1px solid var(--border)}.sev.info{color:var(--muted)}
footer{margin-top:40px;font-size:12px}.none{color:var(--muted)}
"#;
