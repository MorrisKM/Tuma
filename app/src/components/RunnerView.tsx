import { Gauge, Play, Square } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { PastRun, RunReport, RunStep } from "../types";
import { formatMs } from "./ResponsePanel";
import { ConsoleView, TestsView } from "./ScriptResults";

/** Runs every request in a collection or folder, with scripts and checks, and keeps past runs. */
export function RunnerView({ path }: { path: string }) {
  const environment = useStore((s) => s.environment);
  const [iterations, setIterations] = useState(1);
  const [delayMs, setDelayMs] = useState(0);
  const [dataFile, setDataFile] = useState<string | null>(null);
  const [onlyTests, setOnlyTests] = useState(false);
  const [runId, setRunId] = useState<string | null>(null);
  const [steps, setSteps] = useState<RunStep[]>([]);
  const [report, setReport] = useState<RunReport | null>(null);
  const [file, setFile] = useState<string | null>(null);
  const [past, setPast] = useState<PastRun[]>([]);
  const [open, setOpen] = useState<number | null>(null);
  const live = useRef<string | null>(null);
  const notify = useStore.getState().notify;

  const loadPast = () => api.runsList().then(setPast, () => setPast([]));
  useEffect(() => {
    void loadPast();
    const stop = listen<{ runId: string; step: RunStep }>("run-step", (e) => {
      if (e.payload.runId === live.current) setSteps((s) => [...s, e.payload.step]);
    });
    return () => void stop.then((f) => f());
  }, []);

  const start = async () => {
    const id = crypto.randomUUID();
    live.current = id;
    setRunId(id);
    setSteps([]);
    setReport(null);
    setFile(null);
    setOpen(null);
    try {
      const done = await api.runCollection({ runId: id, path, environment, iterations, delayMs, dataFile, onlyTests });
      setReport(done.report);
      setSteps(done.report.steps);
      setFile(done.file);
      void loadPast();
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    } finally {
      live.current = null;
      setRunId(null);
      void useStore.getState().loadHistory();
    }
  };

  const openPast = async (run: PastRun) => {
    try {
      const r = await api.runRead(run.file);
      setReport(r);
      setSteps(r.steps);
      setFile(run.file);
      setOpen(null);
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };

  const exportRun = async (format: "junit" | "json") => {
    if (!file) return;
    const target = await saveDialog({
      title: format === "junit" ? "Export as JUnit XML" : "Export as JSON",
      defaultPath: `${path.replace(/[\\/]/g, " - ")}-results.${format === "junit" ? "xml" : "json"}`,
      filters: [{ name: format === "junit" ? "JUnit XML" : "JSON", extensions: [format === "junit" ? "xml" : "json"] }],
    });
    if (!target) return;
    try {
      await api.runExport(file, target, format);
      notify({ tone: "info", text: `Exported the results to ${target}.` });
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };

  const pickData = async () => {
    const chosen = await openDialog({
      title: "A data file: CSV with a header row, or a JSON array of objects",
      filters: [{ name: "CSV or JSON", extensions: ["csv", "json"] }],
    });
    if (typeof chosen === "string") setDataFile(chosen);
  };

  const summary = report?.summary;
  const tests = steps.flatMap((s) => [...(s.checks ?? []), ...(s.tests ?? []).filter((t) => !t.skipped)]);
  const failedTests = tests.filter((t) => !t.passed).length;
  const failed = steps.filter((s) => !s.passed && !s.skipped).length;

  return (
    <section className="folder-editor runner">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Run {path}</span>
          <span className="muted small">{environment ? `Environment: ${environment}` : "No environment"}</span>
        </div>
        <button className="button ghost" onClick={() => useStore.setState({ view: { kind: "perf", path } })}>
          <Gauge size={14} />
          Performance test
        </button>
        {runId ? (
          <button className="button" onClick={() => void api.stopRun(runId)}>
            <Square size={13} />
            Stop
          </button>
        ) : (
          <button className="button primary" onClick={() => void start()}>
            <Play size={14} />
            Run
          </button>
        )}
      </div>
      <div className="runner-options">
        <label>
          Iterations
          <input
            type="number"
            min={1}
            value={iterations}
            onChange={(e) => setIterations(Math.max(1, Number(e.target.value) || 1))}
          />
        </label>
        <label>
          Delay (ms)
          <input type="number" min={0} value={delayMs} onChange={(e) => setDelayMs(Math.max(0, Number(e.target.value) || 0))} />
        </label>
        <label className="data-file">
          Data file
          <span className="mono small">{dataFile ? dataFile.split(/[\\/]/).pop() : "none"}</span>
          <button className="link-button" onClick={() => void pickData()}>
            Choose…
          </button>
          {dataFile && (
            <button className="link-button" onClick={() => setDataFile(null)}>
              Clear
            </button>
          )}
        </label>
        <label className="check">
          <input type="checkbox" checked={onlyTests} onChange={(e) => setOnlyTests(e.target.checked)} />
          Only requests with tests or checks
        </label>
      </div>
      <div className="runner-body">
        <div className="runner-results">
          {(steps.length > 0 || runId) && (
            <div className="toolbar spaced runner-summary">
              <span>
                {runId && <span className="spinner" />} {steps.length} request{steps.length === 1 ? "" : "s"}
                {failed > 0 ? <span className="fail"> · {failed} failed</span> : steps.length > 0 && <span className="ok"> · all passed</span>}
                {tests.length > 0 && (
                  <span className="muted">
                    {" "}
                    · {tests.length - failedTests}/{tests.length} tests passed
                  </span>
                )}
                {summary && <span className="muted"> · {formatMs(report.durationMs)}</span>}
                {report?.stopped && <span className="muted"> · {report.stopped}</span>}
              </span>
              {file && !runId && (
                <span>
                  <button className="link-button" onClick={() => void exportRun("junit")}>
                    Export JUnit
                  </button>{" "}
                  <button className="link-button" onClick={() => void exportRun("json")}>
                    Export JSON
                  </button>
                </span>
              )}
            </div>
          )}
          {steps.length === 0 && !runId && (
            <p className="hint">
              Runs every request in {path} in order, with the collection's, folders' and requests' scripts and checks.
              Scripts can change the order with pm.execution.setNextRequest(). With a data file, each row is one
              iteration: its columns are {"{{variables}}"} and pm.iterationData.
            </p>
          )}
          <ul className="run-steps">
            {steps.map((s, i) => {
              const own = [...(s.checks ?? []), ...(s.tests ?? []).filter((t) => !t.skipped)];
              const ownFailed = own.filter((t) => !t.passed).length;
              return (
                <li key={i} className={s.skipped ? "skipped" : s.passed ? "ok" : "fail"}>
                  <button className="run-step" onClick={() => setOpen(open === i ? null : i)}>
                    <span className="check-mark">{s.skipped ? "–" : s.passed ? "✓" : "✗"}</span>
                    {report && report.iterations > 1 && <span className="muted small">#{s.iteration + 1}</span>}
                    <span className={`method-tag method-${s.method.toLowerCase()}`}>{s.method}</span>
                    <span className="run-step-name">{s.name}</span>
                    <span className="muted small">
                      {s.status ? `${s.status} ${s.reason ?? ""}` : s.skipped ? "skipped" : "error"}
                      {s.timeMs !== undefined && ` · ${s.timeMs} ms`}
                      {own.length > 0 && ` · ${own.length - ownFailed}/${own.length}`}
                    </span>
                  </button>
                  {open === i && (
                    <div className="run-step-detail">
                      <p className="mono small">
                        {s.method} {s.url}
                      </p>
                      {s.error && <p className={s.skipped ? "hint" : "notice error"}>{s.error}</p>}
                      <TestsView checks={s.checks ?? []} tests={s.tests ?? []} errors={s.scriptErrors ?? []} />
                      {(s.console?.length ?? 0) > 0 && <ConsoleView lines={s.console ?? []} />}
                      {s.path && (
                        <button className="link-button" onClick={() => void useStore.getState().openRequest(s.path!)}>
                          Open the request
                        </button>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
        <aside className="runner-past">
          <h4>Past runs</h4>
          {past.length === 0 && <p className="muted small">None yet.</p>}
          {past.map((r) => (
            <button key={r.file} className={`past-run${r.file === file ? " active" : ""}`} onClick={() => void openPast(r)}>
              <span className={r.summary.failed ? "fail" : "ok"}>{r.summary.failed ? "✗" : "✓"}</span>
              <span>
                {r.path || "Everything"}
                <span className="muted small">
                  {" "}
                  {new Date(r.startedAt).toLocaleString()} · {r.summary.passed}/{r.summary.requests} passed
                </span>
              </span>
            </button>
          ))}
        </aside>
      </div>
    </section>
  );
}
