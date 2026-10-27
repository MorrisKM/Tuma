import { ask, open as openDialog, save } from "@tauri-apps/plugin-dialog";
import { Download, Eraser, FolderOpen, Play, Plus, Save, Square, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "../api";
import { scripts, useScripts } from "../scripts";
import { useStore } from "../store";
import { CodeEditor } from "./CodeEditor";
import { KeyValueEditor } from "./KeyValueEditor";

const TYPES: { id: api.InputType; label: string }[] = [
  { id: "text", label: "Text" },
  { id: "number", label: "Number" },
  { id: "boolean", label: "Checkbox" },
  { id: "choice", label: "Choice" },
];

const valuesKey = (root: string, name: string) => `tuma.script-inputs:${root}:${name}`;

interface Remembered {
  values: Record<string, string>;
  extraArgs: string;
}

function remembered(root: string, name: string): Remembered {
  try {
    return { values: {}, extraArgs: "", ...(JSON.parse(localStorage.getItem(valuesKey(root, name)) ?? "{}") as Partial<Remembered>) };
  } catch {
    return { values: {}, extraArgs: "" };
  }
}

/** The settings as they are saved: empty fields left out, so an unchanged script compares equal. */
function tidy(script: api.Script): api.Script {
  const s = script.settings;
  const settings: api.ScriptSettings = {};
  if (s.description?.trim()) settings.description = s.description;
  const inputs = (s.inputs ?? []).filter((i) => i.name.trim());
  if (inputs.length) {
    settings.inputs = inputs.map((i) => {
      const out: api.ScriptInput = { name: i.name.trim() };
      if (i.type && i.type !== "text") out.type = i.type;
      if (i.label) out.label = i.label;
      if (i.description) out.description = i.description;
      if (i.default) out.default = i.default;
      if (i.type === "choice" && i.options?.length) out.options = i.options;
      if (script.kind === "command" && i.arg) out.arg = i.arg;
      return out;
    });
  }
  if (script.kind === "command") {
    if (s.command) settings.command = s.command;
    if (s.cwd) settings.cwd = s.cwd;
    const env = (s.env ?? []).filter((e) => e.name.trim());
    if (env.length) settings.env = env.map((e) => (e.enabled === false ? e : { name: e.name, value: e.value }));
  }
  return { ...script, settings };
}

/** A script from the Scripts page (JavaScript) or the Shell page (a command), with its inputs and its last run. */
export function ScriptView({ name }: { name: string }) {
  const root = useStore((s) => s.workspace?.root ?? "");
  const environment = useStore((s) => s.environment);
  const run = useScripts((s) => s.runs[name]);
  const [saved, setSaved] = useState<api.Script | null>(null);
  const [draft, setDraft] = useState<api.Script | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [title, setTitle] = useState(name);
  const [values, setValues] = useState<Record<string, string>>({});
  const [extraArgs, setExtraArgs] = useState("");
  const notify = useStore.getState().notify;

  useEffect(() => {
    setTitle(name);
    let current = true;
    api.scriptRead(name).then(
      (script) => {
        if (!current) return;
        const clean = tidy(script);
        setSaved(clean);
        setDraft(clean);
        setError(null);
      },
      (e) => current && setError(String(e)),
    );
    const r = remembered(root, name);
    setValues(r.values);
    setExtraArgs(r.extraArgs);
    return () => {
      current = false;
    };
  }, [name, root]);

  useEffect(() => {
    try {
      localStorage.setItem(valuesKey(root, name), JSON.stringify({ values, extraArgs }));
    } catch {
      // Not remembered; the values still apply now.
    }
  }, [root, name, values, extraArgs]);

  const dirty = useMemo(
    () => !!draft && !!saved && JSON.stringify(tidy(draft)) !== JSON.stringify(saved),
    [draft, saved],
  );

  if (error) {
    return (
      <section className="folder-editor">
        <p className="empty muted">{error}</p>
      </section>
    );
  }
  if (!draft) return <section className="folder-editor" />;

  const shell = draft.kind === "command";
  const settings = draft.settings;
  const inputs = settings.inputs ?? [];
  const change = (patch: Partial<api.ScriptSettings>) => setDraft({ ...draft, settings: { ...settings, ...patch } });
  const setInput = (i: number, patch: Partial<api.ScriptInput>) =>
    change({ inputs: inputs.map((input, j) => (j === i ? { ...input, ...patch } : input)) });

  const store = async (): Promise<boolean> => {
    const clean = tidy(draft);
    try {
      await api.scriptWrite(clean);
      setSaved(clean);
      void scripts.load();
      return true;
    } catch (e) {
      notify({ tone: "error", text: String(e) });
      return false;
    }
  };
  const start = async () => {
    if (dirty && !(await store())) return;
    const given: Record<string, string> = {};
    for (const input of inputs) {
      if (!input.name.trim()) continue;
      given[input.name] = values[input.name] ?? input.default ?? "";
    }
    void scripts.run(name, given, shell ? extraArgs : "");
  };
  const rename = async () => {
    const next = title.trim();
    if (!next || next === name) {
      setTitle(name);
      return;
    }
    try {
      if (dirty) await store();
      await scripts.rename(name, next);
    } catch (e) {
      setTitle(name);
      notify({ tone: "error", text: String(e) });
    }
  };
  const remove = async () => {
    const ok = await ask(`Delete the script "${name}"? Its files in the scripts folder will be removed from disk.`, {
      title: "Delete script",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Keep",
    });
    if (!ok) return;
    try {
      await scripts.remove(name);
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };
  const browse = async () => {
    const folder = await openDialog({ directory: true, title: "The folder the command runs in" });
    if (typeof folder === "string") change({ cwd: folder });
  };

  return (
    <section
      className="folder-editor script-view"
      onKeyDown={(e) => {
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
          e.preventDefault();
          void store();
        }
        if ((e.ctrlKey || e.metaKey) && e.key === "Enter" && !run?.running) {
          e.preventDefault();
          void start();
        }
      }}
    >
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb">
            {shell ? "Shell" : "Scripts"}
            <span className="crumb-sep">/</span>
          </span>
          <input
            className="crumb-input"
            value={title}
            aria-label="Script name"
            spellCheck={false}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={() => void rename()}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") {
                setTitle(name);
                e.currentTarget.blur();
              }
            }}
          />
          {dirty && <span className="pill">Unsaved</span>}
        </div>
        <div className="actions">
          <button className="button ghost" onClick={() => void remove()} title="Delete this script">
            <Trash2 size={14} />
          </button>
          <button className="button" onClick={() => void store()} disabled={!dirty} title="Save (Ctrl+S)">
            <Save size={14} />
            Save
          </button>
          {run?.running ? (
            <button className="button" onClick={() => scripts.stop(name)}>
              <Square size={13} />
              Stop
            </button>
          ) : (
            <button className="button primary" onClick={() => void start()} title="Run (Ctrl+Enter)">
              <Play size={14} />
              Run
            </button>
          )}
        </div>
      </div>
      <div className="panel-body script-body">
        <div className="script-edit">
          <input
            className="script-description"
            value={settings.description ?? ""}
            placeholder="What it does, for the list"
            aria-label="Description"
            onChange={(e) => change({ description: e.target.value })}
          />
          {shell ? (
            <>
              <p className="hint">
                Runs on this computer, as you, like typing it in a terminal. <code>{"{{variables}}"}</code> come from the
                inputs and the selected environment, so tokens can stay in a secret variable.
              </p>
              <div className="form-grid script-form">
                <label htmlFor="script-command">Command</label>
                <input
                  id="script-command"
                  className="mono"
                  value={settings.command ?? ""}
                  placeholder="env/bin/python flow.py --verbose"
                  spellCheck={false}
                  onChange={(e) => change({ command: e.target.value })}
                />
                <label htmlFor="script-cwd">Runs in</label>
                <span className="script-cwd">
                  <input
                    id="script-cwd"
                    className="mono"
                    value={settings.cwd ?? ""}
                    placeholder="The workspace's folder"
                    spellCheck={false}
                    onChange={(e) => change({ cwd: e.target.value })}
                  />
                  <button className="button small" onClick={() => void browse()}>
                    <FolderOpen size={13} />
                    Choose…
                  </button>
                </span>
              </div>
              <h3 className="script-heading">Environment variables</h3>
              <KeyValueEditor
                rows={(settings.env ?? []).map((e) => ({ ...e, enabled: e.enabled !== false }))}
                onChange={(rows) => change({ env: rows })}
                blank={() => ({ name: "", value: "", enabled: true })}
                namePlaceholder="API_TOKEN"
                valuePlaceholder="{{token}}"
              />
            </>
          ) : (
            <>
              <p className="hint">
                Runs on its own, not as part of a request: it can loop, send many requests and offer files to save.{" "}
                <code>tuma.inputs</code> has the inputs; <code>tuma.get</code>, <code>tuma.post</code> and{" "}
                <code>tuma.request</code> send requests; <code>tuma.saveFile(name, text)</code> offers a file;{" "}
                <code>pm.environment</code> and the rest of <code>pm.*</code> work as in request scripts. What it
                returns is shown at the end.
              </p>
              <div className="editor-frame script-editor">
                <CodeEditor value={draft.source} onChange={(source) => setDraft({ ...draft, source })} language="javascript" />
              </div>
            </>
          )}
          <div className="toolbar spaced">
            <h3 className="script-heading">Inputs</h3>
            <button
              className="button small ghost"
              onClick={() => change({ inputs: [...inputs, { name: "", type: "text" }] })}
            >
              <Plus size={13} />
              Add input
            </button>
          </div>
          {inputs.length === 0 ? (
            <p className="hint">
              Values to fill in before each run, such as a count or an email.
              {shell ? " Each can be passed as an argument (--count 5) and is in TUMA_INPUT_<NAME>." : ""}
            </p>
          ) : (
            <table className="header-table script-inputs">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Type</th>
                  <th>Default</th>
                  {shell && <th title="How it is passed: --count gives --count 5, --count= gives --count=5">Argument</th>}
                  <th>Choices</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {inputs.map((input, i) => (
                  <tr key={i}>
                    <td>
                      <input
                        className="mono"
                        value={input.name}
                        placeholder="count"
                        spellCheck={false}
                        aria-label="Input name"
                        onChange={(e) => setInput(i, { name: e.target.value })}
                      />
                    </td>
                    <td>
                      <select
                        value={input.type ?? "text"}
                        aria-label="Input type"
                        onChange={(e) => setInput(i, { type: e.target.value as api.InputType })}
                      >
                        {TYPES.map((t) => (
                          <option key={t.id} value={t.id}>
                            {t.label}
                          </option>
                        ))}
                      </select>
                    </td>
                    <td>
                      {input.type === "boolean" ? (
                        <input
                          type="checkbox"
                          aria-label="Ticked by default"
                          checked={input.default === "true"}
                          onChange={(e) => setInput(i, { default: e.target.checked ? "true" : "" })}
                        />
                      ) : (
                        <input
                          value={input.default ?? ""}
                          aria-label="Default value"
                          onChange={(e) => setInput(i, { default: e.target.value })}
                        />
                      )}
                    </td>
                    {shell && (
                      <td>
                        <input
                          className="mono"
                          value={input.arg ?? ""}
                          placeholder={input.type === "boolean" ? "--verbose" : "--count"}
                          spellCheck={false}
                          aria-label="Argument"
                          onChange={(e) => setInput(i, { arg: e.target.value })}
                        />
                      </td>
                    )}
                    <td>
                      {input.type === "choice" ? (
                        <input
                          value={(input.options ?? []).join(", ")}
                          placeholder="a, b, c"
                          aria-label="Choices, separated by commas"
                          onChange={(e) =>
                            setInput(i, { options: e.target.value.split(",").map((o) => o.trim()).filter(Boolean) })
                          }
                        />
                      ) : (
                        <span className="muted small">—</span>
                      )}
                    </td>
                    <td>
                      <button
                        className="icon-button"
                        aria-label="Remove input"
                        onClick={() => change({ inputs: inputs.filter((_, j) => j !== i) })}
                      >
                        <X size={13} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        <RunPanel
          name={name}
          shell={shell}
          inputs={inputs.filter((i) => i.name.trim())}
          values={values}
          setValues={setValues}
          extraArgs={extraArgs}
          setExtraArgs={setExtraArgs}
          environment={environment}
          start={() => void start()}
        />
      </div>
    </section>
  );
}

function RunPanel({
  name,
  shell,
  inputs,
  values,
  setValues,
  extraArgs,
  setExtraArgs,
  environment,
  start,
}: {
  name: string;
  shell: boolean;
  inputs: api.ScriptInput[];
  values: Record<string, string>;
  setValues: (v: Record<string, string>) => void;
  extraArgs: string;
  setExtraArgs: (v: string) => void;
  environment: string | null;
  start: () => void;
}) {
  const run = useScripts((s) => s.runs[name]);
  const output = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const notify = useStore.getState().notify;

  // Keep the newest line in view, unless the person scrolled up to read.
  useEffect(() => {
    const el = output.current;
    if (el && follow.current) el.scrollTop = el.scrollHeight;
  }, [run?.lines.length, run?.outcome]);

  const value = (input: api.ScriptInput) => values[input.name] ?? input.default ?? "";
  const set = (input: api.ScriptInput, v: string) => setValues({ ...values, [input.name]: v });
  const saveFile = async (file: { name: string; content: string }) => {
    const target = await save({ title: "Save the file", defaultPath: file.name });
    if (!target) return;
    try {
      await api.scriptSaveFile(target, file.content);
      notify({ tone: "info", text: `Saved ${target}.` });
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };
  const outcome = run?.outcome;

  return (
    <div className="script-run">
      <form
        className="form-grid script-values"
        onSubmit={(e) => {
          e.preventDefault();
          if (!run?.running) start();
        }}
      >
        <span className="muted">Environment</span>
        <span>
          {environment ?? <span className="muted">None: choose one at the top to use its variables</span>}
        </span>
        {inputs.map((input) => (
          <InputField key={input.name} input={input} value={value(input)} onChange={(v) => set(input, v)} />
        ))}
        {shell && (
          <>
            <label htmlFor="script-extra">More arguments</label>
            <input
              id="script-extra"
              className="mono"
              value={extraArgs}
              placeholder="--no-facility --owner-type Private"
              spellCheck={false}
              onChange={(e) => setExtraArgs(e.target.value)}
            />
          </>
        )}
        <button type="submit" hidden />
      </form>
      <div className="toolbar spaced script-output-head">
        <span className="small">
          {!run && <span className="muted">Not run yet</span>}
          {run?.running && (
            <>
              <span className="spinner" /> Running…
            </>
          )}
          {run && !run.running && <Status run={run} />}
        </span>
        {run && !run.running && (
          <button className="icon-button" title="Clear the output" aria-label="Clear the output" onClick={() => scripts.clear(name)}>
            <Eraser size={14} />
          </button>
        )}
      </div>
      <div
        className="script-output mono"
        ref={output}
        onScroll={(e) => {
          const el = e.currentTarget;
          follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {!run && <p className="muted script-output-empty">Output appears here as the script runs.</p>}
        {run && run.dropped > 0 && <div className="script-line debug">… {run.dropped} earlier lines left out</div>}
        {run?.lines.map((line, i) => (
          <div key={i} className={`script-line ${line.kind}`}>
            {line.kind === "command" ? `$ ${line.text}` : line.text || " "}
          </div>
        ))}
        {run?.error && <div className="script-line error">{run.error}</div>}
        {outcome?.error && <div className="script-line error">{outcome.error}</div>}
        {outcome && outcome.result !== undefined && outcome.result !== null && (
          <pre className="script-result">{JSON.stringify(outcome.result, null, 2)}</pre>
        )}
      </div>
      {outcome && outcome.files.length > 0 && (
        <div className="script-files">
          {outcome.files.map((file, i) => (
            <span key={i} className="script-file">
              <span className="mono">{file.name}</span>
              <span className="muted small">{sizeOf(file.content)}</span>
              <button className="button small" onClick={() => void saveFile(file)}>
                <Download size={13} />
                Save…
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function InputField({ input, value, onChange }: { input: api.ScriptInput; value: string; onChange: (v: string) => void }) {
  const id = `script-input-${input.name}`;
  const label = (
    <label htmlFor={id} title={input.description || undefined}>
      {input.label || input.name}
    </label>
  );
  switch (input.type) {
    case "boolean":
      return (
        <>
          {label}
          <input id={id} type="checkbox" className="script-check" checked={value === "true"} onChange={(e) => onChange(e.target.checked ? "true" : "false")} />
        </>
      );
    case "choice":
      return (
        <>
          {label}
          <select id={id} value={value} onChange={(e) => onChange(e.target.value)}>
            {!(input.options ?? []).includes(value) && <option value={value}>{value || "—"}</option>}
            {(input.options ?? []).map((o) => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
        </>
      );
    default:
      return (
        <>
          {label}
          <input
            id={id}
            value={value}
            inputMode={input.type === "number" ? "decimal" : undefined}
            placeholder={input.description}
            onChange={(e) => onChange(e.target.value)}
          />
        </>
      );
  }
}

function Status({ run }: { run: import("../scripts").ScriptRun }) {
  const o = run.outcome;
  if (!o) return <span className="fail">Could not run</span>;
  const time = o.durationMs < 1000 ? `${o.durationMs} ms` : `${(o.durationMs / 1000).toFixed(1)} s`;
  if (o.stopped) return <span className="muted">Stopped after {time}</span>;
  if (o.ok) return <span className="ok">Finished in {time}{o.exitCode !== null ? " · exit code 0" : ""}</span>;
  return (
    <span className="fail">
      Failed after {time}
      {o.exitCode !== null && ` · exit code ${o.exitCode}`}
    </span>
  );
}

function sizeOf(text: string): string {
  const bytes = new TextEncoder().encode(text).length;
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
}
