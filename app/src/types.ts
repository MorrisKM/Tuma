// Mirrors the Rust types sent over IPC: tuma_core::workspace::files (the on-disk format), tuma_core::workspace and the
// views in src-tauri/src/lib.rs. Fields the Rust side leaves out when they hold their default are optional here.

export interface KeyValue {
  name: string;
  value: string;
  enabled: boolean;
}

export interface Part extends KeyValue {
  /** When true, `value` is a file path and the file's contents are sent. */
  file: boolean;
  contentType?: string | null;
}

export type Body =
  | { type: "none" }
  | { type: "json" | "xml" | "html" | "text" | "javascript"; content: string }
  | { type: "raw"; content: string; contentType: string }
  | { type: "form"; fields: KeyValue[] }
  | { type: "multipart"; parts: Part[] }
  | { type: "file"; path: string; contentType?: string | null }
  | { type: "graphql"; query: string; variables?: string; operationName?: string | null };

export type Auth =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apiKey"; key: string; value: string; location: "header" | "query" }
  | { type: "digest"; username: string; password: string }
  | { type: "aws"; accessKey: string; secretKey: string; sessionToken?: string; region: string; service: string }
  | {
      type: "jwt";
      algorithm: JwtAlgorithm;
      secret: string;
      secretBase64?: boolean;
      payload: string;
      headers?: string;
      prefix: string;
      queryParam?: string | null;
    }
  | {
      type: "oauth1";
      consumerKey: string;
      consumerSecret: string;
      token?: string;
      tokenSecret?: string;
      signatureMethod: "HMAC-SHA1" | "HMAC-SHA256" | "HMAC-SHA512" | "PLAINTEXT";
      location: "header" | "query" | "body";
      realm?: string;
      callback?: string;
      verifier?: string;
    }
  | ({ type: "oauth2" } & OAuth2Settings);

export type JwtAlgorithm =
  | "HS256" | "HS384" | "HS512" | "RS256" | "RS384" | "RS512" | "PS256" | "PS384" | "PS512" | "ES256" | "ES384";

export interface OAuth2Settings {
  grant: "authorizationCode" | "implicit" | "clientCredentials" | "password";
  authUrl?: string;
  accessTokenUrl?: string;
  clientId: string;
  clientSecret?: string;
  scope?: string;
  redirectUri: string;
  pkce: boolean;
  username?: string;
  password?: string;
  audience?: string;
  clientAuthentication: "header" | "body";
  prefix: string;
}

/** The stored token for some OAuth 2.0 settings. */
export interface TokenStatus {
  key: string;
  obtainedAt: number | null;
  expiresAt: number | null;
  refreshable: boolean;
  scope: string | null;
  present: boolean;
}

export interface TokenInfo {
  key: string;
  clientId: string;
  tokenUrl: string;
  grant: OAuth2Settings["grant"];
  scope?: string;
  obtainedAt: number;
  expiresAt?: number;
  refreshable: boolean;
}

export interface Settings {
  timeoutMs: number | null;
  followRedirects: boolean;
  maxRedirects: number;
  verifyTls: boolean;
}

export interface Scripts {
  preRequest?: string;
  test?: string;
}

/** A request file. A missing `auth` means "inherit from the folder". */
export interface RequestFile {
  method: string;
  url: string;
  description?: string;
  /** The OpenAPI operation this request calls, such as "POST /users/{id}". */
  operation?: string | null;
  headers?: Partial<KeyValue>[];
  body?: Body;
  auth?: Auth | null;
  settings?: Settings;
  /** Checks on the response (status, headers, JSON paths); kept as is, the editor does not show them yet. */
  expect?: Record<string, unknown>;
  scripts?: Scripts;
  /** WebSocket requests: messages to send with a click. */
  messages?: SavedMessage[];
  /** gRPC requests: the method and where it is described. */
  grpc?: GrpcSettings | null;
  /** Saved responses, for documentation and mock servers. */
  examples?: Example[];
  /** Body fields to encrypt before sending and response fields to decrypt. */
  crypto?: CryptoSettings | null;
}

export type CryptoAlgorithm =
  | "aes-256-gcm"
  | "aes-128-gcm"
  | "aes-256-cbc"
  | "aes-128-cbc"
  | "rsa-oaep-sha1"
  | "rsa-oaep-sha256";

/** A response carrying its own RSA-encrypted AES key and IV: base64(RSA(key):RSA(iv):AES-CBC(data)). */
export interface HybridFormat {
  separator: string;
  wrapped: boolean;
  keysBase64: boolean;
  hash: "sha1" | "sha256";
}
export type CryptoEncoding = "base64" | "hex" | "utf8";

export interface CryptoSettings {
  algorithm?: CryptoAlgorithm;
  /** Such as {{encryptionKey}}. */
  key?: string;
  keyEncoding?: CryptoEncoding;
  iv?: string;
  output?: CryptoEncoding;
  /** The RSA private key (PEM), such as {{rsaPrivateKey}}. */
  privateKey?: string;
  hybrid?: HybridFormat | null;
  /** JSON paths; "$" is the whole body. */
  encrypt?: string[];
  decrypt?: string[];
}

export interface Example {
  name: string;
  status: number;
  headers?: { name: string; value: string; enabled?: boolean }[];
  body?: string;
}

export interface GrpcSettings {
  /** package.Service/Method */
  method: string;
  /** .proto files relative to the workspace; none means server reflection. */
  protos?: string[];
  imports?: string[];
}

export interface ServiceInfo {
  name: string;
  methods: MethodInfo[];
}

export interface MethodInfo {
  name: string;
  path: string;
  clientStreaming: boolean;
  serverStreaming: boolean;
  input: string;
  output: string;
  example: string;
}

export type GrpcEvent =
  | { kind: "headers"; metadata: Header[] }
  | { kind: "message"; incoming: boolean; json: string; at: number }
  | { kind: "done"; code: number; status: string; message: string; metadata: Header[]; elapsedMs: number };

/** A tab's gRPC call. */
export interface CallState {
  status: "starting" | "open" | "done";
  events: GrpcEvent[];
  error?: string;
}

export interface SavedMessage {
  name: string;
  content: string;
}

/** What happens on a WebSocket connection. Binary data is base64. */
export type WsEvent =
  | { kind: "open"; url: string; status: number; headers: Header[]; protocol: string | null }
  | { kind: "message"; incoming: boolean; text?: string; binary?: string; size: number; at: number }
  | { kind: "closed"; code: number | null; reason: string; byServer: boolean }
  | { kind: "error"; message: string };

/** A tab's WebSocket connection. */
export interface SocketState {
  status: "connecting" | "open" | "closed";
  events: WsEvent[];
  /** Why connecting failed. */
  error?: string;
}

export interface Variable {
  name: string;
  value?: string;
  secret?: boolean;
  enabled?: boolean;
}

/** A `_folder.yaml`. A missing `auth` means "inherit". */
export interface FolderFile {
  description?: string;
  order?: string[];
  auth?: Auth | null;
  headers?: Partial<KeyValue>[];
  variables?: Variable[];
  scripts?: Scripts;
  /** Where the spec of a collection imported from OpenAPI comes from. */
  openapi?: string | null;
}

/** A variable with its secret value filled in from the keychain. */
export interface VariableValue {
  name: string;
  value: string;
  secret: boolean;
  enabled: boolean;
}

export interface TreeNode {
  kind: "collection" | "folder" | "request";
  name: string;
  path: string;
  method?: string;
  children?: TreeNode[];
}

export interface WorkspaceInfo {
  name: string;
  root: string;
}

export type ImportResult =
  | { kind: "collection"; path: string; warnings: string[] }
  | { kind: "environment"; name: string; warnings: string[] }
  | { kind: "globals"; warnings: string[] }
  | { kind: "several"; collections: string[]; environments: string[]; warnings: string[] };

export interface Header {
  name: string;
  value: string;
}

/** Milliseconds. `dns` and `tls` are null when that phase did not happen. */
export interface Timing {
  dns: number | null;
  connect: number;
  tls: number | null;
  wait: number;
  download: number;
  total: number;
}

export interface Response extends ScriptResults {
  status: number;
  reason: string;
  version: string;
  url: string;
  remoteAddr: string | null;
  headers: Header[];
  body: string;
  bodyBase64: boolean;
  size: number;
  encodedSize: number;
  timing: Timing;
  redirects: { status: number; url: string }[];
  sent: { method: string; url: string; headers: Header[]; bodySize: number };
  unresolved: string[];
  /** Results of the request's `expect:` checks. */
  checks: CheckResult[];
  /** For a text/event-stream response: its events. */
  events?: SseEvent[] | null;
  /** What was decrypted, and the body as it came; or why it could not be decrypted. */
  decryption?: { original: string | null; fields: [string, number][]; error: string | null } | null;
}

/** A server-sent event. `at` is when it arrived (Unix ms), for live ones. */
export interface SseEvent {
  event: string;
  data: string;
  id?: string;
  retry?: number;
  at?: number;
}

/** What the collection's, folders' and request's scripts did. Sent with responses and with errors. */
export interface ScriptResults {
  tests: TestResult[];
  scriptErrors: ScriptError[];
  console: LogLine[];
  /** A rendered pm.visualizer template. */
  visualizer: { url: string | null; error: string | null } | null;
}

export interface TestResult {
  name: string;
  passed: boolean;
  skipped?: boolean;
  error?: string;
}

export interface ScriptError {
  /** Such as "collection Users API", "folder Admin" or "request". */
  script: string;
  message: string;
}

export interface LogLine {
  level: "log" | "info" | "warn" | "error" | "debug";
  text: string;
}

export interface CheckResult {
  check: string;
  passed: boolean;
  expected: string;
  actual: string;
}

/** A variable a request can use. Secret values are never sent to the UI. */
export interface VariableHint {
  name: string;
  value: string;
  secret: boolean;
  /** Such as "environment Staging" or "collection Users API". */
  source: string;
}

export type Language = "curl" | "python" | "javaScript";

export interface Snippet {
  code: string;
  /** The code contains a secret variable's value. */
  containsSecrets: boolean;
  unresolved: string[];
}

export interface AgentSetup {
  /** The `tuma` command on PATH, if installed. */
  cli: string | null;
  /** The project folder agents should run in. */
  project: string | null;
}

export interface RequestError extends Partial<ScriptResults> {
  kind: string;
  message: string;
  unresolved?: string[];
}

/** One request of a collection run. */
export interface RunStep {
  iteration: number;
  name: string;
  path?: string;
  method: string;
  url: string;
  status?: number;
  reason?: string;
  timeMs?: number;
  size?: number;
  error?: string;
  skipped?: boolean;
  checks?: CheckResult[];
  tests?: TestResult[];
  scriptErrors?: ScriptError[];
  console?: LogLine[];
  body?: string;
  passed: boolean;
}

export interface RunSummary {
  requests: number;
  passed: number;
  failed: number;
  checks: number;
  failedChecks: number;
}

export interface RunReport {
  path: string;
  environment: string | null;
  iterations: number;
  startedAt: number;
  durationMs: number;
  summary: RunSummary;
  steps: RunStep[];
  stopped?: string;
}

export interface PastRun {
  file: string;
  path: string;
  environment: string | null;
  iterations: number;
  startedAt: number;
  durationMs: number;
  summary: RunSummary;
  stopped: string | null;
}

/** Values scripts set on this machine, by scope. null means unset. */
export interface CurrentValues {
  environments: Record<string, Record<string, string | null>>;
  collections: Record<string, Record<string, string | null>>;
  globals: Record<string, string | null>;
}

export interface HistoryEntry {
  id: string;
  /** Unix time in milliseconds. */
  sentAt: number;
  request: RequestFile;
  path?: string | null;
  status: number | null;
  error: string | null;
  totalMs: number | null;
  size: number | null;
  /** Who sent it when not the app: "cli", or "mcp:<agent name>". */
  source?: string | null;
}

/** This machine's network settings. */
export interface MachineSettings {
  proxy: {
    mode: "system" | "none" | "custom";
    http: string;
    https: string;
    bypass: string[];
    username: string;
  };
  clientCertificates: CertificateSetting[];
  caCertificates: string[];
  cookies: { enabled: boolean; scriptDomains: string[] };
}

export interface CertificateSetting {
  host: string;
  cert?: string;
  key?: string;
  pfx?: string;
}

export interface SettingsView {
  settings: MachineSettings;
  proxyPasswordSet: boolean;
  passphrasesSet: string[];
  systemProxy: string | null;
}

export interface CookieView {
  domain: string;
  path: string;
  name: string;
  value: string;
  expires?: number;
  secure: boolean;
  httpOnly: boolean;
  sameSite?: string;
  /** As Set-Cookie text, for editing. */
  raw: string;
}
