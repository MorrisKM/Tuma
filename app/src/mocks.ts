// Mock servers running in the app, and the requests each one answered (the most recent ones, this session only).

import { create } from "zustand";
import * as api from "./api";
import { useStore } from "./store";

interface Mocks {
  servers: api.MockInfo[];
  events: Record<string, api.MockEvent[]>;
}

export const useMocks = create<Mocks>(() => ({ servers: [], events: {} }));

const KEPT = 300;
/** Tried in turn, so a mock usually keeps its address between runs; then any free port. */
const PORTS = [4010, 4011, 4012, 4013, 4014, 4015];

export const mocks = {
  refresh: async () => useMocks.setState({ servers: await api.mockList() }),

  /** Starts a mock server for `scope` and shows it. */
  start: async (scope: string | null, port?: number, delayMs = 0) => {
    const { notify } = useStore.getState();
    let lastError = "";
    for (const p of port ? [port] : [...PORTS, 0]) {
      try {
        const info = await api.mockStart(scope, p, delayMs);
        useMocks.setState((s) => ({ servers: [...s.servers, info] }));
        useStore.setState({ view: { kind: "mock", id: info.id } });
        return;
      } catch (e) {
        lastError = String(e);
        // Another port only helps when this one is taken.
        if (!lastError.includes("could not listen")) break;
      }
    }
    notify({ tone: "error", text: lastError });
  },

  stop: async (id: string) => {
    await api.mockStop(id);
    useMocks.setState((s) => ({ servers: s.servers.filter((m) => m.id !== id) }));
    const view = useStore.getState().view;
    if (view.kind === "mock" && view.id === id) useStore.setState({ view: { kind: "request" } });
  },

  record: (id: string, event: api.MockEvent) =>
    useMocks.setState((s) => ({ events: { ...s.events, [id]: [...(s.events[id] ?? []).slice(-(KEPT - 1)), event] } })),

  clear: (id: string) => useMocks.setState((s) => ({ events: { ...s.events, [id]: [] } })),
};

/** "Shop/Orders" → "Orders"; the whole workspace when null. */
export const mockName = (m: api.MockInfo) => (m.scope ? m.scope.split("/").pop()! : "Whole workspace");
