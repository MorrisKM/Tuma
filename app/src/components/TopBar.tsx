import { Code2, Eye, Save } from "lucide-react";
import { useStore } from "../store";

/** Above the URL: where the request lives, whether it has unsaved changes, the code snippet and Save. */
export function TopBar() {
  const draft = useStore((s) => s.draft);
  const dirty = useStore((s) => s.isDirty());
  const { save } = useStore.getState();

  const crumbs = draft.path ? draft.path.split("/").slice(0, -1) : [];

  return (
    <div className="top-bar">
      <div className="crumbs" title={draft.path ?? "Not saved"}>
        {crumbs.map((c, i) => (
          <span key={i} className="crumb">
            {c}
            <span className="crumb-sep">/</span>
          </span>
        ))}
        <span className="crumb current">{draft.name}</span>
        {dirty && <span className="dirty-dot" title="Unsaved changes" />}
        {!draft.path && <span className="pill neutral">Not saved</span>}
      </div>
      <button
        className="button ghost"
        onClick={() => useStore.setState({ snippetOpen: true })}
        title="Copy as cURL, Python or JavaScript"
      >
        <Code2 size={15} />
        Code
      </button>
      <button className="button" onClick={() => void save()} title="Save (Ctrl+S)">
        <Save size={15} />
        Save
      </button>
    </div>
  );
}

/** The environment requests use, as in Postman: at the top right, beside the tabs. */
export function EnvironmentPicker() {
  const workspace = useStore((s) => s.workspace);
  const environments = useStore((s) => s.environments);
  const environment = useStore((s) => s.environment);
  const { setEnvironment, openVariables } = useStore.getState();
  if (!workspace) return null;
  return (
    <div className="env-picker">
      <span className={`env-dot${environment ? " on" : ""}`} />
      <select
        value={environment ?? ""}
        onChange={(e) => setEnvironment(e.target.value || null)}
        aria-label="Environment"
        title="The environment whose variables requests use"
      >
        <option value="">No environment</option>
        {environments.map((name) => (
          <option key={name} value={name}>
            {name}
          </option>
        ))}
      </select>
      <button
        className="icon-button"
        onClick={() => void openVariables(environment)}
        title="Edit variables"
        aria-label="Edit variables"
      >
        <Eye size={15} />
      </button>
    </div>
  );
}
