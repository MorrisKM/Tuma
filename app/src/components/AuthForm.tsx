import type { Auth } from "../types";

const TYPES: { id: Auth["type"] | "inherit"; label: string }[] = [
  { id: "inherit", label: "Inherit from parent" },
  { id: "none", label: "No auth" },
  { id: "bearer", label: "Bearer token" },
  { id: "basic", label: "Basic auth" },
  { id: "apiKey", label: "API key" },
];

function blank(type: Auth["type"]): Auth {
  switch (type) {
    case "bearer":
      return { type, token: "" };
    case "basic":
      return { type, username: "", password: "" };
    case "apiKey":
      return { type, key: "", value: "", location: "header" };
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
}

export function AuthForm({ auth, onChange, inheritHint }: Props) {
  const type = auth?.type ?? "inherit";

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
          <input value={auth.token} spellCheck={false} onChange={(e) => onChange({ ...auth, token: e.target.value })} />
        </label>
      )}
      {auth?.type === "basic" && (
        <>
          <label>
            <span>Username</span>
            <input
              value={auth.username}
              spellCheck={false}
              onChange={(e) => onChange({ ...auth, username: e.target.value })}
            />
          </label>
          <label>
            <span>Password</span>
            <input
              type="password"
              value={auth.password}
              onChange={(e) => onChange({ ...auth, password: e.target.value })}
            />
          </label>
        </>
      )}
      {auth?.type === "apiKey" && (
        <>
          <label>
            <span>Key</span>
            <input
              value={auth.key}
              placeholder="X-Api-Key"
              spellCheck={false}
              onChange={(e) => onChange({ ...auth, key: e.target.value })}
            />
          </label>
          <label>
            <span>Value</span>
            <input value={auth.value} spellCheck={false} onChange={(e) => onChange({ ...auth, value: e.target.value })} />
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
      <p className="hint">Use {"{{variables}}"} for secrets, and keep their values in an environment marked secret.</p>
    </div>
  );
}
