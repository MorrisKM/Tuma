import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { VariableValue } from "../types";
import { EditableName } from "./ui";
import { VariableTable } from "./VariableTable";

/** Global variables, or one environment's. `environment` null means globals. */
export function VariablesEditor({ environment }: { environment: string | null }) {
  const active = useStore((s) => s.environment);
  const [variables, setVariables] = useState<VariableValue[] | null>(null);
  const [saved, setSaved] = useState("");
  const { notify, openVariables, refresh, setEnvironment } = useStore.getState();
  const fail = (e: unknown) => notify({ tone: "error", text: String(e) });

  useEffect(() => {
    setVariables(null);
    (environment === null ? api.globalsRead() : api.environmentRead(environment)).then((vars) => {
      setVariables(vars);
      setSaved(JSON.stringify(vars));
    }, fail);
  }, [environment]);

  if (!variables) return null;
  const dirty = JSON.stringify(variables) !== saved;

  const save = async () => {
    try {
      await (environment === null ? api.globalsWrite(variables) : api.environmentWrite(environment, variables));
      setSaved(JSON.stringify(variables));
    } catch (e) {
      fail(e);
    }
  };
  const rename = async (newName: string) => {
    if (!environment || !newName.trim() || newName === environment) return;
    try {
      const name = await api.environmentRename(environment, newName);
      await refresh();
      if (useStore.getState().environment === environment) setEnvironment(name);
      await openVariables(name);
    } catch (e) {
      fail(e);
    }
  };
  const remove = async () => {
    if (!environment) return;
    const ok = await ask(`Delete the environment "${environment}" and its secrets?`, {
      title: "Delete environment",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    try {
      await api.environmentDelete(environment);
      if (useStore.getState().environment === environment) setEnvironment(null);
      await refresh();
      await openVariables(null);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <section className="folder-editor" onKeyDown={(e) => e.key === "s" && (e.ctrlKey || e.metaKey) && save()}>
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb">
            {environment === null ? "Variables" : "Environments"}
            <span className="crumb-sep">/</span>
          </span>
          <span className="crumb current">
            {environment ? (
              // A new environment opens ready to be named.
              <EditableName
                name={environment}
                onRename={(name) => void rename(name)}
                editing={/^New environment( \(\d+\))?$/.test(environment)}
              />
            ) : (
              "Globals"
            )}
          </span>
          {environment && environment === active && <span className="pill">Active</span>}
          {dirty && <span className="dirty-dot" title="Unsaved changes" />}
        </div>
        {environment && environment !== active && (
          <button className="button ghost" onClick={() => setEnvironment(environment)}>
            Use this environment
          </button>
        )}
        {environment && (
          <button className="button ghost danger" onClick={() => void remove()}>
            Delete
          </button>
        )}
        <button className="button primary" disabled={!dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <div className="panel-body">
        <p className="hint">
          {environment
            ? "Requests use these values while this environment is selected. They win over collection and folder variables."
            : "Available to every request in the workspace, with the lowest priority."}
        </p>
        <VariableTable variables={variables} onChange={setVariables} allowSecrets />
        <CurrentValues environment={environment} />
      </div>
    </section>
  );
}

/** Values scripts set in this scope (pm.environment.set, pm.globals.set). They win over the table, on this machine
 * only, until reset. */
function CurrentValues({ environment }: { environment: string | null }) {
  const [values, setValues] = useState<Record<string, string | null>>({});
  const load = () =>
    api.currentValues().then(
      (all) => setValues((environment === null ? all.globals : all.environments[environment]) ?? {}),
      () => setValues({}),
    );
  useEffect(() => {
    void load();
  }, [environment]);
  const entries = Object.entries(values);
  if (entries.length === 0) return null;
  const reset = async () => {
    await api.resetCurrentValues(environment === null ? { kind: "globals" } : { kind: "environment", name: environment });
    await load();
    void useStore.getState().loadScope();
  };
  return (
    <div className="current-values">
      <div className="toolbar spaced">
        <h3>Set by scripts</h3>
        <button className="link-button" onClick={() => void reset()}>
          Reset to the values above
        </button>
      </div>
      <p className="hint">
        Scripts changed these. The new values are used instead of the ones above, on this machine only: they are not
        saved in the workspace files, so tokens don't end up in git.
      </p>
      <table className="current-table">
        <tbody>
          {entries.map(([name, value]) => (
            <tr key={name}>
              <td className="mono">{name}</td>
              <td className="mono">{value === null ? <span className="muted">(unset)</span> : value}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
