import { BookmarkPlus, Copy, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../store";
import type { Example, Header, Response, Timing } from "../types";
import { uniqueName } from "./ExamplesEditor";
import { select } from "../jsonpath";
import { CodeEditor, type Language, openSearchIn } from "./CodeEditor";
import { isGrpc, isWebSocket } from "../draft";
import { GrpcLog } from "./GrpcViews";
import { EventsView } from "./EventsView";
import { MessageLog } from "./WebSocketViews";
import { ConsoleView, TestsView, testsLabel, VisualizerView } from "./ScriptResults";
import { Logo } from "./ui";
import { Tabs } from "./Tabs";
import { findUrls, urlLinks } from "../links";
import { CompareView } from "./CompareView";

/** Links in a response open a new request; made once, so the editor is not reconfigured on every render. */
const links = urlLinks((url) => useStore.getState().openUrl(url));

type Tab = "compare" | "body" | "events" | "tests" | "console" | "visualize" | "headers" | "cookies" | "timing" | "request";
type View = "pretty" | "raw" | "preview";

export function ResponsePanel() {
  const response = useStore((s) => s.response);
  const error = useStore((s) => s.error);
  const sending = useStore((s) => s.sending);
  const live = useStore((s) => (s.sending ? s.live[s.sending] : undefined));
  const websocket = useStore((s) => isWebSocket(s.draft.url));
  const grpc = useStore((s) => isGrpc(s.draft.url));
  const kept = useStore((s) => s.kept.length);
  const comparing = useStore((s) => s.compare !== null);
  const setCompare = useStore((s) => s.setCompare);
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
  // Comparing, before anything came back in this tab: from the command palette.
  if (comparing && !response) {
    return (
      <section className="response-panel">
        <div className="panel-body">
          <CompareView />
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
  const setCookies = response.headers.filter((h) => h.name === "set-cookie").map((h) => parseSetCookie(h.value));
  // A tab that this response doesn't have shows the body instead.
  const missing =
    (tab === "events" && !response.events) ||
    (tab === "tests" && !tests) ||
    (tab === "console" && !response.console.length && !response.scriptErrors.length) ||
    (tab === "visualize" && !response.visualizer) ||
    (tab === "cookies" && !setCookies.length);
  const shown: Tab = comparing ? "compare" : missing ? "body" : tab;
  const choose = (next: Tab) => {
    if (next === "compare") {
      setCompare({ a: null, b: null });
    } else {
      setCompare(null);
      setTab(next);
    }
  };

  return (
    <section className={`response-panel${sending ? " stale" : ""}`}>
      <Tabs
        active={shown}
        onChange={choose}
        tabs={[
          ...(response.events ? [{ id: "events" as Tab, label: "Events", count: response.events.length }] : []),
          { id: "body", label: "Body" },
          ...(tests ? [{ id: "tests" as Tab, label: tests }] : []),
          ...(response.console.length || response.scriptErrors.length
            ? [{ id: "console" as Tab, label: "Console", count: response.console.length }]
            : []),
          ...(response.visualizer ? [{ id: "visualize" as Tab, label: "Visualize" }] : []),
          { id: "headers", label: "Headers", count: response.headers.length },
          ...(setCookies.length ? [{ id: "cookies" as Tab, label: "Cookies", count: setCookies.length }] : []),
          { id: "timing", label: "Timing" },
          { id: "request", label: "Request" },
          { id: "compare", label: "Compare", count: kept > 1 ? kept : undefined },
        ]}
      >
        <span className="response-actions">
          {!response.bodyBase64 && !response.events && <SaveExample response={response} />}
          <StatusLine response={response} />
        </span>
      </Tabs>
      <Unresolved names={response.unresolved} />
      <div className="panel-body">
        {shown === "events" && response.events && <EventsView events={response.events} live={false} />}
        {shown === "compare" && <CompareView />}
        {shown === "body" && <BodyView response={response} />}
        {shown === "tests" && (
          <TestsView checks={response.checks} tests={response.tests} errors={response.scriptErrors} />
        )}
        {shown === "console" && <ConsoleView lines={response.console} />}
        {shown === "visualize" && response.visualizer && (
          <VisualizerView url={response.visualizer.url} error={response.visualizer.error} />
        )}
        {shown === "headers" && <HeaderTable headers={response.headers} />}
        {shown === "cookies" && <CookiesView cookies={setCookies} url={response.url} />}
        {shown === "timing" && <TimingView response={response} />}
        {shown === "request" && <SentView response={response} />}
      </div>
    </section>
  );
}

interface SetCookie {
  name: string;
  value: string;
  attributes: Record<string, string | true>;
}

/** "a=1; Path=/; HttpOnly" → name, value and attributes (lower-case names). */
function parseSetCookie(header: string): SetCookie {
  const [pair, ...rest] = header.split(";");
  const eq = pair.indexOf("=");
  const attributes: Record<string, string | true> = {};
  for (const part of rest) {
    const [k, ...v] = part.trim().split("=");
    if (k) attributes[k.toLowerCase()] = v.length ? v.join("=") : true;
  }
  return { name: pair.slice(0, eq).trim(), value: pair.slice(eq + 1).trim(), attributes };
}

/** The cookies this response set, and a way to the cookie manager. */
function CookiesView({ cookies, url }: { cookies: SetCookie[]; url: string }) {
  let host: string | null = null;
  try {
    host = new URL(url).hostname;
  } catch {
    host = null;
  }
  return (
    <div className="response-cookies">
      <table className="header-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Value</th>
            <th>Domain</th>
            <th>Path</th>
            <th>Expires</th>
            <th>Flags</th>
          </tr>
        </thead>
        <tbody>
          {cookies.map((c, i) => (
            <tr key={i}>
              <td className="mono">{c.name}</td>
              <td className="mono cookie-cell">{c.value}</td>
              <td className="mono">{String(c.attributes.domain ?? host ?? "")}</td>
              <td className="mono">{String(c.attributes.path ?? "/")}</td>
              <td>
                {c.attributes["max-age"]
                  ? `in ${c.attributes["max-age"]} s`
                  : c.attributes.expires
                    ? String(c.attributes.expires)
                    : "Session"}
              </td>
              <td className="muted small">
                {["secure", "httponly"].filter((f) => c.attributes[f]).map((f) => (f === "httponly" ? "HttpOnly" : "Secure")).join(", ")}
                {c.attributes.samesite ? ` SameSite=${c.attributes.samesite}` : ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <button className="link-button" onClick={() => useStore.setState({ cookies: { domain: host } })}>
        Open the cookie manager
      </button>
    </div>
  );
}

/** Keeps this response with the request, for its docs and mock servers. */
function SaveExample({ response }: { response: Response }) {
  const [saved, setSaved] = useState(false);
  useEffect(() => setSaved(false), [response]);
  const save = () => {
    const { draft, update } = useStore.getState();
    const type = response.headers.find((h) => h.name === "content-type")?.value;
    const example: Example = {
      name: uniqueName(draft.examples, `${response.status} ${response.reason}`.trim()),
      status: response.status,
      headers: type ? [{ name: "Content-Type", value: type }] : [],
      body: response.body,
    };
    update({ examples: [...draft.examples, example] });
    setSaved(true);
  };
  return (
    <button
      className="button ghost small"
      onClick={save}
      disabled={saved}
      title="Keep this response with the request, for API docs and mock servers. Save the request to keep it."
    >
      <BookmarkPlus size={14} />
      {saved ? "Saved as example" : "Save as example"}
    </button>
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

export function contentType(response: Response): string {
  return response.headers.find((h) => h.name === "content-type")?.value.split(";")[0].trim().toLowerCase() ?? "";
}

export function languageFor(type: string): Language {
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
  // A copy changed in the viewer (with find and replace); the response itself is kept as it came.
  const [edited, setEdited] = useState<string | null>(null);
  // With decryption: the body as it came over the wire, instead of decrypted.
  const [received, setReceived] = useState(false);
  const decrypted = response.decryption?.original != null;
  const bodyText = received && decrypted ? response.decryption!.original! : response.body;
  const frame = useRef<HTMLDivElement>(null);
  const language = languageFor(type);
  useEffect(() => setEdited(null), [response, view, filter, received]);

  const json = useMemo(() => {
    if (response.bodyBase64 || language !== "json") return undefined;
    try {
      return JSON.parse(bodyText) as unknown;
    } catch {
      return undefined;
    }
  }, [bodyText, response.bodyBase64, language]);

  // With a filter, Pretty shows only the selected values.
  const [pretty, filterError] = useMemo((): [string, string | null] => {
    if (json === undefined) return [bodyText, null];
    if (!filter.trim()) return [JSON.stringify(json, null, 2), null];
    try {
      const found = select(json, filter);
      return [JSON.stringify(found.length === 1 ? found[0] : found, null, 2), null];
    } catch (e) {
      return [JSON.stringify(json, null, 2), (e as Error).message];
    }
  }, [json, filter, bodyText]);

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
        {response.decryption?.error && (
          <span className="pill decrypt-failed" title={response.decryption.error}>
            Not decrypted
          </span>
        )}
        {decrypted && (
          <span className="segmented decrypt-toggle" title="The response's encrypted fields, decrypted with the request's Crypto settings">
            <button className={received ? "" : "active"} onClick={() => setReceived(false)}>
              Decrypted
            </button>
            <button className={received ? "active" : ""} onClick={() => setReceived(true)}>
              As received
            </button>
          </span>
        )}
        {shown !== "preview" && !response.bodyBase64 && (
          <span className="body-actions">
            {edited !== null && (
              <>
                <span className="pill neutral" title="Changed here only; the response is kept as it came">
                  Edited
                </span>
                <button className="link-button" onClick={() => setEdited(null)}>
                  Reset
                </button>
              </>
            )}
            <button
              className="button ghost small"
              onClick={() => openSearchIn(frame.current)}
              title="Find and replace (Ctrl+F)"
            >
              <Search size={13} />
              Find
            </button>
            <button
              className="button ghost small"
              onClick={() => void navigator.clipboard.writeText(edited ?? (shown === "pretty" ? pretty : bodyText))}
              title="Copy the body"
            >
              <Copy size={13} />
              Copy
            </button>
          </span>
        )}
      </div>
      {shown === "preview" ? (
        <Preview response={response} type={type} />
      ) : response.bodyBase64 ? (
        <p className="hint">Binary data ({formatBytes(response.size)}).</p>
      ) : (
        <div className="editor-frame response-body" ref={frame}>
          <CodeEditor
            value={edited ?? (shown === "pretty" ? pretty : bodyText)}
            onChange={setEdited}
            language={shown === "pretty" ? language : "text"}
            extension={links}
          />
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
            <td className="mono">
              <Linked text={h.value} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Text with its URLs as links that open a new request, such as a Location header. */
function Linked({ text }: { text: string }) {
  const found = findUrls(text);
  if (!found.length) return <>{text}</>;
  const parts: React.ReactNode[] = [];
  let at = 0;
  for (const { url, index } of found) {
    parts.push(text.slice(at, index));
    parts.push(
      <button key={index} className="link-button inline-link" title="Open in a new request" onClick={() => useStore.getState().openUrl(url)}>
        {url}
      </button>,
    );
    at = index + url.length;
  }
  parts.push(text.slice(at));
  return <>{parts}</>;
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
      {response.sentBody ? (
        <>
          <p className="muted sent-body-title">
            Body, as sent ({formatBytes(sent.bodySize)}): after variables, scripts and encryption
          </p>
          <pre className="console-body sent-body">{prettyIfJson(response.sentBody)}</pre>
        </>
      ) : (
        <p className="muted">Body: {formatBytes(sent.bodySize)}</p>
      )}
    </div>
  );
}

function prettyIfJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

export function formatMs(ms: number): string {
  return ms < 1 ? `${ms.toFixed(2)} ms` : ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
