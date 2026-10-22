import { save } from "@tauri-apps/plugin-dialog";
import { Bot, Copy, Download } from "lucide-react";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import { CodeEditor } from "./CodeEditor";

const FORMATS: { id: api.DocsFormat; label: string; extension: string }[] = [
  { id: "openapi", label: "OpenAPI YAML", extension: "yaml" },
  { id: "openapi-json", label: "OpenAPI JSON", extension: "json" },
  { id: "markdown", label: "Markdown", extension: "md" },
  { id: "html", label: "HTML page", extension: "html" },
];

/** Generates API docs for a request, folder, collection or the whole workspace, from what the requests say. */
export function DocsDialog({ scope }: { scope: string | null }) {
  const workspace = useStore((s) => s.workspace);
  const environments = useStore((s) => s.environments);
  const active = useStore((s) => s.environment);
  const [format, setFormat] = useState<api.DocsFormat>("openapi");
  const [environment, setEnvironment] = useState<string | null>(active);
  const [docs, setDocs] = useState<{ text: string; warnings: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"source" | "preview">("source");
  const [copied, setCopied] = useState<string | null>(null);
  const close = () => useStore.setState({ docs: null });
  const name = scope ? scope.replace(/\.yaml$/, "").split("/").pop()! : (workspace?.name ?? "Workspace");
  const what = scope === null ? "the whole workspace" : scope.endsWith(".yaml") ? "this request" : "everything in it";

  useEffect(() => {
    setError(null);
    api.docsGenerate(scope, format, environment).then(setDocs, (e) => {
      setDocs(null);
      setError(String(e));
    });
  }, [scope, format, environment]);

  const copy = (text: string, label: string) =>
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(label);
      setTimeout(() => setCopied(null), 1500);
    });
  const saveFile = async () => {
    const f = FORMATS.find((f) => f.id === format)!;
    const file = await save({
      title: "Save the API docs",
      defaultPath: `${name}${format.startsWith("openapi") ? ".openapi" : ""}.${f.extension}`,
      filters: [{ name: f.label, extensions: [f.extension] }],
    });
    if (!file) return;
    try {
      await api.docsGenerate(scope, format, environment, file);
      useStore.getState().notify({ tone: "info", text: `Saved the API docs to ${file}.` });
      close();
    } catch (e) {
      setError(String(e));
    }
  };
  const prompt = agentPrompt(scope, name, format);

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <div
        className="modal docs-modal"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && close()}
        role="dialog"
        aria-label="Generate API docs"
      >
        <h2>
          API docs for {name} <span className="muted small">({what})</span>
        </h2>
        <div className="docs-options">
          <div className="segmented">
            {FORMATS.map((f) => (
              <button key={f.id} className={format === f.id ? "active" : ""} onClick={() => setFormat(f.id)}>
                {f.label}
              </button>
            ))}
          </div>
          <label className="example-field">
            Base URL from
            <select value={environment ?? ""} onChange={(e) => setEnvironment(e.target.value || null)}>
              <option value="">No environment</option>
              {environments.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          {format === "html" && (
            <div className="segmented">
              {(["source", "preview"] as const).map((v) => (
                <button key={v} className={view === v ? "active" : ""} onClick={() => setView(v)}>
                  {v === "source" ? "Source" : "Preview"}
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="docs-output">
          {error ? (
            <p className="notice error">{error}</p>
          ) : format === "html" && view === "preview" && docs ? (
            <iframe className="html-preview docs-preview" sandbox="allow-scripts" srcDoc={docs.text} title="Preview" />
          ) : (
            <div className="editor-frame">
              <CodeEditor
                value={docs?.text ?? ""}
                readOnly
                language={format === "openapi-json" ? "json" : format === "html" ? "html" : "text"}
              />
            </div>
          )}
        </div>
        {docs && docs.warnings.length > 0 && (
          <ul className="docs-warnings">
            {docs.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        )}
        <div className="docs-agent">
          <Bot size={18} className="muted" />
          <p className="hint">
            Docs are as good as the requests: descriptions and saved examples (their JSON gives the response schemas).
            A coding agent connected to Tuma can write them for you. Copy this prompt into it.
          </p>
          <button className="button small" onClick={() => copy(prompt, "prompt")}>
            {copied === "prompt" ? "Copied" : "Copy agent prompt"}
          </button>
        </div>
        <div className="modal-actions">
          <button className="button" onClick={close}>
            Close
          </button>
          <button className="button" disabled={!docs} onClick={() => docs && copy(docs.text, "docs")}>
            <Copy size={14} />
            {copied === "docs" ? "Copied" : "Copy"}
          </button>
          <button className="button primary" disabled={!docs} onClick={() => void saveFile()}>
            <Download size={14} />
            Save as file…
          </button>
        </div>
      </div>
    </div>
  );
}

/** Instructions for a coding agent with Tuma's MCP server: improve the requests, then generate the docs. */
function agentPrompt(scope: string | null, name: string, format: api.DocsFormat): string {
  const target = scope === null ? "every request in the Tuma workspace" : `the Tuma requests in "${scope}"`;
  const file = format.startsWith("openapi") ? "docs/openapi.yaml" : format === "html" ? "docs/api.html" : "docs/API.md";
  return [
    `Write API documentation for ${name} using the Tuma MCP tools.`,
    `1. Call list_requests, then read_request for ${target}.`,
    "2. Read this project's code for each endpoint, then call describe_request with a clear Markdown description: what it does, its parameters, its auth, and its error cases.",
    "3. Send each request (send_request) for a success case and the main error cases, and keep each response with save_example.",
    `4. Call generate_docs${scope ? ` with scope "${scope}"` : ""} and format "${format}", and write the result to ${file}.`,
    "Don't put secret values anywhere; refer to them as {{variables}}.",
  ].join("\n");
}
