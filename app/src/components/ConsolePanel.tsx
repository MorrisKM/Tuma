import { ChevronRight, Copy, Trash2, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { clearConsole, type ConsoleEntry, failedEntry, markSeen, useConsole } from "../console";
import { layout, useLayout } from "../layout";
import type { Header } from "../types";
import { formatBytes, formatMs } from "./ResponsePanel";
import { Splitter } from "./Splitter";
import { MethodTag } from "./ui";

type Filter = "all" | "errors" | "logs";

/** Everything sent this session, newest at the bottom, along the bottom of the window. */
export function ConsolePanel() {
  const entries = useConsole((s) => s.entries);
  const height = useLayout((s) => s.consoleHeight);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [open, setOpen] = useState<Set<number>>(new Set());
  const panel = useRef<HTMLElement>(null);
  const list = useRef<HTMLDivElement>(null);
  // Follows new entries, unless scrolled up to read older ones.
  const pinned = useRef(true);

  useEffect(markSeen, [entries]);
  useLayoutEffect(() => {
    if (pinned.current && list.current) list.current.scrollTop = list.current.scrollHeight;
  }, [entries, filter, query]);

  const q = query.trim().toLowerCase();
  const shown = entries.filter(
    (e) =>
      (filter === "all" ||
        (filter === "errors" && failedEntry(e)) ||
        (filter === "logs" && (e.logs.length > 0 || e.scriptErrors.length > 0))) &&
      (!q || `${e.method} ${e.url} ${e.status ?? ""} ${e.source ?? ""} ${e.error ?? ""}`.toLowerCase().includes(q)),
  );
  const errors = entries.filter(failedEntry).length;
  const toggle = (id: number) =>
    setOpen((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <>
      <Splitter
        axis="y"
        label="Resize the console"
        onDrag={(y) => layout.setConsoleHeight((panel.current?.getBoundingClientRect().bottom ?? 0) - y)}
        onStep={(dy) => layout.setConsoleHeight(useLayout.getState().consoleHeight - dy)}
        onReset={layout.resetConsole}
      />
      <section className="console-panel" ref={panel} style={{ height }} aria-label="Console">
        <div className="console-head">
          <span className="console-title">Console</span>
          <div className="segmented">
            {(
              [
                ["all", `All ${entries.length}`],
                ["errors", `Errors ${errors}`],
                ["logs", "Script output"],
              ] as [Filter, string][]
            ).map(([id, label]) => (
              <button key={id} className={filter === id ? "active" : ""} onClick={() => setFilter(id)}>
                {label}
              </button>
            ))}
          </div>
          <input
            className="console-filter"
            type="search"
            value={query}
            placeholder="Filter by URL, status or name"
            aria-label="Filter the console"
            spellCheck={false}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && setQuery("")}
          />
          <span className="console-actions">
            <button className="icon-button" onClick={clearConsole} title="Clear the console" aria-label="Clear">
              <Trash2 size={15} />
            </button>
            <button
              className="icon-button"
              onClick={layout.toggleConsole}
              title="Close the console (Ctrl+`)"
              aria-label="Close the console"
            >
              <X size={16} />
            </button>
          </span>
        </div>
        <div
          className="console-list"
          ref={list}
          onScroll={(e) => {
            const el = e.currentTarget;
            pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          }}
        >
          {shown.length === 0 && (
            <p className="console-empty muted">
              {entries.length === 0
                ? "Requests you send, collection runs, WebSocket connections and gRPC calls show up here, with what was sent, what came back and what scripts printed."
                : "Nothing matches."}
            </p>
          )}
          {shown.map((e) => (
            <Entry key={e.id} entry={e} open={open.has(e.id)} onToggle={() => toggle(e.id)} />
          ))}
        </div>
      </section>
    </>
  );
}

function Entry({ entry: e, open, onToggle }: { entry: ConsoleEntry; open: boolean; onToggle: () => void }) {
  const failed = failedEntry(e);
  return (
    <div className={`console-entry${open ? " open" : ""}${failed ? " failed" : ""}`}>
      <button className="console-row" onClick={onToggle} aria-expanded={open}>
        <ChevronRight size={13} className={`chevron${open ? " open" : ""}`} />
        <span className="console-time">{clock(e.at)}</span>
        <MethodTag method={e.method} />
        <span className="console-url">{e.url}</span>
        {e.source && <span className="console-source">{e.source}</span>}
        {(e.logs.length > 0 || e.scriptErrors.length > 0) && (
          <span className="source-tag" title="Scripts printed something">
            {e.logs.length + e.scriptErrors.length} log{e.logs.length + e.scriptErrors.length === 1 ? "" : "s"}
          </span>
        )}
        <span className={`console-status ${tone(e)}`}>{statusText(e)}</span>
        <span className="console-num">{e.timeMs !== undefined ? formatMs(e.timeMs) : ""}</span>
        <span className="console-num">{e.size !== undefined ? formatBytes(e.size) : ""}</span>
      </button>
      {open && <Details entry={e} />}
    </div>
  );
}

function Details({ entry: e }: { entry: ConsoleEntry }) {
  const logged = e.logs.length + e.scriptErrors.length > 0;
  return (
    <div className="console-details">
      {e.error && <p className="console-error mono">{e.error}</p>}
      <Section title="Network">
        <Network entry={e} />
      </Section>
      {e.request && (
        <Section title="Request headers">
          {e.request.headers.length > 0 ? (
            <Headers headers={e.request.headers} />
          ) : (
            <p className="console-none">{e.error ? "Not known: the request was not sent." : "None"}</p>
          )}
        </Section>
      )}
      {e.request && (e.request.body || e.request.bodySize) ? (
        <Section title="Request body" copy={e.request.body}>
          {e.request.body ? (
            <pre className="console-body">{pretty(e.request.body)}</pre>
          ) : (
            <p className="console-none">{formatBytes(e.request.bodySize ?? 0)}</p>
          )}
        </Section>
      ) : null}
      {(e.response?.redirects?.length ?? 0) > 0 && (
        <Section title="Redirects">
          {e.response!.redirects!.map((r, i) => (
            <div key={i} className="mono small console-indent">
              {r.status} → {r.url}
            </div>
          ))}
        </Section>
      )}
      {e.response && e.response.headers.length > 0 && (
        <Section title={e.kind === "grpc" ? "Response metadata" : "Response headers"}>
          <Headers headers={e.response.headers} />
        </Section>
      )}
      {e.response?.body && (
        <Section title={e.kind === "grpc" ? "Messages received" : "Response body"} copy={e.response.body}>
          <pre className="console-body">{pretty(e.response.body)}</pre>
        </Section>
      )}
      {logged && (
        <Section title="Script output">
          <div className="console-indent">
            {e.scriptErrors.map((s, i) => (
              <pre key={`e${i}`} className="console-line error">
                {s.script}: {s.message}
              </pre>
            ))}
            {e.logs.map((l, i) => (
              <pre key={i} className={`console-line ${l.level}`}>
                {l.text}
              </pre>
            ))}
          </div>
        </Section>
      )}
    </div>
  );
}

/** Where the request went and how long each part took. */
function Network({ entry: e }: { entry: ConsoleEntry }) {
  const n = e.network;
  const t = n?.timing;
  const rows: [string, string | undefined][] = [
    ["Request", `${e.method} ${e.url}`],
    ["Status", e.status !== undefined ? `${e.status}${e.reason ? ` ${e.reason}` : ""}` : e.reason],
    ["HTTP version", n?.httpVersion],
    ["Remote address", n?.remoteAddress ?? undefined],
    ["Sent from", e.source],
    ["Started", new Date(e.at).toLocaleString()],
    [
      "Received",
      e.size !== undefined
        ? `${formatBytes(e.size)}${n?.encodedSize !== undefined && n.encodedSize !== e.size ? ` (${formatBytes(n.encodedSize)} on the wire)` : ""}`
        : undefined,
    ],
    ["Sent", e.request?.bodySize ? `${formatBytes(e.request.bodySize)} of body` : undefined],
  ];
  const phases: [string, number | null | undefined][] = t
    ? [
        ["DNS lookup", t.dns],
        ["TCP connection", t.connect],
        ["TLS handshake", t.tls],
        ["Waiting (time to first byte)", t.wait],
        ["Download", t.download],
      ]
    : [];
  const total = t?.total ?? e.timeMs;
  let offset = 0;
  return (
    <div className="console-network console-indent">
      <dl className="console-facts">
        {rows
          .filter(([, v]) => v)
          .map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd className="mono">{v}</dd>
            </div>
          ))}
      </dl>
      {phases.length > 0 && total ? (
        <div className="console-timing">
          {phases
            .filter(([, ms]) => ms !== null && ms !== undefined)
            .map(([name, ms]) => {
              const left = (offset / total) * 100;
              offset += ms!;
              return (
                <div key={name} className="console-phase">
                  <span>{name}</span>
                  <span className="timing-track">
                    <span
                      className="timing-bar"
                      style={{ left: `${left}%`, width: `${Math.max(0.5, (ms! / total) * 100)}%` }}
                    />
                  </span>
                  <span className="console-num">{formatMs(ms!)}</span>
                </div>
              );
            })}
          <div className="console-phase total">
            <span>Total</span>
            <span />
            <span className="console-num">{formatMs(total)}</span>
          </div>
        </div>
      ) : (
        total !== undefined && <p className="console-none">Took {formatMs(total)}</p>
      )}
    </div>
  );
}

function Section({ title, copy, children }: { title: string; copy?: string; children: React.ReactNode }) {
  const [shown, setShown] = useState(true);
  return (
    <div className="console-section">
      <div className="console-section-head">
        <button className="link-button muted-link" onClick={() => setShown(!shown)}>
          <ChevronRight size={12} className={`chevron${shown ? " open" : ""}`} />
          {title}
        </button>
        {copy && (
          <button
            className="icon-button"
            title="Copy"
            aria-label={`Copy the ${title.toLowerCase()}`}
            onClick={() => void navigator.clipboard.writeText(copy)}
          >
            <Copy size={13} />
          </button>
        )}
      </div>
      {shown && children}
    </div>
  );
}

function Headers({ headers }: { headers: Header[] }) {
  return (
    <table className="header-table console-headers">
      <tbody>
        {headers.map((h, i) => (
          <tr key={i}>
            <th className="mono">{h.name}</th>
            <td className="mono">{h.value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const clock = (at: number) => {
  const d = new Date(at);
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
};

function statusText(e: ConsoleEntry): string {
  if (e.error) return "Error";
  if (e.kind === "ws") return e.status === 101 ? "Open" : "Closed";
  if (e.kind === "grpc") return e.reason?.split(":")[0] ?? String(e.status ?? "");
  return e.status !== undefined ? `${e.status}${e.reason ? ` ${e.reason}` : ""}` : "";
}

function tone(e: ConsoleEntry): string {
  if (failedEntry(e)) return "fail";
  if (e.kind === "ws" || e.kind === "grpc") return e.status === 101 || e.status === 0 ? "ok" : "";
  return e.status !== undefined && e.status >= 300 && e.status < 400 ? "redirect" : "ok";
}

/** JSON indented, anything else as it came. */
function pretty(body: string): string {
  const t = body.trimStart();
  if (!t.startsWith("{") && !t.startsWith("[")) return body;
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}
