import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import type { Header, Response } from "./types";

/** How responses are made comparable: noise left out, JSON keys in one order. */
export interface Normalize {
  /** JSON keys to leave out wherever they are, and header names (in any case). */
  ignore: string[];
  /** Sorts JSON object keys, so a different order is not a difference. */
  sortKeys: boolean;
}

/** "id, createdAt date" → ["id", "createdAt", "date"]. */
export function ignoredNames(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((n) => n.trim())
    .filter(Boolean);
}

/** A response's body as text to compare: JSON pretty-printed, without ignored keys; and whether it was JSON. */
export function comparableBody(response: Response, options: Normalize): { text: string; json: boolean } {
  if (response.bodyBase64) return { text: `(binary data, ${response.size} bytes)`, json: false };
  try {
    const value = JSON.parse(response.body) as unknown;
    return { text: JSON.stringify(prune(value, new Set(options.ignore), options.sortKeys), null, 2), json: true };
  } catch {
    return { text: response.body, json: false };
  }
}

/** JSON text pretty-printed the way comparisons show it; null when it is not JSON. */
export function formatJson(text: string, options: Normalize): string | null {
  try {
    return JSON.stringify(prune(JSON.parse(text), new Set(options.ignore), options.sortKeys), null, 2);
  } catch {
    return null;
  }
}

/** What pasted text looks like, to color it. */
export function detectLanguage(text: string): "json" | "xml" | "html" | "text" {
  const t = text.trim();
  if (!t) return "text";
  if (/^[[{]/.test(t)) {
    try {
      JSON.parse(t);
      return "json";
    } catch {
      // Not JSON after all.
    }
  }
  if (t.startsWith("<")) return /^<!doctype html|<html[\s>]/i.test(t) ? "html" : "xml";
  return "text";
}

function prune(value: unknown, ignore: Set<string>, sort: boolean): unknown {
  if (Array.isArray(value)) return value.map((v) => prune(v, ignore, sort));
  if (value && typeof value === "object") {
    const keys = Object.keys(value).filter((k) => !ignore.has(k));
    if (sort) keys.sort();
    return Object.fromEntries(keys.map((k) => [k, prune((value as Record<string, unknown>)[k], ignore, sort)]));
  }
  return value;
}

/** Headers as "name: value" lines, sorted by name, without ignored ones. */
export function comparableHeaders(headers: Header[], ignore: string[]): string {
  const skip = new Set(ignore.map((n) => n.toLowerCase()));
  return headers
    .filter((h) => !skip.has(h.name.toLowerCase()))
    .map((h) => `${h.name.toLowerCase()}: ${h.value}`)
    .sort()
    .join("\n");
}

/** How many separate changes there are between two texts. */
export function changeCount(a: string, b: string): number {
  if (a === b) return 0;
  return Chunk.build(Text.of(a.split("\n")), Text.of(b.split("\n"))).length;
}
