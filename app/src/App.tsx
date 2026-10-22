import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    invoke<string>("app_version").then(setVersion);
  }, []);

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">Tuma</div>
        <p className="muted">No collections yet</p>
      </aside>
      <main className="workspace">
        <div className="empty">
          <h1>Send your first request</h1>
          <p className="muted">The request editor arrives in M1.</p>
        </div>
        <footer className="statusbar">{version && `v${version}`}</footer>
      </main>
    </div>
  );
}

export default App;
