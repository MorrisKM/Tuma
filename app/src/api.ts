import { invoke } from "@tauri-apps/api/core";
import type { HistoryEntry, Request, Response } from "./types";

export const appVersion = () => invoke<string>("app_version");

/** Rejects with a `RequestError`. */
export const sendRequest = (id: string, request: Request) =>
  invoke<Response>("send_request", { id, request });

export const cancelRequest = (id: string) => invoke<void>("cancel_request", { id });

/** Newest first. */
export const historyList = () => invoke<HistoryEntry[]>("history_list");

export const historyClear = () => invoke<void>("history_clear");
