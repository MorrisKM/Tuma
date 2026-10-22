import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";

/** Asks for a new workspace's name; it is created in the workspaces folder, which can be changed here. */
export function NewWorkspaceDialog() {
  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const close = () => useStore.setState({ creatingWorkspace: false });
  const load = () => api.workspaceList().then((l) => setFolder(l.folder), () => {});
  useEffect(() => void load(), []);

  const change = async () => {
    const picked = await open({ directory: true, title: "Where should new workspaces go?", defaultPath: folder });
    if (typeof picked !== "string") return;
    try {
      await api.workspacesDirSet(picked);
      await load();
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: String(e) });
    }
  };
  const sep = folder.includes("\\") ? "\\" : "/";

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <form
        className="modal"
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) void useStore.getState().newWorkspace(name.trim());
        }}
        onKeyDown={(e) => e.key === "Escape" && close()}
      >
        <h2>New workspace</h2>
        <div className="form">
          <label>
            <span>Name</span>
            <input
              value={name}
              autoFocus
              placeholder="Such as Shop API"
              onChange={(e) => setName(e.target.value)}
            />
          </label>
        </div>
        <p className="hint">
          Created in <span className="mono">{folder ? `${folder}${sep}${name.trim() || "…"}` : "…"}</span>{" "}
          <button type="button" className="link-button" onClick={() => void change()}>
            Change folder
          </button>
        </p>
        <p className="hint">
          A workspace is a folder of plain files: collections, requests and environments. Secret values stay in this
          computer's keychain.
        </p>
        <div className="modal-actions">
          <button type="button" className="button" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={!name.trim()}>
            Create workspace
          </button>
        </div>
      </form>
    </div>
  );
}
