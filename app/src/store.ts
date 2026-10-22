import { create } from "zustand";
import * as api from "./api";
import { type Draft, fromRequest, newDraft, parseQuery, toRequest, withQuery } from "./draft";
import type { HistoryEntry, KeyValue, RequestError, Response } from "./types";

interface State {
  draft: Draft;
  /** The id of the request in flight, if any. */
  sending: string | null;
  response: Response | null;
  error: RequestError | null;
  history: HistoryEntry[];

  update: (patch: Partial<Draft>) => void;
  setUrl: (url: string) => void;
  setParams: (params: KeyValue[]) => void;
  send: () => Promise<void>;
  cancel: () => void;
  open: (entry: HistoryEntry) => void;
  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;
}

export const useStore = create<State>((set, get) => ({
  draft: newDraft(),
  sending: null,
  response: null,
  error: null,
  history: [],

  update: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

  // The URL and the params table edit the same query string. Disabled rows exist only in the table, so they are kept.
  setUrl: (url) =>
    set((s) => ({
      draft: { ...s.draft, url, params: [...parseQuery(url), ...s.draft.params.filter((p) => !p.enabled)] },
    })),
  setParams: (params) => set((s) => ({ draft: { ...s.draft, params, url: withQuery(s.draft.url, params) } })),

  send: async () => {
    const { draft, sending } = get();
    if (sending || !draft.url.trim()) return;
    const id = crypto.randomUUID();
    set({ sending: id, error: null });
    try {
      const response = await api.sendRequest(id, toRequest(draft));
      if (get().sending === id) set({ response, error: null });
    } catch (e) {
      if (get().sending === id) set({ response: null, error: e as RequestError });
    } finally {
      if (get().sending === id) set({ sending: null });
      void get().loadHistory();
    }
  },

  cancel: () => {
    const id = get().sending;
    if (id) void api.cancelRequest(id);
  },

  open: (entry) => set({ draft: fromRequest(entry.request), response: null, error: null }),

  loadHistory: async () => set({ history: await api.historyList() }),

  clearHistory: async () => {
    await api.historyClear();
    set({ history: [] });
  },
}));
