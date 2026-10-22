import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { type AuthMode, type BodyMode, RAW_TYPES } from "../draft";
import { useStore } from "../store";
import type { KeyValue } from "../types";
import { CodeEditor, type Language } from "./CodeEditor";
import { blankPart, blankRow, KeyValueEditor } from "./KeyValueEditor";
import { Tabs } from "./Tabs";

type Tab = "params" | "headers" | "body" | "auth" | "settings";

const active = (rows: KeyValue[]) => rows.filter((r) => r.enabled && r.name).length;

export function RequestPanel() {
  const [tab, setTab] = useState<Tab>("params");
  const draft = useStore((s) => s.draft);

  return (
    <section className="request-panel">
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "params", label: "Params", count: active(draft.params) },
          { id: "headers", label: "Headers", count: active(draft.headers) },
          { id: "body", label: "Body" },
          { id: "auth", label: "Auth" },
          { id: "settings", label: "Settings" },
        ]}
      />
      <div className="panel-body">
        {tab === "params" && <Params />}
        {tab === "headers" && <Headers />}
        {tab === "body" && <BodyEditor />}
        {tab === "auth" && <AuthEditor />}
        {tab === "settings" && <SettingsEditor />}
      </div>
    </section>
  );
}

function Params() {
  const params = useStore((s) => s.draft.params);
  return <KeyValueEditor rows={params} onChange={useStore.getState().setParams} blank={blankRow} />;
}

function Headers() {
  const headers = useStore((s) => s.draft.headers);
  return (
    <>
      <KeyValueEditor rows={headers} onChange={(headers) => useStore.getState().update({ headers })} blank={blankRow} />
      <p className="hint">
        Tuma adds User-Agent, Accept, Accept-Encoding and Content-Type unless you set them here. The Request tab of the
        response shows exactly what was sent.
      </p>
    </>
  );
}

const BODY_MODES: { id: BodyMode; label: string }[] = [
  { id: "none", label: "None" },
  { id: "json", label: "JSON" },
  { id: "xml", label: "XML" },
  { id: "html", label: "HTML" },
  { id: "text", label: "Text" },
  { id: "javascript", label: "JavaScript" },
  { id: "form", label: "Form URL-encoded" },
  { id: "multipart", label: "Multipart form" },
  { id: "file", label: "Binary file" },
];

async function pickFile(): Promise<string | null> {
  const path = await open({ multiple: false, directory: false });
  return typeof path === "string" ? path : null;
}

function BodyEditor() {
  const draft = useStore((s) => s.draft);
  const update = useStore.getState().update;
  const mode = draft.bodyMode;

  return (
    <div className="body-editor">
      <div className="toolbar">
        <select value={mode} onChange={(e) => update({ bodyMode: e.target.value as BodyMode })} aria-label="Body type">
          {BODY_MODES.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
        {mode === "json" && (
          <button className="link-button" onClick={() => update({ raw: prettyJson(draft.raw) })}>
            Format
          </button>
        )}
      </div>

      {mode === "none" && <p className="hint">This request has no body.</p>}
      {RAW_TYPES[mode] && (
        <div className="editor-frame">
          <CodeEditor
            value={draft.raw}
            onChange={(raw) => update({ raw })}
            language={mode === "text" ? "text" : (mode as Language)}
          />
        </div>
      )}
      {mode === "form" && (
        <KeyValueEditor rows={draft.form} onChange={(form) => update({ form })} blank={blankRow} />
      )}
      {mode === "multipart" && (
        <KeyValueEditor
          rows={draft.parts}
          onChange={(parts) => update({ parts })}
          blank={blankPart}
          renderValue={(row, set) =>
            row.file ? (
              <button
                className="kv-value file-button"
                onClick={async () => {
                  const path = await pickFile();
                  if (path) set({ value: path });
                }}
              >
                {row.value || "Choose a file…"}
              </button>
            ) : null
          }
          renderExtra={(row, set) => (
            <select
              className="kv-kind"
              value={row.file ? "file" : "text"}
              onChange={(e) => set({ file: e.target.value === "file", value: "" })}
              aria-label="Field type"
            >
              <option value="text">Text</option>
              <option value="file">File</option>
            </select>
          )}
        />
      )}
      {mode === "file" && (
        <div className="file-body">
          <button
            className="button"
            onClick={async () => {
              const path = await pickFile();
              if (path) update({ filePath: path });
            }}
          >
            {draft.filePath ? "Change file" : "Choose a file"}
          </button>
          <span className="muted mono">{draft.filePath}</span>
        </div>
      )}
    </div>
  );
}

function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

const AUTH_MODES: { id: AuthMode; label: string }[] = [
  { id: "none", label: "No auth" },
  { id: "bearer", label: "Bearer token" },
  { id: "basic", label: "Basic auth" },
  { id: "apiKey", label: "API key" },
];

function AuthEditor() {
  const d = useStore((s) => s.draft);
  const update = useStore.getState().update;

  return (
    <div className="form">
      <label>
        <span>Type</span>
        <select value={d.authMode} onChange={(e) => update({ authMode: e.target.value as AuthMode })}>
          {AUTH_MODES.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      {d.authMode === "bearer" && (
        <label>
          <span>Token</span>
          <input value={d.token} spellCheck={false} onChange={(e) => update({ token: e.target.value })} />
        </label>
      )}
      {d.authMode === "basic" && (
        <>
          <label>
            <span>Username</span>
            <input value={d.username} spellCheck={false} onChange={(e) => update({ username: e.target.value })} />
          </label>
          <label>
            <span>Password</span>
            <input type="password" value={d.password} onChange={(e) => update({ password: e.target.value })} />
          </label>
        </>
      )}
      {d.authMode === "apiKey" && (
        <>
          <label>
            <span>Key</span>
            <input
              value={d.apiKey}
              placeholder="X-Api-Key"
              spellCheck={false}
              onChange={(e) => update({ apiKey: e.target.value })}
            />
          </label>
          <label>
            <span>Value</span>
            <input value={d.apiValue} spellCheck={false} onChange={(e) => update({ apiValue: e.target.value })} />
          </label>
          <label>
            <span>Add to</span>
            <select
              value={d.apiLocation}
              onChange={(e) => update({ apiLocation: e.target.value as "header" | "query" })}
            >
              <option value="header">Header</option>
              <option value="query">Query params</option>
            </select>
          </label>
        </>
      )}
    </div>
  );
}

function SettingsEditor() {
  const settings = useStore((s) => s.draft.settings);
  const set = (patch: Partial<typeof settings>) => useStore.getState().update({ settings: { ...settings, ...patch } });

  return (
    <div className="form">
      <label>
        <span>Timeout (ms)</span>
        <input
          type="number"
          min={0}
          value={settings.timeoutMs ?? 0}
          onChange={(e) => set({ timeoutMs: Number(e.target.value) || null })}
        />
        <small className="muted">0 waits forever</small>
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={settings.followRedirects}
          onChange={(e) => set({ followRedirects: e.target.checked })}
        />
        <span>Follow redirects</span>
      </label>
      <label>
        <span>Maximum redirects</span>
        <input
          type="number"
          min={0}
          value={settings.maxRedirects}
          disabled={!settings.followRedirects}
          onChange={(e) => set({ maxRedirects: Number(e.target.value) || 0 })}
        />
      </label>
      <label className="check">
        <input type="checkbox" checked={settings.verifyTls} onChange={(e) => set({ verifyTls: e.target.checked })} />
        <span>Verify TLS certificates</span>
      </label>
      {!settings.verifyTls && (
        <p className="hint">Any certificate is accepted. Only turn this off for development servers you trust.</p>
      )}
    </div>
  );
}
