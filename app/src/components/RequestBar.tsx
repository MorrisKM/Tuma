import { Plug, PlugZap, SendHorizontal, Square, Zap } from "lucide-react";
import { isGrpc, isWebSocket } from "../draft";
import { useStore } from "../store";
import { UrlEditor } from "./UrlEditor";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function RequestBar() {
  const method = useStore((s) => s.draft.method);
  const generation = useStore((s) => s.generation);
  const url = useStore((s) => s.draft.url);
  const sending = useStore((s) => s.sending);
  const scope = useStore((s) => s.scope);
  const socket = useStore((s) => s.sockets[s.activeTab]?.status);
  const call = useStore((s) => s.calls[s.activeTab]?.status);
  const { update, setUrl, send, cancel, importCurl, connect, disconnect, invoke, grpcCancel } = useStore.getState();
  const ws = isWebSocket(url);
  const grpc = isGrpc(url);

  return (
    <div className="request-bar">
      <div className="url-box">
        {/* A text input with suggestions, so custom methods such as PURGE work too. */}
        <input
          className={`method method-${ws ? "ws" : grpc ? "grpc" : method.toLowerCase()}`}
          list="methods"
          value={ws ? "WS" : grpc ? "GRPC" : method}
          disabled={ws || grpc}
          spellCheck={false}
          aria-label="Method"
          onChange={(e) => update({ method: e.target.value.toUpperCase() })}
        />
        <datalist id="methods">
          {METHODS.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
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
