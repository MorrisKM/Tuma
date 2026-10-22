import { isWebSocket } from "../draft";
import { useStore } from "../store";
import { UrlEditor } from "./UrlEditor";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function RequestBar() {
  const method = useStore((s) => s.draft.method);
  const url = useStore((s) => s.draft.url);
  const sending = useStore((s) => s.sending);
  const scope = useStore((s) => s.scope);
  const socket = useStore((s) => s.sockets[s.activeTab]?.status);
  const { update, setUrl, send, cancel, importCurl, connect, disconnect } = useStore.getState();
  const ws = isWebSocket(url);

  return (
    <div className="request-bar">
      {/* A text input with suggestions, so custom methods such as PURGE work too. */}
      <input
        className={`method method-${ws ? "ws" : method.toLowerCase()}`}
        list="methods"
        value={ws ? "WS" : method}
        disabled={ws}
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
        value={url}
        onChange={setUrl}
        onSubmit={() => void send()}
        onPasteCurl={(command) => void importCurl(command)}
        scope={scope}
        placeholder="URL, /path with {{baseUrl}}, or paste a cURL command"
      />
      {ws ? (
        socket === "open" || socket === "connecting" ? (
          <button type="button" className="button" onClick={disconnect}>
            Disconnect
          </button>
        ) : (
          <button type="button" className="button primary" disabled={!url.trim()} onClick={() => void connect()}>
            Connect
          </button>
        )
      ) : sending ? (
        <button type="button" className="button" onClick={cancel}>
          Cancel
        </button>
      ) : (
        <button type="button" className="button primary" disabled={!url.trim()} onClick={() => void send()}>
          Send
        </button>
      )}
    </div>
  );
}
