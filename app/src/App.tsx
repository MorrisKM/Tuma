import { useEffect, useState } from "react";
import { appVersion } from "./api";
import { RequestBar } from "./components/RequestBar";
import { RequestPanel } from "./components/RequestPanel";
import { ResponsePanel } from "./components/ResponsePanel";
import { Sidebar } from "./components/Sidebar";
import { useStore } from "./store";
import "./App.css";

function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    appVersion().then(setVersion);
    // Ctrl+Enter (Cmd+Enter on macOS) sends from anywhere, including the body editor.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        void useStore.getState().send();
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
        <RequestBar />
        <div className="panes">
          <RequestPanel />
          <ResponsePanel />
        </div>
        <footer className="statusbar">
          <span>Ctrl+Enter to send</span>
          <span>{version && `v${version}`}</span>
        </footer>
      </main>
    </div>
  );
}

export default App;
