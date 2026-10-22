import type { CheckResult, LogLine, ScriptError, TestResult } from "../types";

/** Checks from the expect: block, then pm.test results, then scripts that failed. */
export function TestsView({
  checks,
  tests,
  errors,
}: {
  checks: CheckResult[];
  tests: TestResult[];
  errors: ScriptError[];
}) {
  return (
    <ul className="checks">
      {errors.map((e, i) => (
        <li key={`e${i}`} className="fail">
          <span className="check-mark">✗</span>
          <span className="check-name">The {e.script} script failed</span>
          <span className="check-detail mono">{e.message}</span>
        </li>
      ))}
      {checks.map((c, i) => (
        <li key={`c${i}`} className={c.passed ? "ok" : "fail"}>
          <span className="check-mark">{c.passed ? "✓" : "✗"}</span>
          <span className="check-name">{c.check}</span>
          {!c.passed && (
            <span className="check-detail mono">
              expected {c.expected}, got {c.actual}
            </span>
          )}
        </li>
      ))}
      {tests.map((t, i) => (
        <li key={`t${i}`} className={t.skipped ? "skipped" : t.passed ? "ok" : "fail"}>
          <span className="check-mark">{t.skipped ? "–" : t.passed ? "✓" : "✗"}</span>
          <span className="check-name">
            {t.name}
            {t.skipped && <span className="muted"> (skipped)</span>}
          </span>
          {t.error && <span className="check-detail mono">{t.error}</span>}
        </li>
      ))}
    </ul>
  );
}

/** A label such as "Tests ✓ 4" or "Tests ✗ 1/4", or null when there is nothing to show. */
export function testsLabel(checks: CheckResult[], tests: TestResult[], errors: ScriptError[]): string | null {
  const counted = tests.filter((t) => !t.skipped);
  const total = checks.length + counted.length;
  const failed = checks.filter((c) => !c.passed).length + counted.filter((t) => !t.passed).length + errors.length;
  if (total === 0 && errors.length === 0) return tests.length ? "Tests" : null;
  return failed ? `Tests ✗ ${failed}/${total + errors.length}` : `Tests ✓ ${total}`;
}

/** What scripts logged. */
export function ConsoleView({ lines }: { lines: LogLine[] }) {
  if (lines.length === 0) return <p className="hint">Scripts logged nothing. Use console.log() in a script.</p>;
  return (
    <div className="console mono">
      {lines.map((l, i) => (
        <pre key={i} className={`console-line ${l.level}`}>
          {l.text}
        </pre>
      ))}
    </div>
  );
}

/** pm.visualizer output, in a sandboxed frame: it can run its own scripts but can't reach the app. */
export function VisualizerView({ url, error }: { url: string | null; error: string | null }) {
  if (error) return <p className="notice error">{error}</p>;
  if (!url) return null;
  return <iframe className="visualizer" title="Visualization" src={url} sandbox="allow-scripts" />;
}
