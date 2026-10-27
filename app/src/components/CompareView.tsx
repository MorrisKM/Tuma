import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { ArrowLeftRight, Layers } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { changeCount, comparableBody, comparableHeaders, ignoredNames } from "../compare";
import { type Kept, useStore } from "../store";
import { type Language, viewerExtensions } from "./CodeEditor";
import { contentType, formatBytes, formatMs, languageFor } from "./ResponsePanel";

type Part = "body" | "headers";
type Layout = "split" | "inline";

interface Settings {
  part: Part;
  layout: Layout;
  sortKeys: boolean;
  ignore: string;
}

// Remembered on this machine, for the next comparison.
const KEY = "tuma.compare";
function loadSettings(): Settings {
  const fallback: Settings = { part: "body", layout: "split", sortKeys: true, ignore: "" };
  try {
    return { ...fallback, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Settings>) };
  } catch {
    return fallback;
  }
}

/** Two of the tab's responses side by side, with what changed between them marked. */
export function CompareView() {
  const kept = useStore((s) => s.kept);
  const compare = useStore((s) => s.compare);
  const setCompare = useStore((s) => s.setCompare);
  const [settings, setSettings] = useState(loadSettings);
  const change = (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
      // Not being remembered is fine.
    }
  };

  // The ones picked, or else the two latest.
  const find = (id: string | null | undefined) => kept.find((k) => k.id === id);
  const b = find(compare?.b) ?? kept[kept.length - 1];
  const a = find(compare?.a) ?? kept.filter((k) => k !== b).pop();

  if (!a || !b) {
    return (
      <div className="compare-view">
        <div className="compare-empty">
          <strong>Compare responses</strong>
          <p className="hint">
            {kept.length
              ? "Change something, such as a query parameter, a header or the environment, and send the request again: the two responses show here side by side."
              : "Send the request, change something and send it again: the two responses show here side by side."}{" "}
            Or send it in several environments at once.
          </p>
        </div>
        <EnvironmentPicker open />
      </div>
    );
  }

  return (
    <div className="compare-view">
      <div className="toolbar compare-toolbar">
        <Pick label="Baseline" value={a} kept={kept} onChange={(id) => setCompare({ a: id, b: b.id })} />
        <button
          className="icon-button"
          title="Swap the baseline and the compared response"
          aria-label="Swap"
          onClick={() => setCompare({ a: b.id, b: a.id })}
        >
          <ArrowLeftRight size={14} />
        </button>
        <Pick label="Compared" value={b} kept={kept} onChange={(id) => setCompare({ a: a.id, b: id })} />
      </div>
      <div className="toolbar compare-toolbar">
        <div className="segmented">
          {(["body", "headers"] as Part[]).map((p) => (
            <button key={p} className={settings.part === p ? "active" : ""} onClick={() => change({ part: p })}>
              {p === "body" ? "Body" : "Headers"}
            </button>
          ))}
        </div>
        <div className="segmented">
          {(["split", "inline"] as Layout[]).map((l) => (
            <button key={l} className={settings.layout === l ? "active" : ""} onClick={() => change({ layout: l })}>
              {l === "split" ? "Side by side" : "Inline"}
            </button>
          ))}
        </div>
        <input
          className="compare-ignore mono"
          value={settings.ignore}
          placeholder="Ignore: id, timestamp, date"
          aria-label="Fields and headers to ignore"
          title="JSON keys (at any depth) and headers left out of the comparison, such as ones that change on every call"
          spellCheck={false}
          onChange={(e) => change({ ignore: e.target.value })}
        />
        <label className="check" title="Sort JSON object keys, so the same fields in another order are no difference">
          <input type="checkbox" checked={settings.sortKeys} onChange={(e) => change({ sortKeys: e.target.checked })} />
          Sort keys
        </label>
        <EnvironmentPicker open={false} />
      </div>
      <Summary a={a} b={b} />
      <Comparison a={a} b={b} settings={settings} />
    </div>
  );
}

function label(k: Kept, kept: Kept[]): string {
  const time = new Date(k.at).toLocaleTimeString();
  const latest = k === kept[kept.length - 1] ? " (latest)" : "";
  return `${time} · ${k.response.status} · ${k.environment ?? "no environment"}${latest}`;
}

function Pick({ label: name, value, kept, onChange }: { label: string; value: Kept; kept: Kept[]; onChange: (id: string) => void }) {
  return (
    <label className="compare-pick">
      <span className="muted">{name}</span>
      <select value={value.id} onChange={(e) => onChange(e.target.value)} title={`${value.response.sent.method} ${value.response.sent.url}`}>
        {[...kept].reverse().map((k) => (
          <option key={k.id} value={k.id}>
            {label(k, kept)}
          </option>
        ))}
      </select>
    </label>
  );
}

/** What was sent and what came back, a row for each, with the cells that differ marked. */
function Summary({ a, b }: { a: Kept; b: Kept }) {
  const facts = (k: Kept) => [
    k.environment ?? "None",
    `${k.response.sent.method} ${k.response.sent.url}`,
    `${k.response.status} ${k.response.reason}`,
    formatMs(k.response.timing.total),
    formatBytes(k.response.size),
  ];
  const x = facts(a);
  const y = facts(b);
  // Time always differs a little, so it is never marked.
  const differs = x.map((v, i) => i !== 3 && v !== y[i]);
  const tone = (k: Kept) => (k.response.status < 300 ? "ok" : k.response.status < 400 ? "redirect" : "fail");
  return (
    <table className="header-table compare-summary">
      <thead>
        <tr>
          <th />
          {["Environment", "Request", "Status", "Time", "Size"].map((name) => (
            <th key={name}>{name}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {[
          ["Baseline", a, x],
          ["Compared", b, y],
        ].map(([name, k, values]) => (
          <tr key={name as string}>
            <th>{name as string}</th>
            {(values as string[]).map((v, i) => (
              <td key={i} className={`mono${differs[i] ? " differs" : ""}${i === 2 ? ` status-cell ${tone(k as Kept)}` : ""}`}>
                {v}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Comparison({ a, b, settings }: { a: Kept; b: Kept; settings: Settings }) {
  const ignore = useMemo(() => ignoredNames(settings.ignore), [settings.ignore]);
  const [left, right, language] = useMemo((): [string, string, Language] => {
    if (settings.part === "headers") {
      return [comparableHeaders(a.response.headers, ignore), comparableHeaders(b.response.headers, ignore), "text"];
    }
    const options = { ignore, sortKeys: settings.sortKeys };
    const x = comparableBody(a.response, options);
    const y = comparableBody(b.response, options);
    return [x.text, y.text, x.json && y.json ? "json" : languageFor(contentType(b.response))];
  }, [a, b, settings.part, settings.sortKeys, ignore]);
  const changes = useMemo(() => changeCount(left, right), [left, right]);

  return (
    <>
      <p className={`compare-count${changes ? "" : " same"}`}>
        {changes === 0
          ? `The ${settings.part === "body" ? "bodies" : "headers"} are the same${ignore.length ? ", apart from what is ignored" : ""}.`
          : `${changes} ${changes === 1 ? "difference" : "differences"} in the ${settings.part}.`}
      </p>
      <Diff a={left} b={right} language={language} inline={settings.layout === "inline"} />
    </>
  );
}

const COLLAPSE = { margin: 3, minSize: 6 };

/** A merge view: side by side, or one text with removed lines shown where they were. */
function Diff({ a, b, language, inline }: { a: string; b: string; language: Language; inline: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const extensions = viewerExtensions(language);
    if (inline) {
      const view = new EditorView({
        parent: host.current!,
        state: EditorState.create({
          doc: b,
          extensions: [
            extensions,
            unifiedMergeView({ original: a, mergeControls: false, collapseUnchanged: COLLAPSE, syntaxHighlightDeletions: true }),
          ],
        }),
      });
      return () => view.destroy();
    }
    const view = new MergeView({
      a: { doc: a, extensions },
      b: { doc: b, extensions },
      parent: host.current!,
      collapseUnchanged: COLLAPSE,
    });
    return () => view.destroy();
  }, [a, b, language, inline]);
  return <div className={`editor-frame compare-diff${inline ? " inline" : ""}`} ref={host} />;
}

/** Sends the request once in each chosen environment, then compares the first two responses. */
function EnvironmentPicker({ open: startOpen }: { open: boolean }) {
  const environments = useStore((s) => s.environments);
  const active = useStore((s) => s.environment);
  const sending = useStore((s) => s.sending);
  const compareEnvironments = useStore((s) => s.compareEnvironments);
  const [open, setOpen] = useState(startOpen);
  // "" stands for no environment.
  const [chosen, setChosen] = useState<string[]>(() => [active ?? ""]);
  const options = ["", ...environments];

  if (!open) {
    return (
      <button className="button ghost small compare-envs-toggle" onClick={() => setOpen(true)} disabled={!environments.length}
        title={environments.length ? "Send the request in several environments and compare them" : "The workspace has no environments"}>
        <Layers size={13} />
        Compare environments…
      </button>
    );
  }
  const toggle = (name: string) =>
    setChosen((c) => (c.includes(name) ? c.filter((n) => n !== name) : options.filter((o) => c.includes(o) || o === name)));
  return (
    <div className="compare-envs">
      <span className="muted">Send in</span>
      {environments.length === 0 && <span className="hint">The workspace has no environments yet.</span>}
      {environments.length > 0 &&
        options.map((name) => (
          <label key={name || "(none)"} className="check">
            <input type="checkbox" checked={chosen.includes(name)} onChange={() => toggle(name)} />
            {name || "No environment"}
          </label>
        ))}
      <button
        className="button small"
        disabled={chosen.length < 2 || !!sending}
        title={chosen.length < 2 ? "Choose two or more" : "Sends the request once in each, in this order"}
        onClick={() => void compareEnvironments(chosen.map((n) => n || null))}
      >
        {sending ? <span className="spinner" /> : <Layers size={13} />}
        Send and compare
      </button>
      {!startOpen && (
        <button className="link-button" onClick={() => setOpen(false)}>
          Close
        </button>
      )}
    </div>
  );
}
