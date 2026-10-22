// The request being edited. It keeps more than a request file does (the query table, body text for every body type)
// so switching tabs or body types never loses what was typed, and it carries fields the editor does not show yet
// (description, checks, scripts) so saving never drops them.

import type { Auth, Body, GrpcSettings, KeyValue, Part, RequestFile, SavedMessage, Scripts, Settings } from "./types";

/** gRPC settings as files keep them: empty lists left out. */
const cleanGrpc = (g: GrpcSettings): GrpcSettings => ({
  method: g.method,
  ...(g.protos?.length ? { protos: g.protos } : {}),
  ...(g.imports?.length ? { imports: g.imports } : {}),
});

/** A WebSocket request: its URL starts with ws:// or wss://. */
export const isWebSocket = (url: string) => /^\s*wss?:\/\//i.test(url);

/** A gRPC request: its URL starts with grpc:// or grpcs://. */
export const isGrpc = (url: string) => /^\s*grpcs?:\/\//i.test(url);

export type BodyMode =
  | "none"
  | "json"
  | "xml"
  | "html"
  | "text"
  | "javascript"
  | "raw"
  | "form"
  | "multipart"
  | "file"
  | "graphql";

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
  graphql: { query: string; variables: string; operationName: string };
  /** null means inherit from the folder. */
  auth: Auth | null;
  settings: Settings;
  description: string;
  operation: string | null;
  expect: Record<string, unknown>;
  scripts: Scripts;
  /** WebSocket requests: saved messages. */
  messages: SavedMessage[];
  /** gRPC requests: the method and its .proto files. The message is `raw`. */
  grpc: GrpcSettings;
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
  graphql: { query: "", variables: "", operationName: "" },
  auth: null,
  settings: defaultSettings(),
  description: "",
  operation: null,
  expect: {},
  scripts: {},
  messages: [],
  grpc: { method: "" },
});

const kv = (rows: Partial<KeyValue>[] | undefined): KeyValue[] =>
  (rows ?? []).map((r) => ({ name: r.name ?? "", value: r.value ?? "", enabled: r.enabled !== false }));

export function toRequestFile(d: Draft): RequestFile {
  return {
    // WebSockets and gRPC have no HTTP method; "WS" and "GRPC" label them in the tree.
    method: isWebSocket(d.url) ? "WS" : isGrpc(d.url) ? "GRPC" : d.method,
    url: d.url,
    description: d.description,
    operation: d.operation,
    headers: d.headers,
    body: isGrpc(d.url) ? (d.raw.trim() ? { type: "json", content: d.raw } : { type: "none" }) : toBody(d),
    auth: d.auth,
    settings: d.settings,
    expect: d.expect,
    scripts: d.scripts,
    ...(d.messages.length ? { messages: d.messages } : {}),
    ...(isGrpc(d.url) ? { grpc: cleanGrpc(d.grpc) } : {}),
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
    case "graphql":
      return {
        type: "graphql",
        query: d.graphql.query,
        ...(d.graphql.variables.trim() ? { variables: d.graphql.variables } : {}),
        ...(d.graphql.operationName.trim() ? { operationName: d.graphql.operationName } : {}),
      };
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
    messages: r.messages ?? [],
    grpc: r.grpc ?? { method: "" },
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
  if (body.type === "graphql")
    d.graphql = { query: body.query, variables: body.variables ?? "", operationName: body.operationName ?? "" };
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
