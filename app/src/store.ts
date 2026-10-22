import { ask, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
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
import type {
  HistoryEntry,
  KeyValue,
  RequestError,
  Response,
  SseEvent,
  TreeNode,
  VariableHint,
  WorkspaceInfo,
} from "./types";

/** What the main area shows. */
export type View =
  | { kind: "request" }
  | { kind: "folder"; path: string }
  /** Global variables, or an environment's. */
  | { kind: "variables"; environment: string | null }
  /** How to connect a coding agent over MCP. */
  | { kind: "agents" }
  /** The collection runner, for a collection or folder. */
  | { kind: "runner"; path: string }
  /** This machine's settings: proxy, certificates, cookies. */
  | { kind: "settings" };

export interface Notice {
  tone: "info" | "error";
  text: string;
  details?: string[];
}

/** An open request tab. The active tab's live state is kept in the store's top-level fields. */
export interface Tab {
  id: string;
  draft: Draft;
  /** The draft as last opened or saved, to tell whether there are unsaved changes. */
  saved: string;
  response: Response | null;
  error: RequestError | null;
  /** The id of the request in flight, if any. */
  sending: string | null;
  /** A preview tab is replaced by the next request opened from the tree, until it is edited. */
  preview: boolean;
}

interface State {
  workspace: WorkspaceInfo | null;
  tree: TreeNode[];
  environments: string[];
  /** The selected environment, or null for none. */
  environment: string | null;
  expanded: Record<string, boolean>;
  view: View;

  tabs: Tab[];
  activeTab: string;
  // The active tab, live. Components read these; the copy in `tabs` is refreshed when switching away.
  draft: Draft;
  saved: string;
  response: Response | null;
  error: RequestError | null;
  sending: string | null;
  preview: boolean;

  /** Variables the active request can use, for the URL bar's hints. */
  scope: VariableHint[];
  /** Shows the "save to" dialog for a request that is not saved yet. */
  savingAs: boolean;
  /** The path of the tree item being renamed. */
  renaming: string | null;
  paletteOpen: boolean;
  /** Shows the code snippet dialog. */
  snippetOpen: boolean;
  history: HistoryEntry[];
  notice: Notice | null;
  /** Server-sent events arriving for requests in flight, by send id. */
  live: Record<string, SseEvent[]>;

  init: () => Promise<void>;
  openWorkspace: () => Promise<void>;
  closeWorkspace: () => Promise<void>;
  refresh: () => Promise<void>;
  loadScope: () => Promise<void>;
  setEnvironment: (name: string | null) => void;
  toggle: (path: string, open?: boolean) => void;

  update: (patch: Partial<Draft>) => void;
  setUrl: (url: string) => void;
  setParams: (params: KeyValue[]) => void;
  isDirty: () => boolean;
  /** The tabs with the active one up to date, for display. */
  allTabs: () => Tab[];
  newTab: () => void;
  switchTab: (id: string) => void;
  closeTab: (id: string) => Promise<void>;
  /** Moves to the next (1) or previous (-1) tab. */
  cycleTab: (step: number) => void;
  /** Keeps the active preview tab open. */
  pin: () => void;
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
  exportCollection: (path: string) => Promise<void>;
  /** Replaces the active request with a cURL command's. */
  importCurl: (command: string) => Promise<void>;

  send: () => Promise<void>;
  cancel: () => void;
  openHistory: (entry: HistoryEntry) => Promise<void>;
  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;
  notify: (notice: Notice | null) => void;
}

const snapshot = (d: Draft) => JSON.stringify({ ...toRequestFile(d), path: d.path });
const message = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/** Whether a tab has changes worth keeping. A blank new request has none. */
export const tabDirty = (t: Pick<Tab, "draft" | "saved">) =>
  !(!t.draft.path && !t.draft.url.trim()) && snapshot(t.draft) !== t.saved;

const blankTab = (): Tab => {
  const draft = newDraft();
  return { id: crypto.randomUUID(), draft, saved: snapshot(draft), response: null, error: null, sending: null, preview: false };
};

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

const first = blankTab();

export const useStore = create<State>((set, get) => {
  /** Runs an action and shows its error, if any, as a notice. */
  const attempt = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (e) {
      set({ notice: { tone: "error", text: message(e) } });
    }
  };

  const ask_ = (text: string) =>
    ask(text, { title: "Unsaved changes", kind: "warning", okLabel: "Discard", cancelLabel: "Keep editing" });

  /** The active tab as it is now. */
  const current = (): Tab => {
    const s = get();
    return {
      id: s.activeTab,
      draft: s.draft,
      saved: s.saved,
      response: s.response,
      error: s.error,
      sending: s.sending,
      preview: s.preview,
    };
  };

  /** Makes `tab` the active one, storing the one that was active. */
  const activate = (tab: Tab, tabs: Tab[]) => {
    set({
      tabs,
      activeTab: tab.id,
      draft: tab.draft,
      saved: tab.saved,
      response: tab.response,
      error: tab.error,
      sending: tab.sending,
      preview: tab.preview,
      view: { kind: "request" },
    });
    void get().loadScope();
  };

  /** Updates a tab, whether or not it is the active one. */
  const patchTab = (id: string, patch: Partial<Tab>) => {
    if (id === get().activeTab) {
      const { id: _, ...rest } = patch;
      set(rest);
    } else {
      set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
    }
  };

  /** Opens a draft: in the active tab when it is a preview or blank, otherwise in a new tab. */
  const openDraft = (draft: Draft, preview: boolean) => {
    const tab: Tab = { ...blankTab(), draft, saved: snapshot(draft), preview };
    const active = current();
    const reusable = (active.preview && !tabDirty(active)) || (!active.draft.path && !active.draft.url.trim());
    const tabs = get().allTabs();
    if (reusable && !active.sending) {
      activate(tab, tabs.map((t) => (t.id === active.id ? tab : t)));
    } else {
      const at = tabs.findIndex((t) => t.id === active.id);
      activate(tab, [...tabs.slice(0, at + 1), tab, ...tabs.slice(at + 1)]);
    }
  };

  /** Applies a change of path (a rename or delete) to every tab's draft. */
  const repath = (change: (path: string | null) => string | null | undefined) => {
    const fix = (t: Tab): Tab => {
      const path = change(t.draft.path);
      if (path === undefined || path === t.draft.path) return t;
      const wasClean = !tabDirty(t);
      const draft = { ...t.draft, path, name: path ? nameFromPath(path) : t.draft.name };
      // A deleted request stays open as an unsaved one.
      return { ...t, draft, saved: path && wasClean ? snapshot(draft) : path ? t.saved : "" };
    };
    const tabs = get().allTabs().map(fix);
    const active = tabs.find((t) => t.id === get().activeTab)!;
    set({ tabs, draft: active.draft, saved: active.saved });
  };

  const anyDirty = () => get().allTabs().some(tabDirty);

  const setWorkspace = async (workspace: WorkspaceInfo | null) => {
    const environment = workspace ? loadEnvironment(workspace.root) : null;
    set({ workspace, environment, tree: [], environments: [], expanded: {} });
    await get().refresh();
    // A remembered environment that no longer exists is dropped.
    if (environment && !get().environments.includes(environment)) get().setEnvironment(null);
    void get().loadScope();
  };

  const resetTabs = () => {
    const tab = blankTab();
    activate(tab, [tab]);
  };

  return {
    workspace: null,
    tree: [],
    environments: [],
    environment: null,
    expanded: {},
    view: { kind: "request" },
    tabs: [first],
    activeTab: first.id,
    draft: first.draft,
    saved: first.saved,
    response: null,
    error: null,
    sending: null,
    preview: false,
    scope: [],
    savingAs: false,
    renaming: null,
    paletteOpen: false,
    snippetOpen: false,
    history: [],
    notice: null,
    live: {},

    init: () =>
      attempt(async () => {
        await setWorkspace(await api.workspaceCurrent());
        await get().loadHistory();
      }),

    openWorkspace: () =>
      attempt(async () => {
        const root = await openDialog({ directory: true, title: "Open or create a workspace folder" });
        if (typeof root !== "string") return;
        if (anyDirty() && !(await ask_("Some open requests have unsaved changes. Discard them?"))) return;
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
        resetTabs();
        await setWorkspace(workspace);
      }),

    closeWorkspace: () =>
      attempt(async () => {
        if (anyDirty() && !(await ask_("Some open requests have unsaved changes. Discard them?"))) return;
        await api.workspaceClose();
        resetTabs();
        await setWorkspace(null);
      }),

    refresh: async () => {
      if (!get().workspace) return;
      const [tree, environments] = await Promise.all([api.tree(), api.environments()]);
      set({ tree, environments });
    },

    loadScope: async () => {
      const { workspace, draft, environment } = get();
      if (!workspace) {
        set({ scope: [] });
        return;
      }
      try {
        set({ scope: await api.variablesInScope(draft.path, environment) });
      } catch {
        set({ scope: [] });
      }
    },

    setEnvironment: (environment) => {
      const root = get().workspace?.root;
      if (root) storeEnvironment(root, environment);
      set({ environment });
      void get().loadScope();
    },

    toggle: (path, open) =>
      set((s) => ({ expanded: { ...s.expanded, [path]: open ?? !s.expanded[path] } })),

    update: (patch) => set((s) => ({ draft: { ...s.draft, ...patch }, preview: false })),

    // The URL and the params table edit the same query string. Disabled rows exist only in the table, so they are kept.
    setUrl: (url) =>
      set((s) => ({
        draft: { ...s.draft, url, params: [...parseQuery(url), ...s.draft.params.filter((p) => !p.enabled)] },
        preview: false,
      })),
    setParams: (params) =>
      set((s) => ({ draft: { ...s.draft, params, url: withQuery(s.draft.url, params) }, preview: false })),

    isDirty: () => tabDirty(get()),

    allTabs: () => {
      const active = current();
      return get().tabs.map((t) => (t.id === active.id ? active : t));
    },

    newTab: () => {
      const tab = blankTab();
      const tabs = get().allTabs();
      const at = tabs.findIndex((t) => t.id === get().activeTab);
      activate(tab, [...tabs.slice(0, at + 1), tab, ...tabs.slice(at + 1)]);
    },

    switchTab: (id) => {
      if (id === get().activeTab) {
        set({ view: { kind: "request" } });
        return;
      }
      const tabs = get().allTabs();
      const tab = tabs.find((t) => t.id === id);
      if (tab) activate(tab, tabs);
    },

    closeTab: (id) =>
      attempt(async () => {
        const tabs = get().allTabs();
        const index = tabs.findIndex((t) => t.id === id);
        const tab = tabs[index];
        if (!tab) return;
        if (tabDirty(tab) && !(await ask_(`"${tab.draft.name}" has unsaved changes. Discard them?`))) return;
        if (tab.sending) void api.cancelRequest(tab.sending);
        const rest = tabs.filter((t) => t.id !== id);
        if (rest.length === 0) {
          resetTabs();
        } else if (id === get().activeTab) {
          activate(rest[Math.min(index, rest.length - 1)], rest);
        } else {
          set({ tabs: rest });
        }
      }),

    cycleTab: (step) => {
      const tabs = get().allTabs();
      const index = tabs.findIndex((t) => t.id === get().activeTab);
      const next = tabs[(index + step + tabs.length) % tabs.length];
      if (next) activate(next, tabs);
    },

    pin: () => set({ preview: false }),

    openRequest: (path) =>
      attempt(async () => {
        const open = get().allTabs().find((t) => t.draft.path === path);
        if (open) {
          get().switchTab(open.id);
          return;
        }
        openDraft(fromRequestFile(await api.requestRead(path), path, nameFromPath(path)), true);
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
        set({ saved: snapshot(draft), preview: false });
        await get().refresh();
      }),

    saveAs: (parent, name) =>
      attempt(async () => {
        const { draft } = get();
        const path = await api.requestCreate(parent, name, toRequestFile(draft));
        const saved = { ...draft, path, name: nameFromPath(path) };
        set({ draft: saved, saved: snapshot(saved), savingAs: false, preview: false });
        get().toggle(parent, true);
        await get().refresh();
        void get().loadScope();
      }),

    addRequest: (parent) =>
      attempt(async () => {
        const draft = newDraft();
        const path = await api.requestCreate(parent, "New request", toRequestFile(draft));
        get().toggle(parent, true);
        await get().refresh();
        openDraft({ ...draft, path, name: nameFromPath(path) }, false);
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
        repath(moved);
        const { view } = get();
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
        repath((p) => (inside(p) ? null : undefined));
        const { view } = get();
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
          title: "Import an OpenAPI spec, a Postman export or a HAR file",
          filters: [{ name: "OpenAPI, Postman or HAR", extensions: ["json", "yaml", "yml", "har"] }],
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

    exportCollection: (path) =>
      attempt(async () => {
        const file = await saveDialog({
          title: "Export as a Postman collection",
          defaultPath: `${path}.postman_collection.json`,
          filters: [{ name: "Postman collection", extensions: ["json"] }],
        });
        if (!file) return;
        await api.exportPostman(path, file);
        set({ notice: { tone: "info", text: `Exported "${path}" to ${file}.` } });
      }),

    importCurl: (command) =>
      attempt(async () => {
        const [file, ignored] = await api.parseCurl(command);
        const { draft } = get();
        // Keep where the request is saved and what it is called; take everything else from the command.
        const next = fromRequestFile(file, draft.path, draft.name);
        set({ draft: next, preview: false });
        if (ignored.length) {
          set({ notice: { tone: "info", text: `Imported the cURL command. Ignored: ${ignored.join(", ")}` } });
        }
      }),

    send: async () => {
      const { draft, sending, environment, activeTab } = get();
      if (sending || !draft.url.trim()) return;
      const id = crypto.randomUUID();
      set({ sending: id, error: null, view: { kind: "request" }, preview: false });
      try {
        const response = await api.sendRequest(id, toRequestFile(draft), draft.path, environment);
        patchTab(activeTab, { response, error: null });
      } catch (e) {
        patchTab(activeTab, { response: null, error: e as RequestError });
      } finally {
        patchTab(activeTab, { sending: null });
        set((s) => {
          const { [id]: _, ...live } = s.live;
          return { live };
        });
        void get().loadHistory();
      }
    },

    cancel: () => {
      const id = get().sending;
      if (id) void api.cancelRequest(id);
    },

    openHistory: (entry) =>
      attempt(async () => {
        // History keeps the request as sent; it opens unsaved, so saving it does not overwrite the original.
        openDraft(fromRequestFile(entry.request, null, "Untitled"), false);
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

/** Every saved request, for search. */
export function requests(tree: TreeNode[]): TreeNode[] {
  return tree.flatMap((n) => (n.kind === "request" ? [n] : requests(n.children ?? [])));
}
