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
  const [result, setResult] = useState<Snippet | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const close = () => useStore.setState({ snippetOpen: false });

  useEffect(() => {
    try {
      localStorage.setItem(KEY, language);
    } catch {
      // Not remembered.
    }
    snippet(toRequestFile(draft), draft.path, environment, language).then(
      (s) => {
        setResult(s);
        setError(null);
      },
      (e) => setError(String(e)),
    );
  }, [draft, environment, language]);

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
