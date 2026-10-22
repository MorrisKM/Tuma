import { useStore } from "../store";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function RequestBar() {
  const method = useStore((s) => s.draft.method);
  const url = useStore((s) => s.draft.url);
  const sending = useStore((s) => s.sending);
  const { update, setUrl, send, cancel } = useStore.getState();

  return (
    <form
      className="request-bar"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      {/* A text input with suggestions, so custom methods such as PURGE work too. */}
      <input
        className={`method method-${method.toLowerCase()}`}
        list="methods"
        value={method}
        spellCheck={false}
        aria-label="Method"
        onChange={(e) => update({ method: e.target.value.toUpperCase() })}
      />
      <datalist id="methods">
        {METHODS.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
      <input
        className="url"
        value={url}
        placeholder="Enter a URL, such as localhost:3000/users"
        spellCheck={false}
        autoFocus
        aria-label="URL"
        onChange={(e) => setUrl(e.target.value)}
      />
      {sending ? (
        <button type="button" className="button" onClick={cancel}>
          Cancel
        </button>
      ) : (
        <button type="submit" className="button primary" disabled={!url.trim()}>
          Send
        </button>
      )}
    </form>
  );
}
