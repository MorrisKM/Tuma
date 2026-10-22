import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { grpcKey, methodInfo, useStore } from "../store";
import type { GrpcEvent, Header } from "../types";
import { CodeEditor } from "./CodeEditor";

/** Picks the method, edits the message, and for client-streaming calls sends more messages. */
export function GrpcMessage() {
  const draft = useStore((s) => s.draft);
  const services = useStore((s) => s.grpcMethods[grpcKey(s.draft)]);
  const call = useStore((s) => s.calls[s.activeTab]);
  const { update, loadGrpcMethods, grpcSend, grpcEnd } = useStore.getState();
  const [loading, setLoading] = useState(false);
  const method = methodInfo(services, draft.grpc.method);
  const streaming = method?.clientStreaming && call?.status === "open";

  const load = async () => {
    setLoading(true);
    await loadGrpcMethods();
    setLoading(false);
  };
  const choose = (path: string) => {
    const info = methodInfo(services, path);
    // A new method starts from its example message unless something is typed already.
    update({ grpc: { ...draft.grpc, method: path }, ...(info && !draft.raw.trim() ? { raw: info.example } : {}) });
  };

  return (
    <div className="grpc-message">
      <div className="toolbar">
        {services ? (
          <select value={draft.grpc.method} onChange={(e) => choose(e.target.value)} aria-label="Method">
            <option value="">Choose a method…</option>
            {services.map((s) => (
              <optgroup key={s.name} label={s.name}>
                {s.methods.map((m) => (
                  <option key={m.path} value={m.path}>
                    {m.name}
                    {m.clientStreaming && m.serverStreaming
                      ? " (bidirectional)"
                      : m.clientStreaming
                        ? " (client stream)"
                        : m.serverStreaming
                          ? " (server stream)"
                          : ""}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        ) : (
          <input
            className="mono"
            value={draft.grpc.method}
            placeholder="package.Service/Method"
            onChange={(e) => update({ grpc: { ...draft.grpc, method: e.target.value } })}
          />
        )}
        <button className="button small" disabled={loading || !draft.url.trim()} onClick={() => void load()}>
          {loading ? "Loading…" : services ? "Reload methods" : "Load methods"}
        </button>
        {method && (
          <>
            <span className="muted small">
              {method.input} → {method.output}
            </span>
            <button className="link-button" onClick={() => update({ raw: method.example })}>
              Example message
            </button>
          </>
        )}
      </div>
      <div className="editor-frame grpc-editor">
        <CodeEditor value={draft.raw} onChange={(raw) => update({ raw })} language="json" placeholder="{ } (JSON)" />
      </div>
      {method?.clientStreaming && (
        <div className="toolbar">
          <span className="muted small">
            {streaming
              ? "The request stream is open: send more messages, then end it."
              : "Invoke sends this message first and keeps the request stream open."}
          </span>
          {streaming && (
            <>
              <button className="button small" onClick={() => void grpcSend(draft.raw)}>
                Send this message
              </button>
              <button className="button small" onClick={grpcEnd}>
                End stream
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** Where the methods come from: the server (reflection) or .proto files. */
export function GrpcProtos() {
  const grpc = useStore((s) => s.draft.grpc);
  const workspace = useStore((s) => s.workspace);
  const { update } = useStore.getState();
  const protos = grpc.protos ?? [];
  const imports = grpc.imports ?? [];
  const relative = (file: string) => {
    const root = workspace?.root;
    return root && file.startsWith(root) ? file.slice(root.length).replace(/^[\\/]+/, "") : file;
  };
  const add = async (directory: boolean) => {
    const chosen = await openDialog(
      directory
        ? { directory: true, title: "A folder imports are found in" }
        : { title: "A .proto file", filters: [{ name: "Protocol Buffers", extensions: ["proto"] }] },
    );
    if (typeof chosen !== "string") return;
    const value = relative(chosen);
    update({
      grpc: directory ? { ...grpc, imports: [...imports, value] } : { ...grpc, protos: [...protos, value] },
    });
  };
  return (
    <div className="settings">
      <p className="hint">
        {protos.length
          ? "Methods come from these files."
          : "Methods come from the server, by gRPC reflection. If it has none, add the service's .proto files."}{" "}
        Paths inside the workspace are kept relative to it, so they work in every clone.
      </p>
      <h3>.proto files</h3>
      {protos.map((p) => (
        <div key={p} className="cert-row">
          <span className="mono small">{p}</span>
          <button className="link-button danger" onClick={() => update({ grpc: { ...grpc, protos: protos.filter((x) => x !== p) } })}>
            Remove
          </button>
        </div>
      ))}
      <button className="button small" onClick={() => void add(false)}>
        Add a .proto file
      </button>
      <h3>Import folders</h3>
      {imports.map((p) => (
        <div key={p} className="cert-row">
          <span className="mono small">{p}</span>
          <button className="link-button danger" onClick={() => update({ grpc: { ...grpc, imports: imports.filter((x) => x !== p) } })}>
            Remove
          </button>
        </div>
      ))}
      <button className="button small" onClick={() => void add(true)}>
        Add a folder
      </button>
    </div>
  );
}

/** The call: status, messages each way, and metadata. */
export function GrpcLog() {
  const call = useStore((s) => s.calls[s.activeTab]);
  const [open, setOpen] = useState<number | null>(null);
  const [showMeta, setShowMeta] = useState(false);
  if (!call) return <p className="empty muted">Invoke the method to see its answer here.</p>;
  const done = call.events.find((e): e is Extract<GrpcEvent, { kind: "done" }> => e.kind === "done");
  const headers = call.events.find((e): e is Extract<GrpcEvent, { kind: "headers" }> => e.kind === "headers");
  const messages = call.events.filter((e): e is Extract<GrpcEvent, { kind: "message" }> => e.kind === "message");
  const received = messages.filter((m) => m.incoming);
  const only = received.length === 1 && messages.length <= 2;

  return (
    <div className="ws-log">
      <div className="toolbar spaced">
        <span className="small">
          {done ? (
            <span className={done.code === 0 ? "ok" : "fail"}>
              {done.status}
              {done.message && `: ${done.message}`}
            </span>
          ) : (
            <span>
              <span className="spinner" /> {call.status === "starting" ? "Connecting…" : "In progress"}
            </span>
          )}
          <span className="muted">
            {done && ` · ${done.elapsedMs} ms`} · {received.length} received
          </span>
        </span>
        {(headers || done?.metadata.length) && (
          <button className="link-button" onClick={() => setShowMeta(!showMeta)}>
            {showMeta ? "Hide metadata" : "Metadata"}
          </button>
        )}
      </div>
      {call.error && <p className="notice error">{call.error}</p>}
      {showMeta && (
        <div className="small">
          {headers && <Meta title="Headers" list={headers.metadata} />}
          {done && done.metadata.length > 0 && <Meta title="Trailers" list={done.metadata} />}
        </div>
      )}
      {only ? (
        <div className="editor-frame grpc-answer">
          <CodeEditor value={received[0].json} language="json" readOnly />
        </div>
      ) : (
        <ul className="event-list">
          {messages.map((m, i) => (
            <li key={i} onClick={() => setOpen(open === i ? null : i)}>
              <div className="event-head">
                <span className={m.incoming ? "ws-in" : "ws-out"}>{m.incoming ? "↓" : "↑"}</span>
                <span className="muted small">{new Date(m.at).toLocaleTimeString()}</span>
                <span className="event-preview mono small">{m.json.replace(/\s+/g, " ")}</span>
              </div>
              {open === i && <pre className="mono small event-data">{m.json}</pre>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Meta({ title, list }: { title: string; list: Header[] }) {
  return (
    <div>
      <h4 className="muted">{title}</h4>
      {list.map((h, i) => (
        <div key={i} className="mono">
          {h.name}: {h.value}
        </div>
      ))}
    </div>
  );
}
