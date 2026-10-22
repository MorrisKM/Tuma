import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";
import * as api from "./api";
import {
  type Draft,
  fromRequestFile,
  nameFromPath,
  newDraft,
  parseQuery,
  toRequestFile,
  withQuery,
} from "./draft";
import type { HistoryEntry, KeyValue, RequestError, Response, TreeNode, WorkspaceInfo } from "./types";

/** What the main area shows. */
export type View =
  | { kind: "request" }
  | { kind: "folder"; path: string }
  /** Global variables, or an environment's. */
  | { kind: "variables"; environment: string | null }
  /** How to connect a coding agent over MCP. */
  | { kind: "agents" };

export interface Notice {
  tone: "info" | "error";
  text: string;
  details?: string[];
}

interface State {
  workspace: WorkspaceInfo | null;
  tree: TreeNode[];
  environments: string[];
  /** The selected environment, or null for none. */
  environment: string | null;
  expanded: Record<string, boolean>;
  view: View;
  draft: Draft;
  /** The draft as last opened or saved, to tell whether there are unsaved changes. */
  saved: string;
  /** Shows the "save to" dialog for a request that is not saved yet. */
  savingAs: boolean;
  /** The path of the tree item being renamed. */
  renaming: string | null;
  sending: string | null;
  response: Response | null;
  error: RequestError | null;
  history: HistoryEntry[];
  notice: Notice | null;

  init: () => Promise<void>;
  openWorkspace: () => Promise<void>;
  closeWorkspace: () => Promise<void>;
  refresh: () => Promise<void>;
  setEnvironment: (name: string | null) => void;
  toggle: (path: string, open?: boolean) => void;

  update: (patch: Partial<Draft>) => void;
  setUrl: (url: string) => void;
  setParams: (params: KeyValue[]) => void;
  isDirty: () => boolean;
  newRequest: () => Promise<void>;
  openRequest: (path: string) => Promise<void>;
  openFolder: (path: string) => Promise<void>;
  openVariables: (environment: string | null) => Promise<void>;
  save: () => Promise<void>;
  saveAs: (parent: string, name: string) => Promise<void>;
  addRequest: (parent: string) => Promise<void>;
  addFolder: (parent: string | null) => Promise<void>;
  rename: (path: string, name: string) => Promise<void>;
  remove: (node: TreeNode) => Promise<void>;
  importFile: () => Promise<void>;

  send: () => Promise<void>;
  cancel: () => void;
  openHistory: (entry: HistoryEntry) => Promise<void>;
  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;
  notify: (notice: Notice | null) => void;
}

const snapshot = (d: Draft) => JSON.stringify({ ...toRequestFile(d), path: d.path });
const message = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

// The selected environment is remembered per workspace, on this machine only.
const envKey = (root: string) => `tuma.environment:${root}`;
function loadEnvironment(root: string): string | null {
  try {
    return localStorage.getItem(envKey(root));
  } catch {
    return null;
  }
}
function storeEnvironment(root: string, name: string | null) {
  try {
    if (name) localStorage.setItem(envKey(root), name);
    else localStorage.removeItem(envKey(root));
  } catch {
    // Not being remembered is fine.
  }
}

export const useStore = create<State>((set, get) => {
  /** Runs an action and shows its error, if any, as a notice. */
  const attempt = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (e) {
      set({ notice: { tone: "error", text: message(e) } });
    }
  };

  /** Asks before throwing away unsaved changes. Resolves true when it is fine to continue. */
  const confirmDiscard = async () =>
    !get().isDirty() ||
    (await ask("You have unsaved changes to this request. Discard them?", {
      title: "Unsaved changes",
      kind: "warning",
      okLabel: "Discard",
      cancelLabel: "Keep editing",
    }));

  const load = (draft: Draft) =>
    set({ draft, saved: snapshot(draft), view: { kind: "request" }, response: null, error: null });

  const setWorkspace = async (workspace: WorkspaceInfo | null) => {
    const environment = workspace ? loadEnvironment(workspace.root) : null;
    set({ workspace, environment, tree: [], environments: [], expanded: {} });
    await get().refresh();
    // A remembered environment that no longer exists is dropped.
    if (environment && !get().environments.includes(environment)) get().setEnvironment(null);
  };

  return {
    workspace: null,
    tree: [],
    environments: [],
    environment: null,
    expanded: {},
    view: { kind: "request" },
    draft: newDraft(),
    saved: snapshot(newDraft()),
    savingAs: false,
    renaming: null,
    sending: null,
    response: null,
    error: null,
    history: [],
    notice: null,

    init: () =>
      attempt(async () => {
        await setWorkspace(await api.workspaceCurrent());
        await get().loadHistory();
      }),

    openWorkspace: () =>
      attempt(async () => {
        const root = await openDialog({ directory: true, title: "Open or create a workspace folder" });
        if (typeof root !== "string" || !(await confirmDiscard())) return;
        let workspace: WorkspaceInfo;
        try {
          workspace = await api.workspaceOpen(root);
        } catch {
          const name = root.split(/[\\/]/).filter(Boolean).pop() ?? "Workspace";
          const create = await ask(`${root} is not a Tuma workspace yet. Make it one?`, {
            title: "New workspace",
            okLabel: "Create workspace",
            cancelLabel: "Cancel",
          });
          if (!create) return;
          workspace = await api.workspaceCreate(root, name);
        }
        load(newDraft());
        await setWorkspace(workspace);
      }),

    closeWorkspace: () =>
      attempt(async () => {
        if (!(await confirmDiscard())) return;
        await api.workspaceClose();
        load(newDraft());
        await setWorkspace(null);
      }),

    refresh: async () => {
      if (!get().workspace) return;
      const [tree, environments] = await Promise.all([api.tree(), api.environments()]);
      set({ tree, environments });
    },

    setEnvironment: (environment) => {
      const root = get().workspace?.root;
      if (root) storeEnvironment(root, environment);
      set({ environment });
    },

    toggle: (path, open) =>
      set((s) => ({ expanded: { ...s.expanded, [path]: open ?? !s.expanded[path] } })),

    update: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

    // The URL and the params table edit the same query string. Disabled rows exist only in the table, so they are kept.
    setUrl: (url) =>
      set((s) => ({
        draft: { ...s.draft, url, params: [...parseQuery(url), ...s.draft.params.filter((p) => !p.enabled)] },
      })),
    setParams: (params) => set((s) => ({ draft: { ...s.draft, params, url: withQuery(s.draft.url, params) } })),

    isDirty: () => {
      const { draft, saved } = get();
      // A blank new request has nothing worth keeping.
      if (!draft.path && !draft.url.trim()) return false;
      return snapshot(draft) !== saved;
    },

    newRequest: () =>
      attempt(async () => {
        if (await confirmDiscard()) load(newDraft());
      }),

    openRequest: (path) =>
      attempt(async () => {
        if (get().draft.path === path && get().view.kind === "request") return;
        if (!(await confirmDiscard())) return;
        load(fromRequestFile(await api.requestRead(path), path, nameFromPath(path)));
      }),

    openFolder: async (path) => set({ view: { kind: "folder", path } }),

    openVariables: async (environment) => set({ view: { kind: "variables", environment } }),

    save: () =>
      attempt(async () => {
        const { draft, workspace } = get();
        if (!workspace) {
          set({ notice: { tone: "info", text: "Open a workspace folder to save requests." } });
          return;
        }
        if (!draft.path) {
          set({ savingAs: true });
          return;
        }
        await api.requestWrite(draft.path, toRequestFile(draft));
        set({ saved: snapshot(draft) });
        await get().refresh();
      }),

    saveAs: (parent, name) =>
      attempt(async () => {
        const { draft } = get();
        const path = await api.requestCreate(parent, name, toRequestFile(draft));
        const saved = { ...draft, path, name: nameFromPath(path) };
        set({ draft: saved, saved: snapshot(saved), savingAs: false });
        get().toggle(parent, true);
        await get().refresh();
      }),

    addRequest: (parent) =>
      attempt(async () => {
        if (!(await confirmDiscard())) return;
        const draft = newDraft();
        const path = await api.requestCreate(parent, "New request", toRequestFile(draft));
        get().toggle(parent, true);
        await get().refresh();
        load({ ...draft, path, name: nameFromPath(path) });
        set({ renaming: path });
      }),

    addFolder: (parent) =>
      attempt(async () => {
        const path = await api.folderCreate(parent, parent ? "New folder" : "New collection");
        if (parent) get().toggle(parent, true);
        await get().refresh();
        set({ renaming: path });
      }),

    rename: (path, name) =>
      attempt(async () => {
        set({ renaming: null });
        const newPath = await api.itemRename(path, name);
        if (newPath === path) return;
        // Requests and views inside a renamed folder move with it.
        const moved = (p: string | null) =>
          p === path ? newPath : p?.startsWith(`${path}/`) ? newPath + p.slice(path.length) : p;
        const { draft, saved, view } = get();
        const draftPath = moved(draft.path);
        if (draftPath !== draft.path) {
          const wasClean = snapshot(draft) === saved;
          const next = { ...draft, path: draftPath, name: nameFromPath(draftPath!) };
          set({ draft: next, saved: wasClean ? snapshot(next) : saved });
        }
        if (view.kind === "folder") set({ view: { kind: "folder", path: moved(view.path)! } });
        await get().refresh();
      }),

    remove: (node) =>
      attempt(async () => {
        const what = node.kind === "request" ? "request" : `${node.kind} and everything in it`;
        const ok = await ask(`Delete the ${what}? "${node.name}" will be removed from disk.`, {
          title: "Delete",
          kind: "warning",
          okLabel: "Delete",
          cancelLabel: "Cancel",
        });
        if (!ok) return;
        await api.itemDelete(node.path);
        const inside = (p: string | null) => p === node.path || !!p?.startsWith(`${node.path}/`);
        const { draft, view } = get();
        // The open request stays in the editor as an unsaved one.
        if (inside(draft.path)) set({ draft: { ...draft, path: null }, saved: "" });
        if (view.kind === "folder" && inside(view.path)) set({ view: { kind: "request" } });
        await get().refresh();
      }),

    importFile: () =>
      attempt(async () => {
        if (!get().workspace) {
          set({ notice: { tone: "info", text: "Open a workspace folder first, then import into it." } });
          return;
        }
        const file = await openDialog({
          title: "Import an OpenAPI spec or a Postman export",
          filters: [{ name: "OpenAPI spec or Postman export", extensions: ["json", "yaml", "yml"] }],
        });
        if (typeof file !== "string") return;
        const result = await api.importFile(file);
        await get().refresh();
        const what =
          result.kind === "collection"
            ? `collection "${result.path}"`
            : result.kind === "environment"
              ? `environment "${result.name}"`
              : "global variables";
        if (result.kind === "collection") get().toggle(result.path, true);
        set({
          notice: {
            tone: "info",
            text: result.warnings.length
              ? `Imported ${what}. Some things could not be carried over:`
              : `Imported ${what}.`,
            details: result.warnings,
          },
        });
      }),

    send: async () => {
      const { draft, sending, environment } = get();
      if (sending || !draft.url.trim()) return;
      const id = crypto.randomUUID();
      set({ sending: id, error: null, view: { kind: "request" } });
      try {
        const response = await api.sendRequest(id, toRequestFile(draft), draft.path, environment);
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

    openHistory: (entry) =>
      attempt(async () => {
        if (!(await confirmDiscard())) return;
        // History keeps the request as sent; it opens unsaved, so saving it does not overwrite the original.
        load({ ...fromRequestFile(entry.request, null, "Untitled") });
      }),

    loadHistory: async () => set({ history: await api.historyList() }),

    clearHistory: async () => {
      await api.historyClear();
      set({ history: [] });
    },

    notify: (notice) => set({ notice }),
  };
});

/** Collections and folders, flattened with their depth, for pickers. */
export function folders(tree: TreeNode[], depth = 0): { path: string; name: string; depth: number }[] {
  return tree.flatMap((n) =>
    n.kind === "request" ? [] : [{ path: n.path, name: n.name, depth }, ...folders(n.children ?? [], depth + 1)],
  );
}
