import {
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Download,
  Folder,
  FolderOpen,
  FolderPlus,
  Globe,
  Layers,
  Library,
  Monitor,
  Moon,
  MoreHorizontal,
  Play,
  Plus,
  Search,
  Settings,
  Sun,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import * as api from "../api";
import { layout, type Section, useLayout } from "../layout";
import { requests, useStore } from "../store";
import { applyTheme, type Theme, useTheme } from "../theme";
import type { HistoryEntry, TreeNode } from "../types";
import { Splitter } from "./Splitter";
import { Logo, Menu, MethodTag } from "./ui";

const SECTIONS: { id: Section; label: string; icon: typeof Library }[] = [
  { id: "collections", label: "Collections", icon: Library },
  { id: "environments", label: "Environments", icon: Layers },
  { id: "history", label: "History", icon: Clock },
];

/** The icon rail and, beside it, the panel of the chosen section, which can be resized and hidden. */
export function Sidebar() {
  const { section, sidebarOpen, sidebarWidth } = useLayout();
  const panel = useRef<HTMLElement>(null);

  return (
    <>
      <Rail />
      {sidebarOpen && (
        <>
          <aside className="sidebar" ref={panel} style={{ width: sidebarWidth }}>
            <WorkspaceSwitcher />
            {section === "collections" && <Collections />}
            {section === "environments" && <Environments />}
            {section === "history" && <History />}
          </aside>
          <Splitter
            axis="x"
            label="Resize the sidebar"
            onDrag={(x) => layout.setSidebarWidth(x - (panel.current?.getBoundingClientRect().left ?? 0))}
            onStep={(dx) => layout.setSidebarWidth(useLayout.getState().sidebarWidth + dx)}
            onReset={layout.resetSidebar}
          />
        </>
      )}
    </>
  );
}

const THEMES: { id: Theme; label: string; icon: typeof Sun }[] = [
  { id: "system", label: "Theme: follow the system", icon: Monitor },
  { id: "light", label: "Theme: light", icon: Sun },
  { id: "dark", label: "Theme: dark", icon: Moon },
];

function Rail() {
  const { section, sidebarOpen } = useLayout();
  const view = useStore((s) => s.view.kind);
  const theme = useTheme();
  const current = THEMES.find((t) => t.id === theme) ?? THEMES[0];
  const next = THEMES[(THEMES.indexOf(current) + 1) % THEMES.length];

  return (
    <nav className="rail" aria-label="Sections">
      <div className="rail-logo" title="Tuma">
        <Logo />
      </div>
      {SECTIONS.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          className={`rail-button${sidebarOpen && section === id ? " active" : ""}`}
          onClick={() => layout.show(id)}
          aria-label={label}
          aria-pressed={sidebarOpen && section === id}
        >
          <Icon size={18} strokeWidth={1.75} />
          <span className="rail-label">{label}</span>
        </button>
      ))}
      <div className="rail-spacer" />
      <button
        className={`rail-button${view === "agents" ? " active" : ""}`}
        onClick={() => useStore.setState({ view: { kind: "agents" } })}
        aria-label="Connect a coding agent"
      >
        <Bot size={18} strokeWidth={1.75} />
        <span className="rail-label">Connect an agent</span>
      </button>
      <button
        className="rail-button"
        onClick={() => applyTheme(next.id)}
        aria-label={current.label}
      >
        <current.icon size={18} strokeWidth={1.75} />
        <span className="rail-label">{current.label}</span>
      </button>
      <button
        className={`rail-button${view === "settings" ? " active" : ""}`}
        onClick={() => useStore.setState({ view: { kind: "settings" } })}
        aria-label="Settings"
      >
        <Settings size={18} strokeWidth={1.75} />
        <span className="rail-label">Settings</span>
      </button>
    </nav>
  );
}

function WorkspaceSwitcher() {
  const workspace = useStore((s) => s.workspace);
  const [menu, setMenu] = useState(false);
  const [list, setList] = useState<api.WorkspaceList | null>(null);
  const { openWorkspace, closeWorkspace, switchWorkspace, importWorkspace, exportWorkspace } = useStore.getState();
  const toggle = () => {
    if (!menu) void api.workspaceList().then(setList, () => setList(null));
    setMenu(!menu);
  };

  return (
    <div className="sidebar-workspace">
      <button className="workspace-name" onClick={toggle} title={workspace?.root} aria-haspopup="menu">
        {workspace ? (
          <>
            <span className="workspace-badge">{workspace.name.slice(0, 1).toUpperCase()}</span>
            <span className="workspace-label">{workspace.name}</span>
          </>
        ) : (
          <>
            <FolderOpen size={15} />
            <span className="workspace-label">Choose a workspace</span>
          </>
        )}
        <ChevronDown size={14} className="muted" />
      </button>
      {menu && (
        <Menu onClose={() => setMenu(false)} className="menu-left workspace-menu">
          {list && list.workspaces.length > 0 && (
            <>
              <div className="menu-head">
                <span>Workspaces</span>
              </div>
              {list.workspaces.map((w) => (
                <button
                  key={w.root}
                  className="menu-check"
                  onClick={() => void switchWorkspace(w.root)}
                  title={w.root}
                >
                  <Check size={14} className={w.root === workspace?.root ? "accent" : "hidden"} />
                  <span className="menu-text">{w.name}</span>
                  {!w.inFolder && <span className="muted small">elsewhere</span>}
                </button>
              ))}
              <hr />
            </>
          )}
          <button className="menu-check" onClick={() => useStore.setState({ creatingWorkspace: true })}>
            <Plus size={14} />
            New workspace…
          </button>
          <button className="menu-check" onClick={() => void importWorkspace("folder")}>
            <Download size={14} />
            Import a workspace folder…
          </button>
          <button className="menu-check" onClick={() => void importWorkspace("zip")}>
            <Download size={14} />
            Import a workspace .zip…
          </button>
          <button className="menu-check" onClick={() => void openWorkspace()}>
            <FolderOpen size={14} />
            Open a folder in place…
          </button>
          {workspace && (
            <>
              <hr />
              <button className="menu-check" onClick={() => void exportWorkspace()}>
                <Upload size={14} />
                Export as .zip…
              </button>
              <button className="menu-check" onClick={() => void closeWorkspace()}>
                <X size={14} />
                Close workspace
              </button>
            </>
          )}
          {list && <div className="menu-caption mono">New workspaces go in {list.folder}</div>}
        </Menu>
      )}
    </div>
  );
}

/** A section's title with its actions on the right. */
function SectionHead({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="section-head">
      <span className="section-name">{title}</span>
      <span className="section-actions">{children}</span>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  children,
  className = "",
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <button className={`icon-button ${className}`} onClick={onClick} title={label} aria-label={label}>
      {children}
    </button>
  );
}

function SearchBox({
  value,
  onChange,
  onEnter,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  onEnter?: () => void;
  placeholder: string;
}) {
  return (
    <label className="search-box">
      <Search size={14} />
      <input
        type="search"
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onChange("");
          if (e.key === "Enter") onEnter?.();
        }}
      />
    </label>
  );
}

function Collections() {
  const workspace = useStore((s) => s.workspace);
  const tree = useStore((s) => s.tree);
  const { openWorkspace, importFile, addFolder, newTab } = useStore.getState();
  const [query, setQuery] = useState("");

  if (!workspace) {
    return (
      <div className="sidebar-empty">
        <FolderOpen size={28} strokeWidth={1.5} className="muted" />
        <p className="muted small">
          A workspace holds your collections, requests and environments as plain files, so you can keep them in git and
          share them.
        </p>
        <button className="button primary" onClick={() => useStore.setState({ creatingWorkspace: true })}>
          Create a workspace
        </button>
        <button className="button" onClick={() => void useStore.getState().importWorkspace("folder")}>
          Import a workspace
        </button>
        <button className="button ghost" onClick={() => void openWorkspace()}>
          Open a folder in place
        </button>
        <button className="button ghost" onClick={newTab}>
          Send a request without one
        </button>
      </div>
    );
  }
  const q = query.trim().toLowerCase();
  const found = q ? requests(tree).filter((r) => `${r.path} ${r.method}`.toLowerCase().includes(q)) : [];
  return (
    <>
      <SectionHead title="Collections">
        <IconButton label="New request (Ctrl+T)" onClick={newTab}>
          <Plus size={16} />
        </IconButton>
        <IconButton label="New collection" onClick={() => void addFolder(null)}>
          <FolderPlus size={16} />
        </IconButton>
        <IconButton label="Import OpenAPI, Postman or cURL" onClick={() => void importFile()}>
          <Download size={16} />
        </IconButton>
      </SectionHead>
      {tree.length > 0 && (
        <div className="sidebar-search">
          <SearchBox
            value={query}
            onChange={setQuery}
            placeholder="Filter requests"
            onEnter={() => found[0] && void useStore.getState().openRequest(found[0].path)}
          />
        </div>
      )}
      <div className="sidebar-body">
        {tree.length === 0 ? (
          <div className="sidebar-empty">
            <Library size={28} strokeWidth={1.5} className="muted" />
            <p className="muted small">
              No collections yet. A collection groups requests with shared auth, headers and variables.
            </p>
            <button className="button primary" onClick={() => void addFolder(null)}>
              New collection
            </button>
            <button className="button" onClick={() => void importFile()}>
              Import OpenAPI or Postman
            </button>
          </div>
        ) : q ? (
          <ul className="tree" role="listbox">
            {found.map((r) => (
              <li key={r.path}>
                <button
                  className="list-item"
                  onClick={() => void useStore.getState().openRequest(r.path)}
                  title={r.path}
                >
                  <MethodTag method={r.method ?? ""} />
                  <span className="list-item-text">
                    {r.name}
                    <span className="muted small"> {r.path.replace(/\/[^/]*$/, "")}</span>
                  </span>
                </button>
              </li>
            ))}
            {found.length === 0 && <li className="muted small sidebar-note">No request matches.</li>}
          </ul>
        ) : (
          <ul className="tree" role="tree">
            {tree.map((node) => (
              <TreeItem key={node.path} node={node} depth={0} />
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

function TreeItem({ node, depth }: { node: TreeNode; depth: number }) {
  const open = useStore((s) => !!s.expanded[node.path]);
  const active = useStore((s) =>
    node.kind === "request"
      ? s.view.kind === "request" && s.draft.path === node.path
      : s.view.kind === "folder" && s.view.path === node.path,
  );
  const renaming = useStore((s) => s.renaming === node.path);
  // Where the menu opens: below the row's button, or where it was right-clicked.
  const [menu, setMenu] = useState<false | { x: number; y: number }>(false);
  const { toggle, openRequest, openFolder, addRequest, addFolder, rename, remove } = useStore.getState();
  const isRequest = node.kind === "request";

  const select = () => {
    if (isRequest) void openRequest(node.path);
    else toggle(node.path);
  };

  return (
    <li role="treeitem" aria-expanded={isRequest ? undefined : open}>
      <div
        className={`tree-row${active ? " active" : ""}${menu ? " menu-open" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={select}
        // Double-clicking the name renames, as in a file manager.
        onDoubleClick={() => useStore.setState({ renaming: node.path })}
        title={isRequest ? undefined : "Double-click to rename"}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
      >
        {isRequest ? (
          <MethodTag method={node.method ?? ""} />
        ) : (
          <span className="tree-icon">
            <ChevronRight size={14} className={`chevron${open ? " open" : ""}`} />
            {node.kind === "collection" ? (
              <Library size={14} className="muted" />
            ) : open ? (
              <FolderOpen size={14} className="muted" />
            ) : (
              <Folder size={14} className="muted" />
            )}
          </span>
        )}
        {renaming ? (
          <RenameInput name={node.name} onDone={(name) => void rename(node.path, name)} />
        ) : (
          <span className={`tree-name${node.kind === "collection" ? " collection" : ""}`}>{node.name}</span>
        )}
        {!isRequest && (
          <button
            className="row-menu"
            aria-label="New request"
            title={`New request in ${node.name}`}
            onClick={(e) => {
              e.stopPropagation();
              void addRequest(node.path);
            }}
          >
            <Plus size={14} />
          </button>
        )}
        {!isRequest && (
          <button
            className="row-menu"
            aria-label="Run"
            title="Run…"
            onClick={(e) => {
              e.stopPropagation();
              useStore.setState({ view: { kind: "runner", path: node.path } });
            }}
          >
            <Play size={13} />
          </button>
        )}
        <button
          className="row-menu"
          aria-label="Actions"
          title="Actions"
          onClick={(e) => {
            e.stopPropagation();
            const r = e.currentTarget.getBoundingClientRect();
            setMenu(menu ? false : { x: r.right - 170, y: r.bottom + 2 });
          }}
        >
          <MoreHorizontal size={15} />
        </button>
        {menu && (
          <Menu at={menu} onClose={() => setMenu(false)}>
            {!isRequest && <button onClick={() => void addRequest(node.path)}>New request</button>}
            {!isRequest && <button onClick={() => void addFolder(node.path)}>New folder</button>}
            {!isRequest && <button onClick={() => void openFolder(node.path)}>Settings</button>}
            {!isRequest && (
              <button onClick={() => useStore.setState({ view: { kind: "runner", path: node.path } })}>Run…</button>
            )}
            {node.kind === "collection" && (
              <button onClick={() => void useStore.getState().exportCollection(node.path)}>Export to Postman…</button>
            )}
            <button onClick={() => useStore.setState({ renaming: node.path })}>Rename</button>
            <button onClick={() => void useStore.getState().duplicate(node.path)}>Duplicate</button>
            <hr />
            <button className="danger" onClick={() => void remove(node)}>
              Delete
            </button>
          </Menu>
        )}
      </div>
      {!isRequest && open && (
        <ul role="group" className="tree-group" style={{ ["--depth" as string]: depth }}>
          {(node.children ?? []).map((child) => (
            <TreeItem key={child.path} node={child} depth={depth + 1} />
          ))}
          {(node.children ?? []).length === 0 && (
            <li className="tree-empty muted small" style={{ paddingLeft: 30 + (depth + 1) * 14 }}>
              Empty
            </li>
          )}
        </ul>
      )}
    </li>
  );
}

function RenameInput({ name, onDone }: { name: string; onDone: (name: string) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.select(), []);
  const finish = (value: string) => (value.trim() ? onDone(value.trim()) : useStore.setState({ renaming: null }));
  return (
    <input
      ref={ref}
      className="rename"
      defaultValue={name}
      onClick={(e) => e.stopPropagation()}
      onBlur={(e) => finish(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(e.currentTarget.value);
        if (e.key === "Escape") useStore.setState({ renaming: null });
      }}
    />
  );
}

function Environments() {
  const workspace = useStore((s) => s.workspace);
  const environments = useStore((s) => s.environments);
  const environment = useStore((s) => s.environment);
  const view = useStore((s) => s.view);
  const { openVariables, setEnvironment } = useStore.getState();
  const editing = view.kind === "variables" ? view.environment : undefined;

  if (!workspace) return <p className="muted small sidebar-note">Open a workspace to keep environments in it.</p>;

  const add = useStore.getState().addEnvironment;

  return (
    <>
      <SectionHead title="Environments">
        <IconButton label="New environment" onClick={() => void add()}>
          <Plus size={16} />
        </IconButton>
      </SectionHead>
      <div className="sidebar-body">
        <ul className="tree">
          <li>
            <div
              className={`tree-row flat${editing === null ? " active" : ""}`}
              onClick={() => void openVariables(null)}
              title="Variables every request can use"
            >
              <Globe size={14} className="muted" />
              <span className="tree-name">Globals</span>
            </div>
          </li>
          {environments.map((name) => (
            <li key={name}>
              <div
                className={`tree-row flat${editing === name ? " active" : ""}`}
                onClick={() => void openVariables(name)}
              >
                <Layers size={14} className={name === environment ? "accent" : "muted"} />
                <span className="tree-name">{name}</span>
                {name === environment ? (
                  <span className="pill" title="Requests use this environment">
                    Active
                  </span>
                ) : (
                  <button
                    className="row-menu"
                    title="Use this environment"
                    aria-label="Use this environment"
                    onClick={(e) => {
                      e.stopPropagation();
                      setEnvironment(name);
                    }}
                  >
                    <Check size={14} />
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
        {environments.length === 0 && (
          <div className="sidebar-empty">
            <p className="muted small">
              An environment is a set of variables, such as the base URL and keys of staging or production. Switch
              between them above the request.
            </p>
            <button className="button" onClick={() => void add()}>
              New environment
            </button>
          </div>
        )}
      </div>
    </>
  );
}

function History() {
  const history = useStore((s) => s.history);
  const { clearHistory, openHistory, loadHistory } = useStore.getState();
  const [query, setQuery] = useState("");

  // Agents and the CLI add to history too; refresh while it is on screen.
  useEffect(() => {
    const timer = setInterval(() => void loadHistory().catch(() => {}), 2000);
    return () => clearInterval(timer);
  }, [loadHistory]);

  const q = query.trim().toLowerCase();
  const shown = q
    ? history.filter((e) => `${e.request.method} ${e.request.url} ${e.status ?? ""}`.toLowerCase().includes(q))
    : history;

  return (
    <>
      <SectionHead title="History">
        {history.length > 0 && (
          <IconButton label="Clear history" onClick={() => void clearHistory()}>
            <Trash2 size={15} />
          </IconButton>
        )}
      </SectionHead>
      {history.length > 0 && (
        <div className="sidebar-search">
          <SearchBox value={query} onChange={setQuery} placeholder="Filter history" />
        </div>
      )}
      <div className="sidebar-body">
        {history.length === 0 && (
          <div className="sidebar-empty">
            <Clock size={28} strokeWidth={1.5} className="muted" />
            <p className="muted small">Requests you, the CLI and agents send appear here.</p>
          </div>
        )}
        <ul className="history">
          {groupByDay(shown).map(([day, entries]) => (
            <li key={day}>
              <div className="history-day">{day}</div>
              <ul>
                {entries.map((entry) => (
                  <li key={entry.id}>
                    <button className="list-item" onClick={() => void openHistory(entry)} title={entry.request.url}>
                      <MethodTag method={entry.request.method} />
                      <span className="list-item-text mono-ish">{displayUrl(entry.request.url)}</span>
                      {entry.source && (
                        <span className="source-tag" title={`Sent by ${sourceName(entry.source)}`}>
                          {entry.source === "cli" ? "CLI" : "agent"}
                        </span>
                      )}
                      <span className={`history-status ${statusTone(entry)}`}>{entry.status ?? "ERR"}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </div>
    </>
  );
}

function groupByDay(entries: HistoryEntry[]): [string, HistoryEntry[]][] {
  const groups = new Map<string, HistoryEntry[]>();
  const today = new Date().toDateString();
  for (const entry of entries) {
    const date = new Date(entry.sentAt);
    const day = date.toDateString() === today ? "Today" : date.toLocaleDateString();
    groups.set(day, [...(groups.get(day) ?? []), entry]);
  }
  return [...groups];
}

const displayUrl = (url: string) => url.replace(/^https?:\/\//, "");

/** "mcp:claude-code" → "claude-code (MCP)". */
const sourceName = (source: string) =>
  source === "cli" ? "the tuma CLI" : source.startsWith("mcp:") ? `${source.slice(4)} (MCP)` : source;

function statusTone(entry: HistoryEntry): string {
  if (entry.status === null) return "fail";
  return entry.status < 300 ? "ok" : entry.status < 400 ? "redirect" : "fail";
}
