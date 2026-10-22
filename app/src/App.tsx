import { useEffect, useState } from "react";
import { appVersion } from "./api";
import { AgentsView } from "./components/AgentsView";
import { CommandPalette } from "./components/CommandPalette";
import { FolderEditor } from "./components/FolderEditor";
import { NoticeBar } from "./components/NoticeBar";
import { RequestBar } from "./components/RequestBar";
import { RequestPanel } from "./components/RequestPanel";
import { ResponsePanel } from "./components/ResponsePanel";
import { SaveDialog } from "./components/SaveDialog";
import { Sidebar } from "./components/Sidebar";
import { SnippetDialog } from "./components/SnippetDialog";
import { TabStrip } from "./components/TabStrip";
import { TopBar } from "./components/TopBar";
import { focusUrl } from "./components/UrlEditor";
import { VariablesEditor } from "./components/VariablesEditor";
import { useStore } from "./store";
import { applyTheme, loadTheme } from "./theme";
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
    // Capture phase, so editors' own bindings (such as CodeMirror's Ctrl+Enter) don't get these first.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div className="shell">
      <Sidebar />
      <main className="workspace">
        <NoticeBar />
        {view.kind === "request" && (
          <>
            <TabStrip />
            <TopBar />
            <RequestBar />
            <div className="panes">
              <RequestPanel />
              <ResponsePanel />
            </div>
          </>
        )}
        {view.kind === "folder" && <FolderEditor key={view.path} path={view.path} />}
        {view.kind === "variables" && <VariablesEditor key={view.environment ?? ""} environment={view.environment} />}
        {view.kind === "agents" && <AgentsView />}
        <footer className="statusbar">
          <span>Ctrl+K commands · Ctrl+Enter send · Ctrl+S save · Ctrl+T new tab</span>
          <span>
            {workspace && (environment ? `Environment: ${environment} · ` : "No environment · ")}
            {version && `v${version}`}
          </span>
        </footer>
      </main>
      {savingAs && <SaveDialog />}
      {paletteOpen && <CommandPalette />}
      {snippetOpen && <SnippetDialog />}
    </div>
  );
}

export default App;
