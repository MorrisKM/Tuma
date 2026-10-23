import { history, historyKeymap, isolateHistory } from "@codemirror/commands";
import { EditorState, type Extension, StateEffect } from "@codemirror/state";
import { external } from "./CodeEditor";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  hoverTooltip,
  keymap,
  placeholder as placeholderExt,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { useEffect, useRef } from "react";
import type { VariableHint } from "../types";
import { DYNAMIC, variableCompletion } from "../variables";

const VARIABLE = /\{\{\s*([^{}]+?)\s*\}\}/g;

/** Tells the decorations to recompute when the variables in scope change. */
const rescan = StateEffect.define<null>();

interface Props {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  /** Called instead of inserting text that looks like a cURL command. */
  onPasteCurl: (command: string) => void;
  scope: VariableHint[];
  placeholder?: string;
  /** Which request is shown; when it changes, the undo history starts afresh. */
  doc?: unknown;
}

/** A one-line editor for URLs that colors {{variables}}, explains them on hover and completes them. */
export function UrlEditor({ value, onChange, onSubmit, onPasteCurl, scope, placeholder, doc }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>(null);
  // The editor is created once; these refs give its extensions the latest props.
  const props = useRef({ onChange, onSubmit, onPasteCurl, scope });
  props.current = { onChange, onSubmit, onPasteCurl, scope };
  const extensions = useRef<Extension[]>([]);
  const shown = useRef(doc);

  useEffect(() => {
    const lookup = (name: string) =>
      props.current.scope.find((v) => v.name === name) ?? (DYNAMIC[name] ? { dynamic: DYNAMIC[name] } : undefined);

    const highlight = ViewPlugin.fromClass(
      class {
        decorations: DecorationSet;
        constructor(v: EditorView) {
          this.decorations = this.build(v);
        }
        update(u: ViewUpdate) {
          if (u.docChanged || u.transactions.some((t) => t.effects.some((e) => e.is(rescan)))) {
            this.decorations = this.build(u.view);
          }
        }
        build(v: EditorView) {
          const text = v.state.doc.toString();
          const marks = [];
          for (const m of text.matchAll(VARIABLE)) {
            const found = lookup(m[1]);
            const kind = !found ? "missing" : "secret" in found && found.secret ? "secret" : "known";
            marks.push(Decoration.mark({ class: `cm-var cm-var-${kind}` }).range(m.index!, m.index! + m[0].length));
          }
          return Decoration.set(marks);
        }
      },
      { decorations: (p) => p.decorations },
    );

    const tooltip = hoverTooltip((v, pos) => {
      const text = v.state.doc.toString();
      for (const m of text.matchAll(VARIABLE)) {
        const start = m.index!;
        const end = start + m[0].length;
        if (pos < start || pos > end) continue;
        const found = lookup(m[1]);
        return {
          pos: start,
          end,
          above: true,
          create: () => {
            const dom = document.createElement("div");
            dom.className = "var-tooltip";
            const name = document.createElement("strong");
            name.textContent = m[1];
            const detail = document.createElement("div");
            if (!found) {
              detail.textContent = "No value in scope: it is sent as written. Set it in an environment or folder.";
              detail.className = "missing";
            } else if ("dynamic" in found) {
              detail.textContent = `Generated on each send: ${found.dynamic}`;
            } else {
              detail.textContent = found.secret ? "Secret (kept in the OS keychain)" : found.value || "(empty)";
              const from = document.createElement("div");
              from.className = "source";
              from.textContent = `from ${found.source}`;
              dom.append(name, detail, from);
              return { dom };
            }
            dom.append(name, detail);
            return { dom };
          },
        };
      }
      return null;
    });

    extensions.current = [
      // One line: Enter sends, and pasted line breaks become spaces.
      EditorState.transactionFilter.of((tr) => (tr.newDoc.lines > 1 ? [] : tr)),
      history(),
      keymap.of([
        ...historyKeymap,
        {
          key: "Enter",
          run: () => {
            props.current.onSubmit();
            return true;
          },
        },
      ]),
      EditorView.domEventHandlers({
        paste: (event, v) => {
          const text = event.clipboardData?.getData("text/plain") ?? "";
          if (/^\s*curl\s/i.test(text)) {
            event.preventDefault();
            props.current.onPasteCurl(text);
            return true;
          }
          if (text.includes("\n")) {
            event.preventDefault();
            v.dispatch(v.state.replaceSelection(text.replace(/\s*\n\s*/g, " ")));
            return true;
          }
          return false;
        },
      }),
      EditorView.updateListener.of((u) => {
        // Only the user's edits; text set from outside (opening a request, switching tabs) is not news.
        if (u.docChanged && !u.transactions.some((t) => t.annotation(external))) {
          props.current.onChange(u.state.doc.toString());
        }
      }),
      highlight,
      tooltip,
      variableCompletion(() => props.current.scope),
      placeholder ? placeholderExt(placeholder) : [],
      EditorView.contentAttributes.of({ "aria-label": "URL", spellcheck: "false" }),
    ];
    view.current = new EditorView({
      state: EditorState.create({ doc: value, extensions: extensions.current }),
      parent: host.current!,
    });
    return () => view.current?.destroy();
    // Created once; value and scope changes are applied below.
  }, []);

  useEffect(() => {
    const v = view.current;
    if (!v) return;
    if (doc !== shown.current) {
      // Another request: its text, with no undo history, so Ctrl+Z never brings back the last one's URL.
      shown.current = doc;
      v.setState(EditorState.create({ doc: value, extensions: extensions.current }));
    } else if (v.state.doc.toString() !== value) {
      // The same request changed from outside, such as from the query parameters: one step of its own for Ctrl+Z.
      v.dispatch({
        changes: { from: 0, to: v.state.doc.length, insert: value },
        annotations: [external.of(true), isolateHistory.of("full")],
      });
    }
  }, [value, doc]);

  useEffect(() => {
    view.current?.dispatch({ effects: rescan.of(null) });
  }, [scope]);

  return <div className="url" ref={host} />;
}

/** Focuses the URL editor, for Ctrl+L. */
export function focusUrl() {
  document.querySelector<HTMLElement>(".url .cm-content")?.focus();
}
