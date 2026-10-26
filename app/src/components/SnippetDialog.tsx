import { useEffect, useState } from "react";
import { snippet } from "../api";
import { toRequestFile } from "../draft";
import { useStore } from "../store";
import type { Language, Snippet } from "../types";
import { CodeEditor, type Language as EditorLanguage } from "./CodeEditor";

const LANGUAGES: { id: Language; label: string; editor: EditorLanguage }[] = [
  { id: "curl", label: "cURL", editor: "text" },
  { id: "python", label: "Python", editor: "text" },
  { id: "javaScript", label: "JavaScript", editor: "javascript" },
];

const KEY = "tuma.snippetLanguage";
const FULL_KEY = "tuma.snippetFull";

/** The active request as code, with variables and inherited auth filled in. */
export function SnippetDialog() {
  const draft = useStore((s) => s.draft);
  const environment = useStore((s) => s.environment);
  const [language, setLanguage] = useState<Language>(() => {
    try {
      return (localStorage.getItem(KEY) as Language) || "curl";
    } catch {
      return "curl";
    }
  });
  // With auth and pre-request scripts, or the request as written.
  const [full, setFull] = useState(() => {
    try {
      return localStorage.getItem(FULL_KEY) !== "false";
    } catch {
      return true;
    }
  });
  const [result, setResult] = useState<Snippet | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const close = () => useStore.setState({ snippetOpen: false });

  useEffect(() => {
    try {
      localStorage.setItem(KEY, language);
      localStorage.setItem(FULL_KEY, String(full));
    } catch {
      // Not remembered.
    }
    let current = true;
    snippet(toRequestFile(draft), draft.path, environment, language, full).then(
      (s) => {
        if (!current) return;
        setResult(s);
        setError(null);
      },
      (e) => {
        if (!current) return;
        setResult(null);
        setError(String(e));
      },
    );
    // Scripts can take a while: an answer for a choice since changed is dropped.
    return () => {
      current = false;
    };
  }, [draft, environment, language, full]);

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <div
        className="modal wide"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && close()}
        role="dialog"
        aria-label="Copy as code"
      >
        <div className="toolbar spaced">
          <div className="segmented">
            {LANGUAGES.map((l) => (
              <button key={l.id} className={language === l.id ? "active" : ""} onClick={() => setLanguage(l.id)}>
                {l.label}
              </button>
            ))}
          </div>
          <label
            className="check snippet-full"
            title="On: the request's auth, inherited too, and what its pre-request scripts change, with the scripts as comments above. The scripts run, but what they set is not saved. Off: the request as written, with no auth and no Authorization headers."
          >
            <input type="checkbox" checked={full} onChange={(e) => setFull(e.target.checked)} />
            Auth and scripts
          </label>
          <button
            className="button primary"
            disabled={!result}
            onClick={() =>
              result &&
              navigator.clipboard.writeText(result.code).then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              })
            }
          >
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
        {error && <p className="notice error">{error}</p>}
        {result && (
          <>
            {result.containsSecrets && (
              <p className="hint warning">This code contains the value of a secret variable. Mind where you paste it.</p>
            )}
            {result.unresolved.length > 0 && (
              <p className="hint warning">
                No value for {result.unresolved.map((n) => `{{${n}}}`).join(", ")}: written as is.
              </p>
            )}
            <div className="editor-frame snippet-code">
              <CodeEditor
                value={result.code}
                language={LANGUAGES.find((l) => l.id === language)!.editor}
                readOnly
              />
            </div>
          </>
        )}
        <div className="modal-actions">
          <button className="button" onClick={close}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
