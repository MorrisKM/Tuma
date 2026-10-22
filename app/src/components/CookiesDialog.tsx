import { Pencil, Plus, Search, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import * as api from "../api";
import { useStore } from "../store";
import type { CookieView } from "../types";

/** The workspace's cookie jar, as Postman's cookie manager shows it: domains, their cookies, and editing them. */
export function CookiesDialog({ domain: initial }: { domain: string | null }) {
  const workspace = useStore((s) => s.workspace);
  const [cookies, setCookies] = useState<Record<string, CookieView[]>>({});
  const [domain, setDomain] = useState<string | null>(initial);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{ cookie: CookieView | null; raw: string } | null>(null);
  const [newDomain, setNewDomain] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const close = () => useStore.setState({ cookies: null });

  const load = async () => {
    try {
      const all = await api.cookiesList();
      setCookies(all);
      setDomain((d) => (d && (all[d] || d === newDomain) ? d : (Object.keys(all)[0] ?? null)));
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => void load(), []);

  const run = async (action: () => Promise<void>) => {
    setError(null);
    try {
      await action();
      await load();
    } catch (e) {
      setError(String(e));
    }
  };
  const save = () =>
    run(async () => {
      if (!domain || !editing) return;
      await api.cookiesSet(domain, editing.raw, editing.cookie ? [editing.cookie.path, editing.cookie.name] : null);
      setEditing(null);
      setNewDomain(null);
    });

  const q = query.trim().toLowerCase();
  const domains = [...new Set([...Object.keys(cookies), ...(newDomain ? [newDomain] : [])])]
    .filter((d) => !q || d.includes(q) || cookies[d]?.some((c) => c.name.toLowerCase().includes(q)))
    .sort();
  const list = (domain && cookies[domain]) || [];
  const total = Object.values(cookies).reduce((n, l) => n + l.length, 0);

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <div
        className="modal cookies-modal"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && !editing && close()}
        role="dialog"
        aria-label="Cookies"
      >
        <div className="cookies-head">
          <h2>
            Cookies <span className="muted small">{workspace ? `in ${workspace.name}` : ""}</span>
          </h2>
          <label className="search-box cookies-search">
            <Search size={14} />
            <input
              type="search"
              value={query}
              placeholder="Filter domains and cookies"
              aria-label="Filter"
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
        </div>
        {!workspace ? (
          <p className="hint">Cookies are kept per workspace. Open one to see its cookies.</p>
        ) : (
          <div className="cookies-body">
            <div className="cookie-domains">
              {domains.map((d) => (
                <button
                  key={d}
                  className={`list-item${d === domain ? " selected" : ""}`}
                  onClick={() => {
                    setDomain(d);
                    setEditing(null);
                  }}
                >
                  <span className="list-item-text mono-ish">{d}</span>
                  <span className="muted small">{cookies[d]?.length ?? 0}</span>
                </button>
              ))}
              {domains.length === 0 && (
                <p className="muted small cookies-none">
                  {total === 0 ? "No cookies yet. Servers set them as you send requests." : "Nothing matches."}
                </p>
              )}
              {newDomain === null ? (
                <button className="link-button cookies-add-domain" onClick={() => setNewDomain("")}>
                  + Add a domain
                </button>
              ) : (
                <form
                  className="cookies-new-domain"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const d = newDomain.trim().replace(/^https?:\/\//, "").split("/")[0];
                    if (!d) return;
                    setNewDomain(d);
                    setDomain(d);
                    setEditing({ cookie: null, raw: "name=value; Path=/" });
                  }}
                >
                  <input
                    autoFocus
                    className="mono"
                    value={newDomain}
                    placeholder="api.example.com"
                    onChange={(e) => setNewDomain(e.target.value)}
                  />
                </form>
              )}
            </div>
            <div className="cookie-list">
              {domain ? (
                <>
                  <div className="toolbar spaced">
                    <h3 className="mono">{domain}</h3>
                    <span className="actions">
                      <button className="button small" onClick={() => setEditing({ cookie: null, raw: "name=value; Path=/" })}>
                        <Plus size={13} />
                        Add cookie
                      </button>
                      {list.length > 0 && (
                        <button
                          className="button small ghost danger"
                          onClick={() => void run(() => api.cookiesRemove(domain, null, null))}
                        >
                          Clear {domain}
                        </button>
                      )}
                    </span>
                  </div>
                  {editing && (
                    <form
                      className="cookie-editor"
                      onSubmit={(e) => {
                        e.preventDefault();
                        void save();
                      }}
                    >
                      <textarea
                        autoFocus
                        className="mono"
                        value={editing.raw}
                        spellCheck={false}
                        rows={3}
                        onChange={(e) => setEditing({ ...editing, raw: e.target.value })}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" && !e.shiftKey) {
                            e.preventDefault();
                            void save();
                          }
                          if (e.key === "Escape") setEditing(null);
                        }}
                      />
                      <p className="hint">
                        As a Set-Cookie header: <code>name=value; Path=/; Max-Age=3600; Secure; HttpOnly; SameSite=Lax</code>
                      </p>
                      <div className="actions">
                        <button type="button" className="button small" onClick={() => setEditing(null)}>
                          Cancel
                        </button>
                        <button type="submit" className="button small primary">
                          Save cookie
                        </button>
                      </div>
                    </form>
                  )}
                  <ul className="cookie-cards">
                    {list.map((c) => (
                      <li key={`${c.path} ${c.name}`} className="cookie-card">
                        <div className="cookie-line">
                          <span className="cookie-name mono">{c.name}</span>
                          <span className="cookie-value mono" title={c.value}>
                            {c.value || <span className="muted">(empty)</span>}
                          </span>
                          <button className="icon-button" title="Edit" aria-label="Edit" onClick={() => setEditing({ cookie: c, raw: c.raw })}>
                            <Pencil size={13} />
                          </button>
                          <button
                            className="icon-button"
                            title="Delete"
                            aria-label="Delete"
                            onClick={() => void run(() => api.cookiesRemove(domain, c.path, c.name))}
                          >
                            <Trash2 size={13} />
                          </button>
                        </div>
                        <div className="cookie-facts">
                          <span>Path {c.path}</span>
                          <span>{c.expires ? `Expires ${new Date(c.expires * 1000).toLocaleString()}` : "Session"}</span>
                          {c.secure && <span className="pill neutral">Secure</span>}
                          {c.httpOnly && <span className="pill neutral">HttpOnly</span>}
                          {c.sameSite && <span className="pill neutral">SameSite={c.sameSite}</span>}
                        </div>
                      </li>
                    ))}
                  </ul>
                  {list.length === 0 && !editing && <p className="hint">No cookies for this domain yet.</p>}
                </>
              ) : (
                <p className="hint">Choose a domain, or add one to set a cookie by hand.</p>
              )}
            </div>
          </div>
        )}
        {error && <p className="notice error">{error}</p>}
        <div className="modal-actions">
          {total > 0 && (
            <button className="button ghost danger" onClick={() => void run(() => api.cookiesRemove(null, null, null))}>
              <Trash2 size={14} />
              Clear all cookies
            </button>
          )}
          <span className="spacer" />
          <button className="button" onClick={close}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
