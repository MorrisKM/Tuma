import { autocompletion, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { useStore } from "./store";
import type { RequestFile, VariableHint } from "./types";

/** Postman-style variables Tuma generates on each send. */
export const DYNAMIC: Record<string, string> = {
  $guid: "a new UUID",
  $randomUUID: "a new UUID",
  $timestamp: "the current Unix time in seconds",
  $isoTimestamp: "the current time, ISO 8601",
  $randomInt: "a random number from 0 to 1000",
};

export interface Suggestion {
  name: string;
  /** Where the value comes from, such as "environment Staging". */
  detail?: string;
  /** The value, "secret", or what a dynamic variable generates. */
  info: string;
  dynamic: boolean;
}

/**
 * What a pasted request and its script read but nothing provides: {{variables}} anywhere in them, and
 * pm.environment.get / pm.variables.get in the script. Left out: dynamic variables, those in scope, and those the
 * script sets itself.
 */
export function neededVariables(file: RequestFile, scope: VariableHint[]): string[] {
  const script = file.scripts?.preRequest ?? "";
  const names = (text: string, pattern: RegExp) => [...text.matchAll(pattern)].map((m) => m[1]);
  const provided = new Set([
    ...scope.map((v) => v.name),
    ...names(script, /\bpm\.(?:environment|variables)\.set\(\s*["'`]([^"'`]+)["'`]/g),
  ]);
  const read = [
    ...names(JSON.stringify(file), /\{\{\s*([^{}]+?)\s*\}\}/g),
    ...names(script, /\bpm\.(?:environment|variables)\.get\(\s*["'`]([^"'`]+)["'`]/g),
  ];
  return [...new Set(read)].filter((name) => !name.startsWith("$") && !provided.has(name));
}

/** The variables to offer, those in scope first, then the dynamic ones. */
export function suggestions(scope: VariableHint[]): Suggestion[] {
  return [
    ...scope.map((v) => ({ name: v.name, detail: v.source, info: v.secret ? "secret" : v.value, dynamic: false })),
    ...Object.entries(DYNAMIC).map(([name, info]) => ({ name, info, dynamic: true })),
  ];
}

/** Shown under the suggestions when environment variables are missing from them because none is selected. */
export const NO_ENVIRONMENT = "No environment selected";

export function noEnvironment(): boolean {
  const { workspace, environment } = useStore.getState();
  return !!workspace && environment === null;
}

/** Suggestions whose name contains the typed text, ignoring case. */
export function matching(options: Suggestion[], typed: string): Suggestion[] {
  const t = typed.toLowerCase();
  return options.filter((s) => s.name.toLowerCase().includes(t));
}

/** The name being typed after an unclosed {{ that ends `text`, and where it starts; null when there is none. */
export function typedVariable(text: string): { from: number; typed: string } | null {
  const m = text.match(/\{\{\s*([\w$.-]*)$/);
  return m ? { from: text.length - m[1].length, typed: m[1] } : null;
}

/** Completes {{variables}} in a CodeMirror editor; `scope` gives the variables in scope when asked. */
export function variableCompletion(scope: () => VariableHint[]) {
  const complete = (context: CompletionContext): CompletionResult | null => {
    const line = context.state.doc.lineAt(context.pos);
    const found = typedVariable(line.text.slice(0, context.pos - line.from));
    if (!found) return null;
    const options: Completion[] = matching(suggestions(scope()), found.typed).map((s) => ({
      label: s.name,
      detail: s.detail,
      info: s.info,
      apply: `${s.name}}}`,
      type: s.dynamic ? "function" : "variable",
    }));
    if (noEnvironment()) {
      // A note, not a variable: picking it inserts nothing and closes the list.
      options.push({ label: NO_ENVIRONMENT, detail: "select one to use its variables", type: "note", apply: () => {} });
    }
    // Filtered here, so the note stays at the end whatever is typed; asked again on each keystroke.
    return { from: line.from + found.from, options, filter: false };
  };
  return autocompletion({
    override: [complete],
    activateOnTyping: true,
    optionClass: (c) => (c.type === "note" ? "cm-completion-note" : ""),
  });
}

/** variableCompletion with the current request's variables, for editors outside the URL bar. */
export const requestVariableCompletion = () => variableCompletion(() => useStore.getState().scope);

/** What a {{variable}} hover shows: its value, or why it has none, and where it comes from. */
export function describeVariable(
  name: string,
  scope: VariableHint[],
): { detail: string; source?: string; missing?: boolean } {
  const found = scope.find((v) => v.name === name);
  if (found) {
    return {
      detail: found.secret ? "Secret (kept in the OS keychain)" : found.value || "(empty)",
      source: `from ${found.source}`,
    };
  }
  if (DYNAMIC[name]) return { detail: `Generated on each send: ${DYNAMIC[name]}` };
  return { detail: "No value in scope: it is sent as written. Set it in an environment or folder.", missing: true };
}
