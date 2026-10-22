// The request being edited. It keeps more than a `Request` does (the query table, body text for every body type,
// fields for every auth type) so switching tabs or types never loses what was typed.

import type { Auth, Body, KeyValue, Part, Request, Settings } from "./types";

export type BodyMode = "none" | "json" | "xml" | "html" | "text" | "javascript" | "form" | "multipart" | "file";
export type AuthMode = Auth["type"];

export const RAW_TYPES: Partial<Record<BodyMode, string>> = {
  json: "application/json",
  xml: "application/xml",
  html: "text/html",
  text: "text/plain",
  javascript: "application/javascript",
};

export interface Draft {
  method: string;
  url: string;
  params: KeyValue[];
  headers: KeyValue[];
  bodyMode: BodyMode;
  raw: string;
  form: KeyValue[];
  parts: Part[];
  filePath: string;
  authMode: AuthMode;
  token: string;
  username: string;
  password: string;
  apiKey: string;
  apiValue: string;
  apiLocation: "header" | "query";
  settings: Settings;
}

export const newDraft = (): Draft => ({
  method: "GET",
  url: "",
  params: [],
  headers: [],
  bodyMode: "none",
  raw: "",
  form: [],
  parts: [],
  filePath: "",
  authMode: "none",
  token: "",
  username: "",
  password: "",
  apiKey: "",
  apiValue: "",
  apiLocation: "header",
  settings: { timeoutMs: 30000, followRedirects: true, maxRedirects: 10, verifyTls: true },
});

export function toRequest(d: Draft): Request {
  return { method: d.method, url: d.url, headers: d.headers, body: toBody(d), auth: toAuth(d), settings: d.settings };
}

function toBody(d: Draft): Body {
  const rawType = RAW_TYPES[d.bodyMode];
  if (rawType) return { type: "raw", content: d.raw, contentType: rawType };
  switch (d.bodyMode) {
    case "form":
      return { type: "formUrlEncoded", fields: d.form };
    case "multipart":
      return { type: "multipart", parts: d.parts };
    case "file":
      return d.filePath ? { type: "file", path: d.filePath } : { type: "none" };
    default:
      return { type: "none" };
  }
}

function toAuth(d: Draft): Auth {
  switch (d.authMode) {
    case "bearer":
      return { type: "bearer", token: d.token };
    case "basic":
      return { type: "basic", username: d.username, password: d.password };
    case "apiKey":
      return { type: "apiKey", key: d.apiKey, value: d.apiValue, location: d.apiLocation };
    default:
      return { type: "none" };
  }
}

/** Rebuilds a draft from a sent request, for reopening history. */
export function fromRequest(r: Request): Draft {
  const d: Draft = { ...newDraft(), method: r.method, url: r.url, headers: r.headers, settings: r.settings };
  d.params = parseQuery(r.url);
  const body = r.body;
  if (body.type === "raw") {
    const mode = (Object.keys(RAW_TYPES) as BodyMode[]).find((m) => RAW_TYPES[m] === body.contentType);
    d.bodyMode = mode ?? "text";
    d.raw = body.content;
  } else if (body.type === "formUrlEncoded") {
    d.bodyMode = "form";
    d.form = body.fields;
  } else if (body.type === "multipart") {
    d.bodyMode = "multipart";
    d.parts = body.parts;
  } else if (body.type === "file") {
    d.bodyMode = "file";
    d.filePath = body.path;
  }
  const auth = r.auth;
  d.authMode = auth.type;
  if (auth.type === "bearer") d.token = auth.token;
  if (auth.type === "basic") Object.assign(d, { username: auth.username, password: auth.password });
  if (auth.type === "apiKey") Object.assign(d, { apiKey: auth.key, apiValue: auth.value, apiLocation: auth.location });
  return d;
}

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
