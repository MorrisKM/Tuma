import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { formatJsonc } from "../jsonc";
import { useStore } from "../store";
import type { Example } from "../types";
import { CodeEditor, type Language } from "./CodeEditor";

/** The request's saved responses: what it documents and what mock servers answer with. */
export function ExamplesEditor() {
  const examples = useStore((s) => s.draft.examples);
  const [selected, setSelected] = useState(0);
  const set = (examples: Example[]) => useStore.getState().update({ examples });
  const current = examples[Math.min(selected, examples.length - 1)];
  const index = examples.indexOf(current);
  const patch = (p: Partial<Example>) => set(examples.map((e, i) => (i === index ? { ...e, ...p } : e)));
  const add = () => {
    set([
      ...examples,
      {
        name: uniqueName(examples, "Example"),
        status: 200,
        headers: [{ name: "Content-Type", value: "application/json" }],
        body: "{\n  \n}",
      },
    ]);
    setSelected(examples.length);
  };

  if (examples.length === 0) {
    return (
      <div className="examples-empty">
        <p className="hint">
          Examples are responses kept with the request. They show up in generated API docs, and mock servers answer
          with them. Send the request and choose <strong>Save as example</strong> on the response, or write one.
        </p>
        <button className="button" onClick={add}>
          <Plus size={14} />
          New example
        </button>
      </div>
    );
  }

  const type = contentType(current);
  return (
    <div className="examples">
      <div className="examples-list">
        {examples.map((e, i) => (
          <button key={i} className={`list-item${i === index ? " selected" : ""}`} onClick={() => setSelected(i)}>
            <span className={`history-status ${e.status < 300 ? "ok" : e.status < 400 ? "redirect" : "fail"}`}>
              {e.status}
            </span>
            <span className="list-item-text">{e.name}</span>
          </button>
        ))}
        <button className="link-button examples-add" onClick={add}>
          + New example
        </button>
      </div>
      <div className="example-detail">
        <div className="toolbar">
          <input
            className="example-name"
            value={current.name}
            aria-label="Example name"
            onChange={(e) => patch({ name: e.target.value })}
          />
          <label className="example-field">
            Status
            <input
              type="number"
              min={100}
              max={599}
              value={current.status}
              onChange={(e) => patch({ status: Number(e.target.value) || 200 })}
            />
          </label>
          <label className="example-field">
            Content type
            <input
              className="mono"
              value={type}
              placeholder="application/json"
              onChange={(e) => patch({ headers: withContentType(current, e.target.value) })}
            />
          </label>
          {type.includes("json") && (
            <button className="link-button" onClick={() => patch({ body: formatJsonc(current.body ?? "") })}>
              Format
            </button>
          )}
          <button
            className="icon-button"
            title="Delete this example"
            aria-label="Delete this example"
            onClick={() => set(examples.filter((_, i) => i !== index))}
          >
            <Trash2 size={15} />
          </button>
        </div>
        <div className="editor-frame">
          <CodeEditor
            key={index}
            value={current.body ?? ""}
            onChange={(body) => patch({ body })}
            language={languageOf(type)}
            placeholder="The response body"
          />
        </div>
      </div>
    </div>
  );
}

const contentType = (e: Example) =>
  e.headers?.find((h) => h.enabled !== false && h.name.toLowerCase() === "content-type")?.value ?? "";

function withContentType(e: Example, value: string) {
  const others = (e.headers ?? []).filter((h) => h.name.toLowerCase() !== "content-type");
  return value.trim() ? [{ name: "Content-Type", value }, ...others] : others;
}

function languageOf(type: string): Language {
  if (type.includes("json")) return "json";
  if (type.includes("html")) return "html";
  if (type.includes("xml")) return "xml";
  return "text";
}

/** "200 OK", or "200 OK (2)" if that is taken. */
export function uniqueName(examples: Example[], base: string): string {
  const taken = new Set(examples.map((e) => e.name));
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base} (${n})`)) n++;
  return `${base} (${n})`;
}
