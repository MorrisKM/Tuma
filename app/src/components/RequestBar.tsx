import { Plug, PlugZap, SendHorizontal, Square, Zap } from "lucide-react";
import { useState } from "react";
import { type Protocol, protocolOf, withProtocol } from "../draft";
import { useStore } from "../store";
import { focusUrl, UrlEditor } from "./UrlEditor";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const CUSTOM = "__custom";

const PROTOCOLS: { id: Protocol; label: string }[] = [
  { id: "http", label: "HTTP" },
  { id: "ws", label: "WebSocket" },
  { id: "grpc", label: "gRPC" },
];

export function RequestBar() {
  const method = useStore((s) => s.draft.method);
  const generation = useStore((s) => s.generation);
  const url = useStore((s) => s.draft.url);
  const sending = useStore((s) => s.sending);
  const scope = useStore((s) => s.scope);
  const socket = useStore((s) => s.sockets[s.activeTab]?.status);
  const call = useStore((s) => s.calls[s.activeTab]?.status);
  const { update, setUrl, send, cancel, importCurl, connect, disconnect, invoke, grpcCancel } = useStore.getState();
  const protocol = protocolOf(url);
  const ws = protocol === "ws";
  const grpc = protocol === "grpc";
  const choose = (next: Protocol) => {
    if (next === protocol) return;
    setUrl(withProtocol(url, next));
    setTimeout(focusUrl);
  };

  return (
    <div className="request-bar">
      <div className="url-box">
        {/* The protocol comes from the URL's scheme; choosing one here rewrites the scheme. */}
        <select
          className={`protocol protocol-${protocol}`}
          value={protocol}
          aria-label="Protocol"
          title="HTTP, WebSocket or gRPC: sets the URL's scheme"
          onChange={(e) => choose(e.target.value as Protocol)}
        >
          {PROTOCOLS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        {protocol === "http" && <MethodPicker method={method} onChange={(m) => update({ method: m })} />}
        <UrlEditor
          doc={generation}
          value={url}
          onChange={setUrl}
          onSubmit={() => void send()}
          onPasteCurl={(command) => void importCurl(command)}
          scope={scope}
          placeholder="URL, /path with {{baseUrl}}, or paste a cURL command"
        />
      </div>
      {grpc ? (
        call === "starting" || call === "open" ? (
          <button type="button" className="button send" onClick={grpcCancel}>
            <Square size={13} />
            Cancel
          </button>
        ) : (
          <button type="button" className="button primary send" disabled={!url.trim()} onClick={() => void invoke()}>
            <Zap size={15} />
            Invoke
          </button>
        )
      ) : ws ? (
        socket === "open" || socket === "connecting" ? (
          <button type="button" className="button send" onClick={disconnect}>
            <PlugZap size={15} />
            Disconnect
          </button>
        ) : (
          <button type="button" className="button primary send" disabled={!url.trim()} onClick={() => void connect()}>
            <Plug size={15} />
            Connect
          </button>
        )
      ) : sending ? (
        <button type="button" className="button send" onClick={cancel}>
          <span className="spinner" />
          Cancel
        </button>
      ) : (
        <button
          type="button"
          className="button primary send"
          disabled={!url.trim()}
          onClick={() => void send()}
          title="Send (Ctrl+Enter)"
        >
          <SendHorizontal size={15} />
          Send
        </button>
      )}
    </div>
  );
}

/** Every method in a list; "Custom…" turns it into a text box for others, such as PURGE. */
function MethodPicker({ method, onChange }: { method: string; onChange: (method: string) => void }) {
  // While typing a custom method: the one before, to go back to if the box is left empty.
  const [before, setBefore] = useState<string | null>(null);
  const className = `method method-${method.toLowerCase()}`;
  if (before !== null) {
    return (
      <input
        className={className}
        value={method}
        autoFocus
        spellCheck={false}
        aria-label="Method"
        placeholder="METHOD"
        onChange={(e) => onChange(e.target.value.toUpperCase())}
        onBlur={() => {
          if (!method.trim()) onChange(before);
          setBefore(null);
        }}
        onKeyDown={(e) => (e.key === "Enter" || e.key === "Escape") && e.currentTarget.blur()}
      />
    );
  }
  const custom = !METHODS.includes(method);
  return (
    <select
      className={className}
      value={method}
      aria-label="Method"
      onChange={(e) => {
        if (e.target.value !== CUSTOM) return onChange(e.target.value);
        setBefore(method);
        onChange("");
      }}
    >
      {METHODS.map((m) => (
        <option key={m} value={m}>
          {m}
        </option>
      ))}
      {custom && <option value={method}>{method}</option>}
      <option value={CUSTOM}>Custom…</option>
    </select>
  );
}
