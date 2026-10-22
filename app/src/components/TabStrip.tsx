import { Plus, X } from "lucide-react";
import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { isGrpc, isWebSocket } from "../draft";
import { tabDirty, useStore } from "../store";
import { Menu, MethodTag } from "./ui";
import { EnvironmentPicker } from "./TopBar";

/** Open requests. A preview tab (italic) is replaced by the next request opened from the tree until it is edited. */
export function TabStrip() {
  // Re-render on any change to the active tab or the tab list.
  useStore(useShallow((s) => [s.tabs, s.activeTab, s.draft, s.saved, s.preview, s.sending, s.view.kind]));
  const { allTabs, activeTab, switchTab, closeTab, newTab, pin } = useStore.getState();
  const tabs = allTabs();
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  // One at a time, so each unsaved one asks in turn.
  const closeAll = async (ids: string[]) => {
    for (const id of ids) await closeTab(id);
  };

  return (
    <div className="tab-bar">
      <div className="tab-strip" role="tablist" aria-label="Open requests">
        {tabs.map((tab) => {
          // Another view (settings, a folder) has the workbench: no tab is shown as active.
          const active = tab.id === activeTab && useStore.getState().view.kind === "request";
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
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ id: tab.id, x: e.clientX, y: e.clientY });
              }}
              title={tab.draft.path ?? "Not saved"}
            >
              <MethodTag method={tabMethod(tab.draft.method, tab.draft.url)} />
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
                <span className="dot" />
                <X size={13} className="x" />
              </button>
            </div>
          );
        })}
        <button className="tab-new" onClick={newTab} aria-label="New request (Ctrl+T)" title="New request (Ctrl+T)">
          <Plus size={16} />
        </button>
      </div>
      {menu && (
        <Menu at={menu} onClose={() => setMenu(null)}>
          <button onClick={() => void closeTab(menu.id)}>Close</button>
          <button onClick={() => void closeAll(tabs.filter((t) => t.id !== menu.id).map((t) => t.id))}>
            Close others
          </button>
          <button
            onClick={() => void closeAll(tabs.slice(tabs.findIndex((t) => t.id === menu.id) + 1).map((t) => t.id))}
          >
            Close to the right
          </button>
          <hr />
          <button onClick={() => void closeAll(tabs.map((t) => t.id))}>Close all</button>
        </Menu>
      )}
      <EnvironmentPicker />
    </div>
  );
}

/** WebSocket and gRPC requests are labelled by protocol, as in the sidebar. */
const tabMethod = (method: string, url: string) => (isWebSocket(url) ? "WS" : isGrpc(url) ? "GRPC" : method);

/** A saved request's name; for an unsaved one, its URL without the scheme, which says more than "Untitled". */
function tabLabel(path: string | null, name: string, url: string): string {
  if (path || !url.trim()) return name;
  return (
    url
      .trim()
      .replace(/^[a-z]+:\/\//i, "")
      .replace(/^\{\{baseUrl\}\}/, "") || name
  );
}
