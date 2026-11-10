import { isolateHistory } from "@codemirror/commands";
import type { ChangeSpec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { ArrowLeftToLine, Code, Columns2, Combine, Copy, Eraser, Eye, Link2, PencilLine, Sparkles, WrapText, X } from "lucide-react";
import { type ClipboardEvent, type MouseEvent, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  alignLeft,
  type Blocks,
  type Cleaned,
  cleanMarkdown,
  joinLines,
  markdownStats,
  renderBlocks,
  renderMarkdown,
  toMarkdown,
} from "../markdown";
import { useStore } from "../store";
import { CodeEditor } from "./CodeEditor";

type Mode = "split" | "edit" | "preview";

interface Saved {
  text: string;
  mode: Mode;
  cleanOnPaste: boolean;
  wrap: boolean;
  syncScroll: boolean;
}

// The text stays on this machine between sessions, unless it is too big to keep.
const KEY = "tuma.markdown";
const KEEP_LIMIT = 1_000_000;
function load(): Saved {
  const fallback: Saved = { text: "", mode: "split", cleanOnPaste: true, wrap: true, syncScroll: true };
  try {
    return { ...fallback, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Saved>) };
  } catch {
    return fallback;
  }
}

const MODES: { id: Mode; label: string; icon: typeof Eye }[] = [
  { id: "edit", label: "Editor", icon: PencilLine },
  { id: "split", label: "Both", icon: Columns2 },
  { id: "preview", label: "Preview", icon: Eye },
];

/**
 * Cleans what is pasted, as its own step after the paste, so Ctrl+Z brings back the text as it was pasted.
 * `enabled` and `report` are read when a paste happens, so the extension is made once.
 */
function cleanPastes(enabled: () => boolean, report: (fixes: string[]) => void) {
  return EditorView.updateListener.of((update) => {
    if (!enabled() || !update.docChanged || !update.transactions.some((t) => t.isUserEvent("input.paste"))) return;
    const changes: ChangeSpec[] = [];
    const fixes: string[] = [];
    update.changes.iterChangedRanges((_fromA, _toA, from, to) => {
      const pasted = update.state.sliceDoc(from, to);
      const cleaned = cleanMarkdown(pasted);
      // Cleaning ends the text with a line break; a paste into the middle of a line shouldn't gain one.
      const text = !pasted.endsWith("\n") && cleaned.text.endsWith("\n") ? cleaned.text.slice(0, -1) : cleaned.text;
      if (text === pasted) return;
      changes.push({ from, to, insert: text });
      fixes.push(...cleaned.fixes);
    });
    if (!changes.length) return;
    const view = update.view;
    // An editor can't be changed while it reports a change.
    queueMicrotask(() => {
      report(fixes);
      view.dispatch({ changes, annotations: isolateHistory.of("full"), userEvent: "input.paste.clean" });
    });
  });
}

/** Paste Markdown to tidy it, edit it and see it rendered. */
export function MarkdownView() {
  const [saved, setSaved] = useState(load);
  const change = (patch: Partial<Saved>) => setSaved((s) => ({ ...s, ...patch }));
  const [copied, setCopied] = useState<"markdown" | "html" | null>(null);
  // What the last clean changed, until the text changes again.
  const [cleaned, setCleaned] = useState<{ label: string; fixes: string[]; pasted: boolean } | null>(null);
  // Set when a paste is cleaned, so the change it makes doesn't dismiss its own message.
  const keepMessage = useRef(false);

  const cleanOnPaste = useRef(saved.cleanOnPaste);
  cleanOnPaste.current = saved.cleanOnPaste;
  const extension = useMemo(
    () =>
      cleanPastes(
        () => cleanOnPaste.current,
        (fixes) => {
          keepMessage.current = true;
          setCleaned({ label: "Cleaned the paste", fixes, pasted: true });
        },
      ),
    [],
  );

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
  const rendered = useMemo(() => (saved.mode === "edit" ? null : renderBlocks(text)), [text, saved.mode]);
  const stats = useMemo(() => (empty ? null : markdownStats(text)), [text, empty]);
  const tidy = useMemo(() => (empty ? null : cleanMarkdown(saved.text)), [saved.text, empty]);
  const needsCleaning = !!tidy && tidy.fixes.length > 0;
  const aligned = useMemo(() => (empty ? null : alignLeft(saved.text)), [saved.text, empty]);
  const joined = useMemo(() => (empty ? null : joinLines(saved.text)), [saved.text, empty]);

  /** Replaces the text with what an edit made of it, saying what changed; Ctrl+Z undoes it. */
  const apply = (label: string, result: Cleaned | null) => {
    if (!result || !result.fixes.length) return;
    setCleaned({ label, fixes: result.fixes, pasted: false });
    change({ text: result.text });
  };
  const clean = () => apply("Cleaned", tidy);
  /** Replaces the text from a button, which ends what a clean said. */
  const set = (text: string) => {
    setCleaned(null);
    change({ text });
  };
  const copy = (what: "markdown" | "html") =>
    void navigator.clipboard.writeText(what === "html" ? renderMarkdown(saved.text) : saved.text).then(() => {
      setCopied(what);
      setTimeout(() => setCopied(null), 1500);
    });

  // The preview can be edited too. Its HTML is set here rather than by React, so the caret stays put while typing in
  // it: what it shows is the blocks in `shown`, and `snapshot` is each block element's HTML as it was set, to tell
  // which blocks were edited. Only those are turned back into Markdown; the others keep their source as it was.
  const preview = useRef<HTMLDivElement>(null);
  const editorFrame = useRef<HTMLDivElement>(null);
  const shown = useRef<Blocks>({ lead: "", blocks: [] });
  const snapshot = useRef(new WeakMap<Element, string>());
  // Whether the text last changed by typing in the preview, which already shows it.
  const fromPreview = useRef(false);
  // When the preview was last typed in: the editor moving because of it shouldn't scroll the preview.
  const typedInPreview = useRef(0);
  const [redraw, setRedraw] = useState(0);

  useLayoutEffect(() => {
    const el = preview.current;
    if (!el || !rendered || fromPreview.current) return;
    el.innerHTML = rendered.blocks.map((b, i) => `<div data-block="${i}">${b.html}</div>`).join("");
    shown.current = rendered;
    const html = new WeakMap<Element, string>();
    for (const child of Array.from(el.children)) html.set(child, child.innerHTML);
    snapshot.current = html;
  }, [rendered, redraw, saved.mode]);

  /** The preview's content as Markdown. */
  const fromHtml = () => {
    const { lead, blocks } = shown.current;
    let out = lead;
    for (const node of Array.from(preview.current!.childNodes)) {
      const block = node instanceof HTMLElement ? node.dataset.block : undefined;
      if (block !== undefined && snapshot.current.get(node as Element) === (node as Element).innerHTML && blocks[+block]) {
        out += blocks[+block].raw + blocks[+block].tail;
        continue;
      }
      const html =
        node instanceof HTMLElement
          ? block !== undefined
            ? node.innerHTML
            : node.outerHTML
          : (node.textContent ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;");
      const markdown = toMarkdown(html);
      if (!markdown) continue;
      out = out.trim() ? out.replace(/\n*$/, "\n\n") : "";
      out += `${markdown}\n\n`;
      // Link definitions after an edited block stay, as other blocks may use them.
      const definitions = block !== undefined ? blocks[+block]?.tail.trim() : "";
      if (definitions) out += `${definitions}\n\n`;
    }
    return out.trim() ? out.replace(/\s*$/, "\n") : "";
  };
  const onPreviewInput = () => {
    fromPreview.current = true;
    typedInPreview.current = Date.now();
    setCleaned(null);
    change({ text: fromHtml() });
  };
  // Once done typing in it, the preview is drawn again from the Markdown, with blocks that match it.
  const onPreviewBlur = () => {
    if (!fromPreview.current) return;
    fromPreview.current = false;
    setRedraw((n) => n + 1);
  };
  // Formatting from elsewhere would come along with a paste, and anything it brings: only the text is pasted.
  const onPreviewPaste = (e: ClipboardEvent<HTMLDivElement>) => {
    e.preventDefault();
    document.execCommand("insertText", false, e.clipboardData.getData("text/plain"));
  };
  // A link in the preview would take the whole window away. Ctrl+click opens a web link as a new request instead,
  // like in responses; a plain click puts the caret there, to edit it.
  const onPreviewClick = (e: MouseEvent<HTMLDivElement>) => {
    const link = (e.target as HTMLElement).closest("a");
    if (!link) return;
    e.preventDefault();
    const href = link.getAttribute("href") ?? "";
    if ((e.ctrlKey || e.metaKey) && /^https?:\/\//i.test(href)) useStore.getState().openUrl(href);
  };

  // Scrolling one side scrolls the other to the same place, block by block: each block's top in the editor is matched
  // with its top in the preview, and positions between two blocks are matched in proportion.
  useEffect(() => {
    if (!saved.syncScroll || saved.mode !== "split") return;
    const dom = editorFrame.current?.querySelector<HTMLElement>(".cm-editor");
    const view = dom ? EditorView.findFromDOM(dom) : null;
    const pane = preview.current?.parentElement;
    if (!view || !pane) return;
    const editor = view.scrollDOM;

    /** Matching positions, [editor, preview], increasing in both. */
    const anchors = () => {
      const points: [number, number][] = [[0, 0]];
      const length = view.state.doc.length;
      const editorEnd = editor.scrollHeight - editor.clientHeight;
      const previewEnd = pane.scrollHeight - pane.clientHeight;
      shown.current.blocks.forEach((block, i) => {
        const el = pane.querySelector<HTMLElement>(`[data-block="${i}"]`);
        if (!el) return;
        const point: [number, number] = [view.lineBlockAt(Math.min(block.from, length)).top, el.offsetTop];
        const last = points[points.length - 1];
        if (point[0] > last[0] && point[1] > last[1] && point[0] < editorEnd && point[1] < previewEnd) points.push(point);
      });
      const last = points[points.length - 1];
      if (editorEnd > last[0] && previewEnd > last[1]) points.push([editorEnd, previewEnd]);
      return points;
    };
    const map = (y: number, from: 0 | 1) => {
      const to = from === 0 ? 1 : 0;
      const points = anchors();
      for (let i = points.length - 1; i >= 0; i--) {
        const a = points[i];
        if (y < a[from] && i > 0) continue;
        const b = points[i + 1];
        if (!b) return a[to];
        return a[to] + ((y - a[from]) / (b[from] - a[from])) * (b[to] - a[to]);
      }
      return 0;
    };

    // The side being scrolled leads until it has been still for a moment, so the scrolling it causes on the other
    // side isn't sent back to it.
    let leader: HTMLElement | null = null;
    let release = 0;
    const follow = (from: HTMLElement) => () => {
      if (leader && leader !== from) return;
      if (from === editor && Date.now() - typedInPreview.current < 300) return;
      leader = from;
      clearTimeout(release);
      release = window.setTimeout(() => (leader = null), 120);
      if (from === editor) pane.scrollTop = map(editor.scrollTop, 0);
      else editor.scrollTop = map(pane.scrollTop, 1);
    };
    const onEditor = follow(editor);
    const onPane = follow(pane);
    editor.addEventListener("scroll", onEditor, { passive: true });
    pane.addEventListener("scroll", onPane, { passive: true });
    return () => {
      clearTimeout(release);
      editor.removeEventListener("scroll", onEditor);
      pane.removeEventListener("scroll", onPane);
    };
  }, [saved.syncScroll, saved.mode]);

  const showEditor = saved.mode !== "preview";
  const showPreview = saved.mode !== "edit";

  return (
    <section className="folder-editor json-view markdown-view">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Markdown</span>
          {stats && (
            <span className="muted markdown-stats">
              {stats.words.toLocaleString()} {stats.words === 1 ? "word" : "words"} · {Math.max(1, Math.round(stats.words / 230))} min
              read
              {stats.headings > 0 && ` · ${stats.headings} ${stats.headings === 1 ? "heading" : "headings"}`}
              {stats.links > 0 && ` · ${stats.links} ${stats.links === 1 ? "link" : "links"}`}
              {stats.codeBlocks > 0 && ` · ${stats.codeBlocks} code ${stats.codeBlocks === 1 ? "block" : "blocks"}`}
            </span>
          )}
        </div>
      </div>
      <div className="panel-body diff-body">
        <div className="toolbar compare-toolbar">
          <button
            className={`button small${needsCleaning ? " primary" : ""}`}
            disabled={!needsCleaning}
            title={
              needsCleaning
                ? `Tidy it up: ${tidy!.fixes.join("; ")}. Ctrl+Z undoes it.`
                : empty
                  ? "Paste some Markdown first"
                  : "Already tidy"
            }
            onClick={clean}
          >
            <Sparkles size={13} />
            Clean
          </button>
          <label
            className="check"
            title="Tidy pasted Markdown as it comes in: line endings, invisible characters, odd spaces and bullets, headings, blank lines and code blocks. Ctrl+Z brings back the paste as it was."
          >
            <input type="checkbox" checked={saved.cleanOnPaste} onChange={(e) => change({ cleanOnPaste: e.target.checked })} />
            Clean on paste
          </label>
          <button
            className="button small"
            disabled={!aligned?.fixes.length}
            title={
              aligned?.fixes.length
                ? "Bring indented lines back to the left edge, so they aren't shown as code. Nested lists keep their nesting."
                : "Every line already starts at the edge"
            }
            onClick={() => apply("Aligned", aligned)}
          >
            <ArrowLeftToLine size={13} />
            Align left
          </button>
          <button
            className="button small"
            disabled={!joined?.fixes.length}
            title={
              joined?.fixes.length
                ? "Join lines broken in the middle of a sentence (as text from PDFs and emails often is), so each paragraph and list item is one line"
                : "No broken lines to join"
            }
            onClick={() => apply("Joined", joined)}
          >
            <Combine size={13} />
            Join lines
          </button>
          <label className="check" title="Wrap long lines in the editor and in the preview's code blocks, rather than scrolling sideways">
            <input type="checkbox" checked={saved.wrap} onChange={(e) => change({ wrap: e.target.checked })} />
            <WrapText size={13} />
            Word wrap
          </label>
          <label
            className="check"
            title={
              saved.mode === "split"
                ? "Scrolling the editor scrolls the preview to the same place, and the other way round"
                : "Scroll the editor and the preview together, when both are shown"
            }
          >
            <input type="checkbox" checked={saved.syncScroll} onChange={(e) => change({ syncScroll: e.target.checked })} />
            <Link2 size={13} />
            Sync scroll
          </label>
          <div className="segmented markdown-modes" role="group" aria-label="What to show">
            {MODES.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                className={saved.mode === id ? "active" : ""}
                aria-pressed={saved.mode === id}
                onClick={() => change({ mode: id })}
                title={id === "split" ? "The editor and the preview side by side" : `Only the ${label.toLowerCase()}`}
              >
                <Icon size={13} />
                {label}
              </button>
            ))}
          </div>
          <span className="diff-actions">
            <button className="button ghost small" onClick={() => copy("markdown")} disabled={empty} title="Copy the Markdown">
              <Copy size={13} />
              {copied === "markdown" ? "Copied" : "Copy"}
            </button>
            <button className="button ghost small" onClick={() => copy("html")} disabled={empty} title="Copy it rendered, as HTML">
              <Code size={13} />
              {copied === "html" ? "Copied" : "Copy HTML"}
            </button>
            <button className="button ghost small" onClick={() => set("")} disabled={!saved.text} title="Empty the editor">
              <Eraser size={13} />
              Clear
            </button>
          </span>
        </div>
        {cleaned && (
          <div className="json-status repaired" role="status">
            <Sparkles size={14} />
            <span>
              <strong>{cleaned.label}:</strong> {cleaned.fixes.join("; ")}.{" "}
              {cleaned.pasted ? "Ctrl+Z brings back the paste as it was." : "Ctrl+Z brings back the original."}
            </span>
            <button className="icon-button" aria-label="Dismiss" onClick={() => setCleaned(null)}>
              <X size={13} />
            </button>
          </div>
        )}
        {empty && (
          <p className="hint">
            Paste Markdown below: {saved.cleanOnPaste ? "it is tidied as it comes in, and " : ""}the preview follows as you type.
            You can type in the preview too, and the Markdown follows.
          </p>
        )}
        <div className={`markdown-panes ${saved.mode}`}>
          {showEditor && (
            <div className="editor-frame json-editor" ref={editorFrame}>
              <CodeEditor
                value={saved.text}
                onChange={(t) => {
                  if (keepMessage.current) keepMessage.current = false;
                  else setCleaned(null);
                  change({ text: t });
                }}
                language="markdown"
                placeholder="# Paste Markdown here"
                extension={extension}
                wrap={saved.wrap}
              />
            </div>
          )}
          {showPreview && (
            <div className="editor-frame markdown-preview-frame">
              <div
                ref={preview}
                className={`markdown-preview${saved.wrap ? " wrap" : ""}`}
                contentEditable
                suppressContentEditableWarning
                spellCheck
                role="textbox"
                aria-multiline
                aria-label="Preview, which can be edited"
                data-placeholder="The preview shows here. You can type here too."
                title="Edit here and the Markdown follows. Ctrl+click a link to open it."
                onInput={onPreviewInput}
                onBlur={onPreviewBlur}
                onPaste={onPreviewPaste}
                onClick={onPreviewClick}
              />
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
