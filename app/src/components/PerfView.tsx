import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, Gauge, Play, ShieldCheck, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import { MethodTag } from "./ui";

const fmt = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(2)} s` : `${Math.round(ms)} ms`);

/** A load test of a request, folder or collection, with passive security checks of its responses. */
export function PerfView({ path }: { path: string }) {
  const environment = useStore((s) => s.environment);
  const [options, setOptions] = useState<api.PerfOptions>({
    users: 10,
    durationS: 30,
    iterations: 1,
    rampUpS: 0,
    delayMs: 0,
    maxP95Ms: null,
    maxErrorRate: null,
  });
  const [byTime, setByTime] = useState(true);
  const [runId, setRunId] = useState<string | null>(null);
  const [report, setReport] = useState<api.PerfReport | null>(null);
  const live = useRef<string | null>(null);
  const set = (patch: Partial<api.PerfOptions>) => setOptions((o) => ({ ...o, ...patch }));
  const name = path.replace(/\.yaml$/, "");

  useEffect(() => {
    const stop = listen<{ runId: string; report: api.PerfReport }>("perf-progress", (e) => {
      if (e.payload.runId === live.current) setReport(e.payload.report);
    });
    return () => void stop.then((f) => f());
  }, []);

  const start = async () => {
    const id = crypto.randomUUID();
    live.current = id;
    setRunId(id);
    setReport(null);
    try {
      const done = await api.perfStart(id, path, environment, {
        ...options,
        durationS: byTime ? options.durationS : 0,
      });
      if (live.current === id) setReport(done);
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: String(e) });
    } finally {
      if (live.current === id) setRunId(null);
    }
  };
  const exportReport = async () => {
    if (!report) return;
    const file = await save({
      title: "Export the performance report",
      defaultPath: `${name.split("/").pop()} performance.html`,
      filters: [
        { name: "HTML report", extensions: ["html"] },
        { name: "JSON", extensions: ["json"] },
      ],
    });
    if (!file) return;
    try {
      await api.perfExport(report, file);
      useStore.getState().notify({ tone: "info", text: `Saved the report to ${file}.` });
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: String(e) });
    }
  };

  const number = (value: number | null, onChange: (n: number | null) => void, min = 0, placeholder = "") => (
    <input
      type="number"
      min={min}
      value={value ?? ""}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value === "" ? null : Math.max(min, Number(e.target.value)))}
    />
  );
  const progress =
    report && byTime && options.durationS > 0 && !report.finished
      ? Math.min(100, (report.elapsedMs / (options.durationS * 1000)) * 100)
      : null;

  return (
    <section className="folder-editor perf">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb">
            <Gauge size={14} /> Performance test
            <span className="crumb-sep">/</span>
          </span>
          <span className="crumb current">{name}</span>
          <span className="muted small">{environment ? `Environment: ${environment}` : "No environment"}</span>
        </div>
        {!path.endsWith(".yaml") && (
          <button className="button ghost" onClick={() => useStore.setState({ view: { kind: "runner", path } })}>
            Functional run
          </button>
        )}
        {report?.finished && (
          <button className="button" onClick={() => void exportReport()}>
            <Download size={14} />
            Export report
          </button>
        )}
        {runId ? (
          <button className="button" onClick={() => void api.perfStop(runId)}>
            <Square size={13} />
            Stop
          </button>
        ) : (
          <button className="button primary" onClick={() => void start()}>
            <Play size={14} />
            Start
          </button>
        )}
      </div>
      <div className="runner-options perf-options">
        <label>
          Virtual users
          {number(options.users, (n) => set({ users: Math.min(200, n ?? 1) }), 1)}
        </label>
        <span className="segmented">
          <button className={byTime ? "active" : ""} onClick={() => setByTime(true)}>
            For a time
          </button>
          <button className={byTime ? "" : "active"} onClick={() => setByTime(false)}>
            Iterations
          </button>
        </span>
        {byTime ? (
          <label>
            Duration (s){number(options.durationS, (n) => set({ durationS: n ?? 1 }), 1)}
          </label>
        ) : (
          <label>
            Iterations per user{number(options.iterations, (n) => set({ iterations: n ?? 1 }), 1)}
          </label>
        )}
        <label>
          Ramp-up (s){number(options.rampUpS, (n) => set({ rampUpS: n ?? 0 }))}
        </label>
        <label>
          Think time (ms){number(options.delayMs, (n) => set({ delayMs: n ?? 0 }))}
        </label>
        <label title="Fail the test when the 95th percentile latency is above this">
          Fail if p95 &gt; (ms){number(options.maxP95Ms, (n) => set({ maxP95Ms: n }), 0, "none")}
        </label>
        <label title="Fail the test when more than this percentage of requests fail">
          or errors &gt; (%){number(options.maxErrorRate, (n) => set({ maxErrorRate: n }), 0, "none")}
        </label>
      </div>
      {progress !== null && (
        <div className="perf-progress">
          <div style={{ width: `${progress}%` }} />
        </div>
      )}
      <div className="panel-body perf-body">
        {!report ? (
          <div className="placeholder">
            <Gauge size={44} strokeWidth={1.3} />
            <span>
              Virtual users send {path.endsWith(".yaml") ? "this request" : "the requests in it, one after another,"} over
              and over.
            </span>
            <span className="muted small">
              Use it on servers you run or may test: start small, against a staging server, not production. Scripts don't
              run during a load test.
            </span>
          </div>
        ) : (
          <PerfResults report={report} />
        )}
      </div>
    </section>
  );
}

function PerfResults({ report }: { report: api.PerfReport }) {
  const verdict =
    report.passed === true ? (
      <span className="pill">Passed</span>
    ) : report.passed === false ? (
      <span className="pill decrypt-failed" title={report.failures.join("; ")}>
        Failed: {report.failures.join("; ")}
      </span>
    ) : null;
  // The last second is cut short when the test ends; drawn, it looks like a drop in throughput.
  const seconds = report.finished && report.timeline.length > 2 ? report.timeline.slice(0, -1) : report.timeline;
  const cards: [string, string][] = [
    ["Requests", report.requests.toLocaleString()],
    ["Throughput", `${report.throughput.toFixed(1)}/s`],
    ["Error rate", `${report.errorRate.toFixed(2)}%`],
    ["Average", fmt(report.latency.avg)],
    ["p95", fmt(report.latency.p95)],
    ["p99", fmt(report.latency.p99)],
  ];
  return (
    <div className="perf-results">
      <div className="perf-status">
        {report.finished ? (report.stopped ? "Stopped early" : "Finished") : "Running"} · {fmt(report.elapsedMs)}
        {!report.finished && <span className="spinner" />}
        {verdict}
      </div>
      <div className="perf-cards">
        {cards.map(([label, value]) => (
          <div key={label} className="perf-card">
            <div className="muted small">{label}</div>
            <div className="perf-value">{value}</div>
          </div>
        ))}
      </div>
      <div className="perf-charts">
        <Chart title="Requests per second" points={seconds.map((s) => [s.second, s.requests])} tone="accent" />
        <Chart title="Average response time (ms)" points={seconds.map((s) => [s.second, s.avgMs])} tone="put" />
        <Chart title="Errors per second" points={seconds.map((s) => [s.second, s.errors])} tone="fail" />
      </div>
      <table className="header-table perf-table">
        <thead>
          <tr>
            <th>Request</th>
            <th>Count</th>
            <th>Errors</th>
            <th>Avg</th>
            <th>p50</th>
            <th>p90</th>
            <th>p95</th>
            <th>p99</th>
            <th>Max</th>
            <th>Statuses</th>
          </tr>
        </thead>
        <tbody>
          {report.perRequest.map((r) => (
            <tr key={r.path}>
              <td>
                <MethodTag method={r.method} /> {r.name}
              </td>
              <td>{r.requests}</td>
              <td className={r.errors ? "fail" : ""}>{r.errors}</td>
              <td>{fmt(r.latency.avg)}</td>
              <td>{fmt(r.latency.p50)}</td>
              <td>{fmt(r.latency.p90)}</td>
              <td>{fmt(r.latency.p95)}</td>
              <td>{fmt(r.latency.p99)}</td>
              <td>{fmt(r.latency.max)}</td>
              <td className="muted small">
                {Object.entries(r.statuses)
                  .map(([k, v]) => `${k}: ${v}`)
                  .join(", ")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3 className="mock-heading">
        <ShieldCheck size={15} /> Security checks
      </h3>
      <p className="hint">
        Passive checks of the responses the test received; nothing extra is sent. They point out configuration to review,
        not vulnerabilities proven.
      </p>
      {report.findings.length === 0 ? (
        <p className="ok small">{report.finished ? "No findings." : "Checking as responses arrive…"}</p>
      ) : (
        <table className="header-table perf-findings">
          <tbody>
            {report.findings.map((f, i) => (
              <tr key={i}>
                <td>
                  <span className={`severity ${f.severity}`}>{f.severity}</span>
                </td>
                <td>{f.check}</td>
                <td className="muted">{f.request}</td>
                <td>{f.detail}</td>
                <td className="muted small">{f.advice}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {report.notes.length > 0 && (
        <ul className="docs-warnings">
          {report.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** A small line chart: one point per second. */
function Chart({ title, points, tone }: { title: string; points: [number, number][]; tone: "accent" | "put" | "fail" }) {
  const w = 320;
  const h = 110;
  const pad = 6;
  const max = Math.max(1e-9, ...points.map(([, v]) => v));
  const last = Math.max(1, ...points.map(([s]) => s));
  const path = points
    .map(([s, v], i) => `${i ? "L" : "M"}${(pad + (s / last) * (w - 2 * pad)).toFixed(1)},${(h - pad - (v / max) * (h - 2 * pad)).toFixed(1)}`)
    .join(" ");
  return (
    <div className="perf-chart">
      <div className="perf-chart-head">
        <span>{title}</span>
        <span className="muted mono-ish">{points.length ? (max >= 100 ? max.toFixed(0) : max.toFixed(1)) : "–"}</span>
      </div>
      <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label={title}>
        {points.length > 1 && <path d={path} fill="none" className={`line ${tone}`} strokeWidth="2" vectorEffect="non-scaling-stroke" />}
      </svg>
    </div>
  );
}
