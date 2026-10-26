// Links in a response: URLs are underlined, and clicking one opens a new request to it.

import { Decoration, EditorView, MatchDecorator, ViewPlugin, type ViewUpdate } from "@codemirror/view";

/** An http(s) or WebSocket URL, up to a space, a quote or a bracket that cannot be in one. */
const URL = /\b(?:https?|wss?):\/\/[^\s"'<>`\\{}|^]+/g;

/** The URL without the punctuation that usually follows one in text: "see https://x.dev/a." */
export function trimUrl(url: string): string {
  return url.replace(/[.,;:!?)\]]+$/, "");
}

/** The URLs in a piece of text, with where each starts. */
export function findUrls(text: string): { url: string; index: number }[] {
  return [...text.matchAll(URL)].map((m) => ({ url: trimUrl(m[0]), index: m.index }));
}

const decorator = new MatchDecorator({
  regexp: URL,
  decorate: (add, from, _to, match) => {
    const url = trimUrl(match[0]);
    add(
      from,
      from + url.length,
      Decoration.mark({ class: "cm-link", attributes: { "data-url": url, title: "Open in a new request" } }),
    );
  },
});

/** Underlines the URLs in an editor; a click on one, not one that ends a text selection, calls `open`. */
export function urlLinks(open: (url: string) => void) {
  return [
    ViewPlugin.fromClass(
      class {
        decorations;
        constructor(view: EditorView) {
          this.decorations = decorator.createDeco(view);
        }
        update(update: ViewUpdate) {
          this.decorations = decorator.updateDeco(update, this.decorations);
        }
      },
      { decorations: (v) => v.decorations },
    ),
    EditorView.domEventHandlers({
      click: (event, view) => {
        const link = (event.target as HTMLElement).closest<HTMLElement>(".cm-link");
        const url = link?.dataset.url;
        if (!url || event.button !== 0 || !view.state.selection.main.empty) return false;
        event.preventDefault();
        open(url);
        return true;
      },
    }),
    EditorView.theme({
      ".cm-link": { textDecoration: "underline", textUnderlineOffset: "2px", cursor: "pointer" },
      ".cm-link:hover": { color: "var(--accent)" },
    }),
  ];
}
