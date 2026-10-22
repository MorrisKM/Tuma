import { useEffect, useMemo, useState } from "react";
import { useStore } from "../store";
import type { Header, Response, Timing } from "../types";
import { select } from "../jsonpath";
import { CodeEditor, type Language } from "./CodeEditor";
import { isGrpc, isWebSocket } from "../draft";
import { GrpcLog } from "./GrpcViews";
import { EventsView } from "./EventsView";
import { MessageLog } from "./WebSocketViews";
import { ConsoleView, TestsView, testsLabel, VisualizerView } from "./ScriptResults";
import { Logo } from "./ui";
import { Tabs } from "./Tabs";

type Tab = "body" | "events" | "tests" | "console" | "visualize" | "headers" | "timing" | "request";
type View = "pretty" | "raw" | "preview";

export function ResponsePanel() {
  const response = useStore((s) => s.response);
  const error = useStore((s) => s.error);
  const sending = useStore((s) => s.sending);
  const live = useStore((s) => (s.sending ? s.live[s.sending] : undefined));
  const websocket = useStore((s) => isWebSocket(s.draft.url));
  const grpc = useStore((s) => isGrpc(s.draft.url));
  const [tab, setTab] = useState<Tab>("body");
  // A finished event stream opens on its events.
  useEffect(() => {
    if (response?.events) setTab("events");
  }, [response]);

  if (grpc) {
    return (
      <section className="response-panel">
        <div className="panel-body">
          <GrpcLog />
        </div>
      </section>
    );
  }
  if (websocket) {
    return (
      <section className="response-panel">
        <div className="panel-body">
          <MessageLog />
        </div>
      </section>
    );
  }
  // An event stream shows its events while it is open.
  if (sending && live) {
    return (
      <section className="response-panel">
        <div className="panel-body">
          <EventsView events={live} live />
        </div>
      </section>
    );
  }
  if (sending && !response) {
    return (
      <Placeholder>
        <span className="spinner big" />
        <span>Sending…</span>
      </Placeholder>
    );
  }
  if (error && error.kind !== "cancelled") {
    const skipped = error.kind === "skipped";
    return (
      <section className="response-panel">
        <div className={skipped ? "notice" : "error-box"}>
          <strong>
            {skipped ? "Not sent" : error.kind === "script" ? "A pre-request script failed" : "Could not send the request"}
          </strong>
          <p className="mono">{error.message}</p>
        </div>
        <Unresolved names={error.unresolved ?? []} />
        {(error.tests?.length ?? 0) > 0 && (
          <div className="panel-body">
            <TestsView checks={[]} tests={error.tests ?? []} errors={[]} />
          </div>
        )}
        {(error.console?.length ?? 0) > 0 && (
          <div className="panel-body">
            <ConsoleView lines={error.console ?? []} />
          </div>
        )}
      </section>
    );
  }
  if (!response) {
    return (
      <Placeholder>
        <Logo size={56} />
        <span>Enter a URL and send the request to see the response here.</span>
        <span className="placeholder-keys">
          <span>
            <kbd>Ctrl</kbd>
            <kbd>↵</kbd>
          </span>
          <span>Send</span>
          <span>
            <kbd>Ctrl</kbd>
            <kbd>L</kbd>
          </span>
          <span>Focus the URL</span>
          <span>
            <kbd>Ctrl</kbd>
            <kbd>K</kbd>
          </span>
          <span>Find a request or command</span>
        </span>
      </Placeholder>
    );
  }
  const tests = testsLabel(response.checks, response.tests, response.scriptErrors);
  // A tab that this response doesn't have shows the body instead.
  const shown: Tab =
    (tab === "events" && !response.events) ||
    (tab === "tests" && !tests) ||
    (tab === "console" && !response.console.length && !response.scriptErrors.length) ||
    (tab === "visualize" && !response.visualizer)
      ? "body"
      : tab;

  return (
    <section className={`response-panel${sending ? " stale" : ""}`}>
      <Tabs
        active={shown}
        onChange={setTab}
        tabs={[
          ...(response.events ? [{ id: "events" as Tab, label: "Events", count: response.events.length }] : []),
          { id: "body", label: "Body" },
          ...(tests ? [{ id: "tests" as Tab, label: tests }] : []),
          ...(response.console.length || response.scriptErrors.length
            ? [{ id: "console" as Tab, label: "Console", count: response.console.length }]
            : []),
          ...(response.visualizer ? [{ id: "visualize" as Tab, label: "Visualize" }] : []),
          { id: "headers", label: "Headers", count: response.headers.length },
          { id: "timing", label: "Timing" },
          { id: "request", label: "Request" },
        ]}
      >
        <StatusLine response={response} />
      </Tabs>
      <Unresolved names={response.unresolved} />
      <div className="panel-body">
        {shown === "events" && response.events && <EventsView events={response.events} live={false} />}
        {shown === "body" && <BodyView response={response} />}
        {shown === "tests" && (
          <TestsView checks={response.checks} tests={response.tests} errors={response.scriptErrors} />
        )}
        {shown === "console" && <ConsoleView lines={response.console} />}
        {shown === "visualize" && response.visualizer && (
          <VisualizerView url={response.visualizer.url} error={response.visualizer.error} />
        )}
        {shown === "headers" && <HeaderTable headers={response.headers} />}
        {shown === "timing" && <TimingView response={response} />}
        {shown === "request" && <SentView response={response} />}
      </div>
    </section>
  );
}

function Placeholder({ children }: { children: React.ReactNode }) {
  return (
    <section className="response-panel">
      <div className="placeholder">{children}</div>
    </section>
  );
}

/** Variables that had no value, so their {{placeholders}} were sent as written. */
function Unresolved({ names }: { names: string[] }) {
  const environment = useStore((s) => s.environment);
  if (names.length === 0) return null;
  return (
    <div className="unresolved">
      {names.length === 1 ? "This variable has" : "These variables have"} no value
      {environment ? ` in "${environment}" or any other scope` : " (no environment is selected)"}:{" "}
      {names.map((n) => (
        <code key={n}>{`{{${n}}}`}</code>
      ))}
    </div>
  );
}

function StatusLine({ response }: { response: Response }) {
  const tone = response.status < 300 ? "ok" : response.status < 400 ? "redirect" : "fail";
  return (
    <div className="status-line">
      <span className={`status ${tone}`}>
        {response.status} {response.reason}
      </span>
      <span>{formatMs(response.timing.total)}</span>
      <span title={`${response.encodedSize} bytes on the wire`}>{formatBytes(response.size)}</span>
    </div>
  );
}

function contentType(response: Response): string {
  return response.headers.find((h) => h.name === "content-type")?.value.split(";")[0].trim().toLowerCase() ?? "";
}

function languageFor(type: string): Language {
  if (type.includes("json")) return "json";
  if (type.includes("html")) return "html";
  if (type.includes("xml")) return "xml";
  if (type.includes("javascript")) return "javascript";
  return "text";
}

function BodyView({ response }: { response: Response }) {
  const type = contentType(response);
  const previewable = type.startsWith("image/") || type === "text/html";
  const [view, setView] = useState<View>("pretty");
  const [filter, setFilter] = useState("");
  const language = languageFor(type);

  const json = useMemo(() => {
    if (response.bodyBase64 || language !== "json") return undefined;
    try {
      return JSON.parse(response.body) as unknown;
    } catch {
      return undefined;
    }
  }, [response, language]);

  // With a filter, Pretty shows only the selected values.
  const [pretty, filterError] = useMemo((): [string, string | null] => {
    if (json === undefined) return [response.body, null];
    if (!filter.trim()) return [JSON.stringify(json, null, 2), null];
    try {
      const found = select(json, filter);
      return [JSON.stringify(found.length === 1 ? found[0] : found, null, 2), null];
    } catch (e) {
      return [JSON.stringify(json, null, 2), (e as Error).message];
    }
  }, [json, filter, response.body]);

  if (response.size === 0) return <p className="hint">The response has no body.</p>;

  const shown: View = view === "preview" && !previewable ? "pretty" : view;
  return (
    <div className="body-view">
      <div className="toolbar">
        <div className="segmented">
          {(["pretty", "raw", "preview"] as View[]).map((v) => (
            <button
              key={v}
              className={shown === v ? "active" : ""}
              disabled={v === "preview" && !previewable}
              onClick={() => setView(v)}
            >
              {v[0].toUpperCase() + v.slice(1)}
            </button>
          ))}
        </div>
        {json !== undefined && shown === "pretty" && (
          <input
            className={`json-filter mono${filterError ? " invalid" : ""}`}
            value={filter}
            placeholder="Filter: $.items[*].id"
            aria-label="Filter the JSON with a path"
            title={filterError ?? "A JSON path, as in checks: $.name, $.items[0], $.items[*].id"}
            spellCheck={false}
            onChange={(e) => setFilter(e.target.value)}
          />
        )}
        <span className="muted">{type || "no content type"}</span>
      </div>
      {shown === "preview" ? (
        <Preview response={response} type={type} />
      ) : response.bodyBase64 ? (
        <p className="hint">Binary data ({formatBytes(response.size)}).</p>
      ) : (
        <div className="editor-frame">
          <CodeEditor value={shown === "pretty" ? pretty : response.body} language={shown === "pretty" ? language : "text"} readOnly />
        </div>
      )}
    </div>
  );
}

function Preview({ response, type }: { response: Response; type: string }) {
  if (type === "text/html" && !response.bodyBase64) {
    // Sandboxed with no permissions: scripts in the page do not run.
    return <iframe className="html-preview" sandbox="" srcDoc={response.body} title="HTML preview" />;
  }
  const data = response.bodyBase64 ? response.body : btoa(unescape(encodeURIComponent(response.body)));
  return <img className="preview-image" src={`data:${type};base64,${data}`} alt="Response" />;
}

function HeaderTable({ headers }: { headers: Header[] }) {
  return (
    <table className="header-table">
      <tbody>
        {headers.map((h, i) => (
          <tr key={i}>
            <th>{h.name}</th>
            <td className="mono">{h.value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const PHASES: { key: keyof Timing; label: string }[] = [
  { key: "dns", label: "DNS lookup" },
  { key: "connect", label: "TCP connect" },
  { key: "tls", label: "TLS handshake" },
  { key: "wait", label: "Waiting (time to first byte)" },
  { key: "download", label: "Download" },
];

function TimingView({ response }: { response: Response }) {
  const { timing } = response;
  // The phases of the last exchange, laid out end to end. Redirects add to the total but not to the phases.
  let offset = 0;
  const rows = PHASES.flatMap(({ key, label }) => {
    const ms = timing[key];
    if (ms === null) return [];
    const row = { label, ms, start: offset };
    offset += ms;
    return [row];
  });
  const span = Math.max(offset, 0.001);

  return (
    <div className="timing">
      {rows.map((row) => (
        <div className="timing-row" key={row.label}>
          <span>{row.label}</span>
          <div className="timing-track">
            <div
              className="timing-bar"
              style={{ left: `${(row.start / span) * 100}%`, width: `max(2px, ${(row.ms / span) * 100}%)` }}
            />
          </div>
          <span className="mono">{formatMs(row.ms)}</span>
        </div>
      ))}
      <div className="timing-row total">
        <span>Total</span>
        <div />
        <span className="mono">{formatMs(timing.total)}</span>
      </div>
      <dl className="facts">
        <dt>Protocol</dt>
        <dd>{response.version}</dd>
        {response.remoteAddr && (
          <>
            <dt>Remote address</dt>
            <dd className="mono">{response.remoteAddr}</dd>
          </>
        )}
        {response.redirects.length > 0 && (
          <>
            <dt>Redirects</dt>
            <dd>
              {response.redirects.map((r, i) => (
                <div key={i} className="mono">
                  {r.status} {r.url}
                </div>
              ))}
              <div className="mono">→ {response.url}</div>
            </dd>
          </>
        )}
      </dl>
    </div>
  );
}

function SentView({ response }: { response: Response }) {
  const { sent } = response;
  return (
    <div className="sent">
      <p className="mono">
        <strong>{sent.method}</strong> {sent.url}
      </p>
      <HeaderTable headers={sent.headers} />
      <p className="muted">Body: {formatBytes(sent.bodySize)}</p>
    </div>
  );
}

export function formatMs(ms: number): string {
  return ms < 1 ? `${ms.toFixed(2)} ms` : ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
