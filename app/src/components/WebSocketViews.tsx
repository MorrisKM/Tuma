import { useMemo, useState } from "react";
import { useStore } from "../store";
import type { SavedMessage, WsEvent } from "../types";
import { CodeEditor } from "./CodeEditor";

/** Writes and sends messages; keeps them as saved messages of the request. */
export function MessageComposer() {
  const messages = useStore((s) => s.draft.messages);
  const status = useStore((s) => s.sockets[s.activeTab]?.status);
  const { sendMessage, update } = useStore.getState();
  const [text, setText] = useState("");
  const [name, setName] = useState("");
  const open = status === "open";
  const json = useMemo(() => {
    try {
      JSON.parse(text);
      return true;
    } catch {
      return false;
    }
  }, [text]);

  const save = () => {
    const label = name.trim() || text.trim().slice(0, 30) || "Message";
    const rest = messages.filter((m) => m.name !== label);
    update({ messages: [...rest, { name: label, content: text }] });
    setName("");
  };
  const remove = (m: SavedMessage) => update({ messages: messages.filter((x) => x !== m) });

  return (
    <div className="ws-composer">
      <div className="ws-compose">
        <div
          className="editor-frame"
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && open && text) {
              e.preventDefault();
              e.stopPropagation();
              void sendMessage(text);
            }
          }}
        >
          <CodeEditor value={text} onChange={setText} language={json ? "json" : "text"} placeholder="A message; {{variables}} work" />
        </div>
        <div className="toolbar spaced">
          <span className="muted small">{open ? "Ctrl+Enter sends" : "Connect to send messages"}</span>
          <span>
            <input className="small" value={name} placeholder="Name to save as" onChange={(e) => setName(e.target.value)} />{" "}
            <button className="button small" disabled={!text} onClick={save}>
              Save
            </button>{" "}
            <button className="button primary small" disabled={!open || !text} onClick={() => void sendMessage(text)}>
              Send
            </button>
          </span>
        </div>
      </div>
      <aside className="ws-saved">
        <h4>Saved messages</h4>
        {messages.length === 0 && <p className="muted small">Messages you save stay with the request.</p>}
        {messages.map((m) => (
          <div key={m.name} className="ws-saved-item">
            <button className="link-button" title={m.content} onClick={() => setText(m.content)}>
              {m.name}
            </button>
            <span>
              <button className="link-button" disabled={!open} onClick={() => void sendMessage(m.content)}>
                Send
              </button>{" "}
              <button className="link-button danger" onClick={() => remove(m)}>
                ×
              </button>
            </span>
          </div>
        ))}
      </aside>
    </div>
  );
}

/** The connection's messages and events, newest last. */
export function MessageLog() {
  const socket = useStore((s) => s.sockets[s.activeTab]);
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<number | null>(null);
  if (!socket) return <p className="empty muted">Connect to see messages here.</p>;
  const events = socket.events;
  const q = filter.trim().toLowerCase();
  const sent = events.filter((e) => e.kind === "message" && !e.incoming).length;
  const received = events.filter((e) => e.kind === "message" && e.incoming).length;

  return (
    <div className="ws-log">
      <div className="toolbar spaced">
        <span className="small">
          <span className={`ws-status ${socket.status}`}>
            {socket.status === "open" ? "Connected" : socket.status === "connecting" ? "Connecting…" : "Disconnected"}
          </span>
          <span className="muted">
            {" "}
            · {sent} sent · {received} received
          </span>
        </span>
        <input className="json-filter" value={filter} placeholder="Filter messages" onChange={(e) => setFilter(e.target.value)} />
      </div>
      {socket.error && <p className="notice error">{socket.error}</p>}
      <ul className="event-list">
        {events.map((e, i) =>
          q && !(e.kind === "message" && (e.text ?? "").toLowerCase().includes(q)) ? null : (
            <li key={i} onClick={() => setOpen(open === i ? null : i)} className={`ws-${e.kind}`}>
              <Row event={e} expanded={open === i} />
            </li>
          ),
        )}
      </ul>
    </div>
  );
}

function Row({ event, expanded }: { event: WsEvent; expanded: boolean }) {
  switch (event.kind) {
    case "open":
      return (
        <div className="event-head">
          <span className="ok">Connected</span>
          <span className="muted small">
            {event.url} · {event.status}
            {event.protocol ? ` · ${event.protocol}` : ""}
          </span>
        </div>
      );
    case "closed":
      return (
        <div className="event-head">
          <span className="muted">Closed{event.byServer ? " by the server" : ""}</span>
          <span className="muted small">
            {event.code ?? ""} {event.reason}
          </span>
        </div>
      );
    case "error":
      return <div className="fail small">{event.message}</div>;
    case "message": {
      const body = event.text ?? `(${event.size} bytes of binary data)`;
      return (
        <>
          <div className="event-head">
            <span className={event.incoming ? "ws-in" : "ws-out"}>{event.incoming ? "↓" : "↑"}</span>
            <span className="muted small">{new Date(event.at).toLocaleTimeString()}</span>
            <span className="event-preview mono small">{body.split("\n")[0]}</span>
          </div>
          {expanded && <pre className="mono small event-data">{pretty(body)}</pre>}
        </>
      );
    }
  }
}

function pretty(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}
