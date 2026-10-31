import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import { type JsonLeaf, jsonLeaves } from "../jsonpath";
import type { Auth, RequestFile, TokenStatus, TreeNode } from "../types";
import { VariableInput } from "./VariableInput";

const TYPES: { id: Auth["type"] | "inherit"; label: string }[] = [
  { id: "inherit", label: "Inherit from parent" },
  { id: "none", label: "No auth" },
  { id: "bearer", label: "Bearer token" },
  { id: "requestToken", label: "Token from another request" },
  { id: "basic", label: "Basic auth" },
  { id: "apiKey", label: "API key" },
  { id: "oauth2", label: "OAuth 2.0" },
  { id: "jwt", label: "JWT bearer" },
  { id: "digest", label: "Digest auth" },
  { id: "aws", label: "AWS Signature" },
  { id: "oauth1", label: "OAuth 1.0" },
];

function blank(type: Auth["type"]): Auth {
  switch (type) {
    case "bearer":
      return { type, token: "" };
    case "requestToken":
      return { type, request: "", path: "$.token" };
    case "basic":
    case "digest":
      return { type, username: "", password: "" };
    case "apiKey":
      return { type, key: "", value: "", location: "header" };
    case "aws":
      return { type, accessKey: "", secretKey: "", region: "us-east-1", service: "" };
    case "jwt":
      return { type, algorithm: "HS256", secret: "", payload: '{\n  "sub": "1234567890"\n}', prefix: "Bearer" };
    case "oauth1":
      return { type, consumerKey: "", consumerSecret: "", signatureMethod: "HMAC-SHA1", location: "header" };
    case "oauth2":
      return {
        type,
        grant: "authorizationCode",
        clientId: "",
        redirectUri: "http://localhost:8765/callback",
        pkce: true,
        clientAuthentication: "header",
        prefix: "Bearer",
      };
    default:
      return { type: "none" };
  }
}

interface Props {
  /** null means inherit. */
  auth: Auth | null;
  onChange: (auth: Auth | null) => void;
  /** What "inherit" means here, shown as a hint. */
  inheritHint: string;
  /** For OAuth 2.0: the request (with this auth) and where it is, so the token can be shown and fetched. */
  tokenFor?: { request: RequestFile; path: string | null };
}

type Field = {
  key: string;
  label: string;
  placeholder?: string;
  secret?: boolean;
  multiline?: boolean;
  options?: { value: string; label: string }[];
  check?: boolean;
  hint?: string;
};

/** The fields of each type that has more than a couple. */
function fields(auth: Auth): Field[] {
  switch (auth.type) {
    case "aws":
      return [
        { key: "accessKey", label: "Access key", placeholder: "{{awsAccessKey}}" },
        { key: "secretKey", label: "Secret key", secret: true, placeholder: "{{awsSecretKey}}" },
        { key: "region", label: "Region", placeholder: "us-east-1" },
        { key: "service", label: "Service", placeholder: "s3, execute-api, ..." },
        { key: "sessionToken", label: "Session token", secret: true, placeholder: "For temporary credentials" },
      ];
    case "jwt":
      return [
        {
          key: "algorithm",
          label: "Algorithm",
          options: ["HS256", "HS384", "HS512", "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384"].map(
            (a) => ({ value: a, label: a }),
          ),
        },
        auth.algorithm.startsWith("HS")
          ? { key: "secret", label: "Secret", secret: true }
          : { key: "secret", label: "Private key", multiline: true, placeholder: "-----BEGIN PRIVATE KEY-----" },
        ...(auth.algorithm.startsWith("HS") ? [{ key: "secretBase64", label: "Secret is base64", check: true }] : []),
        { key: "payload", label: "Payload", multiline: true, hint: "JSON claims; {{variables}} work here." },
        { key: "headers", label: "Extra headers", multiline: true, placeholder: '{"kid": "key-1"}' },
        { key: "prefix", label: "Header prefix", placeholder: "Bearer" },
      ];
    case "oauth1":
      return [
        { key: "consumerKey", label: "Consumer key" },
        { key: "consumerSecret", label: "Consumer secret", secret: true },
        { key: "token", label: "Access token" },
        { key: "tokenSecret", label: "Token secret", secret: true },
        {
          key: "signatureMethod",
          label: "Signature",
          options: ["HMAC-SHA1", "HMAC-SHA256", "HMAC-SHA512", "PLAINTEXT"].map((m) => ({ value: m, label: m })),
        },
        {
          key: "location",
          label: "Add to",
          options: [
            { value: "header", label: "Header" },
            { value: "query", label: "Query params" },
            { value: "body", label: "Form body" },
          ],
        },
        { key: "realm", label: "Realm" },
        { key: "callback", label: "Callback URL" },
        { key: "verifier", label: "Verifier" },
      ];
    case "oauth2": {
      const browser = auth.grant === "authorizationCode" || auth.grant === "implicit";
      return [
        {
          key: "grant",
          label: "Grant type",
          options: [
            { value: "authorizationCode", label: "Authorization code" },
            { value: "implicit", label: "Implicit" },
            { value: "clientCredentials", label: "Client credentials" },
            { value: "password", label: "Password" },
          ],
        },
        ...(auth.grant === "authorizationCode" ? [{ key: "pkce", label: "Use PKCE (S256)", check: true }] : []),
        ...(browser ? [{ key: "authUrl", label: "Auth URL", placeholder: "https://example.com/oauth/authorize" }] : []),
        ...(auth.grant !== "implicit"
          ? [{ key: "accessTokenUrl", label: "Token URL", placeholder: "https://example.com/oauth/token" }]
          : []),
        { key: "clientId", label: "Client ID" },
        { key: "clientSecret", label: "Client secret", secret: true },
        ...(browser
          ? [
              {
                key: "redirectUri",
                label: "Redirect URI",
                hint: "Tuma listens here for the login; register this exact address with the provider.",
              },
            ]
          : []),
        ...(auth.grant === "password"
          ? [
              { key: "username", label: "Username" },
              { key: "password", label: "Password", secret: true },
            ]
          : []),
        { key: "scope", label: "Scope", placeholder: "read write" },
        { key: "audience", label: "Audience" },
        {
          key: "clientAuthentication",
          label: "Send client credentials",
          options: [
            { value: "header", label: "As Basic auth header" },
            { value: "body", label: "In the body" },
          ],
        },
        { key: "prefix", label: "Header prefix", placeholder: "Bearer" },
      ];
    }
    default:
      return [];
  }
}

/** Every saved request under these nodes, with where it is. */
function savedRequests(nodes: TreeNode[], parents = ""): { path: string; label: string }[] {
  return nodes.flatMap((n) =>
    n.kind === "request"
      ? [{ path: n.path, label: `${parents}${n.name}` }]
      : savedRequests(n.children ?? [], `${parents}${n.name} / `),
  );
}

/** Which saved request returns the token, and which part of its response is the token. */
function RequestTokenFields({
  auth,
  onChange,
  current,
}: {
  auth: Extract<Auth, { type: "requestToken" }>;
  onChange: (auth: Auth) => void;
  current: string | null;
}) {
  const tree = useStore((s) => s.tree);
  const environment = useStore((s) => s.environment);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [found, setFound] = useState<JsonLeaf[] | null>(null);
  const requests = savedRequests(tree).filter((r) => r.path !== current);

  const sendIt = async () => {
    setSending(true);
    setError(null);
    setFound(null);
    try {
      const body = await api.requestTokenBody(auth.request, environment);
      try {
        setFound(jsonLeaves(JSON.parse(body)));
      } catch {
        setError("The response isn't JSON. Leave the path empty to use the whole response as the token.");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <>
      <label>
        <span>Request</span>
        <select
          value={auth.request}
          onChange={(e) => {
            setFound(null);
            setError(null);
            onChange({ ...auth, request: e.target.value });
          }}
        >
          <option value="">Choose a saved request…</option>
          {auth.request && !requests.some((r) => r.path === auth.request) && (
            <option value={auth.request}>{auth.request} (not found)</option>
          )}
          {requests.map((r) => (
            <option key={r.path} value={r.path}>
              {r.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span>Token in response</span>
        <input
          className="mono"
          value={auth.path}
          spellCheck={false}
          placeholder="Whole response"
          onChange={(e) => onChange({ ...auth, path: e.target.value })}
        />
      </label>
      <div className="token-source-actions">
        <button type="button" className="button" disabled={!auth.request || sending} onClick={() => void sendIt()}>
          {sending ? "Sending…" : "Send it and choose from the response"}
        </button>
      </div>
      {error && <p className="notice error">{error}</p>}
      {found && found.length === 0 && <p className="hint">The response has no text or number values to use.</p>}
      {found && found.length > 0 && (
        <div className="token-source-values" role="listbox" aria-label="Values in the response">
          {found.map((leaf) => (
            <button
              type="button"
              key={leaf.path}
              role="option"
              aria-selected={leaf.path === auth.path}
              className={leaf.path === auth.path ? "active" : ""}
              onClick={() => onChange({ ...auth, path: leaf.path })}
            >
              <span className="mono">{leaf.path}</span>
              <span className="mono muted">{leaf.value}</span>
            </button>
          ))}
        </div>
      )}
      <p className="hint">
        The value is sent as a bearer token and reused. This request is only sent, without its scripts, when there is
        no saved token, when it changes, or when a saved token gets a 401.
      </p>
    </>
  );
}

export function AuthForm({ auth, onChange, inheritHint, tokenFor }: Props) {
  const type = auth?.type ?? "inherit";
  const set = (key: string, value: unknown) => auth && onChange({ ...auth, [key]: value } as Auth);
  const record = (auth ?? {}) as unknown as Record<string, unknown>;

  return (
    <div className="form">
      <label>
        <span>Type</span>
        <select
          value={type}
          onChange={(e) => onChange(e.target.value === "inherit" ? null : blank(e.target.value as Auth["type"]))}
        >
          {TYPES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.label}
            </option>
          ))}
        </select>
      </label>
      {auth === null && <p className="hint">{inheritHint}</p>}
      {auth?.type === "bearer" && (
        <label>
          <span>Token</span>
          <VariableInput value={auth.token} spellCheck={false} onChange={(token) => onChange({ ...auth, token })} />
        </label>
      )}
      {auth?.type === "requestToken" && (
        <RequestTokenFields auth={auth} onChange={onChange} current={tokenFor?.path ?? null} />
      )}
      {(auth?.type === "basic" || auth?.type === "digest") && (
        <>
          <label>
            <span>Username</span>
            <VariableInput
              value={auth.username}
              spellCheck={false}
              onChange={(username) => onChange({ ...auth, username })}
            />
          </label>
          <label>
            <span>Password</span>
            <VariableInput
              type="password"
              value={auth.password}
              onChange={(password) => onChange({ ...auth, password })}
            />
          </label>
          {auth.type === "digest" && (
            <p className="hint">Tuma sends the request, answers the server's Digest challenge, and sends it again.</p>
          )}
        </>
      )}
      {auth?.type === "apiKey" && (
        <>
          <label>
            <span>Key</span>
            <VariableInput
              value={auth.key}
              placeholder="X-Api-Key"
              spellCheck={false}
              onChange={(key) => onChange({ ...auth, key })}
            />
          </label>
          <label>
            <span>Value</span>
            <VariableInput value={auth.value} spellCheck={false} onChange={(value) => onChange({ ...auth, value })} />
          </label>
          <label>
            <span>Add to</span>
            <select
              value={auth.location}
              onChange={(e) => onChange({ ...auth, location: e.target.value as "header" | "query" })}
            >
              <option value="header">Header</option>
              <option value="query">Query params</option>
            </select>
          </label>
        </>
      )}
      {auth &&
        fields(auth).map((f) => (
          <label key={f.key} className={f.check ? "check" : undefined}>
            {f.check ? (
              <>
                <input type="checkbox" checked={!!record[f.key]} onChange={(e) => set(f.key, e.target.checked)} />
                {f.label}
              </>
            ) : (
              <>
                <span>{f.label}</span>
                {f.options ? (
                  <select value={String(record[f.key] ?? "")} onChange={(e) => set(f.key, e.target.value)}>
                    {f.options.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                ) : f.multiline ? (
                  <textarea
                    className="mono"
                    rows={4}
                    spellCheck={false}
                    value={String(record[f.key] ?? "")}
                    placeholder={f.placeholder}
                    onChange={(e) => set(f.key, e.target.value)}
                  />
                ) : (
                  <VariableInput
                    type={f.secret ? "password" : "text"}
                    spellCheck={false}
                    value={String(record[f.key] ?? "")}
                    placeholder={f.placeholder}
                    onChange={(value) => set(f.key, value)}
                  />
                )}
              </>
            )}
            {f.hint && <small className="muted">{f.hint}</small>}
          </label>
        ))}
      {auth?.type === "jwt" && (
        <label>
          <span>Send as</span>
          <select
            value={auth.queryParam ? "query" : "header"}
            onChange={(e) => onChange({ ...auth, queryParam: e.target.value === "query" ? "token" : null })}
          >
            <option value="header">Authorization header</option>
            <option value="query">Query parameter "token"</option>
          </select>
        </label>
      )}
      {auth?.type === "oauth2" && tokenFor && <OAuth2Token tokenFor={tokenFor} auth={auth} />}
      <p className="hint">Use {"{{variables}}"} for secrets, and keep their values in an environment marked secret.</p>
    </div>
  );
}

/** The stored token for these settings, and getting a new one. */
function OAuth2Token({ tokenFor, auth }: { tokenFor: NonNullable<Props["tokenFor"]>; auth: Auth }) {
  const environment = useStore((s) => s.environment);
  const [status, setStatus] = useState<TokenStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = { ...tokenFor.request, auth };
  const key = JSON.stringify([auth, tokenFor.path, environment]);

  useEffect(() => {
    api.oauth2Status(request, tokenFor.path, environment).then(setStatus, () => setStatus(null));
    // Reload when the settings, the place or the environment change.
  }, [key]);

  const get = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.oauth2Authorize(request, tokenFor.path, environment));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const forget = async () => {
    if (!status) return;
    await api.oauth2Forget(status.key);
    setStatus({ ...status, present: false, expiresAt: null, obtainedAt: null, refreshable: false });
  };

  const now = Date.now() / 1000;
  const state = !status?.present
    ? "No token yet."
    : status.expiresAt && status.expiresAt <= now
      ? status.refreshable
        ? "The token expired; it is refreshed on the next request."
        : "The token expired."
      : status.expiresAt
        ? `Token valid for ${Math.max(1, Math.round((status.expiresAt - now) / 60))} more minutes${status.refreshable ? ", then refreshed" : ""}.`
        : "Token stored; it has no expiry.";
  const browser = "grant" in auth && (auth.grant === "authorizationCode" || auth.grant === "implicit");

  return (
    <div className="token-box">
      <div className="toolbar spaced">
        <span>{state}</span>
        <span>
          {status?.present && (
            <button className="link-button danger" onClick={() => void forget()}>
              Forget
            </button>
          )}{" "}
          <button className="button small" disabled={busy} onClick={() => void get()}>
            {busy ? (browser ? "Waiting for the login in your browser…" : "Getting a token…") : "Get new access token"}
          </button>
        </span>
      </div>
      {error && <p className="notice error">{error}</p>}
      <p className="hint">
        {browser
          ? "Opens the login page in your browser. "
          : "Tuma also gets one by itself when a request needs it. "}
        Tokens are kept in the OS keychain for this workspace, never in its files.
      </p>
    </div>
  );
}
