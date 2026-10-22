// The request being edited. It keeps more than a request file does (the query table, body text for every body type)
// so switching tabs or body types never loses what was typed, and it carries fields the editor does not show yet
// (description, checks, scripts) so saving never drops them.

import type { Auth, Body, KeyValue, Part, RequestFile, Scripts, Settings } from "./types";

export type BodyMode = "none" | "json" | "xml" | "html" | "text" | "javascript" | "raw" | "form" | "multipart" | "file";

export const TEXT_MODES = ["json", "xml", "html", "text", "javascript", "raw"] as const;
export const isTextMode = (mode: BodyMode): mode is (typeof TEXT_MODES)[number] =>
  (TEXT_MODES as readonly string[]).includes(mode);

export interface Draft {
  /** Where the request is saved in the workspace, or null if it is not. */
  path: string | null;
  name: string;
  method: string;
  url: string;
  params: KeyValue[];
  headers: KeyValue[];
  bodyMode: BodyMode;
  raw: string;
  /** The content type for the "raw" (other text) body mode. */
  rawContentType: string;
  form: KeyValue[];
  parts: Part[];
  filePath: string;
  /** null means inherit from the folder. */
  auth: Auth | null;
  settings: Settings;
  description: string;
  operation: string | null;
  expect: Record<string, unknown>;
  scripts: Scripts;
}

// Must match tuma_core's defaults, or every saved request would get a `settings:` block.
export const defaultSettings = (): Settings => ({
  timeoutMs: null,
  followRedirects: true,
  maxRedirects: 10,
  verifyTls: true,
});

export const newDraft = (): Draft => ({
  path: null,
  name: "Untitled",
  method: "GET",
  url: "",
  params: [],
  headers: [],
  bodyMode: "none",
  raw: "",
  rawContentType: "text/plain",
  form: [],
  parts: [],
  filePath: "",
  auth: null,
  settings: defaultSettings(),
  description: "",
  operation: null,
  expect: {},
  scripts: {},
});

const kv = (rows: Partial<KeyValue>[] | undefined): KeyValue[] =>
  (rows ?? []).map((r) => ({ name: r.name ?? "", value: r.value ?? "", enabled: r.enabled !== false }));

export function toRequestFile(d: Draft): RequestFile {
  return {
    method: d.method,
    url: d.url,
    description: d.description,
    operation: d.operation,
    headers: d.headers,
    body: toBody(d),
    auth: d.auth,
    settings: d.settings,
    expect: d.expect,
    scripts: d.scripts,
  };
}

function toBody(d: Draft): Body {
  switch (d.bodyMode) {
    case "json":
    case "xml":
    case "html":
    case "text":
    case "javascript":
      return { type: d.bodyMode, content: d.raw };
    case "raw":
      return { type: "raw", content: d.raw, contentType: d.rawContentType };
    case "form":
      return { type: "form", fields: d.form };
    case "multipart":
      return { type: "multipart", parts: d.parts };
    case "file":
      return d.filePath ? { type: "file", path: d.filePath } : { type: "none" };
    default:
      return { type: "none" };
  }
}

export function fromRequestFile(r: RequestFile, path: string | null, name: string): Draft {
  const d: Draft = {
    ...newDraft(),
    path,
    name,
    method: r.method || "GET",
    url: r.url ?? "",
    headers: kv(r.headers),
    auth: r.auth ?? null,
    settings: { ...defaultSettings(), ...r.settings },
    description: r.description ?? "",
    operation: r.operation ?? null,
    expect: r.expect ?? {},
    scripts: r.scripts ?? {},
  };
  d.params = parseQuery(d.url);
  const body = r.body ?? { type: "none" };
  d.bodyMode = body.type;
  if ("content" in body) d.raw = body.content;
  if (body.type === "raw") d.rawContentType = body.contentType;
  if (body.type === "form") d.form = kv(body.fields);
  if (body.type === "multipart")
    d.parts = body.parts.map((p) => ({ ...p, enabled: p.enabled !== false, file: p.file === true }));
  if (body.type === "file") d.filePath = body.path;
  return d;
}

/** A name for a request, from the end of its file path. */
export const nameFromPath = (path: string) => path.split("/").pop()!.replace(/\.yaml$/, "");

/** The query string's rows, as typed: nothing is decoded. */
export function parseQuery(url: string): KeyValue[] {
  const start = url.indexOf("?");
  if (start < 0) return [];
  const query = url.slice(start + 1).split("#")[0];
  return query
    .split("&")
    .filter((pair) => pair !== "")
    .map((pair) => {
      const eq = pair.indexOf("=");
      return eq < 0
        ? { name: pair, value: "", enabled: true }
        : { name: pair.slice(0, eq), value: pair.slice(eq + 1), enabled: true };
    });
}

/** Replaces the URL's query string with the enabled rows. */
export function withQuery(url: string, params: KeyValue[]): string {
  const hashAt = url.indexOf("#");
  const hash = hashAt >= 0 ? url.slice(hashAt) : "";
  const withoutHash = hashAt >= 0 ? url.slice(0, hashAt) : url;
  const base = withoutHash.split("?")[0];
  const query = params
    .filter((p) => p.enabled && p.name !== "")
    .map((p) => (p.value === "" ? p.name : `${p.name}=${p.value}`))
    .join("&");
  return (query ? `${base}?${query}` : base) + hash;
}

/** Variables as the variables editor shows them. */
export const normalizeVariables = (vars: { name: string; value?: string; secret?: boolean; enabled?: boolean }[]) =>
  vars.map((v) => ({ name: v.name, value: v.value ?? "", secret: v.secret === true, enabled: v.enabled !== false }));
