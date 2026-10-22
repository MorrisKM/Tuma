import { Play } from "lucide-react";
import { useEffect, useState } from "react";
import * as api from "../api";
import { normalizeVariables } from "../draft";
import { useStore } from "../store";
import type { FolderFile, KeyValue, VariableValue } from "../types";
import { AuthForm } from "./AuthForm";
import { blankRow, KeyValueEditor } from "./KeyValueEditor";
import { ScriptsEditor } from "./ScriptsEditor";
import { Tabs } from "./Tabs";
import { VariableTable } from "./VariableTable";

type Tab = "auth" | "headers" | "variables" | "scripts" | "description";

/** Settings shared by every request in a collection or folder. */
export function FolderEditor({ path }: { path: string }) {
  const [tab, setTab] = useState<Tab>("auth");
  const [folder, setFolder] = useState<FolderFile | null>(null);
  const [saved, setSaved] = useState("");
  const isCollection = !path.includes("/");
  const name = path.split("/").pop();

  useEffect(() => {
    setFolder(null);
    api.folderRead(path).then(
      (f) => {
        setFolder(f);
        setSaved(JSON.stringify(f));
      },
      (e) => useStore.getState().notify({ tone: "error", text: String(e) }),
    );
  }, [path]);

  if (!folder) return null;
  const headers: KeyValue[] = (folder.headers ?? []).map((h) => ({
    name: h.name ?? "",
    value: h.value ?? "",
    enabled: h.enabled !== false,
  }));
  const variables: VariableValue[] = normalizeVariables(folder.variables ?? []);
  const dirty = JSON.stringify(folder) !== saved;
  const save = async () => {
    try {
      await api.folderWrite(path, folder);
      setSaved(JSON.stringify(folder));
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: String(e) });
    }
  };

  return (
    <section className="folder-editor" onKeyDown={(e) => e.key === "s" && (e.ctrlKey || e.metaKey) && save()}>
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">{name}</span>
          <span className="muted small">{isCollection ? "Collection settings" : "Folder settings"}</span>
          {dirty && <span className="dirty-dot" title="Unsaved changes" />}
        </div>
        <div className="toolbar">
          <button
            className="button"
            onClick={() => useStore.setState({ view: { kind: "runner", path } })}
            title="Run every request inside, with its scripts and checks"
          >
            <Play size={14} />
            Run…
          </button>
          <button className="button primary" disabled={!dirty} onClick={() => void save()}>
            Save
          </button>
        </div>
      </div>
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "auth", label: "Auth" },
          { id: "headers", label: "Headers", count: headers.filter((h) => h.enabled && h.name).length },
          { id: "variables", label: "Variables", count: variables.length },
          {
            id: "scripts",
            label: "Scripts",
            count: [folder.scripts?.preRequest, folder.scripts?.test].filter((s) => s?.trim()).length,
          },
          { id: "description", label: "Description" },
        ]}
      />
      <div className="panel-body">
        {tab === "auth" && (
          <AuthForm
            auth={folder.auth ?? null}
            onChange={(auth) => setFolder({ ...folder, auth })}
            tokenFor={{ request: { method: "GET", url: "", auth: folder.auth ?? undefined }, path: `${path}/request.yaml` }}
            inheritHint={
              isCollection
                ? "Requests in this collection that inherit auth send none."
                : "Uses the auth of the parent folder or collection."
            }
          />
        )}
        {tab === "headers" && (
          <>
            <KeyValueEditor rows={headers} onChange={(headers) => setFolder({ ...folder, headers })} blank={blankRow} />
            <p className="hint">Sent with every request inside, unless a request sets the same header.</p>
          </>
        )}
        {tab === "variables" && (
          <>
            <VariableTable
              variables={variables}
              allowSecrets={false}
              onChange={(variables) => setFolder({ ...folder, variables })}
            />
            <p className="hint">
              Available to every request inside. Nearer folders win over the collection; the selected environment wins
              over both. Put secrets in an environment instead: these values are saved in the workspace files.
            </p>
          </>
        )}
        {tab === "scripts" && (
          <ScriptsEditor
            scripts={folder.scripts ?? {}}
            onChange={(scripts) => setFolder({ ...folder, scripts })}
            scope={isCollection ? "collection" : "folder"}
          />
        )}
        {tab === "description" && (
          <textarea
            className="description"
            value={folder.description ?? ""}
            placeholder="What this is for. Markdown is fine."
            onChange={(e) => setFolder({ ...folder, description: e.target.value })}
          />
        )}
      </div>
    </section>
  );
}
