import { useState } from "react";
import { folderCreate } from "../api";
import { folders, useStore } from "../store";

/** Asks where to save a request that is not saved yet. */
export function SaveDialog() {
  const tree = useStore((s) => s.tree);
  const draft = useStore((s) => s.draft);
  const options = folders(tree);
  const [name, setName] = useState(draft.name === "Untitled" ? suggestName(draft.url) : draft.name);
  const [parent, setParent] = useState(options[0]?.path ?? "");
  const [newCollection, setNewCollection] = useState(options.length === 0 ? "My collection" : "");
  const close = () => useStore.setState({ savingAs: false });

  const submit = async () => {
    let target = parent;
    if (!target || newCollection) {
      try {
        target = await folderCreate(null, newCollection || "My collection");
      } catch (e) {
        useStore.getState().notify({ tone: "error", text: String(e) });
        return;
      }
    }
    await useStore.getState().saveAs(target, name || "Untitled");
  };

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <form
        className="modal"
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
        onKeyDown={(e) => e.key === "Escape" && close()}
      >
        <h2>Save request</h2>
        <div className="form">
          <label>
            <span>Name</span>
            <input value={name} autoFocus onChange={(e) => setName(e.target.value)} />
          </label>
          {options.length > 0 && (
            <label>
              <span>Save to</span>
              <select
                value={newCollection ? "" : parent}
                onChange={(e) => {
                  setParent(e.target.value);
                  setNewCollection(e.target.value ? "" : "My collection");
                }}
              >
                {options.map((o) => (
                  <option key={o.path} value={o.path}>
                    {"  ".repeat(o.depth) + o.name}
                  </option>
                ))}
                <option value="">New collection…</option>
              </select>
            </label>
          )}
          {(newCollection || options.length === 0) && (
            <label>
              <span>New collection</span>
              <input value={newCollection} onChange={(e) => setNewCollection(e.target.value)} />
            </label>
          )}
        </div>
        <div className="modal-actions">
          <button type="button" className="button" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="button primary">
            Save
          </button>
        </div>
      </form>
    </div>
  );
}

/** "GET https://api.x/users/42?a=1" → "users 42". */
function suggestName(url: string): string {
  const path = url.replace(/^[a-z]+:\/\//i, "").split(/[?#]/)[0].split("/").slice(1).filter(Boolean);
  return path.length ? path.slice(-2).join(" ") : "Untitled";
}
