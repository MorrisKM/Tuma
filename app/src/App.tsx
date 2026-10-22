import { useEffect, useState } from "react";
import { appVersion } from "./api";
import { AgentsView } from "./components/AgentsView";
import { FolderEditor } from "./components/FolderEditor";
import { NoticeBar } from "./components/NoticeBar";
import { RequestBar } from "./components/RequestBar";
import { RequestPanel } from "./components/RequestPanel";
import { ResponsePanel } from "./components/ResponsePanel";
import { SaveDialog } from "./components/SaveDialog";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { VariablesEditor } from "./components/VariablesEditor";
import { useStore } from "./store";
import "./App.css";

function App() {
  const [version, setVersion] = useState<string>();
  const view = useStore((s) => s.view);
  const savingAs = useStore((s) => s.savingAs);
  const workspace = useStore((s) => s.workspace);
  const environment = useStore((s) => s.environment);

  useEffect(() => {
    appVersion().then(setVersion);
    void useStore.getState().init();
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const { send, save, newRequest, view } = useStore.getState();
      if (e.key === "Enter") {
        e.preventDefault();
        void send();
      } else if (e.key === "s" && view.kind === "request") {
        e.preventDefault();
        void save();
      } else if (e.key === "n") {
        e.preventDefault();
        void newRequest();
      }
    };
    // Capture phase, so the body editor's own Ctrl+Enter binding doesn't get it first.
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
          <span>Ctrl+Enter send · Ctrl+S save · Ctrl+N new</span>
          <span>
            {workspace && (environment ? `Environment: ${environment} · ` : "No environment · ")}
            {version && `v${version}`}
          </span>
        </footer>
      </main>
      {savingAs && <SaveDialog />}
    </div>
  );
}

export default App;
