// Copying name/value rows (params, headers, form fields, variables) between requests, as text: one `name: value` per
// line, with `//` before disabled ones, as in Postman's bulk edit. Pasting also understands a query string
// (`?a=1&b=2`), `name=value` lines and tab-separated rows from a spreadsheet.

export interface Row {
  name: string;
  value: string;
  enabled: boolean;
}

export function formatRows(rows: Row[]): string {
  return rows
    .filter((r) => r.name.trim() || r.value)
    .map((r) => `${r.enabled ? "" : "//"}${r.name}: ${r.value}`)
    .join("\n");
}

const decode = (text: string) => {
  try {
    return decodeURIComponent(text.replace(/\+/g, " "));
  } catch {
    return text;
  }
};

/** Splits at the first ":" or "=", whichever comes first; a line with neither is a name alone. */
function parseLine(line: string): Row | null {
  let text = line.trim();
  if (!text) return null;
  let enabled = true;
  if (text.startsWith("//")) {
    enabled = false;
    text = text.slice(2).trim();
  }
  const tab = text.indexOf("\t");
  const colon = text.indexOf(":");
  const equals = text.indexOf("=");
  const at = tab >= 0 ? tab : [colon, equals].filter((i) => i > 0).sort((a, b) => a - b)[0] ?? -1;
  if (at < 0) return { name: text, value: "", enabled };
  return { name: text.slice(0, at).trim(), value: text.slice(at + 1).trim(), enabled };
}

/**
 * The rows in pasted text, or null when it looks like a plain name: a single word with no ":" or "=" in it, which
 * should paste as typed.
 */
export function parseRows(text: string): Row[] | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const lines = trimmed.split(/\r?\n/);
  // A query string or URL: ?a=1&b=2, or https://x/y?a=1&b=2.
  const query = lines.length === 1 && !/\s/.test(trimmed) && trimmed.includes("=") && /[?&]/.test(trimmed);
  if (query) {
    const pairs = trimmed.includes("?") ? trimmed.slice(trimmed.indexOf("?") + 1) : trimmed;
    const rows = pairs
      .split("#")[0]
      .split("&")
      .filter(Boolean)
      .map((pair) => {
        const at = pair.indexOf("=");
        return at < 0
          ? { name: decode(pair), value: "", enabled: true }
          : { name: decode(pair.slice(0, at)), value: decode(pair.slice(at + 1)), enabled: true };
      });
    return rows.length ? rows : null;
  }
  if (lines.length === 1 && !/[:=\t]/.test(trimmed) && !trimmed.startsWith("//")) return null;
  const rows = lines.map(parseLine).filter((r): r is Row => !!r && !!r.name);
  return rows.length ? rows : null;
}
