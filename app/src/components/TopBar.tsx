import { useStore } from "../store";

/** Above the editor: where the request lives, whether it has unsaved changes, the environment, and Save. */
export function TopBar() {
  const draft = useStore((s) => s.draft);
  const dirty = useStore((s) => s.isDirty());
  const workspace = useStore((s) => s.workspace);
  const environments = useStore((s) => s.environments);
  const environment = useStore((s) => s.environment);
  const { setEnvironment, save, openVariables } = useStore.getState();

  const crumbs = draft.path ? draft.path.split("/").slice(0, -1) : [];

  return (
    <div className="top-bar">
      <div className="crumbs" title={draft.path ?? "Not saved"}>
        {crumbs.map((c, i) => (
          <span key={i} className="crumb muted">
            {c} /
          </span>
        ))}
        <span className="crumb current">{draft.name}</span>
        {dirty && (
          <span className="dirty" title="Unsaved changes">
            ●
          </span>
        )}
      </div>
      {workspace && (
        <div className="env-picker">
          <select
            value={environment ?? ""}
            onChange={(e) => setEnvironment(e.target.value || null)}
            aria-label="Environment"
          >
            <option value="">No environment</option>
            {environments.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <button className="button" onClick={() => void openVariables(environment)} title="Edit variables">
            Variables
          </button>
        </div>
      )}
      <button
        className="button"
        onClick={() => useStore.setState({ snippetOpen: true })}
        title="Copy as cURL, Python or JavaScript"
      >
        Code
      </button>
      <button className="button" onClick={() => void save()} title="Save (Ctrl+S)">
        Save
      </button>
    </div>
  );
}
