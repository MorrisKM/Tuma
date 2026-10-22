// JSON with comments, as written in request bodies. The app sends them without the comments (crates/core/src/jsonc.rs);
// here they are formatted and highlighted with the comments kept.

import { RangeSetBuilder } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

type Token = {
  kind: "string" | "comment" | "line-comment" | "punct" | "word";
  text: string;
  /** For a comment: it started its own line, rather than following something on the same line. */
  ownLine?: boolean;
};

function tokens(text: string): Token[] | null {
  const out: Token[] = [];
  let i = 0;
  let lineStart = true;
  while (i < text.length) {
    const c = text[i];
    if (c === "\n") {
      lineStart = true;
      i++;
      continue;
    }
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    const ownLine = lineStart;
    lineStart = false;
    if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      if (j >= text.length) return null;
      out.push({ kind: "string", text: text.slice(i, j + 1) });
      i = j + 1;
    } else if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      out.push({ kind: "line-comment", text: text.slice(i, end < 0 ? undefined : end).trimEnd(), ownLine });
      i = end < 0 ? text.length : end;
    } else if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      if (end < 0) return null;
      out.push({ kind: "comment", text: text.slice(i, end + 2), ownLine });
      i = end + 2;
    } else if ("{}[],:".includes(c)) {
      out.push({ kind: "punct", text: c });
      i++;
    } else {
      let j = i;
      while (j < text.length && !/[\s{}[\],:"]/.test(text[j]) && !text.startsWith("//", j) && !text.startsWith("/*", j)) j++;
      out.push({ kind: "word", text: text.slice(i, j) });
      i = j;
    }
  }
  return out;
}

/** Indents JSON with two spaces, keeping comments where they were. Returns the text unchanged if it isn't JSON. */
export function formatJsonc(text: string): string {
  const all = tokens(text);
  if (!all) return text;
  // Without comments, the browser's own formatter is the reference.
  if (!all.some((t) => t.kind === "comment" || t.kind === "line-comment")) {
    try {
      return JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      return text;
    }
  }
  let out = "";
  let depth = 0;
  // Whether the output is at the start of a fresh, indented line.
  let fresh = true;
  const newline = () => {
    out = out.trimEnd() + "\n" + "  ".repeat(depth);
    fresh = true;
  };
  const write = (text: string) => {
    out += text;
    fresh = false;
  };
  const opens = (t?: Token) => t?.kind === "punct" && (t.text === "{" || t.text === "[");
  const closes = (t?: Token) => t?.kind === "punct" && (t.text === "}" || t.text === "]");
  /** A line comment right after this token, on the same line: it stays there. */
  const trailing = (t?: Token) => t?.kind === "line-comment" && !t.ownLine;
  for (let i = 0; i < all.length; i++) {
    const t = all[i];
    const next = all[i + 1];
    if (opens(t)) {
      write(t.text);
      // An empty object or array stays on one line.
      if (closes(next)) {
        write(next!.text);
        i++;
        continue;
      }
      depth++;
      if (!trailing(next)) newline();
    } else if (closes(t)) {
      depth = Math.max(0, depth - 1);
      newline();
      write(t.text);
    } else if (t.kind === "punct" && t.text === ",") {
      write(",");
      if (!trailing(next)) newline();
    } else if (t.kind === "punct" && t.text === ":") {
      write(": ");
    } else if (t.kind === "line-comment" || t.kind === "comment") {
      if (t.ownLine && !fresh) newline();
      else if (!fresh) out = out.trimEnd() + " ";
      write(t.text);
      if (t.kind === "line-comment") newline();
      else out += " ";
    } else {
      write(t.text);
    }
  }
  return out.trim();
}

const commentMark = Decoration.mark({ class: "cm-json-comment" });

function commentDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const text = view.state.doc.toString();
  let i = 0;
  let inString = false;
  while (i < text.length) {
    const c = text[i];
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"' || c === "\n") inString = false;
      i++;
    } else if (c === '"') {
      inString = true;
      i++;
    } else if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      const to = end < 0 ? text.length : end;
      builder.add(i, to, commentMark);
      i = to;
    } else if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      const to = end < 0 ? text.length : end + 2;
      builder.add(i, to, commentMark);
      i = to;
    } else {
      i++;
    }
  }
  return builder.finish();
}

/** Shows `//` and `/* *\/` comments in a JSON editor as comments (the JSON grammar has no comments of its own). */
export const jsonComments = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = commentDecorations(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged) this.decorations = commentDecorations(u.view);
    }
  },
  { decorations: (v) => v.decorations },
);
