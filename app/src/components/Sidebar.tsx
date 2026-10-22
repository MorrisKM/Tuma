import { useEffect } from "react";
import { useStore } from "../store";
import type { HistoryEntry } from "../types";

export function Sidebar() {
  const history = useStore((s) => s.history);
  const { loadHistory, clearHistory, open } = useStore.getState();

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  return (
    <aside className="sidebar">
      <div className="brand">Tuma</div>
      <div className="sidebar-heading">
        <span>History</span>
        {history.length > 0 && (
          <button className="link-button" onClick={() => void clearHistory()}>
            Clear
          </button>
        )}
      </div>
      {history.length === 0 ? (
        <p className="muted small">Requests you send appear here.</p>
      ) : (
        <ul className="history">
          {groupByDay(history).map(([day, entries]) => (
            <li key={day}>
              <div className="history-day">{day}</div>
              <ul>
                {entries.map((entry) => (
                  <li key={entry.id}>
                    <button className="history-item" onClick={() => open(entry)} title={entry.request.url}>
                      <span className={`method-tag method-${entry.request.method.toLowerCase()}`}>
                        {entry.request.method}
                      </span>
                      <span className="history-url">{displayUrl(entry.request.url)}</span>
                      <span className={`history-status ${statusTone(entry)}`}>{entry.status ?? "ERR"}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}

function groupByDay(entries: HistoryEntry[]): [string, HistoryEntry[]][] {
  const groups = new Map<string, HistoryEntry[]>();
  const today = new Date().toDateString();
  for (const entry of entries) {
    const date = new Date(entry.sentAt);
    const day = date.toDateString() === today ? "Today" : date.toLocaleDateString();
    groups.set(day, [...(groups.get(day) ?? []), entry]);
  }
  return [...groups];
}

const displayUrl = (url: string) => url.replace(/^https?:\/\//, "");

function statusTone(entry: HistoryEntry): string {
  if (entry.status === null) return "fail";
  return entry.status < 300 ? "ok" : entry.status < 400 ? "redirect" : "fail";
}
