import { defaultKeymap, history, historyKeymap, indentWithTab, isolateHistory } from "@codemirror/commands";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json, jsonLanguage } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { xml } from "@codemirror/lang-xml";
import { bracketMatching, foldGutter, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import {
  getSearchQuery,
  highlightSelectionMatches,
  openSearchPanel,
  search,
  searchKeymap,
  searchPanelOpen,
} from "@codemirror/search";
import { Annotation, Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, placeholder as placeholderExt, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useEffect, useRef } from "react";
import { jsonComments } from "../jsonc";

export type Language = "json" | "xml" | "html" | "javascript" | "markdown" | "text";

/** Marks text set from outside the editor, which is not reported back as an edit. */
export const external = Annotation.define<boolean>();

const languages: Record<Language, () => Extension> = {
  // Comments are allowed (and removed before sending), so Ctrl+/ comments out a line.
  json: () => [
    json(),
    jsonLanguage.data.of({ commentTokens: { line: "//", block: { open: "/*", close: "*/" } } }),
    jsonComments,
  ],
  xml: () => xml(),
  html: () => html(),
  javascript: () => javascript(),
  markdown: () => markdown(),
  text: () => [],
};

// Colors come from CSS variables so the editor follows the light and dark themes.
const highlight = HighlightStyle.define([
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--syntax-number)" },
  { tag: [tags.propertyName, tags.attributeName], color: "var(--syntax-property)" },
  { tag: [tags.keyword, tags.tagName, tags.typeName], color: "var(--syntax-keyword)" },
  { tag: [tags.comment], color: "var(--muted)", fontStyle: "italic" },
  { tag: tags.heading, color: "var(--syntax-keyword)", fontWeight: "600" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: [tags.link, tags.url], color: "var(--syntax-property)" },
  { tag: tags.monospace, color: "var(--syntax-string)" },
  { tag: [tags.processingInstruction, tags.contentSeparator, tags.quote], color: "var(--muted)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "transparent", color: "var(--text)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)", fontSize: "12.5px", lineHeight: "1.6" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "var(--faint)" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 10px" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "transparent" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--bg-active)", border: "none", color: "var(--muted)" },
  ".cm-cursor": { borderLeftColor: "var(--text)" },
  // CodeMirror's default caret is black unless an editor is marked dark; this one follows the app's theme.
  ".cm-content": { caretColor: "var(--text)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--selection) !important",
  },
  ".cm-panels": { backgroundColor: "var(--bg-sidebar)", color: "var(--text)" },
  ".cm-placeholder": { color: "var(--muted)" },
});

interface Props {
  value: string;
  onChange?: (value: string) => void;
  language?: Language;
  readOnly?: boolean;
  placeholder?: string;
  /** More extensions, such as a language with a schema. Reconfigured when it changes. */
  extension?: Extension;
  /**
   * Which document is shown, such as the open tab. When it changes, the text is replaced and the undo history starts
   * afresh, so Ctrl+Z never brings back another document's text.
   */
  doc?: unknown;
  /** Whether long lines wrap; otherwise they scroll sideways. */
  wrap?: boolean;
}

/** Counting stops here, so a search in a huge response stays quick. */
const COUNT_LIMIT = 10_000;

/** Shows "3 of 12" in the search panel: how many matches the search has, and which one is selected. */
const searchCount = ViewPlugin.fromClass(
  class {
    label: HTMLElement | null = null;
    constructor(view: EditorView) {
      this.later(view);
    }
    update(update: ViewUpdate) {
      const changed =
        update.docChanged ||
        update.selectionSet ||
        getSearchQuery(update.state) !== getSearchQuery(update.startState) ||
        searchPanelOpen(update.state) !== searchPanelOpen(update.startState);
      if (changed) this.later(update.view);
    }
    /** After the update, when the search panel has been put in or taken out. */
    later(view: EditorView) {
      view.requestMeasure({ read: () => null, write: () => this.show(view) });
    }
    show(view: EditorView) {
      const panel = view.dom.querySelector(".cm-search");
      if (!panel || !searchPanelOpen(view.state)) return;
      if (!this.label || !panel.contains(this.label)) {
        this.label = document.createElement("span");
        this.label.className = "cm-search-count";
        this.label.setAttribute("aria-live", "polite");
        // After the search field, so the count sits where the eye already is.
        const field = panel.querySelector("input[main-field]");
        if (field) field.after(this.label);
        else panel.prepend(this.label);
      }
      this.label.textContent = count(view);
    }
    destroy() {
      this.label?.remove();
    }
  },
);

function count(view: EditorView): string {
  const query = getSearchQuery(view.state);
  if (!query.search) return "";
  if (!query.valid) return "Invalid";
  const selection = view.state.selection.main;
  const cursor = query.getCursor(view.state);
  let total = 0;
  let current = 0;
  for (let next = cursor.next(); !next.done; next = cursor.next()) {
    total++;
    if (next.value.from === selection.from && next.value.to === selection.to) current = total;
    if (total >= COUNT_LIMIT) return `${COUNT_LIMIT.toLocaleString()}+ matches`;
  }
  if (total === 0) return "No matches";
  const all = `${total.toLocaleString()} ${total === 1 ? "match" : "matches"}`;
  return current ? `${current.toLocaleString()} of ${all}` : all;
}

/** A CodeMirror editor. With `readOnly` it is a viewer with search (Ctrl+F) and folding. */
export function CodeEditor({ value, onChange, language = "text", readOnly = false, placeholder, extension, doc, wrap = true }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>(null);
  const languageSlot = useRef(new Compartment());
  const extensionSlot = useRef(new Compartment());
  const wrapSlot = useRef(new Compartment());
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const latest = useRef({ language, extension, placeholder, wrap });
  latest.current = { language, extension, placeholder, wrap };
  const shown = useRef(doc);

  /** A new editor state: the text with no undo history. */
  const fresh = (text: string) =>
    EditorState.create({
      doc: text,
      extensions: [
        lineNumbers(),
        foldGutter(),
        history(),
        bracketMatching(),
        search({ top: true }),
        searchCount,
        highlightSelectionMatches(),
        syntaxHighlighting(highlight),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        wrapSlot.current.of(latest.current.wrap ? EditorView.lineWrapping : []),
        // Read-only, but still focusable: otherwise Ctrl+F (search) and selection with the keyboard don't work.
        EditorState.readOnly.of(readOnly),
        latest.current.placeholder ? placeholderExt(latest.current.placeholder) : [],
        languageSlot.current.of(languages[latest.current.language]()),
        extensionSlot.current.of(latest.current.extension ?? []),
        theme,
        EditorView.updateListener.of((update) => {
          if (update.docChanged && !update.transactions.some((t) => t.annotation(external))) {
            onChangeRef.current?.(update.state.doc.toString());
          }
        }),
      ],
    });

  useEffect(() => {
    view.current = new EditorView({ state: fresh(value), parent: host.current! });
    return () => view.current?.destroy();
    // The editor is created once; value and language changes are applied below.
  }, [readOnly]);

  useEffect(() => {
    const v = view.current;
    if (!v) return;
    if (doc !== shown.current) {
      shown.current = doc;
      v.setState(fresh(value));
    } else if (v.state.doc.toString() !== value) {
      // The same document changed from outside, such as by Format: one step of its own for Ctrl+Z. Only the part
      // that differs is replaced, so the scroll position and the caret stay where they are when they can.
      const current = v.state.doc.toString();
      let from = 0;
      while (from < current.length && from < value.length && current[from] === value[from]) from++;
      let to = current.length;
      let end = value.length;
      while (to > from && end > from && current[to - 1] === value[end - 1]) {
        to--;
        end--;
      }
      v.dispatch({
        changes: { from, to, insert: value.slice(from, end) },
        annotations: [external.of(true), isolateHistory.of("full")],
      });
    }
  }, [value, doc]);

  useEffect(() => {
    view.current?.dispatch({ effects: languageSlot.current.reconfigure(languages[language]()) });
  }, [language]);

  useEffect(() => {
    view.current?.dispatch({ effects: extensionSlot.current.reconfigure(extension ?? []) });
  }, [extension]);

  useEffect(() => {
    view.current?.dispatch({ effects: wrapSlot.current.reconfigure(wrap ? EditorView.lineWrapping : []) });
  }, [wrap]);

  return <div className="code-editor" ref={host} />;
}

/** A viewer's setup, or with `editable` an editor's, for editors made elsewhere such as the comparisons'. */
export function viewerExtensions(language: Language, editable = false): Extension {
  return [
    lineNumbers(),
    search({ top: true }),
    searchCount,
    highlightSelectionMatches(),
    syntaxHighlighting(highlight),
    editable ? history() : [],
    keymap.of([...defaultKeymap, ...searchKeymap, ...(editable ? [...historyKeymap, indentWithTab] : [])]),
    EditorView.lineWrapping,
    EditorState.readOnly.of(!editable),
    languages[language](),
    theme,
  ];
}

/** Opens the search (and replace) panel of the editor inside `container`, if there is one. */
export function openSearchIn(container: Element | null): boolean {
  const dom = container?.querySelector<HTMLElement>(".cm-editor");
  const view = dom ? EditorView.findFromDOM(dom) : null;
  if (!view) return false;
  openSearchPanel(view);
  return true;
}
