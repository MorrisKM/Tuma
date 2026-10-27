import { defaultKeymap, history, historyKeymap, indentWithTab, isolateHistory } from "@codemirror/commands";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json, jsonLanguage } from "@codemirror/lang-json";
import { xml } from "@codemirror/lang-xml";
import { bracketMatching, foldGutter, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { Annotation, Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, placeholder as placeholderExt } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useEffect, useRef } from "react";
import { jsonComments } from "../jsonc";

export type Language = "json" | "xml" | "html" | "javascript" | "text";

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
  text: () => [],
};

// Colors come from CSS variables so the editor follows the light and dark themes.
const highlight = HighlightStyle.define([
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--syntax-number)" },
  { tag: [tags.propertyName, tags.attributeName], color: "var(--syntax-property)" },
  { tag: [tags.keyword, tags.tagName, tags.typeName], color: "var(--syntax-keyword)" },
  { tag: [tags.comment], color: "var(--muted)", fontStyle: "italic" },
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
}

/** A CodeMirror editor. With `readOnly` it is a viewer with search (Ctrl+F) and folding. */
export function CodeEditor({ value, onChange, language = "text", readOnly = false, placeholder, extension, doc }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>(null);
  const languageSlot = useRef(new Compartment());
  const extensionSlot = useRef(new Compartment());
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const latest = useRef({ language, extension, placeholder });
  latest.current = { language, extension, placeholder };
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
        highlightSelectionMatches(),
        syntaxHighlighting(highlight),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        EditorView.lineWrapping,
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
      // The same document changed from outside, such as by Format: one step of its own for Ctrl+Z.
      v.dispatch({
        changes: { from: 0, to: v.state.doc.length, insert: value },
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

  return <div className="code-editor" ref={host} />;
}

/** A viewer's setup, or with `editable` an editor's, for editors made elsewhere such as the comparisons'. */
export function viewerExtensions(language: Language, editable = false): Extension {
  return [
    lineNumbers(),
    search({ top: true }),
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
