// The workspace's scripts and their runs. Runs are kept here rather than in the page, so their output is still there
// after opening something else, and a run goes on while its page is closed.

import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import * as api from "./api";
import { useStore } from "./store";

export interface OutputLine {
  /** command, stdout, stderr, or a console level: log, info, warn, error, debug. */
  kind: string;
  text: string;
}

export interface ScriptRun {
  id: string;
  running: boolean;
  lines: OutputLine[];
  /** Lines left out at the start, past LINE_LIMIT. */
  dropped: number;
  outcome: api.ScriptOutcome | null;
  /** Why it could not run at all. */
  error: string | null;
  environment: string | null;
}

interface State {
  list: api.ScriptSummary[];
  /** The latest run of each script, by name. */
  runs: Record<string, ScriptRun>;
}

const LINE_LIMIT = 5000;

export const useScripts = create<State>(() => ({ list: [], runs: {} }));

const update = (name: string, patch: (run: ScriptRun) => Partial<ScriptRun>) =>
  useScripts.setState((s) => {
    const run = s.runs[name];
    return run ? { runs: { ...s.runs, [name]: { ...run, ...patch(run) } } } : s;
  });

listen<{ runId: string; event: api.ScriptEvent }>("script-event", ({ payload }) => {
  const entry = Object.entries(useScripts.getState().runs).find(([, r]) => r.id === payload.runId);
  if (!entry) return;
  const e = payload.event;
  const line = e.kind === "console" ? { kind: e.text.level, text: e.text.text } : { kind: e.kind, text: e.text };
  update(entry[0], (run) => {
    const lines = [...run.lines, line];
    const over = Math.max(0, lines.length - LINE_LIMIT);
    return { lines: over ? lines.slice(over) : lines, dropped: run.dropped + over };
  });
}).catch(() => {
  // Outside the app (tests, the browser preview) there are no events.
});

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const scripts = {
  async load() {
    try {
      useScripts.setState({ list: await api.scriptsList() });
    } catch {
      useScripts.setState({ list: [] });
    }
  },

  async create(kind: api.ScriptKind) {
    try {
      const name = await api.scriptCreate(kind === "command" ? "New shell script" : "New script", kind);
      await scripts.load();
      useStore.setState({ view: { kind: "script", name } });
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: message(e) });
    }
  },

  /** Returns the new name. */
  async rename(name: string, newName: string): Promise<string> {
    const renamed = await api.scriptRename(name, newName);
    useScripts.setState((s) => {
      const runs = { ...s.runs };
      if (runs[name]) {
        runs[renamed] = runs[name];
        delete runs[name];
      }
      return { runs };
    });
    await scripts.load();
    useStore.setState({ view: { kind: "script", name: renamed } });
    return renamed;
  },

  async remove(name: string) {
    await api.scriptDelete(name);
    useScripts.setState((s) => {
      const runs = { ...s.runs };
      delete runs[name];
      return { runs };
    });
    await scripts.load();
    useStore.setState({ view: { kind: "request" } });
  },

  async run(name: string, inputs: Record<string, string>, extraArgs: string) {
    const environment = useStore.getState().environment;
    const id = crypto.randomUUID();
    useScripts.setState((s) => ({
      runs: { ...s.runs, [name]: { id, running: true, lines: [], dropped: 0, outcome: null, error: null, environment } },
    }));
    try {
      const outcome = await api.scriptRun(id, name, environment, inputs, extraArgs);
      update(name, (run) => (run.id === id ? { running: false, outcome } : {}));
    } catch (e) {
      update(name, (run) => (run.id === id ? { running: false, error: message(e) } : {}));
    }
  },

  stop(name: string) {
    const run = useScripts.getState().runs[name];
    if (run?.running) void api.scriptStop(run.id);
  },

  clear(name: string) {
    useScripts.setState((s) => {
      const runs = { ...s.runs };
      if (!runs[name]?.running) delete runs[name];
      return { runs };
    });
  },
};
