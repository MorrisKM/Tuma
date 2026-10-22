import { useShallow } from "zustand/react/shallow";
import { tabDirty, useStore } from "../store";

/** Open requests. A preview tab (italic) is replaced by the next request opened from the tree until it is edited. */
export function TabStrip() {
  // Re-render on any change to the active tab or the tab list.
  useStore(useShallow((s) => [s.tabs, s.activeTab, s.draft, s.saved, s.preview, s.sending]));
  const { allTabs, activeTab, switchTab, closeTab, newTab, pin } = useStore.getState();
  const tabs = allTabs();

  return (
    <div className="tab-strip" role="tablist" aria-label="Open requests">
      {tabs.map((tab) => {
        const active = tab.id === activeTab;
        const dirty = tabDirty(tab);
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
            className={`request-tab${active ? " active" : ""}${tab.preview ? " preview" : ""}`}
            onClick={() => switchTab(tab.id)}
            onDoubleClick={() => active && pin()}
            onAuxClick={(e) => e.button === 1 && void closeTab(tab.id)}
            title={tab.draft.path ?? "Not saved"}
          >
            <span className={`method-tag method-${tab.draft.method.toLowerCase()}`}>{tab.draft.method}</span>
            <span className="request-tab-name">{tabLabel(tab.draft.path, tab.draft.name, tab.draft.url)}</span>
            {tab.sending && <span className="spinner" aria-label="Sending" />}
            <button
              className={`tab-close${dirty ? " dirty" : ""}`}
              aria-label={dirty ? "Unsaved changes; close" : "Close"}
              onClick={(e) => {
                e.stopPropagation();
                void closeTab(tab.id);
              }}
            >
              <span className="dot">●</span>
              <span className="x">×</span>
            </button>
          </div>
        );
      })}
      <button className="tab-new" onClick={newTab} aria-label="New request (Ctrl+T)" title="New request (Ctrl+T)">
        +
      </button>
    </div>
  );
}

/** A saved request's name; for an unsaved one, its URL without the scheme, which says more than "Untitled". */
function tabLabel(path: string | null, name: string, url: string): string {
  if (path || !url.trim()) return name;
  return url.trim().replace(/^[a-z]+:\/\//i, "").replace(/^\{\{baseUrl\}\}/, "") || name;
}
