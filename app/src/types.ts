// Mirrors tuma_core::http (crates/core/src/http/model.rs) and the views in src-tauri/src/lib.rs.

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
  | { type: "raw"; content: string; contentType: string }
  | { type: "formUrlEncoded"; fields: KeyValue[] }
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

export interface Request {
  method: string;
  url: string;
  headers: KeyValue[];
  body: Body;
  auth: Auth;
  settings: Settings;
}

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
}

export interface RequestError {
  kind: string;
  message: string;
}

export interface HistoryEntry {
  id: string;
  /** Unix time in milliseconds. */
  sentAt: number;
  request: Request;
  status: number | null;
  error: string | null;
  totalMs: number | null;
  size: number | null;
}
