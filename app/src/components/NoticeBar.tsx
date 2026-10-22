import { useStore } from "../store";

/** Results and errors of workspace actions, until dismissed. */
export function NoticeBar() {
  const notice = useStore((s) => s.notice);
  if (!notice) return null;
  return (
    <div className={`notice ${notice.tone}`} role={notice.tone === "error" ? "alert" : "status"}>
      <div>
        <p>{notice.text}</p>
        {notice.details && notice.details.length > 0 && (
          <ul>
            {notice.details.map((d, i) => (
              <li key={i}>{d}</li>
            ))}
          </ul>
        )}
      </div>
      <button className="icon-button" onClick={() => useStore.getState().notify(null)} aria-label="Dismiss">
        ×
      </button>
    </div>
  );
}
