import { useMemo, useState } from "react";
import type { SseEvent } from "../types";

/** Server-sent events, newest last; JSON data is shown formatted. */
export function EventsView({ events, live }: { events: SseEvent[]; live: boolean }) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<number | null>(null);
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return events
      .map((e, i) => ({ e, i }))
      .filter(({ e }) => !q || e.event.toLowerCase().includes(q) || e.data.toLowerCase().includes(q));
  }, [events, filter]);
  const names = [...new Set(events.map((e) => e.event))];

  return (
    <div className="events-view">
      <div className="toolbar spaced">
        <span className="muted small">
          {live && <span className="spinner" />} {events.length} event{events.length === 1 ? "" : "s"}
          {names.length > 0 && ` · ${names.slice(0, 6).join(", ")}${names.length > 6 ? "…" : ""}`}
          {live && " · Cancel stops the stream"}
        </span>
        <input
          className="json-filter"
          value={filter}
          placeholder="Filter events"
          spellCheck={false}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>
      {events.length === 0 && <p className="hint">{live ? "Connected; waiting for events…" : "No events."}</p>}
      <ul className="event-list">
        {shown.map(({ e, i }) => (
          <li key={i} onClick={() => setOpen(open === i ? null : i)}>
            <div className="event-head">
              <span className="event-name">{e.event}</span>
              {e.id && <span className="muted small">#{e.id}</span>}
              {e.at && <span className="muted small">{new Date(e.at).toLocaleTimeString()}</span>}
              <span className="event-preview mono small">{e.data.split("\n")[0]}</span>
            </div>
            {open === i && <pre className="mono small event-data">{pretty(e.data)}</pre>}
          </li>
        ))}
      </ul>
    </div>
  );
}

function pretty(data: string): string {
  try {
    return JSON.stringify(JSON.parse(data), null, 2);
  } catch {
    return data;
  }
}
