import { Columns2, PanelLeft, Rows2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { appVersion } from "./api";
import { AgentsView } from "./components/AgentsView";
import { CommandPalette } from "./components/CommandPalette";
import { RunnerView } from "./components/RunnerView";
import { SettingsView } from "./components/SettingsView";
import { FolderEditor } from "./components/FolderEditor";
import { NoticeBar } from "./components/NoticeBar";
import { RequestBar } from "./components/RequestBar";
import { RequestPanel } from "./components/RequestPanel";
import { ResponsePanel } from "./components/ResponsePanel";
import { SaveDialog } from "./components/SaveDialog";
import { Sidebar } from "./components/Sidebar";
import { SnippetDialog } from "./components/SnippetDialog";
import { Splitter } from "./components/Splitter";
import { TabStrip } from "./components/TabStrip";
import { TopBar } from "./components/TopBar";
import { focusUrl } from "./components/UrlEditor";
import { VariablesEditor } from "./components/VariablesEditor";
import { layout, useLayout } from "./layout";
import { useStore } from "./store";
import type { GrpcEvent, SseEvent, WsEvent } from "./types";
import { listen } from "@tauri-apps/api/event";
import { applyTheme, loadTheme } from "./theme";
import { checkAtStart } from "./updates";
import "./App.css";

applyTheme(loadTheme());

/** Keyboard shortcuts. Ctrl is Cmd on macOS. */
function onKey(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey)) return;
  const s = useStore.getState();
  const key = e.key.toLowerCase();
  const handled = (() => {
    if (key === "enter") void s.send();
    else if (key === "k") useStore.setState({ paletteOpen: !s.paletteOpen });
    else if (key === "s" && s.view.kind === "request") void s.save();
    else if (key === "t" || key === "n") s.newTab();
    else if (key === "w") void s.closeTab(s.activeTab);
    else if (key === "tab") s.cycleTab(e.shiftKey ? -1 : 1);
    else if (key === "b") layout.toggleSidebar();
    else if (key === "l") {
      useStore.setState({ view: { kind: "request" } });
      setTimeout(focusUrl);
    } else return false;
    return true;
  })();
  if (handled) e.preventDefault();
}

function App() {
  const [version, setVersion] = useState<string>();
  const view = useStore((s) => s.view);
  const savingAs = useStore((s) => s.savingAs);
  const paletteOpen = useStore((s) => s.paletteOpen);
  const snippetOpen = useStore((s) => s.snippetOpen);
  const workspace = useStore((s) => s.workspace);
  const environment = useStore((s) => s.environment);

  useEffect(() => {
    appVersion().then(setVersion);
    void useStore.getState().init();
    // After the workspace is up, so a slow network doesn't hold anything back.
    const updates = setTimeout(() => void checkAtStart(), 5000);
    // Server-sent events of requests in flight.
    const stop = listen<{ id: string; event: SseEvent | null }>("sse-event", ({ payload }) =>
      useStore.setState((s) => ({
        live: {
          ...s.live,
          [payload.id]: [...(s.live[payload.id] ?? []), ...(payload.event ? [{ ...payload.event, at: Date.now() }] : [])],
        },
      })),
    );
    // WebSocket events, by tab.
    const stopWs = listen<{ id: string; event: WsEvent }>("ws-event", ({ payload }) =>
      useStore.setState((s) => {
        const current = s.sockets[payload.id] ?? { status: "connecting" as const, events: [] };
        const status =
          payload.event.kind === "open" ? "open" : payload.event.kind === "closed" ? "closed" : current.status;
        return { sockets: { ...s.sockets, [payload.id]: { ...current, status, events: [...current.events, payload.event] } } };
      }),
    );
    // gRPC call events, by tab.
    const stopGrpc = listen<{ id: string; event: GrpcEvent }>("grpc-event", ({ payload }) =>
      useStore.setState((s) => {
        const current = s.calls[payload.id] ?? { status: "open" as const, events: [] };
        const status = payload.event.kind === "done" ? "done" : current.status === "starting" ? "open" : current.status;
        return { calls: { ...s.calls, [payload.id]: { ...current, status, events: [...current.events, payload.event] } } };
      }),
    );
    // The web view's own menu (Back, Reload, Inspect) only where text can be edited, for copy and paste.
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest("input, textarea, [contenteditable=true], .cm-editor")) e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    // Capture phase, so editors' own bindings (such as CodeMirror's Ctrl+Enter) don't get these first.
    window.addEventListener("keydown", onKey, true);
    return () => {
      clearTimeout(updates);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("contextmenu", onContextMenu);
      void stop.then((f) => f());
      void stopWs.then((f) => f());
      void stopGrpc.then((f) => f());
    };
  }, []);

  return (
    <div className="shell">
      <Sidebar />
      <main className="workspace">
        <NoticeBar />
        <TabStrip />
        {view.kind === "request" && (
          <>
            <TopBar />
            <RequestBar />
            <Panes />
          </>
        )}
        {view.kind === "folder" && <FolderEditor key={view.path} path={view.path} />}
        {view.kind === "variables" && <VariablesEditor key={view.environment ?? ""} environment={view.environment} />}
        {view.kind === "agents" && <AgentsView />}
        {view.kind === "runner" && <RunnerView key={view.path} path={view.path} />}
        {view.kind === "settings" && <SettingsView />}
        <StatusBar version={version} environment={workspace ? environment : undefined} />
      </main>
      {savingAs && <SaveDialog />}
      {paletteOpen && <CommandPalette />}
      {snippetOpen && <SnippetDialog />}
    </div>
  );
}

/** The request above the response, or beside it, with a handle between them. */
function Panes() {
  const { orientation, split } = useLayout();
  const box = useRef<HTMLDivElement>(null);
  const side = orientation === "side";
  const size = () => {
    const r = box.current!.getBoundingClientRect();
    return side ? { start: r.left, length: r.width } : { start: r.top, length: r.height };
  };
  return (
    <div className={`panes ${orientation}`} ref={box}>
      <div className="pane" style={{ flexBasis: `${split * 100}%` }}>
        <RequestPanel />
      </div>
      <Splitter
        axis={side ? "x" : "y"}
        label="Resize the request and response"
        onDrag={(position) => {
          const { start, length } = size();
          layout.setSplit((position - start) / length);
        }}
        onStep={(pixels) => layout.setSplit(useLayout.getState().split + pixels / size().length)}
        onReset={layout.resetSplit}
      />
      <div className="pane grow">
        <ResponsePanel />
      </div>
    </div>
  );
}

function StatusBar({ version, environment }: { version?: string; environment?: string | null }) {
  const { sidebarOpen, orientation } = useLayout();
  return (
    <footer className="statusbar">
      <button
        className={`status-button${sidebarOpen ? " on" : ""}`}
        onClick={layout.toggleSidebar}
        title="Show or hide the sidebar (Ctrl+B)"
        aria-label="Toggle the sidebar"
      >
        <PanelLeft size={14} />
      </button>
      <button
        className="status-button"
        onClick={layout.toggleOrientation}
        title={orientation === "stacked" ? "Put the response beside the request" : "Put the response below the request"}
        aria-label="Switch the layout"
      >
        {orientation === "stacked" ? <Columns2 size={14} /> : <Rows2 size={14} />}
      </button>
      <span className="status-hints">
        <span><kbd>Ctrl</kbd><kbd>K</kbd> commands</span>
        <span><kbd>Ctrl</kbd><kbd>↵</kbd> send</span>
        <span><kbd>Ctrl</kbd><kbd>S</kbd> save</span>
        <span><kbd>Ctrl</kbd><kbd>T</kbd> new tab</span>
      </span>
      <span className="status-right">
        {environment !== undefined && (
          <span className="status-env">
            <span className={`env-dot${environment ? " on" : ""}`} />
            {environment ?? "No environment"}
          </span>
        )}
        {version && <span>Tuma {version}</span>}
      </span>
    </footer>
  );
}

export default App;
