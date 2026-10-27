import { linter } from "@codemirror/lint";
import { EditorSelection } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { AlignLeft, Check, Copy, Eraser, Minimize2, TriangleAlert, Wrench, X } from "lucide-react";
import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { type Indent, jsonStats, minifyJson, prettyJson, validateJson } from "../json";
import { type Repair, repairJson } from "../jsonRepair";
import { CodeEditor } from "./CodeEditor";
import { formatBytes } from "./ResponsePanel";

interface Saved {
  text: string;
  indent: Indent;
  sortKeys: boolean;
}

// The text stays on this machine between sessions, unless it is too big to keep.
const KEY = "tuma.json";
const KEEP_LIMIT = 1_000_000;
function load(): Saved {
  const fallback: Saved = { text: "", indent: "2", sortKeys: false };
  try {
    return { ...fallback, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Saved>) };
  } catch {
    return fallback;
  }
}

/** Marks the first mistake in the editor, with the message on hover. Made once. */
const mistakes = linter(
  (view) => {
    const text = view.state.doc.toString();
    const error = text.trim() ? validateJson(text) : null;
    if (!error) return [];
    const from = Math.min(error.pos, Math.max(text.length - 1, 0));
    return [{ from, to: Math.min(from + 1, text.length), severity: "error", message: error.message }];
  },
  { delay: 250 },
);

/** Paste JSON to check it, pretty-print it or squash it onto one line. */
export function JsonView() {
  const [saved, setSaved] = useState(load);
  const change = (patch: Partial<Saved>) => setSaved((s) => ({ ...s, ...patch }));
  const [copied, setCopied] = useState(false);
  // What the last repair fixed, until the text changes again.
  const [repaired, setRepaired] = useState<Repair | null>(null);
  const frame = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        localStorage.setItem(KEY, JSON.stringify(saved.text.length <= KEEP_LIMIT ? saved : { ...saved, text: "" }));
      } catch {
        // Not being remembered is fine.
      }
    }, 400);
    return () => clearTimeout(timer);
  }, [saved]);

  const text = useDeferredValue(saved.text);
  const empty = !text.trim();
  const error = useMemo(() => (empty ? null : validateJson(text)), [text, empty]);
  const stats = useMemo(() => (empty || error ? null : jsonStats(text)), [text, empty, error]);
  const size = useMemo(() => new TextEncoder().encode(text).length, [text]);
  const minified = useMemo(() => (stats ? new TextEncoder().encode(minifyJson(text, false)).length : 0), [text, stats]);
  const valid = !empty && !error;

  const goTo = (pos: number) => {
    const dom = frame.current?.querySelector<HTMLElement>(".cm-editor");
    const view = dom ? EditorView.findFromDOM(dom) : null;
    if (!view) return;
    view.dispatch({ selection: EditorSelection.cursor(pos), scrollIntoView: true });
    view.focus();
  };
  const repair = () => {
    const result = repairJson(saved.text, saved.indent);
    if (!result) return;
    setRepaired(result);
    change({ text: result.text });
  };
  /** Replaces the text from a button, which ends what a repair said. */
  const set = (text: string) => {
    setRepaired(null);
    change({ text });
  };
  const copy = () =>
    void navigator.clipboard.writeText(saved.text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });

  return (
    <section className="folder-editor json-view">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">JSON tools</span>
          {!empty && (
            <span className={`pill ${valid ? "json-valid" : "json-invalid"}`}>{valid ? "Valid JSON" : "Invalid JSON"}</span>
          )}
        </div>
      </div>
      <div className="panel-body diff-body">
        <div className="toolbar compare-toolbar">
          <button
            className="button small"
            disabled={!valid}
            title={valid ? "Pretty-print with the chosen indentation" : "Fix the JSON first"}
            onClick={() => set(prettyJson(saved.text, saved.indent, saved.sortKeys))}
          >
            <AlignLeft size={13} />
            Format
          </button>
          <button
            className="button small"
            disabled={!valid}
            title={valid ? "Put it all on one line, without spaces" : "Fix the JSON first"}
            onClick={() => set(minifyJson(saved.text, saved.sortKeys))}
          >
            <Minimize2 size={13} />
            Minify
          </button>
          <button
            className={`button small${error ? " primary" : ""}`}
            disabled={!error}
            title={
              error
                ? "Fix the usual mistakes: trailing or missing commas, single quotes, unquoted keys, comments, Python and JavaScript values, cut-off endings. Ctrl+Z undoes it."
                : "Nothing to repair"
            }
            onClick={repair}
          >
            <Wrench size={13} />
            Repair
          </button>
          <label className="compare-pick">
            <span className="muted">Indent</span>
            <select value={saved.indent} onChange={(e) => change({ indent: e.target.value as Indent })}>
              <option value="2">2 spaces</option>
              <option value="4">4 spaces</option>
              <option value="tab">Tabs</option>
            </select>
          </label>
          <label className="check" title="Format and Minify put object keys in alphabetical order">
            <input type="checkbox" checked={saved.sortKeys} onChange={(e) => change({ sortKeys: e.target.checked })} />
            Sort keys
          </label>
          <span className="diff-actions">
            <button className="button ghost small" onClick={copy} disabled={empty} title="Copy the JSON">
              <Copy size={13} />
              {copied ? "Copied" : "Copy"}
            </button>
            <button className="button ghost small" onClick={() => set("")} disabled={!saved.text} title="Empty the editor">
              <Eraser size={13} />
              Clear
            </button>
          </span>
        </div>
        {repaired && (
          <div className="json-status repaired" role="status">
            <Wrench size={14} />
            <span>
              <strong>
                {repaired.count ? `Repaired ${repaired.count} ${repaired.count === 1 ? "thing" : "things"}:` : "Repaired:"}
              </strong>{" "}
              {repaired.fixes.join("; ")}. Ctrl+Z brings back the original.
            </span>
            <button className="icon-button" aria-label="Dismiss" onClick={() => setRepaired(null)}>
              <X size={13} />
            </button>
          </div>
        )}
        {error && (
          <div className="json-status invalid" role="status">
            <TriangleAlert size={14} />
            <span>
              <strong>
                Line {error.line}, column {error.column}:
              </strong>{" "}
              {error.message}
            </span>
            <button className="link-button" onClick={() => goTo(error.pos)}>
              Go to the mistake
            </button>
            <button className="link-button" onClick={repair}>
              Repair
            </button>
          </div>
        )}
        {stats && (
          <div className="json-status valid" role="status">
            <Check size={14} />
            <span>
              {describe(stats)} · {formatBytes(size)}
              {minified < size && `, ${formatBytes(minified)} minified`}
            </span>
          </div>
        )}
        {empty && <p className="hint">Paste or drop JSON below: it is checked as you type, and mistakes are marked where they are.</p>}
        <div className="editor-frame json-editor" ref={frame}>
          <CodeEditor
            value={saved.text}
            onChange={(t) => {
              setRepaired(null);
              change({ text: t });
            }}
            language="json"
            placeholder='{"paste": "JSON here"}'
            extension={mistakes}
          />
        </div>
      </div>
    </section>
  );
}

function describe(stats: ReturnType<typeof jsonStats>): string {
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  if (stats.root !== "object" && stats.root !== "array") return `A ${stats.root}`;
  const parts = [plural(stats.keys, "key")];
  if (stats.objects > (stats.root === "object" ? 1 : 0)) parts.push(plural(stats.objects - (stats.root === "object" ? 1 : 0), "nested object"));
  if (stats.arrays) parts.push(plural(stats.arrays, "array"));
  return `An ${stats.root}: ${parts.join(", ")}, ${stats.depth} ${stats.depth === 1 ? "level" : "levels"} deep`;
}
