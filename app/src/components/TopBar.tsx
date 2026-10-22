import { Check, ChevronDown, Code2, Eye, EyeOff, Layers, Plus, Save } from "lucide-react";
import { Fragment, useState } from "react";
import { useStore } from "../store";
import { Menu } from "./ui";

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

/** The environment requests use, as in Postman: at the top right, beside the tabs. A menu of the environments, with
 * a button that adds one. */
export function EnvironmentPicker() {
  const workspace = useStore((s) => s.workspace);
  const environments = useStore((s) => s.environments);
  const environment = useStore((s) => s.environment);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const viewing = useStore((s) => s.view.kind === "variables");
  const { setEnvironment, openVariables, addEnvironment } = useStore.getState();
  if (!workspace) return null;
  const choose = (name: string | null) => (
    <button className="menu-check" onClick={() => setEnvironment(name)}>
      <Check size={14} className={name === environment ? "accent" : "hidden"} />
      {name ?? "No environment"}
    </button>
  );
  return (
    <div className="env-picker">
      <button
        className="env-button"
        aria-haspopup="menu"
        title="The environment whose variables requests use"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu(menu ? null : { x: r.left, y: r.bottom + 4 });
        }}
      >
        <span className={`env-dot${environment ? " on" : ""}`} />
        <span className="env-name">{environment ?? "No environment"}</span>
        <ChevronDown size={14} className="muted" />
      </button>
      <button
        className={`icon-button${viewing ? " on" : ""}`}
        onClick={() =>
          viewing ? useStore.setState({ view: { kind: "request" } }) : void openVariables(environment)
        }
        title={viewing ? "Close the variables" : "Show the variables"}
        aria-label={viewing ? "Close the variables" : "Show the variables"}
        aria-pressed={viewing}
      >
        {viewing ? <EyeOff size={15} /> : <Eye size={15} />}
      </button>
      {menu && (
        <Menu at={menu} onClose={() => setMenu(null)} className="env-menu">
          <div className="menu-head">
            <span>Environments</span>
            <button
              className="icon-button"
              title="New environment"
              aria-label="New environment"
              onClick={() => void addEnvironment()}
            >
              <Plus size={15} />
            </button>
          </div>
          {choose(null)}
          {environments.map((name) => (
            <Fragment key={name}>{choose(name)}</Fragment>
          ))}
          <hr />
          <button className="menu-check" onClick={() => void addEnvironment()}>
            <Plus size={14} />
            New environment
          </button>
          <button className="menu-check" onClick={() => void openVariables(environment)}>
            <Layers size={14} />
            Manage environments
          </button>
        </Menu>
      )}
    </div>
  );
}
