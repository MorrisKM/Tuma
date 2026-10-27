// Checking, formatting and minifying JSON for the JSON tools page. The check is done here rather than by JSON.parse,
// whose messages differ between the webviews Tuma runs in and on some of them say nowhere where the mistake is.

export interface JsonError {
  message: string;
  /** Offset of the mistake in the text. */
  pos: number;
  /** 1-based. */
  line: number;
  column: number;
}

class Mistake extends Error {
  constructor(
    message: string,
    readonly pos: number,
  ) {
    super(message);
  }
}

const NUMBER = /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/y;

/** The first mistake in `text` as JSON, or null when it is valid. */
export function validateJson(text: string): JsonError | null {
  let i = 0;
  const fail = (message: string, at = i): never => {
    throw new Mistake(message, at);
  };
  const space = () => {
    while (i < text.length) {
      const c = text[i];
      if (c === " " || c === "\t" || c === "\n" || c === "\r") i++;
      else if (c === "/" && (text[i + 1] === "/" || text[i + 1] === "*")) fail("Comments aren't allowed in JSON");
      else break;
    }
  };
  const string = () => {
    const start = i;
    i++;
    while (i < text.length) {
      const c = text[i];
      if (c === '"') {
        i++;
        return;
      }
      if (c === "\\") {
        const e = text[i + 1];
        if (e === "u") {
          if (!/^[0-9a-fA-F]{4}$/.test(text.slice(i + 2, i + 6))) fail("\\u needs four hex digits, as in \\u00e9");
          i += 6;
        } else if (e !== undefined && '"\\/bfnrt'.includes(e)) {
          i += 2;
        } else {
          fail(`Unknown escape \\${e ?? ""}: use \\", \\\\, \\/, \\b, \\f, \\n, \\r, \\t or \\u`);
        }
        continue;
      }
      if (c < " ") fail(c === "\n" ? "Line break inside a string: write it as \\n" : "Control character inside a string: escape it, as in \\t");
      i++;
    }
    fail("This string is never closed: add a \"", start);
  };
  const value = (): void => {
    space();
    const c = text[i];
    if (c === undefined) fail("The JSON ends here, but a value should follow");
    if (c === "{") return object();
    if (c === "[") return array();
    if (c === '"') return string();
    if (c === "'") fail("Strings need double quotes, not single ones");
    if (c === "-" || (c >= "0" && c <= "9")) {
      NUMBER.lastIndex = i;
      const m = NUMBER.exec(text);
      if (!m) fail("Not a valid number");
      i += m![0].length;
      if (text[i] === "." || text[i] === "e" || text[i] === "E") fail(`A number needs digits after the ${text[i] === "." ? "dot" : "e"}`);
      if (text[i] >= "0" && text[i] <= "9") fail("Numbers can't start with 0, except 0 itself and 0.5 and such", i - m![0].length);
      return;
    }
    for (const word of ["true", "false", "null"]) {
      if (text.startsWith(word, i)) {
        i += word.length;
        return;
      }
    }
    const bare = /^[A-Za-z_$][\w$]*/.exec(text.slice(i))?.[0];
    if (bare === "undefined" || bare === "NaN" || bare === "Infinity") fail(`${bare} isn't a JSON value: use null or a number`);
    if (bare) fail(`Expected a value; ${bare} needs double quotes if it is a string`);
    fail(`Expected a value, not ${JSON.stringify(c)}`);
  };
  const object = () => {
    i++;
    space();
    if (text[i] === "}") {
      i++;
      return;
    }
    for (;;) {
      space();
      if (text[i] === "}") fail("Trailing comma: remove the comma before }", text.lastIndexOf(",", i));
      if (text[i] === "'") fail("Property names need double quotes, not single ones");
      if (text[i] !== '"') fail(/[A-Za-z_$]/.test(text[i] ?? "") ? "Property names need double quotes" : "Expected a property name in double quotes");
      string();
      space();
      if (text[i] !== ":") fail("Expected : after the property name");
      i++;
      value();
      space();
      if (text[i] === ",") {
        i++;
        continue;
      }
      if (text[i] === "}") {
        i++;
        return;
      }
      fail(text[i] === undefined ? "The JSON ends here: an object is still open, add }" : "Expected , or } after the value: a comma may be missing");
    }
  };
  const array = () => {
    i++;
    space();
    if (text[i] === "]") {
      i++;
      return;
    }
    for (;;) {
      space();
      if (text[i] === "]") fail("Trailing comma: remove the comma before ]", text.lastIndexOf(",", i));
      value();
      space();
      if (text[i] === ",") {
        i++;
        continue;
      }
      if (text[i] === "]") {
        i++;
        return;
      }
      fail(text[i] === undefined ? "The JSON ends here: an array is still open, add ]" : "Expected , or ] after the value: a comma may be missing");
    }
  };

  try {
    if (!text.trim()) fail("Nothing to check yet", 0);
    value();
    space();
    if (i < text.length) fail("Unexpected text after the JSON value");
    return null;
  } catch (e) {
    if (!(e instanceof Mistake)) throw e;
    const pos = Math.min(Math.max(e.pos, 0), text.length);
    const before = text.slice(0, pos);
    const line = before.split("\n").length;
    return { message: e.message, pos, line, column: pos - before.lastIndexOf("\n") };
  }
}

export type Indent = "2" | "4" | "tab";

function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sorted);
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(record).sort().map((k) => [k, sorted(record[k])]));
  }
  return value;
}

/** Valid JSON pretty-printed; throws on invalid JSON. */
export function prettyJson(text: string, indent: Indent, sortKeys: boolean): string {
  const value = JSON.parse(text) as unknown;
  return JSON.stringify(sortKeys ? sorted(value) : value, null, indent === "tab" ? "\t" : Number(indent));
}

/** Valid JSON on one line, with no spaces; throws on invalid JSON. */
export function minifyJson(text: string, sortKeys: boolean): string {
  const value = JSON.parse(text) as unknown;
  return JSON.stringify(sortKeys ? sorted(value) : value);
}

export interface JsonStats {
  /** What the whole document is: object, array, string… */
  root: string;
  keys: number;
  objects: number;
  arrays: number;
  depth: number;
}

/** What a valid document holds, for the status line. */
export function jsonStats(text: string): JsonStats {
  const stats: JsonStats = { root: "", keys: 0, objects: 0, arrays: 0, depth: 0 };
  const walk = (v: unknown, depth: number) => {
    stats.depth = Math.max(stats.depth, depth);
    if (Array.isArray(v)) {
      stats.arrays++;
      v.forEach((x) => walk(x, depth + 1));
    } else if (v && typeof v === "object") {
      stats.objects++;
      const values = Object.values(v);
      stats.keys += values.length;
      values.forEach((x) => walk(x, depth + 1));
    }
  };
  const value = JSON.parse(text) as unknown;
  stats.root = Array.isArray(value) ? "array" : value === null ? "null" : typeof value;
  walk(value, 0);
  return stats;
}
