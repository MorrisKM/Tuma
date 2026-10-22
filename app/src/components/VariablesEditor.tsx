import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { VariableValue } from "../types";
import { VariableTable } from "./VariableTable";

/** Global variables, or one environment's. `environment` null means globals. */
export function VariablesEditor({ environment }: { environment: string | null }) {
  const environments = useStore((s) => s.environments);
  const [variables, setVariables] = useState<VariableValue[] | null>(null);
  const [saved, setSaved] = useState("");
  const [renaming, setRenaming] = useState(false);
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
  const add = async () => {
    try {
      const name = await api.environmentCreate("New environment");
      await refresh();
      await openVariables(name);
      setRenaming(true);
    } catch (e) {
      fail(e);
    }
  };
  const rename = async (newName: string) => {
    setRenaming(false);
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
        <div className="segmented scopes">
          <button className={environment === null ? "active" : ""} onClick={() => void openVariables(null)}>
            Globals
          </button>
          {environments.map((name) => (
            <button key={name} className={environment === name ? "active" : ""} onClick={() => void openVariables(name)}>
              {name}
            </button>
          ))}
          <button onClick={() => void add()} title="New environment">
            +
          </button>
        </div>
        <button className="button primary" disabled={!dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <div className="panel-body">
        <div className="toolbar spaced">
          {renaming && environment ? (
            <input
              autoFocus
              defaultValue={environment}
              onBlur={(e) => void rename(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void rename(e.currentTarget.value);
                if (e.key === "Escape") setRenaming(false);
              }}
            />
          ) : (
            <h2 className="section-title">{environment ?? "Global variables"}</h2>
          )}
          {environment && !renaming && (
            <span className="actions">
              <button className="link-button" onClick={() => setRenaming(true)}>
                Rename
              </button>
              <button className="link-button danger" onClick={() => void remove()}>
                Delete
              </button>
            </span>
          )}
        </div>
        <p className="hint">
          {environment
            ? "Select this environment in the top bar to use these values. They win over collection and folder variables."
            : "Available to every request in the workspace, with the lowest priority."}
        </p>
        <VariableTable variables={variables} onChange={setVariables} allowSecrets />
      </div>
    </section>
  );
}
