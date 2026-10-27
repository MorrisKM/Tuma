// Repairs JSON with the usual mistakes: what people paste from logs, JavaScript, Python and cut-off output. A forgiving
// parser reads it into a tree, noting each fix, and the tree is written out again as strict JSON. Numbers and valid
// strings are kept as written, so large ids keep every digit.

import type { Indent } from "./json";

type Node =
  | { kind: "object"; entries: { key: string; value: Node }[] }
  | { kind: "array"; items: Node[] }
  /** A number, true, false, null or a string, as JSON text. */
  | { kind: "raw"; text: string };

export type Fix =
  | "around"
  | "comment"
  | "trailingComma"
  | "extraComma"
  | "missingComma"
  | "missingColon"
  | "singleQuotes"
  | "unquotedKey"
  | "bareWord"
  | "literal"
  | "number"
  | "string"
  | "closed"
  | "mismatched"
  | "missingValue"
  | "after"
  | "wrapped";

const LABELS: Record<Fix, [string, string]> = {
  around: ["removed text around the JSON", "removed text around the JSON"],
  comment: ["removed a comment", "removed {n} comments"],
  trailingComma: ["removed a trailing comma", "removed {n} trailing commas"],
  extraComma: ["removed an extra comma", "removed {n} extra commas"],
  missingComma: ["added a missing comma", "added {n} missing commas"],
  missingColon: ["added a missing colon", "added {n} missing colons"],
  singleQuotes: ["changed a single-quoted string to double quotes", "changed {n} single-quoted strings to double quotes"],
  unquotedKey: ["quoted a key", "quoted {n} keys"],
  bareWord: ["quoted a bare word as a string", "quoted {n} bare words as strings"],
  literal: ["changed a Python or JavaScript value (True, None, undefined…)", "changed {n} Python or JavaScript values (True, None, undefined…)"],
  number: ["fixed a number", "fixed {n} numbers"],
  string: ["escaped line breaks, tabs, quotes or bad escapes in a string", "escaped line breaks, tabs, quotes or bad escapes in {n} strings"],
  closed: ["closed something left open", "closed {n} things left open"],
  mismatched: ["fixed a mismatched bracket", "fixed {n} mismatched brackets"],
  missingValue: ["filled a missing value with null", "filled {n} missing values with null"],
  after: ["removed text after the JSON", "removed text after the JSON"],
  wrapped: ["put several values in an array", "put several values in an array"],
};

export interface Repair {
  /** The repaired JSON. */
  text: string;
  /** What was fixed, in words, most frequent first. */
  fixes: string[];
  /** How many fixes in all. */
  count: number;
}

const LITERALS: Record<string, string> = {
  true: "true",
  false: "false",
  null: "null",
  True: "true",
  False: "false",
  None: "null",
  TRUE: "true",
  FALSE: "false",
  NULL: "null",
  Null: "null",
  undefined: "null",
  NaN: "null",
  Infinity: "null",
};

/**
 * Strict JSON from `text`, or null when there is nothing to read. Written pretty with `indent` when the input
 * spanned lines, and on one line when it did not.
 */
export function repairJson(input: string, indent: Indent): Repair | null {
  const counts = new Map<Fix, number>();
  const fix = (kind: Fix) => counts.set(kind, (counts.get(kind) ?? 0) + 1);

  let text = input;
  // A code fence, or an assignment such as `const data = {...};`: keep what is between.
  const fence = /^\s*```[\w-]*\n([\s\S]*?)\n?```\s*$/.exec(text);
  if (fence) {
    text = fence[1];
    fix("around");
  }
  // Text before the first bracket, unless it holds a quote and so may be part of the JSON.
  const bracket = text.search(/[{[]/);
  const before = bracket > 0 ? text.slice(0, bracket) : "";
  if (before.trim() && !/["'`]/.test(before)) {
    text = text.slice(bracket);
    fix("around");
  }

  const n = text.length;
  let i = 0;

  const space = () => {
    for (;;) {
      while (i < n && /\s/.test(text[i])) i++;
      if (text.startsWith("//", i) || (text[i] === "#" && /(^|\n)\s*$/.test(text.slice(0, i)))) {
        const end = text.indexOf("\n", i);
        i = end < 0 ? n : end;
        fix("comment");
      } else if (text.startsWith("/*", i)) {
        const end = text.indexOf("*/", i + 2);
        i = end < 0 ? n : end + 2;
        fix("comment");
      } else {
        return;
      }
    }
  };

  /** The next unescaped `quote` from `from`, or -1. */
  const nextQuote = (quote: string, from: number) => {
    for (let j = from; j < n; j++) {
      if (text[j] === "\\") j++;
      else if (text[j] === quote) return j;
    }
    return -1;
  };
  /** Whether a string closed at `j` is followed by what may follow it. */
  const closes = (j: number, key: boolean) => {
    let k = j + 1;
    while (k < n && (text[k] === " " || text[k] === "\t" || text[k] === "\r")) k++;
    const c = text[k];
    // A quote after a space starts the next string, with a comma missing between.
    if (c === text[j] && k > j + 1) return true;
    return key ? c === ":" || c === "=" : c === undefined || c === "\n" || c === "," || c === "}" || c === "]";
  };

  /** A string at `i` in any quotes, as JSON text. */
  const string = (key: boolean): string => {
    const quote = text[i];
    const open = i;
    if (quote !== '"') fix("singleQuotes");
    const lineEnd = text.indexOf("\n", open) < 0 ? n : text.indexOf("\n", open);
    let end = -1;
    let closed = false;
    const first = nextQuote(quote, open + 1);
    // Keys hardly ever hold quotes: the first one closes it, even with the colon missing after.
    if (first >= 0 && (closes(first, key) || (key && first < lineEnd))) {
      end = first;
      closed = true;
    } else {
      // Quotes inside that were not escaped: the last quote on the line that can close it.
      for (let j = nextQuote(quote, open + 1); j >= 0 && j < lineEnd; j = nextQuote(quote, j + 1)) {
        if (closes(j, key)) {
          end = j;
          closed = true;
        }
      }
      if (!closed) {
        // Never closed: up to the end of the line, or where the next string seems to begin.
        fix("closed");
        if (first >= 0 && first < lineEnd) {
          const cut = Math.max(...[",", "}", "]"].map((c) => text.lastIndexOf(c, first)).filter((p) => p > open));
          end = cut > open ? cut : first;
        } else if (key) {
          const colon = text.indexOf(":", open);
          end = colon > open && colon < lineEnd ? colon : lineEnd;
        } else {
          end = lineEnd;
        }
      }
    }
    const body = text.slice(open + 1, end);
    i = closed ? end + 1 : end;
    let value = "";
    let changed = false;
    for (let j = 0; j < body.length; j++) {
      const c = body[j];
      if (c === "\\") {
        const e = body[j + 1];
        if (e !== undefined && '"\\/bfnrt'.includes(e)) {
          value += JSON.parse(`"\\${e}"`) as string;
          j++;
        } else if (e === "u" && /^[0-9a-fA-F]{4}$/.test(body.slice(j + 2, j + 6))) {
          value += String.fromCharCode(parseInt(body.slice(j + 2, j + 6), 16));
          j += 5;
        } else if (e === "\n") {
          // A line continued with a backslash.
          j++;
          changed = true;
        } else if (e === undefined) {
          changed = true;
        } else {
          // \' and unknown escapes: the character itself.
          value += e;
          j++;
          if (e !== "'" || quote !== "'") changed = true;
        }
      } else {
        if (c < " " || (c === '"' && quote === '"')) changed = true;
        value += c;
      }
    }
    if (!closed) value = value.trimEnd();
    if (changed) fix("string");
    // A valid string is kept as written, with its escapes.
    return quote === '"' && closed && !changed ? text.slice(open, end + 1) : JSON.stringify(value);
  };

  const NUMBER = /[+-]?(?:0[xX][0-9a-fA-F]+|(?:\d[\d_]*\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/y;
  const number = (): string | null => {
    NUMBER.lastIndex = i;
    const m = NUMBER.exec(text);
    if (!m) return null;
    i += m[0].length;
    let s = m[0];
    if (/^[+-]?0[xX]/.test(s)) {
      fix("number");
      return String(parseInt(s, 16));
    }
    const strict = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;
    if (strict.test(s)) return s;
    fix("number");
    s = s.replace(/_/g, "").replace(/^\+/, "");
    const sign = s.startsWith("-") ? "-" : "";
    s = s.replace(/^-/, "");
    s = s.replace(/^0+(?=\d)/, "");
    if (s.startsWith(".")) s = `0${s}`;
    s = s.replace(/\.(?=$|[eE])/, "");
    return strict.test(sign + s) ? sign + s : "null";
  };

  /** A word that is not quoted: a literal, or a string someone forgot to quote. */
  const word = (): string | null => {
    const m = /^-?[A-Za-z_$][\w$.-]*/.exec(text.slice(i, i + 200));
    if (!m) return null;
    i += m[0].length;
    const w = m[0].replace(/^-(?=Infinity$)/, "");
    if (w in LITERALS) {
      if (LITERALS[w] !== w) fix("literal");
      return LITERALS[w];
    }
    fix("bareWord");
    return JSON.stringify(m[0]);
  };

  const value = (): Node => {
    space();
    const c = text[i];
    if (c === undefined || c === "," || c === "}" || c === "]" || c === ":") {
      // A colon where a value should be is dropped; the others end what holds the value.
      if (c === ":") i++;
      fix("missingValue");
      return { kind: "raw", text: "null" };
    }
    if (c === "{") return object();
    if (c === "[") return array();
    if (c === '"' || c === "'" || c === "`") return { kind: "raw", text: string(false) };
    const num = /[\d.+-]/.test(c) ? number() : null;
    if (num !== null) return { kind: "raw", text: num };
    const w = word();
    if (w !== null) return { kind: "raw", text: w };
    // Something that cannot start a value: skip it.
    i++;
    fix("missingValue");
    return { kind: "raw", text: "null" };
  };

  const key = (): string => {
    const c = text[i];
    if (c === '"' || c === "'" || c === "`") return JSON.parse(string(true)) as string;
    fix("unquotedKey");
    const m = /^[^\s:=,{}[\]]+/.exec(text.slice(i));
    const k = m ? m[0] : "";
    i += k.length;
    return k;
  };

  const object = (): Node => {
    i++;
    const entries: { key: string; value: Node }[] = [];
    let comma = false;
    for (;;) {
      space();
      const c = text[i];
      if (c === undefined) {
        fix("closed");
        break;
      }
      if (c === "}" || c === "]") {
        if (c === "]") fix("mismatched");
        if (comma) fix("trailingComma");
        i++;
        break;
      }
      if (c === ",") {
        fix("extraComma");
        i++;
        continue;
      }
      if (entries.length && !comma) fix("missingComma");
      const k = key();
      space();
      if (text[i] === ":" || text[i] === "=") {
        if (text[i] === "=") fix("missingColon");
        i++;
      } else {
        fix("missingColon");
      }
      entries.push({ key: k, value: value() });
      space();
      comma = text[i] === ",";
      if (comma) i++;
    }
    return { kind: "object", entries };
  };

  const array = (): Node => {
    i++;
    const items: Node[] = [];
    let comma = false;
    for (;;) {
      space();
      const c = text[i];
      if (c === undefined) {
        fix("closed");
        break;
      }
      if (c === "]" || c === "}") {
        if (c === "}") fix("mismatched");
        if (comma) fix("trailingComma");
        i++;
        break;
      }
      if (c === ",") {
        fix("extraComma");
        i++;
        continue;
      }
      if (items.length && !comma) fix("missingComma");
      items.push(value());
      space();
      comma = text[i] === ",";
      if (comma) i++;
    }
    return { kind: "array", items };
  };

  space();
  if (i >= n) return null;
  let root = value();
  space();
  // More values after the first, as in JSON Lines: all of them, in an array.
  const more: Node[] = [];
  while (i < n && /[{[]/.test(text[i])) {
    more.push(value());
    space();
  }
  if (more.length) {
    root = { kind: "array", items: [root, ...more] };
    fix("wrapped");
  }
  if (i < n) {
    if (/^[;,)\s]*$/.test(text.slice(i))) fix("around");
    else fix("after");
  }

  const pretty = input.trim().includes("\n");
  const out = write(root, pretty ? (indent === "tab" ? "\t" : " ".repeat(Number(indent))) : "", "");
  const sortedFixes = [...counts.entries()].sort((a, b) => b[1] - a[1]);
  return {
    text: out,
    fixes: sortedFixes.map(([kind, count]) => LABELS[kind][count === 1 ? 0 : 1].replace("{n}", String(count))),
    count: [...counts.values()].reduce((a, b) => a + b, 0),
  };
}

function write(node: Node, indent: string, at: string): string {
  if (node.kind === "raw") return node.text;
  const inner = at + indent;
  const nl = indent ? "\n" : "";
  if (node.kind === "array") {
    if (!node.items.length) return "[]";
    return `[${nl}${node.items.map((v) => inner + write(v, indent, inner)).join(`,${nl}`)}${nl}${at}]`;
  }
  if (!node.entries.length) return "{}";
  const colon = indent ? ": " : ":";
  return `{${nl}${node.entries
    .map((e) => `${inner}${JSON.stringify(e.key)}${colon}${write(e.value, indent, inner)}`)
    .join(`,${nl}`)}${nl}${at}}`;
}
