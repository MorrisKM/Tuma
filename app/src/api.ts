import { invoke } from "@tauri-apps/api/core";
import type {
  AgentSetup,
  TokenInfo,
  TokenStatus,
  CookieView,
  MachineSettings,
  SettingsView,
  CurrentValues,
  PastRun,
  RunReport,
  Language,
  Snippet,
  VariableHint,
  FolderFile,
  HistoryEntry,
  ImportResult,
  RequestFile,
  Response,
  TreeNode,
  VariableValue,
  WorkspaceInfo,
} from "./types";

export const appVersion = () => invoke<string>("app_version");

/** Rejects with a `RequestError`. `path` makes it inherit from its folders; `environment` supplies variables. */
export const sendRequest = (id: string, request: RequestFile, path: string | null, environment: string | null) =>
  invoke<Response>("send_request", { id, request, path, environment });
export const cancelRequest = (id: string) => invoke<void>("cancel_request", { id });

/** Newest first. */
export const historyList = () => invoke<HistoryEntry[]>("history_list");
export const historyClear = () => invoke<void>("history_clear");

// Everything below rejects with an error message string.

export const workspaceCurrent = () => invoke<WorkspaceInfo | null>("workspace_current");
export const workspaceOpen = (root: string) => invoke<WorkspaceInfo>("workspace_open", { root });
export const workspaceCreate = (root: string, name: string) => invoke<WorkspaceInfo>("workspace_create", { root, name });
export const workspaceClose = () => invoke<void>("workspace_close");

export const tree = () => invoke<TreeNode[]>("tree");
export const requestRead = (path: string) => invoke<RequestFile>("request_read", { path });
export const requestWrite = (path: string, request: RequestFile) => invoke<void>("request_write", { path, request });
export const requestCreate = (parent: string, name: string, request: RequestFile) =>
  invoke<string>("request_create", { parent, name, request });
export const folderRead = (path: string) => invoke<FolderFile>("folder_read", { path });
export const folderWrite = (path: string, folder: FolderFile) => invoke<void>("folder_write", { path, folder });
/** Creates a collection when `parent` is null. */
export const folderCreate = (parent: string | null, name: string) => invoke<string>("folder_create", { parent, name });
export const itemRename = (path: string, name: string) => invoke<string>("item_rename", { path, name });
export const itemDelete = (path: string) => invoke<void>("item_delete", { path });

export const environments = () => invoke<string[]>("environments");
export const environmentRead = (name: string) => invoke<VariableValue[]>("environment_read", { name });
export const environmentWrite = (name: string, variables: VariableValue[]) =>
  invoke<void>("environment_write", { name, variables });
export const environmentCreate = (name: string) => invoke<string>("environment_create", { name });
export const environmentRename = (name: string, newName: string) =>
  invoke<string>("environment_rename", { name, newName });
export const environmentDelete = (name: string) => invoke<void>("environment_delete", { name });
export const globalsRead = () => invoke<VariableValue[]>("globals_read");
export const globalsWrite = (variables: VariableValue[]) => invoke<void>("globals_write", { variables });

/** Imports an OpenAPI/Swagger spec or a Postman collection, environment or globals export. */
export const importFile = (file: string) => invoke<ImportResult>("import_file", { file });

export const agentSetup = () => invoke<AgentSetup>("agent_setup");

export const variablesInScope = (path: string | null, environment: string | null) =>
  invoke<VariableHint[]>("variables_in_scope", { path, environment });

export const snippet = (request: RequestFile, path: string | null, environment: string | null, language: Language) =>
  invoke<Snippet>("snippet", { request, path, environment, language });

/** The request and the options Tuma ignored. */
export const parseCurl = (command: string) => invoke<[RequestFile, string[]]>("parse_curl", { command });

export const exportPostman = (collection: string, file: string) => invoke<void>("export_postman", { collection, file });

export interface RunRequest {
  runId: string;
  /** A collection or folder. */
  path: string;
  environment: string | null;
  iterations: number;
  delayMs: number;
  dataFile: string | null;
  onlyTests: boolean;
}

/** Runs a collection or folder. Progress arrives as "run-step" events. */
export const runCollection = (r: RunRequest) =>
  invoke<{ file: string | null; report: RunReport }>("run_collection", { ...r });
export const stopRun = (runId: string) => invoke<void>("stop_run", { runId });
export const runsList = () => invoke<PastRun[]>("runs_list");
export const runRead = (file: string) => invoke<RunReport>("run_read", { file });
export const runExport = (file: string, target: string, format: "junit" | "json") =>
  invoke<void>("run_export", { file, target, format });

export const currentValues = () => invoke<CurrentValues>("current_values");
/** Resets one scope, or every scope with no argument. */
export const resetCurrentValues = (scope?: { kind: "environment" | "collection" | "globals"; name?: string }) =>
  invoke<void>("reset_current_values", { scope: scope ?? null });

export const settingsRead = () => invoke<SettingsView>("settings_read");
/** Secrets: undefined leaves one as it is, "" removes it. */
export const settingsWrite = (
  settings: MachineSettings,
  secretsIn: { proxyPassword?: string; passphrases: Record<string, string> },
) => invoke<void>("settings_write", { settings, secretsIn });
export const cookiesList = () => invoke<Record<string, CookieView[]>>("cookies_list");
export const cookiesRemove = (domain: string | null, path: string | null, name: string | null) =>
  invoke<void>("cookies_remove", { domain, path, name });

export const oauth2Status = (request: RequestFile, path: string | null, environment: string | null) =>
  invoke<TokenStatus>("oauth2_status", { request, path, environment });
/** Browser grants open the system browser and resolve after the login. */
export const oauth2Authorize = (request: RequestFile, path: string | null, environment: string | null) =>
  invoke<TokenStatus>("oauth2_authorize", { request, path, environment });
export const oauth2Tokens = () => invoke<TokenInfo[]>("oauth2_tokens");
export const oauth2Forget = (key: string) => invoke<void>("oauth2_forget", { key });
