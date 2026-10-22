import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { CertificateSetting, CookieView, MachineSettings, TokenInfo } from "../types";

/** This machine's network settings: proxy, client certificates, CA certificates and cookies. */
export function SettingsView() {
  const workspace = useStore((s) => s.workspace);
  const [settings, setSettings] = useState<MachineSettings | null>(null);
  const [saved, setSaved] = useState("");
  const [passwordSet, setPasswordSet] = useState(false);
  const [passphrasesSet, setPassphrasesSet] = useState<string[]>([]);
  const [systemProxy, setSystemProxy] = useState<string | null>(null);
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [passphrases, setPassphrases] = useState<Record<string, string>>({});
  const [cookies, setCookies] = useState<Record<string, CookieView[]>>({});
  const [tokens, setTokens] = useState<TokenInfo[]>([]);
  const loadTokens = () => api.oauth2Tokens().then(setTokens, () => setTokens([]));
  const notify = useStore.getState().notify;

  const load = async () => {
    const view = await api.settingsRead();
    setSettings(view.settings);
    setSaved(JSON.stringify(view.settings));
    setPasswordSet(view.proxyPasswordSet);
    setPassphrasesSet(view.passphrasesSet);
    setSystemProxy(view.systemProxy);
    setPassword(undefined);
    setPassphrases({});
  };
  const loadCookies = () => api.cookiesList().then(setCookies, () => setCookies({}));
  useEffect(() => {
    void load();
    if (workspace) {
      void loadCookies();
      void loadTokens();
    }
  }, [workspace]);

  if (!settings) return null;
  const dirty = JSON.stringify(settings) !== saved || password !== undefined || Object.keys(passphrases).length > 0;
  const update = (patch: Partial<MachineSettings>) => setSettings({ ...settings, ...patch });
  const proxy = settings.proxy;
  const setProxy = (patch: Partial<MachineSettings["proxy"]>) => update({ proxy: { ...proxy, ...patch } });
  const certs = settings.clientCertificates;
  const setCert = (i: number, patch: Partial<CertificateSetting>) =>
    update({ clientCertificates: certs.map((c, j) => (j === i ? { ...c, ...patch } : c)) });
  const pick = async (title: string, extensions: string[]) => {
    const file = await openDialog({ title, filters: [{ name: extensions.join(", "), extensions }] });
    return typeof file === "string" ? file : null;
  };

  const save = async () => {
    try {
      await api.settingsWrite(settings, { proxyPassword: password, passphrases });
      await load();
      notify({ tone: "info", text: "Settings saved." });
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };

  const removeCookies = async (domain?: string, c?: CookieView) => {
    try {
      await api.cookiesRemove(domain ?? null, c?.path ?? null, c?.name ?? null);
      await loadCookies();
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };

  const domains = Object.entries(cookies);

  return (
    <section className="folder-editor" onKeyDown={(e) => e.key === "s" && (e.ctrlKey || e.metaKey) && save()}>
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Settings</span>
          <span className="muted small">This machine only: not saved in workspaces</span>
          {dirty && <span className="dirty">●</span>}
        </div>
        <button className="button primary" disabled={!dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <div className="panel-body settings">
        <h3>Proxy</h3>
        <div className="segmented">
          {(["system", "none", "custom"] as const).map((mode) => (
            <button key={mode} className={proxy.mode === mode ? "active" : ""} onClick={() => setProxy({ mode })}>
              {mode === "system" ? "System" : mode === "none" ? "No proxy" : "Custom"}
            </button>
          ))}
        </div>
        {proxy.mode === "system" && (
          <p className="hint">
            Uses HTTP_PROXY, HTTPS_PROXY and NO_PROXY, as curl does.{" "}
            {systemProxy ? `Now: ${systemProxy}.` : "None are set, so requests go direct."}
          </p>
        )}
        {proxy.mode === "custom" && (
          <div className="form-grid">
            <label>HTTP proxy</label>
            <input value={proxy.http} placeholder="proxy.example.com:3128" onChange={(e) => setProxy({ http: e.target.value })} />
            <label>HTTPS proxy</label>
            <input value={proxy.https} placeholder="Same as HTTP" onChange={(e) => setProxy({ https: e.target.value })} />
            <label>Bypass</label>
            <input
              value={proxy.bypass.join(", ")}
              placeholder="localhost, .internal.example.com"
              onChange={(e) => setProxy({ bypass: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) })}
            />
            <label>User name</label>
            <input value={proxy.username} placeholder="If the proxy needs one" onChange={(e) => setProxy({ username: e.target.value })} />
            <label>Password</label>
            <input
              type="password"
              value={password ?? ""}
              placeholder={passwordSet ? "Saved in the keychain; type to change" : "Kept in the OS keychain"}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
        )}

        <h3>Client certificates</h3>
        <p className="hint">Sent to servers that ask for one (mutual TLS), by host. Use a PFX file, or a CRT and a KEY file.</p>
        {certs.map((c, i) => (
          <div key={i} className="cert-row">
            <input value={c.host} placeholder="api.example.com or *.example.com:8443" onChange={(e) => setCert(i, { host: e.target.value })} />
            {c.pfx ? (
              <>
                <span className="mono small file-name" title={c.pfx}>
                  {c.pfx.split(/[\\/]/).pop()}
                </span>
                <input
                  type="password"
                  value={passphrases[c.host] ?? ""}
                  placeholder={passphrasesSet.includes(c.host) ? "Passphrase saved" : "Passphrase"}
                  onChange={(e) => setPassphrases({ ...passphrases, [c.host]: e.target.value })}
                />
              </>
            ) : (
              <>
                <button
                  className="link-button"
                  onClick={async () => {
                    const f = await pick("Certificate (PEM)", ["crt", "pem", "cer"]);
                    if (f) setCert(i, { cert: f });
                  }}
                >
                  {c.cert ? c.cert.split(/[\\/]/).pop() : "CRT…"}
                </button>
                <button
                  className="link-button"
                  onClick={async () => {
                    const f = await pick("Private key (PEM)", ["key", "pem"]);
                    if (f) setCert(i, { key: f });
                  }}
                >
                  {c.key ? c.key.split(/[\\/]/).pop() : "KEY…"}
                </button>
                <button
                  className="link-button"
                  onClick={async () => {
                    const f = await pick("PKCS#12 file", ["pfx", "p12"]);
                    if (f) setCert(i, { pfx: f, cert: undefined, key: undefined });
                  }}
                >
                  or PFX…
                </button>
              </>
            )}
            <button className="link-button danger" onClick={() => update({ clientCertificates: certs.filter((_, j) => j !== i) })}>
              Remove
            </button>
          </div>
        ))}
        <button className="button small" onClick={() => update({ clientCertificates: [...certs, { host: "" }] })}>
          Add a certificate
        </button>

        <h3>CA certificates</h3>
        <p className="hint">
          Trusted on top of the operating system's, for company or development CAs. PEM files.
        </p>
        {settings.caCertificates.map((path, i) => (
          <div key={path} className="cert-row">
            <span className="mono small">{path}</span>
            <button
              className="link-button danger"
              onClick={() => update({ caCertificates: settings.caCertificates.filter((_, j) => j !== i) })}
            >
              Remove
            </button>
          </div>
        ))}
        <button
          className="button small"
          onClick={async () => {
            const f = await pick("CA certificate (PEM)", ["pem", "crt", "cer"]);
            if (f && !settings.caCertificates.includes(f)) update({ caCertificates: [...settings.caCertificates, f] });
          }}
        >
          Add a CA certificate
        </button>

        <h3>Cookies</h3>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.cookies.enabled}
            onChange={(e) => update({ cookies: { ...settings.cookies, enabled: e.target.checked } })}
          />
          Keep cookies that servers set and send them back (per workspace)
        </label>
        <div className="form-grid">
          <label>Script access</label>
          <input
            value={settings.cookies.scriptDomains.join(", ")}
            placeholder="Domains pm.cookies.jar() may use, such as example.com"
            onChange={(e) =>
              update({
                cookies: { ...settings.cookies, scriptDomains: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) },
              })
            }
          />
        </div>
        {workspace && (
          <div className="cookie-manager">
            <div className="toolbar spaced">
              <h4>{domains.length ? `Cookies in ${workspace.name}` : `No cookies in ${workspace.name} yet`}</h4>
              {domains.length > 0 && (
                <button className="link-button danger" onClick={() => void removeCookies()}>
                  Delete all
                </button>
              )}
            </div>
            {domains.map(([domain, list]) => (
              <details key={domain} open={domains.length < 4}>
                <summary>
                  {domain} <span className="muted small">{list.length}</span>{" "}
                  <button className="link-button danger" onClick={() => void removeCookies(domain)}>
                    Delete
                  </button>
                </summary>
                <table className="current-table">
                  <tbody>
                    {list.map((c) => (
                      <tr key={`${c.path}/${c.name}`}>
                        <td className="mono">{c.name}</td>
                        <td className="mono">{c.value}</td>
                        <td className="muted small">
                          {c.path}
                          {c.expires ? ` · until ${new Date(c.expires * 1000).toLocaleString()}` : " · session"}
                          {c.secure ? " · secure" : ""}
                          {c.httpOnly ? " · HttpOnly" : ""}
                        </td>
                        <td>
                          <button className="link-button danger" onClick={() => void removeCookies(domain, c)}>
                            ×
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </details>
            ))}
          </div>
        )}

        {workspace && (
          <>
            <h3>OAuth 2.0 tokens</h3>
            {tokens.length === 0 ? (
              <p className="hint">
                None in {workspace.name}. Get one from a request's or folder's Auth tab, or with tuma auth login.
              </p>
            ) : (
              <table className="current-table">
                <tbody>
                  {tokens.map((t) => (
                    <tr key={t.key}>
                      <td>{t.clientId}</td>
                      <td className="mono small">{t.tokenUrl}</td>
                      <td className="muted small">
                        {t.scope ? `${t.scope} · ` : ""}
                        {t.expiresAt
                          ? t.expiresAt * 1000 < Date.now()
                            ? t.refreshable
                              ? "expired, refreshable"
                              : "expired"
                            : `until ${new Date(t.expiresAt * 1000).toLocaleString()}`
                          : "no expiry"}
                      </td>
                      <td>
                        <button
                          className="link-button danger"
                          onClick={() => void api.oauth2Forget(t.key).then(loadTokens, (e) => notify({ tone: "error", text: String(e) }))}
                        >
                          Forget
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </>
        )}
      </div>
    </section>
  );
}
