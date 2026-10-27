import { MergeView } from "@codemirror/merge";
import { EditorView, placeholder } from "@codemirror/view";
import { ArrowLeftRight, Braces, Eraser } from "lucide-react";
import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { changeCount, detectLanguage, formatJson, ignoredNames } from "../compare";
import { type Language, viewerExtensions } from "./CodeEditor";

type Choice = "auto" | Language;

const LANGUAGES: { id: Choice; label: string }[] = [
  { id: "auto", label: "Detect" },
  { id: "json", label: "JSON" },
  { id: "xml", label: "XML" },
  { id: "html", label: "HTML" },
  { id: "javascript", label: "JavaScript" },
  { id: "text", label: "Plain text" },
];

interface Saved {
  left: string;
  right: string;
  language: Choice;
  sortKeys: boolean;
  ignore: string;
}

// The texts stay on this machine between sessions, unless they are too big to keep.
const KEY = "tuma.diff";
const KEEP_LIMIT = 1_000_000;
function load(): Saved {
  const fallback: Saved = { left: "", right: "", language: "auto", sortKeys: true, ignore: "" };
  try {
    return { ...fallback, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Saved>) };
  } catch {
    return fallback;
  }
}
function store(saved: Saved) {
  try {
    const small = saved.left.length + saved.right.length <= KEEP_LIMIT;
    localStorage.setItem(KEY, JSON.stringify(small ? saved : { ...saved, left: "", right: "" }));
  } catch {
    // Not being remembered is fine.
  }
}

/** Two texts pasted side by side, with what differs between them marked as in a diff tool. Both can be edited. */
export function DiffView() {
  const [saved, setSaved] = useState(load);
  const change = (patch: Partial<Saved>) => setSaved((s) => ({ ...s, ...patch }));
  // Bumped when the texts change from outside the editors (format, swap, clear), so they are made again.
  const [version, setVersion] = useState(0);
  const texts = useRef({ left: saved.left, right: saved.right });
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = setTimeout(() => store(saved), 400);
    return () => clearTimeout(timer);
  }, [saved]);

  const detected = detectLanguage(saved.left) !== "text" ? detectLanguage(saved.left) : detectLanguage(saved.right);
  const language: Language = saved.language === "auto" ? detected : saved.language;

  useEffect(() => {
    const side = (which: "left" | "right", hint: string) => ({
      doc: texts.current[which],
      extensions: [
        viewerExtensions(language, true),
        placeholder(hint),
        EditorView.contentAttributes.of({ "aria-label": which === "left" ? "Original text" : "Changed text" }),
        EditorView.updateListener.of((u) => {
          if (!u.docChanged) return;
          const text = u.state.doc.toString();
          texts.current[which] = text;
          change({ [which]: text });
        }),
      ],
    });
    const view = new MergeView({
      a: side("left", "Paste or drop the original here: text, JSON, XML, HTML or anything else"),
      b: side("right", "Paste or drop the changed version here"),
      parent: host.current!,
      gutter: true,
    });
    return () => view.destroy();
  }, [language, version]);

  /** Replaces both texts, such as after formatting. */
  const replace = (left: string, right: string) => {
    texts.current = { left, right };
    change({ left, right });
    setVersion((v) => v + 1);
  };

  const options = { ignore: ignoredNames(saved.ignore), sortKeys: saved.sortKeys };
  const formattedLeft = formatJson(saved.left, options);
  const formattedRight = formatJson(saved.right, options);
  const canFormat = (formattedLeft !== null || !saved.left.trim()) && (formattedRight !== null || !saved.right.trim()) &&
    (formattedLeft !== null || formattedRight !== null);

  const left = useDeferredValue(saved.left);
  const right = useDeferredValue(saved.right);
  const changes = useMemo(() => changeCount(left, right), [left, right]);
  const empty = !saved.left.trim() || !saved.right.trim();

  return (
    <section className="folder-editor diff-view">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Compare text</span>
          {!empty && (
            <span className={`pill${changes ? " diff-pill" : ""}`}>
              {changes === 0 ? "Identical" : `${changes} ${changes === 1 ? "difference" : "differences"}`}
            </span>
          )}
        </div>
      </div>
      <div className="panel-body diff-body">
        <div className="toolbar compare-toolbar">
          <label className="compare-pick">
            <span className="muted">Format</span>
            <select value={saved.language} onChange={(e) => change({ language: e.target.value as Choice })}>
              {LANGUAGES.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.id === "auto" ? `Detect (${LANGUAGES.find((x) => x.id === detected)?.label})` : l.label}
                </option>
              ))}
            </select>
          </label>
          <button
            className="button ghost small"
            disabled={!canFormat}
            title={
              canFormat
                ? "Pretty-print both sides as JSON, so only real differences show"
                : "Both sides need to be valid JSON"
            }
            onClick={() => replace(formattedLeft ?? saved.left, formattedRight ?? saved.right)}
          >
            <Braces size={13} />
            Format JSON
          </button>
          <label className="check" title="When formatting, sort object keys, so the same fields in another order are no difference">
            <input type="checkbox" checked={saved.sortKeys} onChange={(e) => change({ sortKeys: e.target.checked })} />
            Sort keys
          </label>
          <input
            className="compare-ignore mono"
            value={saved.ignore}
            placeholder="Leave out keys: id, timestamp"
            aria-label="JSON keys to leave out when formatting"
            title="JSON keys (at any depth) that formatting leaves out, such as ones that always differ"
            spellCheck={false}
            onChange={(e) => change({ ignore: e.target.value })}
          />
          <span className="diff-actions">
            <button
              className="button ghost small"
              title="Swap the two sides"
              onClick={() => replace(saved.right, saved.left)}
            >
              <ArrowLeftRight size={13} />
              Swap
            </button>
            <button className="button ghost small" title="Empty both sides" onClick={() => replace("", "")} disabled={!saved.left && !saved.right}>
              <Eraser size={13} />
              Clear
            </button>
          </span>
        </div>
        <div className="diff-labels">
          <span>Original</span>
          <span>Changed</span>
        </div>
        <div className="editor-frame compare-diff diff-editors" ref={host} />
      </div>
    </section>
  );
}
