import { useStore } from "../store";
import { CodeEditor } from "./CodeEditor";

/** Asks whether a pasted cURL command's commented script goes in the request's Scripts tab, showing the script. */
export function CurlImportDialog() {
  const pending = useStore((s) => s.curlImport);
  const environment = useStore((s) => s.environment);
  const { finishCurlImport } = useStore.getState();
  if (!pending) return null;
  const cancel = () => useStore.setState({ curlImport: null });
  const names = pending.variables.join(", ");

  return (
    <div className="modal-backdrop" onMouseDown={cancel}>
      <div
        className="modal wide"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && cancel()}
        role="dialog"
        aria-label="Import the commented script"
      >
        <h2>This cURL command has a commented script</h2>
        <p className="hint">Uncomment it and place it in the request's Scripts tab, to run before each send?</p>
        <div className="editor-frame curl-import-script">
          <CodeEditor value={pending.script} language="javascript" readOnly />
        </div>
        {pending.variables.length > 0 &&
          (environment ? (
            <p className="hint curl-import-variables">
              Adds to the environment {environment}, with empty values: <span className="mono">{names}</span>
            </p>
          ) : (
            <p className="hint warning curl-import-variables">
              No environment is selected, so these needed variables won't be added: <span className="mono">{names}</span>
            </p>
          ))}
        <div className="modal-actions">
          <button className="button" onClick={cancel}>
            Cancel
          </button>
          <button className="button" onClick={() => void finishCurlImport(false)}>
            Import without the script
          </button>
          <button className="button primary" autoFocus onClick={() => void finishCurlImport(true)}>
            Add to Scripts
          </button>
        </div>
      </div>
    </div>
  );
}
