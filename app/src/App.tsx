import { Columns2, PanelLeft, Rows2, SquareTerminal } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { appVersion, type MockEvent } from "./api";
import { AgentsView } from "./components/AgentsView";
import { CommandPalette } from "./components/CommandPalette";
import { ConsolePanel } from "./components/ConsolePanel";
import { useConsole } from "./console";
import { RunnerView } from "./components/RunnerView";
import { SettingsView } from "./components/SettingsView";
import { FolderEditor } from "./components/FolderEditor";
import { MockView } from "./components/MockView";
import { mocks } from "./mocks";
import { NoticeBar } from "./components/NoticeBar";
import { RequestBar } from "./components/RequestBar";
import { RequestPanel } from "./components/RequestPanel";
import { ResponsePanel } from "./components/ResponsePanel";
import { DocsDialog } from "./components/DocsDialog";
import { NewWorkspaceDialog } from "./components/NewWorkspaceDialog";
import { SaveDialog } from "./components/SaveDialog";
import { Sidebar } from "./components/Sidebar";
import { SnippetDialog } from "./components/SnippetDialog";
import { Splitter } from "./components/Splitter";
import { TabStrip } from "./components/TabStrip";
import { TopBar } from "./components/TopBar";
import { focusUrl } from "./components/UrlEditor";
import { openSearchIn } from "./components/CodeEditor";
import { VariablesEditor } from "./components/VariablesEditor";
import { layout, useLayout } from "./layout";
import { consoleLog, grpcTarget, useStore } from "./store";
import type { GrpcEvent, RunStep, SseEvent, WsEvent } from "./types";
import { listen } from "@tauri-apps/api/event";
import { applyTheme, loadTheme } from "./theme";
import { checkAtStart } from "./updates";
import "./App.css";

applyTheme(loadTheme());

/** Keyboard shortcuts. Ctrl is Cmd on macOS. */
function onKey(e: KeyboardEvent) {
  const s = useStore.getState();
  // F2 renames the open request, in the sidebar.
  if (e.key === "F2" && s.view.kind === "request" && s.draft.path) {
    useStore.setState({ renaming: s.draft.path });
    e.preventDefault();
    return;
  }
  if (!(e.ctrlKey || e.metaKey)) return;
  const key = e.key.toLowerCase();
  const handled = (() => {
    if (key === "enter") void s.send();
    else if (key === "k") useStore.setState({ paletteOpen: !s.paletteOpen });
    else if (key === "s" && s.view.kind === "request") void s.save();
    else if (key === "t" || key === "n") s.newTab();
    else if (key === "w") void s.closeTab(s.activeTab);
    else if (key === "tab") s.cycleTab(e.shiftKey ? -1 : 1);
    else if (key === "b") layout.toggleSidebar();
    // Outside an editor, Ctrl+F searches the response body (inside one, the editor's own search opens).
    else if (key === "f" && !(e.target as HTMLElement).closest?.(".cm-editor")) {
      if (!openSearchIn(document.querySelector(".response-body"))) return false;
    }
    else if (key === "`") layout.toggleConsole();
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
  const creatingWorkspace = useStore((s) => s.creatingWorkspace);
  const docs = useStore((s) => s.docs);
  const paletteOpen = useStore((s) => s.paletteOpen);
  const snippetOpen = useStore((s) => s.snippetOpen);
  const workspace = useStore((s) => s.workspace);
  const environment = useStore((s) => s.environment);
  const consoleOpen = useLayout((s) => s.consoleOpen);

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
    const stopWs = listen<{ id: string; event: WsEvent }>("ws-event", ({ payload }) => {
      logSocket(payload.id, payload.event);
      useStore.setState((s) => {
        const current = s.sockets[payload.id] ?? { status: "connecting" as const, events: [] };
        const status =
          payload.event.kind === "open" ? "open" : payload.event.kind === "closed" ? "closed" : current.status;
        return { sockets: { ...s.sockets, [payload.id]: { ...current, status, events: [...current.events, payload.event] } } };
      });
    });
    // gRPC call events, by tab.
    const stopGrpc = listen<{ id: string; event: GrpcEvent }>("grpc-event", ({ payload }) => {
      logCall(payload.id, payload.event);
      useStore.setState((s) => {
        const current = s.calls[payload.id] ?? { status: "open" as const, events: [] };
        const status = payload.event.kind === "done" ? "done" : current.status === "starting" ? "open" : current.status;
        return { calls: { ...s.calls, [payload.id]: { ...current, status, events: [...current.events, payload.event] } } };
      });
    });
    // Requests mock servers answered.
    const stopMocks = listen<{ id: string; event: MockEvent }>("mock-event", ({ payload }) =>
      mocks.record(payload.id, payload.event),
    );
    void mocks.refresh();
    // Collection runs, whether or not the runner is on screen.
    const stopRuns = listen<{ runId: string; step: RunStep }>("run-step", ({ payload }) => logStep(payload.step));
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
      void stopRuns.then((f) => f());
      void stopMocks.then((f) => f());
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
        {view.kind === "mock" && <MockView key={view.id} id={view.id} />}
        {consoleOpen && <ConsolePanel />}
        <StatusBar version={version} environment={workspace ? environment : undefined} />
      </main>
      {savingAs && <SaveDialog />}
      {creatingWorkspace && <NewWorkspaceDialog />}
      {docs && <DocsDialog scope={docs.scope} />}
      {paletteOpen && <CommandPalette />}
      {snippetOpen && <SnippetDialog />}
    </div>
  );
}

/** The request a tab holds, for naming console entries. */
const tabDraft = (id: string) => useStore.getState().allTabs().find((t) => t.id === id)?.draft;

/** A WebSocket opening, closing or failing; messages stay in the request's own log. */
function logSocket(tab: string, event: WsEvent) {
  const draft = tabDraft(tab);
  const base = { kind: "ws" as const, method: "WS", url: draft?.url ?? "", source: draft?.name };
  if (event.kind === "open") {
    consoleLog({ ...base, url: event.url, status: 101, reason: "Connected", response: { headers: event.headers } });
  } else if (event.kind === "closed") {
    const by = event.byServer ? "by the server" : "by you";
    consoleLog({ ...base, reason: `Closed ${by}${event.code ? ` (${event.code}${event.reason ? ` ${event.reason}` : ""})` : ""}` });
  } else if (event.kind === "error") {
    consoleLog({ ...base, error: event.message });
  }
}

/** A gRPC call's result. */
function logCall(tab: string, event: GrpcEvent) {
  if (event.kind !== "done") return;
  const draft = tabDraft(tab);
  const messages = useStore.getState().calls[tab]?.events.filter((e) => e.kind === "message" && e.incoming) ?? [];
  consoleLog({
    kind: "grpc",
    method: "GRPC",
    url: grpcTarget(draft?.url ?? "", draft?.grpc.method),
    source: draft?.name,
    status: event.code,
    reason: event.message ? `${event.status}: ${event.message}` : event.status,
    timeMs: event.elapsedMs,
    request: { headers: [], body: draft?.raw || undefined },
    response: {
      headers: event.metadata,
      body: messages.map((m) => (m.kind === "message" ? m.json : "")).join("\n") || undefined,
    },
  });
}

function logStep(step: RunStep) {
  if (step.skipped) return;
  consoleLog({
    kind: "run",
    method: step.method,
    url: step.url,
    status: step.status,
    reason: step.reason,
    timeMs: step.timeMs,
    size: step.size,
    error: step.error,
    source: step.iteration > 1 ? `${step.name} (run, iteration ${step.iteration})` : `${step.name} (run)`,
    response: step.body !== undefined ? { headers: [], body: step.body } : undefined,
    logs: step.console ?? [],
    scriptErrors: step.scriptErrors ?? [],
  });
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
  const { sidebarOpen, orientation, consoleOpen } = useLayout();
  const { unseen, unseenErrors } = useConsole();
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
      <button
        className={`status-button labelled${consoleOpen ? " on" : ""}`}
        onClick={layout.toggleConsole}
        title="Show or hide the console (Ctrl+`)"
        aria-pressed={consoleOpen}
      >
        <SquareTerminal size={14} />
        Console
        {unseen > 0 && <span className={`badge${unseenErrors ? " fail" : ""}`}>{unseen > 99 ? "99+" : unseen}</span>}
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
