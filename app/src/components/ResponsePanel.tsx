import { useMemo, useState } from "react";
import { useStore } from "../store";
import type { Header, Response, Timing } from "../types";
import { CodeEditor, type Language } from "./CodeEditor";
import { Tabs } from "./Tabs";

type Tab = "body" | "headers" | "timing" | "request";
type View = "pretty" | "raw" | "preview";

export function ResponsePanel() {
  const response = useStore((s) => s.response);
  const error = useStore((s) => s.error);
  const sending = useStore((s) => s.sending);
  const [tab, setTab] = useState<Tab>("body");

  if (sending && !response) return <Placeholder>Sending…</Placeholder>;
  if (error && error.kind !== "cancelled") {
    return (
      <section className="response-panel">
        <div className="error-box">
          <strong>Could not send the request</strong>
          <p className="mono">{error.message}</p>
        </div>
        <Unresolved names={error.unresolved ?? []} />
      </section>
    );
  }
  if (!response) return <Placeholder>Send a request to see the response here.</Placeholder>;

  return (
    <section className={`response-panel${sending ? " stale" : ""}`}>
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "body", label: "Body" },
          { id: "headers", label: "Headers", count: response.headers.length },
          { id: "timing", label: "Timing" },
          { id: "request", label: "Request" },
        ]}
      >
        <StatusLine response={response} />
      </Tabs>
      <Unresolved names={response.unresolved} />
      <div className="panel-body">
        {tab === "body" && <BodyView response={response} />}
        {tab === "headers" && <HeaderTable headers={response.headers} />}
        {tab === "timing" && <TimingView response={response} />}
        {tab === "request" && <SentView response={response} />}
      </div>
    </section>
  );
}

function Placeholder({ children }: { children: React.ReactNode }) {
  return (
    <section className="response-panel">
      <p className="empty muted">{children}</p>
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
  const language = languageFor(type);

  const pretty = useMemo(() => {
    if (response.bodyBase64 || language !== "json") return response.body;
    try {
      return JSON.stringify(JSON.parse(response.body), null, 2);
    } catch {
      return response.body;
    }
  }, [response, language]);

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
    return <iframe className="preview" sandbox="" srcDoc={response.body} title="HTML preview" />;
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
