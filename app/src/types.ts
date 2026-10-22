// Mirrors the Rust types sent over IPC: tuma_core::workspace::files (the on-disk format), tuma_core::workspace and the
// views in src-tauri/src/lib.rs. Fields the Rust side leaves out when they hold their default are optional here.

export interface KeyValue {
  name: string;
  value: string;
  enabled: boolean;
}

export interface Part extends KeyValue {
  /** When true, `value` is a file path and the file's contents are sent. */
  file: boolean;
  contentType?: string | null;
}

export type Body =
  | { type: "none" }
  | { type: "json" | "xml" | "html" | "text" | "javascript"; content: string }
  | { type: "raw"; content: string; contentType: string }
  | { type: "form"; fields: KeyValue[] }
  | { type: "multipart"; parts: Part[] }
  | { type: "file"; path: string; contentType?: string | null };

export type Auth =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apiKey"; key: string; value: string; location: "header" | "query" };

export interface Settings {
  timeoutMs: number | null;
  followRedirects: boolean;
  maxRedirects: number;
  verifyTls: boolean;
}

export interface Scripts {
  preRequest?: string;
  test?: string;
}

/** A request file. A missing `auth` means "inherit from the folder". */
export interface RequestFile {
  method: string;
  url: string;
  description?: string;
  headers?: Partial<KeyValue>[];
  body?: Body;
  auth?: Auth | null;
  settings?: Settings;
  /** Checks on the response (status, headers, JSON paths); kept as is, the editor does not show them yet. */
  expect?: Record<string, unknown>;
  scripts?: Scripts;
}

export interface Variable {
  name: string;
  value?: string;
  secret?: boolean;
  enabled?: boolean;
}

/** A `_folder.yaml`. A missing `auth` means "inherit". */
export interface FolderFile {
  description?: string;
  order?: string[];
  auth?: Auth | null;
  headers?: Partial<KeyValue>[];
  variables?: Variable[];
  scripts?: Scripts;
}

/** A variable with its secret value filled in from the keychain. */
export interface VariableValue {
  name: string;
  value: string;
  secret: boolean;
  enabled: boolean;
}

export interface TreeNode {
  kind: "collection" | "folder" | "request";
  name: string;
  path: string;
  method?: string;
  children?: TreeNode[];
}

export interface WorkspaceInfo {
  name: string;
  root: string;
}

export type ImportResult =
  | { kind: "collection"; path: string; warnings: string[] }
  | { kind: "environment"; name: string; warnings: string[] }
  | { kind: "globals"; warnings: string[] };

export interface Header {
  name: string;
  value: string;
}

/** Milliseconds. `dns` and `tls` are null when that phase did not happen. */
export interface Timing {
  dns: number | null;
  connect: number;
  tls: number | null;
  wait: number;
  download: number;
  total: number;
}

export interface Response {
  status: number;
  reason: string;
  version: string;
  url: string;
  remoteAddr: string | null;
  headers: Header[];
  body: string;
  bodyBase64: boolean;
  size: number;
  encodedSize: number;
  timing: Timing;
  redirects: { status: number; url: string }[];
  sent: { method: string; url: string; headers: Header[]; bodySize: number };
  unresolved: string[];
}

export interface RequestError {
  kind: string;
  message: string;
  unresolved?: string[];
}

export interface HistoryEntry {
  id: string;
  /** Unix time in milliseconds. */
  sentAt: number;
  request: RequestFile;
  path?: string | null;
  status: number | null;
  error: string | null;
  totalMs: number | null;
  size: number | null;
}
