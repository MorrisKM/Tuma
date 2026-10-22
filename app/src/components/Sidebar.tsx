import { useEffect, useRef, useState } from "react";
import { requests, useStore } from "../store";
import type { HistoryEntry, TreeNode } from "../types";

type Tab = "collections" | "history";

export function Sidebar() {
  const [tab, setTab] = useState<Tab>("collections");
  const workspace = useStore((s) => s.workspace);
  const { openWorkspace, closeWorkspace, importFile, addFolder, newTab, openVariables } = useStore.getState();

  return (
    <aside className="sidebar">
      <div className="sidebar-top">
        <div className="brand">Tuma</div>
        <div className="workspace-line">
          <button className="workspace-name" onClick={() => void openWorkspace()} title={workspace?.root ?? ""}>
            {workspace ? workspace.name : "Open a workspace…"}
          </button>
          {workspace && (
            <button className="link-button" onClick={() => void closeWorkspace()} title="Close workspace">
              Close
            </button>
          )}
        </div>
        <div className="sidebar-actions">
          <button className="button small" onClick={newTab} title="New request (Ctrl+T)">
            New request
          </button>
          {workspace && (
            <>
              <button className="button small" onClick={() => void addFolder(null)}>
                New collection
              </button>
              <button className="button small" onClick={() => void importFile()}>
                Import
              </button>
              <button
                className="button small"
                onClick={() => void openVariables(useStore.getState().environment)}
                title="Environments and global variables"
              >
                Variables
              </button>
            </>
          )}
          <button
            className="button small"
            onClick={() => useStore.setState({ view: { kind: "agents" } })}
            title="Let a coding agent send requests and run tests through Tuma"
          >
            Connect an agent
          </button>
        </div>
      </div>
      <div className="sidebar-tabs" role="tablist">
        {(["collections", "history"] as Tab[]).map((t) => (
          <button key={t} role="tab" className={tab === t ? "active" : ""} onClick={() => setTab(t)}>
            {t === "collections" ? "Collections" : "History"}
          </button>
        ))}
      </div>
      <div className="sidebar-body">{tab === "collections" ? <Collections /> : <History />}</div>
    </aside>
  );
}

function Collections() {
  const workspace = useStore((s) => s.workspace);
  const tree = useStore((s) => s.tree);
  const { openWorkspace, importFile, addFolder } = useStore.getState();
  const [query, setQuery] = useState("");

  if (!workspace) {
    return (
      <div className="sidebar-empty">
        <p className="muted small">
          A workspace is a folder of plain files, such as one in your project's repository. Requests, collections and
          environments are saved there, so you can commit them.
        </p>
        <button className="button primary" onClick={() => void openWorkspace()}>
          Open or create a workspace
        </button>
      </div>
    );
  }
  if (tree.length === 0) {
    return (
      <div className="sidebar-empty">
        <p className="muted small">No collections yet.</p>
        <button className="button" onClick={() => void addFolder(null)}>
          New collection
        </button>
        <button className="button" onClick={() => void importFile()}>
          Import OpenAPI or Postman
        </button>
      </div>
    );
  }
  const q = query.trim().toLowerCase();
  const found = q
    ? requests(tree).filter((r) => `${r.path} ${r.method}`.toLowerCase().includes(q))
    : [];
  return (
    <>
      <input
        className="tree-search"
        type="search"
        value={query}
        placeholder="Filter requests"
        aria-label="Filter requests"
        spellCheck={false}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setQuery("");
          if (e.key === "Enter" && found[0]) void useStore.getState().openRequest(found[0].path);
        }}
      />
      {q ? (
        <ul className="tree" role="listbox">
          {found.map((r) => (
            <li key={r.path}>
              <button className="history-item" onClick={() => void useStore.getState().openRequest(r.path)} title={r.path}>
                <span className={`method-tag method-${r.method?.toLowerCase()}`}>{r.method}</span>
                <span className="history-url">
                  {r.name}
                  <span className="muted small"> {r.path.replace(/\/[^/]*$/, "")}</span>
                </span>
              </button>
            </li>
          ))}
          {found.length === 0 && <li className="muted small sidebar-empty">No request matches.</li>}
        </ul>
      ) : (
        <ul className="tree" role="tree">
          {tree.map((node) => (
            <TreeItem key={node.path} node={node} depth={0} />
          ))}
        </ul>
      )}
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
  const [menu, setMenu] = useState(false);
  const { toggle, openRequest, openFolder, addRequest, addFolder, rename, remove } = useStore.getState();
  const isRequest = node.kind === "request";

  const select = () => {
    if (isRequest) void openRequest(node.path);
    else toggle(node.path);
  };

  return (
    <li role="treeitem" aria-expanded={isRequest ? undefined : open}>
      <div
        className={`tree-row${active ? " active" : ""}`}
        style={{ paddingLeft: 6 + depth * 14 }}
        onClick={select}
        onDoubleClick={() => !isRequest && void openFolder(node.path)}
      >
        {isRequest ? (
          <span className={`method-tag method-${node.method?.toLowerCase()}`}>{node.method}</span>
        ) : (
          <span className="chevron">{open ? "▾" : "▸"}</span>
        )}
        {renaming ? (
          <RenameInput name={node.name} onDone={(name) => void rename(node.path, name)} />
        ) : (
          <span className={`tree-name${node.kind === "collection" ? " collection" : ""}`}>{node.name}</span>
        )}
        <button
          className="row-menu"
          aria-label="Actions"
          onClick={(e) => {
            e.stopPropagation();
            setMenu(!menu);
          }}
        >
          ⋯
        </button>
        {menu && (
          <Menu onClose={() => setMenu(false)}>
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
            <button className="danger" onClick={() => void remove(node)}>
              Delete
            </button>
          </Menu>
        )}
      </div>
      {!isRequest && open && (
        <ul role="group">
          {(node.children ?? []).map((child) => (
            <TreeItem key={child.path} node={child} depth={depth + 1} />
          ))}
          {(node.children ?? []).length === 0 && (
            <li className="tree-empty muted small" style={{ paddingLeft: 20 + (depth + 1) * 14 }}>
              Empty
            </li>
          )}
        </ul>
      )}
    </li>
  );
}

/** A small popup menu that closes on any click or Escape. */
function Menu({ children, onClose }: { children: React.ReactNode; onClose: () => void }) {
  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // Deferred, so the click that opened the menu does not close it.
    const timer = setTimeout(() => window.addEventListener("click", close));
    window.addEventListener("keydown", onKey);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  return (
    <div className="menu" role="menu" onClick={(e) => e.stopPropagation()}>
      <div onClick={onClose}>{children}</div>
    </div>
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

function History() {
  const history = useStore((s) => s.history);
  const { clearHistory, openHistory, loadHistory } = useStore.getState();

  // Agents and the CLI add to history too; refresh while it is on screen.
  useEffect(() => {
    const timer = setInterval(() => void loadHistory().catch(() => {}), 2000);
    return () => clearInterval(timer);
  }, [loadHistory]);

  if (history.length === 0) return <p className="muted small sidebar-empty">Requests you send appear here.</p>;
  return (
    <>
      <div className="sidebar-heading">
        <span>{history.length} requests</span>
        <button className="link-button" onClick={() => void clearHistory()}>
          Clear
        </button>
      </div>
      <ul className="history">
        {groupByDay(history).map(([day, entries]) => (
          <li key={day}>
            <div className="history-day">{day}</div>
            <ul>
              {entries.map((entry) => (
                <li key={entry.id}>
                  <button className="history-item" onClick={() => void openHistory(entry)} title={entry.request.url}>
                    <span className={`method-tag method-${entry.request.method.toLowerCase()}`}>
                      {entry.request.method}
                    </span>
                    <span className="history-url">{displayUrl(entry.request.url)}</span>
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
