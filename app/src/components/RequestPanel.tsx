import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { type BodyMode, type Draft, isGrpc, isTextMode, isWebSocket, toRequestFile } from "../draft";
import { formatJsonc } from "../jsonc";
import { useStore } from "../store";
import type { KeyValue } from "../types";
import { AuthForm } from "./AuthForm";
import { CryptoEditor } from "./CryptoEditor";
import { ExamplesEditor } from "./ExamplesEditor";
import { CodeEditor, type Language } from "./CodeEditor";
import { blankPart, blankRow, KeyValueEditor } from "./KeyValueEditor";
import { GraphqlEditor } from "./GraphqlEditor";
import { GrpcMessage, GrpcProtos } from "./GrpcViews";
import { MessageComposer } from "./WebSocketViews";
import { ScriptsEditor } from "./ScriptsEditor";
import { Tabs } from "./Tabs";

type Tab = "params" | "headers" | "body" | "auth" | "scripts" | "crypto" | "examples" | "settings";

const active = (rows: KeyValue[]) => rows.filter((r) => r.enabled && r.name).length;

export function RequestPanel() {
  const [tab, setTab] = useState<Tab>("params");
  const draft = useStore((s) => s.draft);
  if (isWebSocket(draft.url)) return <WebSocketPanel />;
  if (isGrpc(draft.url)) return <GrpcPanel />;

  return (
    <section className="request-panel">
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "params", label: "Params", count: active(draft.params) },
          { id: "headers", label: "Headers", count: active(draft.headers) },
          { id: "body", label: "Body", dot: hasBody(draft) },
          { id: "auth", label: "Auth", dot: !!draft.auth && draft.auth.type !== "none" },
          {
            id: "scripts",
            label: "Scripts",
            count: [draft.scripts.preRequest, draft.scripts.test].filter((s) => s?.trim()).length,
          },
          {
            id: "crypto",
            label: "Crypto",
            count: (draft.crypto?.encrypt?.length ?? 0) + (draft.crypto?.decrypt?.length ?? 0),
          },
          { id: "examples", label: "Examples", count: draft.examples.length },
          { id: "settings", label: "Settings" },
        ]}
      />
      <div className="panel-body">
        {tab === "crypto" && <CryptoEditor key={draft.path ?? ""} />}
        {tab === "examples" && <ExamplesEditor />}
        {tab === "params" && <Params />}
        {tab === "headers" && <Headers />}
        {tab === "body" && <BodyEditor />}
        {tab === "auth" && <AuthEditor />}
        {tab === "scripts" && (
          <ScriptsEditor
            scripts={draft.scripts}
            onChange={(scripts) => useStore.getState().update({ scripts })}
            scope="request"
            path={draft.path}
          />
        )}
        {tab === "settings" && <SettingsEditor />}
      </div>
    </section>
  );
}

/** Whether the body would send anything. */
function hasBody(d: Draft): boolean {
  switch (d.bodyMode) {
    case "none":
      return false;
    case "form":
      return d.form.some((r) => r.enabled && r.name);
    case "multipart":
      return d.parts.some((p) => p.enabled && p.name);
    case "file":
      return !!d.filePath;
    case "graphql":
      return !!d.graphql.query.trim();
    default:
      return !!d.raw.trim();
  }
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
        Headers set on the collection and its folders are sent too, unless you set the same header here. Tuma adds
        User-Agent, Accept, Accept-Encoding and Content-Type unless you set them. The Request tab of the response shows
        exactly what was sent.
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
  { id: "raw", label: "Other text" },
  { id: "form", label: "Form URL-encoded" },
  { id: "multipart", label: "Multipart form" },
  { id: "file", label: "Binary file" },
  { id: "graphql", label: "GraphQL" },
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
        <select
          value={mode}
          onChange={(e) => {
            const bodyMode = e.target.value as BodyMode;
            // GraphQL goes over POST; a GET with a body would be ignored by most servers.
            update(bodyMode === "graphql" && draft.method === "GET" ? { bodyMode, method: "POST" } : { bodyMode });
          }}
          aria-label="Body type"
        >
          {BODY_MODES.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
        {mode === "json" && (
          <button className="link-button" onClick={() => update({ raw: formatJsonc(draft.raw) })}>
            Format
          </button>
        )}
        {mode === "raw" && (
          <input
            className="mono"
            value={draft.rawContentType}
            placeholder="Content type"
            aria-label="Content type"
            onChange={(e) => update({ rawContentType: e.target.value })}
          />
        )}
      </div>

      {mode === "none" && <p className="hint">This request has no body.</p>}
      {mode === "graphql" && <GraphqlEditor />}
      {isTextMode(mode) && (
        <div className="editor-frame">
          <CodeEditor
            value={draft.raw}
            onChange={(raw) => update({ raw })}
            language={mode === "raw" ? "text" : (mode as Language)}
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

function AuthEditor() {
  const draft = useStore((s) => s.draft);
  const auth = draft.auth;
  const saved = draft.path !== null;
  return (
    <AuthForm
      auth={auth}
      onChange={(auth) => useStore.getState().update({ auth })}
      tokenFor={{ request: toRequestFile(draft), path: draft.path }}
      inheritHint={
        saved
          ? "Uses the auth of the nearest folder or collection that sets one."
          : "This request is not saved in a collection, so it has nothing to inherit: no auth is sent."
      }
    />
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

type WsTab = "message" | "params" | "headers" | "auth" | "settings";

/** A WebSocket request: messages instead of a body; the handshake has params, headers and auth. */
function WebSocketPanel() {
  const [tab, setTab] = useState<WsTab>("message");
  const draft = useStore((s) => s.draft);
  return (
    <section className="request-panel">
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "message", label: "Message", count: draft.messages.length },
          { id: "params", label: "Params", count: active(draft.params) },
          { id: "headers", label: "Headers", count: active(draft.headers) },
          { id: "auth", label: "Auth" },
          { id: "settings", label: "Settings" },
        ]}
      />
      <div className="panel-body">
        {tab === "message" && <MessageComposer />}
        {tab === "params" && <Params />}
        {tab === "headers" && <Headers />}
        {tab === "auth" && <AuthEditor />}
        {tab === "settings" && <SettingsEditor />}
      </div>
    </section>
  );
}

type GrpcTab = "message" | "metadata" | "auth" | "protos" | "settings";

/** A gRPC request: the method and its message; headers are sent as metadata. */
function GrpcPanel() {
  const [tab, setTab] = useState<GrpcTab>("message");
  const draft = useStore((s) => s.draft);
  return (
    <section className="request-panel">
      <Tabs
        active={tab}
        onChange={setTab}
        tabs={[
          { id: "message", label: "Message" },
          { id: "metadata", label: "Metadata", count: active(draft.headers) },
          { id: "auth", label: "Auth" },
          { id: "protos", label: "Proto files", count: draft.grpc.protos?.length ?? 0 },
          { id: "settings", label: "Settings" },
        ]}
      />
      <div className="panel-body">
        {tab === "message" && <GrpcMessage />}
        {tab === "metadata" && <Headers />}
        {tab === "auth" && <AuthEditor />}
        {tab === "protos" && <GrpcProtos />}
        {tab === "settings" && <SettingsEditor />}
      </div>
    </section>
  );
}
