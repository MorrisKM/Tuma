import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Cookie,
  FolderOpen,
  Globe,
  KeyRound,
  type LucideIcon,
  Monitor,
  Moon,
  Palette as PaletteIcon,
  RefreshCw,
  ShieldCheck,
  Sun,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { CertificateSetting, CookieView, MachineSettings, TokenInfo } from "../types";
import { ACCENTS, accentOf, applyColors, applyTheme, PALETTES, type Theme, useColors, useTheme } from "../theme";
import { autoCheckEnabled, type Available, checkForUpdate, install, setAutoCheck, type UpdateStatus, updateStatus } from "../updates";

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
  const sections = [
    { id: "appearance", label: "Appearance", icon: PaletteIcon },
    { id: "workspaces", label: "Workspaces", icon: FolderOpen },
    { id: "updates", label: "Updates", icon: RefreshCw },
    { id: "proxy", label: "Proxy", icon: Globe },
    { id: "certificates", label: "Certificates", icon: ShieldCheck },
    { id: "cookies", label: "Cookies", icon: Cookie },
    ...(workspace ? [{ id: "tokens", label: "OAuth tokens", icon: KeyRound }] : []),
  ];

  return (
    <section className="folder-editor" onKeyDown={(e) => e.key === "s" && (e.ctrlKey || e.metaKey) && save()}>
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Settings</span>
          <span className="muted small">This machine only: not saved in workspaces</span>
          {dirty && <span className="dirty-dot" title="Unsaved changes" />}
        </div>
        <button className="button primary" disabled={!dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <div className="panel-body settings-layout">
        <SettingsNav sections={sections} />
        <div className="settings">
          <Card id="appearance" icon={PaletteIcon} title="Appearance" hint="The theme, the palette that tints the surfaces, and the accent that colors outlines, focus rings and buttons.">
            <Appearance />
          </Card>
          <Card id="workspaces" icon={FolderOpen} title="Workspaces folder" hint="New workspaces are created here, and imported ones are copied here.">
            <WorkspacesFolder />
          </Card>
          <Updates />
          <Card id="proxy" icon={Globe} title="Proxy">
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
          </Card>

          <Card
            id="certificates"
            icon={ShieldCheck}
            title="Client certificates"
            hint="Sent to servers that ask for one (mutual TLS), by host. Use a PFX file, or a CRT and a KEY file."
          >
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

            <h4>CA certificates</h4>
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
          </Card>

          <Card id="cookies" icon={Cookie} title="Cookies">
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
                  <span className="actions">
                    <button className="link-button" onClick={() => useStore.setState({ cookies: { domain: null } })}>
                      Open the cookie manager
                    </button>
                    {domains.length > 0 && (
                      <button className="link-button danger" onClick={() => void removeCookies()}>
                        Delete all
                      </button>
                    )}
                  </span>
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
          </Card>

          {workspace && (
            <Card id="tokens" icon={KeyRound} title="OAuth 2.0 tokens">
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
            </Card>
          )}
        </div>
      </div>
    </section>
  );
}

/** One group of settings: an icon, a title, what it is for. */
function Card({ id, icon: Icon, title, hint, children }: { id: string; icon: LucideIcon; title: string; hint?: string; children?: ReactNode }) {
  return (
    <section className="settings-card" id={`settings-${id}`}>
      <header>
        <span className="settings-icon">
          <Icon size={16} strokeWidth={1.75} />
        </span>
        <div>
          <h3>{title}</h3>
          {hint && <p className="hint">{hint}</p>}
        </div>
      </header>
      <div className="settings-card-body">{children}</div>
    </section>
  );
}

/** Jumps to a group, and marks the one in view. */
function SettingsNav({ sections }: { sections: { id: string; label: string; icon: LucideIcon }[] }) {
  const [active, setActive] = useState(sections[0]?.id);
  const nav = useRef<HTMLElement>(null);
  useEffect(() => {
    const scroller = nav.current?.parentElement;
    if (!scroller) return;
    // The last group whose top has scrolled past, or the last one at the bottom, where short ones can't reach the top.
    const mark = () => {
      const ids = sections.map((s) => s.id);
      if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2) return setActive(ids[ids.length - 1]);
      const top = scroller.getBoundingClientRect().top + 60;
      const passed = ids.filter((id) => (document.getElementById(`settings-${id}`)?.getBoundingClientRect().top ?? Infinity) <= top);
      setActive(passed[passed.length - 1] ?? ids[0]);
    };
    scroller.addEventListener("scroll", mark, { passive: true });
    return () => scroller.removeEventListener("scroll", mark);
  }, [sections.map((s) => s.id).join()]);
  return (
    <nav className="settings-nav" ref={nav} aria-label="Settings sections">
      {sections.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          className={active === id ? "active" : ""}
          onClick={() => {
            setActive(id);
            document.getElementById(`settings-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
          }}
        >
          <Icon size={15} strokeWidth={1.75} />
          {label}
        </button>
      ))}
    </nav>
  );
}

const THEMES: { id: Theme; label: string; icon: LucideIcon }[] = [
  { id: "system", label: "System", icon: Monitor },
  { id: "light", label: "Light", icon: Sun },
  { id: "dark", label: "Dark", icon: Moon },
];

/** Theme, palette and accent, applied as they are picked. */
function Appearance() {
  const theme = useTheme();
  const colors = useColors();
  const accent = accentOf(colors);
  const palette = PALETTES.find((p) => p.id === colors.palette)!;
  return (
    <>
      <h4>Theme</h4>
      <div className="theme-tiles">
        {THEMES.map(({ id, label, icon: Icon }) => (
          <button key={id} className={`theme-tile${theme === id ? " active" : ""}`} onClick={() => applyTheme(id)} aria-pressed={theme === id}>
            <span className={`theme-preview theme-preview-${id}`} aria-hidden>
              <span className="tp-rail" />
              <span className="tp-side" />
              <span className="tp-main">
                <span className="tp-bar" />
                <span className="tp-line" />
                <span className="tp-line short" />
              </span>
            </span>
            <span className="theme-tile-label">
              <Icon size={13} strokeWidth={1.75} />
              {label}
            </span>
          </button>
        ))}
      </div>

      <h4>Palette</h4>
      <div className="palette-cards">
        {PALETTES.map((p) => (
          <button
            key={p.id}
            className={`palette-card${colors.palette === p.id ? " active" : ""}`}
            onClick={() => applyColors({ ...colors, palette: p.id })}
            aria-pressed={colors.palette === p.id}
          >
            <span className="palette-strip" aria-hidden>
              {p.swatches.map((c) => (
                <span key={c} style={{ background: c }} />
              ))}
            </span>
            <span className="palette-name">
              {p.label}
              {colors.palette === p.id && <Check size={13} />}
            </span>
            <span className="muted small">{p.description}</span>
          </button>
        ))}
      </div>

      <h4>Accent and outline color</h4>
      <div className="accent-swatches" role="radiogroup" aria-label="Accent color">
        <button
          role="radio"
          aria-checked={colors.accent === "auto"}
          className={`accent-option${colors.accent === "auto" ? " active" : ""}`}
          onClick={() => applyColors({ ...colors, accent: "auto" })}
          title={`The palette's own: ${ACCENTS.find((a) => a.id === palette.accent)!.label}`}
        >
          <span className="accent-dot auto" style={{ background: ACCENTS.find((a) => a.id === palette.accent)!.color }} />
          <span>{palette.label}'s own</span>
        </button>
        {ACCENTS.map((a) => (
          <button
            key={a.id}
            role="radio"
            aria-checked={colors.accent === a.id}
            className={`accent-option${colors.accent === a.id ? " active" : ""}`}
            onClick={() => applyColors({ ...colors, accent: a.id })}
          >
            <span className="accent-dot" style={{ background: a.color }} />
            <span>{a.label}</span>
          </button>
        ))}
      </div>
      <div className="accent-preview" aria-label="Preview">
        <button className="button primary small" tabIndex={-1}>
          Send
        </button>
        <span className="accent-preview-input">https://api.example.com</span>
        <span className="link-button">A link</span>
        <span className="pill accent-preview-pill">{ACCENTS.find((a) => a.id === accent)!.label}</span>
      </div>
    </>
  );
}

/** Where new and imported workspaces go. */
function WorkspacesFolder() {
  const [folder, setFolder] = useState("");
  const load = () => api.workspaceList().then((l) => setFolder(l.folder), () => {});
  useEffect(() => void load(), []);
  const change = async () => {
    const picked = await openDialog({ directory: true, title: "Where should new workspaces go?", defaultPath: folder });
    if (typeof picked !== "string") return;
    try {
      await api.workspacesDirSet(picked);
      await load();
    } catch (e) {
      useStore.getState().notify({ tone: "error", text: String(e) });
    }
  };
  return (
    <div className="cert-row">
      <span className="mono">{folder}</span>
      <button className="button small" onClick={() => void change()}>
        Change
      </button>
    </div>
  );
}

/** The version, and checking for and installing updates. */
function Updates() {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [auto, setAuto] = useState(autoCheckEnabled);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<Available | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    updateStatus().then(setStatus, () => setStatus(null));
  }, []);
  if (!status) return <Card id="updates" icon={RefreshCw} title="Updates" />;
  const check = async () => {
    setChecking(true);
    setError(null);
    try {
      setResult(await checkForUpdate());
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  };
  return (
    <Card
      id="updates"
      icon={RefreshCw}
      title="Updates"
      hint={`Tuma ${status.current}. ${
        status.enabled
          ? "Updates are signed, and checked against the key built into this version before they are installed."
          : "This build doesn't update itself (it was built from source); update it the way you installed it."
      }`}
    >
      {status.enabled && (
        <>
          <label className="check">
            <input
              type="checkbox"
              checked={auto}
              onChange={(e) => {
                setAuto(e.target.checked);
                setAutoCheck(e.target.checked);
              }}
            />
            Check for updates when Tuma starts
          </label>
          <div className="toolbar">
            <button className="button small" disabled={checking} onClick={() => void check()}>
              {checking ? "Checking…" : "Check now"}
            </button>
            {result === null && <span className="muted small">You have the newest version.</span>}
            {result && (
              <>
                <span className="small">Tuma {result.version} is available.</span>
                <button className="button small primary" onClick={() => void install()}>
                  Update and restart
                </button>
              </>
            )}
          </div>
          {error && <p className="notice error">{error}</p>}
        </>
      )}
    </Card>
  );
}
