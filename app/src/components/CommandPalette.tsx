import { useEffect, useMemo, useRef, useState } from "react";
import { requests, useStore } from "../store";
import { layout } from "../layout";
import { mocks } from "../mocks";
import { applyTheme, type Theme } from "../theme";
import { focusUrl } from "./UrlEditor";

interface Item {
  id: string;
  label: string;
  /** Shown on the right: a path, a shortcut or a category. */
  hint?: string;
  run: () => void;
}

/** Ctrl+K: jump to any saved request, or run any command, by typing. */
export function CommandPalette() {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const tree = useStore((s) => s.tree);
  const environments = useStore((s) => s.environments);
  const workspace = useStore((s) => s.workspace);
  const s = useStore.getState();
  const close = () => useStore.setState({ paletteOpen: false });

  const items = useMemo<Item[]>(() => {
    const theme = (t: Theme, label: string): Item => ({ id: `theme-${t}`, label, hint: "Theme", run: () => applyTheme(t) });
    const commands: Item[] = [
      { id: "new", label: "New request", hint: "Ctrl+T", run: s.newTab },
      { id: "send", label: "Send request", hint: "Ctrl+Enter", run: () => void s.send() },
      { id: "save", label: "Save request", hint: "Ctrl+S", run: () => void s.save() },
      { id: "close", label: "Close tab", hint: "Ctrl+W", run: () => void s.closeTab(s.activeTab) },
      { id: "url", label: "Focus the URL", hint: "Ctrl+L", run: () => setTimeout(focusUrl) },
      { id: "code", label: "Copy as code (cURL, Python, JavaScript)", run: () => useStore.setState({ snippetOpen: true }) },
      {
        id: "curl",
        label: "Import a cURL command from the clipboard",
        run: () =>
          void navigator.clipboard.readText().then(
            (text) => s.importCurl(text),
            () => s.notify({ tone: "error", text: "Could not read the clipboard. Paste the command into the URL instead." }),
          ),
      },
      { id: "new-workspace", label: "New workspace…", hint: "Workspace", run: () => useStore.setState({ creatingWorkspace: true }) },
      { id: "import-workspace", label: "Import a workspace folder…", hint: "Workspace", run: () => void s.importWorkspace("folder") },
      { id: "import-workspace-zip", label: "Import a workspace .zip…", hint: "Workspace", run: () => void s.importWorkspace("zip") },
      { id: "open", label: "Open a folder as a workspace, in place…", hint: "Workspace", run: () => void s.openWorkspace() },
      ...(workspace
        ? [
            { id: "mock-workspace", label: "Start a mock server for the workspace", hint: "Mock", run: () => void mocks.start(null) },
            { id: "docs-workspace", label: "Generate API docs for the workspace…", hint: "Docs", run: () => useStore.setState({ docs: { scope: null } }) },
            ...(s.draft.path
              ? [{ id: "docs-request", label: "Generate API docs for this request…", hint: "Docs", run: () => useStore.setState({ docs: { scope: s.draft.path } }) }]
              : []),
          ]
        : []),
      ...(workspace
        ? [{ id: "export-workspace", label: "Export the workspace as .zip…", hint: "Workspace", run: () => void s.exportWorkspace() }]
        : []),
      ...(workspace
        ? [
            { id: "collection", label: "New collection", run: () => void s.addFolder(null) },
            { id: "import", label: "Import a file (OpenAPI, Postman, HAR)…", run: () => void s.importFile() },
            { id: "vars", label: "Edit variables and environments", run: () => void s.openVariables(s.environment) },
            { id: "env-none", label: "Use no environment", hint: "Environment", run: () => s.setEnvironment(null) },
            ...environments.map((name) => ({
              id: `env-${name}`,
              label: `Use environment: ${name}`,
              hint: "Environment",
              run: () => s.setEnvironment(name),
            })),
          ]
        : []),
      { id: "settings", label: "Settings: proxy, certificates, cookies", run: () => useStore.setState({ view: { kind: "settings" } }) },
      { id: "agents", label: "Connect a coding agent (MCP)", run: () => useStore.setState({ view: { kind: "agents" } }) },
      { id: "console", label: "Show or hide the console", hint: "Ctrl+`", run: layout.toggleConsole },
      { id: "sidebar", label: "Show or hide the sidebar", hint: "Ctrl+B", run: layout.toggleSidebar },
      { id: "layout", label: "Response beside or below the request", hint: "Layout", run: layout.toggleOrientation },
      { id: "history", label: "Show history", hint: "Sidebar", run: () => layout.open("history") },
      theme("light", "Light theme"),
      theme("dark", "Dark theme"),
      theme("system", "Theme: follow the system"),
    ];
    const runs: Item[] = tree.map((c) => ({
      id: `run-${c.path}`,
      label: `Run collection: ${c.name}`,
      hint: "Runner",
      run: () => useStore.setState({ view: { kind: "runner", path: c.path } }),
    }));
    const saved: Item[] = requests(tree).map((r) => ({
      id: `req-${r.path}`,
      label: r.name,
      hint: `${r.method} · ${r.path.replace(/\/[^/]*$/, "")}`,
      run: () => void s.openRequest(r.path),
    }));
    return [...saved, ...commands, ...runs];
    // The store's actions are stable; tree, environments and workspace change what is listed.
  }, [tree, environments, workspace]);

  const matches = useMemo(() => rank(items, query).slice(0, 50), [items, query]);

  useEffect(() => input.current?.focus(), []);
  useEffect(() => setSelected(0), [query]);

  const run = (item: Item | undefined) => {
    if (!item) return;
    close();
    item.run();
  };

  return (
    <div className="modal-backdrop palette-backdrop" onMouseDown={close}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Command palette">
        <input
          ref={input}
          value={query}
          placeholder="Search requests and commands"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") close();
            else if (e.key === "ArrowDown") {
              e.preventDefault();
              setSelected((i) => Math.min(i + 1, matches.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSelected((i) => Math.max(i - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              run(matches[selected]);
            }
          }}
        />
        <ul className="palette-list" role="listbox">
          {matches.map((item, i) => (
            <li
              key={item.id}
              role="option"
              aria-selected={i === selected}
              className={i === selected ? "selected" : ""}
              onMouseEnter={() => setSelected(i)}
              onClick={() => run(item)}
            >
              <span>{item.label}</span>
              {item.hint && <span className="muted small">{item.hint}</span>}
            </li>
          ))}
          {matches.length === 0 && <li className="muted">Nothing matches.</li>}
        </ul>
      </div>
    </div>
  );
}

/** Fuzzy matching: every typed character appears in order. Earlier, closer and word-start matches rank higher. */
function rank(items: Item[], query: string): Item[] {
  const q = query.trim().toLowerCase();
  if (!q) return items;
  const scored = items.flatMap((item) => {
    const text = `${item.label} ${item.hint ?? ""}`.toLowerCase();
    let score = 0;
    let at = -1;
    for (const c of q) {
      if (c === " ") continue;
      const next = text.indexOf(c, at + 1);
      if (next < 0) return [];
      score += next - at - 1;
      if (next === 0 || " /·-_".includes(text[next - 1])) score -= 2;
      at = next;
    }
    if (item.label.toLowerCase().includes(q)) score -= 10;
    return [{ item, score }];
  });
  return scored.sort((a, b) => a.score - b.score).map((s) => s.item);
}
