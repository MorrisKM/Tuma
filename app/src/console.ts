// The console: one log of everything the app sends (requests, collection runs, WebSocket connections, gRPC calls),
// with what went out, what came back and what scripts printed. Kept in memory for this session only.

import { create } from "zustand";
import type { Header, LogLine, ScriptError, Timing } from "./types";

export type EntryKind = "http" | "run" | "ws" | "grpc";

export interface ConsoleEntry {
  id: number;
  /** Unix ms. */
  at: number;
  kind: EntryKind;
  /** HTTP method, or WS or GRPC. */
  method: string;
  url: string;
  status?: number;
  /** Such as "OK", or how a connection ended. */
  reason?: string;
  timeMs?: number;
  size?: number;
  /** Why it failed, when it did. */
  error?: string;
  /** Where it came from: the request's or run's name. */
  source?: string;
  /** How the request went over the network. */
  network?: {
    httpVersion?: string;
    remoteAddress?: string | null;
    /** Bytes received, before decompression. */
    encodedSize?: number;
    timing?: Timing;
  };
  request?: { headers: Header[]; body?: string; bodySize?: number };
  response?: { headers: Header[]; body?: string; redirects?: { status: number; url: string }[] };
  logs: LogLine[];
  scriptErrors: ScriptError[];
}

/** Entries kept; older ones are dropped. */
const LIMIT = 500;
/** Bodies longer than this are cut, so a large download doesn't stay in memory twice. */
const BODY_LIMIT = 64 * 1024;

export const cut = (body: string | undefined) =>
  body && body.length > BODY_LIMIT
    ? `${body.slice(0, BODY_LIMIT)}\n… (${(body.length - BODY_LIMIT).toLocaleString()} more characters; the response panel has all of it)`
    : body;

interface Console {
  entries: ConsoleEntry[];
  /** Entries added while the console was closed, for the badge on its button. */
  unseen: number;
  /** Errors among them, which make the badge red. */
  unseenErrors: number;
}

export const useConsole = create<Console>(() => ({ entries: [], unseen: 0, unseenErrors: 0 }));

let next = 1;

/** Adds an entry. `open` says whether the console is on screen, so it isn't counted as unseen. */
export function log(
  entry: Omit<ConsoleEntry, "id" | "at" | "logs" | "scriptErrors"> & Partial<ConsoleEntry>,
  open: boolean,
) {
  const full: ConsoleEntry = { logs: [], scriptErrors: [], ...entry, id: next++, at: entry.at ?? Date.now() };
  const failed = failedEntry(full);
  useConsole.setState((s) => ({
    entries: [...s.entries.slice(-(LIMIT - 1)), full],
    unseen: open ? 0 : s.unseen + 1,
    unseenErrors: open ? 0 : s.unseenErrors + (failed ? 1 : 0),
  }));
}

export const clearConsole = () => useConsole.setState({ entries: [], unseen: 0, unseenErrors: 0 });
export const markSeen = () => useConsole.setState({ unseen: 0, unseenErrors: 0 });

/** Failed to send, a 4xx or 5xx, a gRPC error status, or a script that threw or logged an error. */
export function failedEntry(e: ConsoleEntry): boolean {
  if (e.error || e.scriptErrors.length) return true;
  if (e.logs.some((l) => l.level === "error")) return true;
  if (e.kind === "grpc") return e.status !== undefined && e.status !== 0;
  return e.status !== undefined && e.status >= 400;
}
