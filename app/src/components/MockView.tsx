import { Copy, Square, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import * as api from "../api";
import { mockName, mocks, useMocks } from "../mocks";
import { useStore } from "../store";
import { MethodTag } from "./ui";

// The same empty list each time, so the selector's result is stable.
const NO_EVENTS: api.MockEvent[] = [];

/** A running mock server: its address, what it answers, and the requests it answered. */
export function MockView({ id }: { id: string }) {
  const server = useMocks((s) => s.servers.find((m) => m.id === id));
  const events = useMocks((s) => s.events[id] ?? NO_EVENTS);
  const environments = useStore((s) => s.environments);
  const [routes, setRoutes] = useState<api.MockRoute[]>([]);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (server) void api.mockRoutes(server.scope).then(setRoutes, () => setRoutes([]));
  }, [server]);

  if (!server) {
    return (
      <section className="folder-editor">
        <p className="empty muted">This mock server has stopped.</p>
      </section>
    );
  }
  const copy = () =>
    void navigator.clipboard.writeText(server.url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  const useInEnvironment = async () => {
    const { refresh, setEnvironment, notify } = useStore.getState();
    try {
      const name = environments.includes("Mock") ? "Mock" : await api.environmentCreate("Mock");
      const variables = (await api.environmentRead(name)).filter((v) => v.name !== "baseUrl");
      await api.environmentWrite(name, [{ name: "baseUrl", value: server.url, secret: false, enabled: true }, ...variables]);
      await refresh();
      setEnvironment(name);
      notify({
        tone: "info",
        text: `The "${name}" environment is active, with baseUrl ${server.url}. Requests written as {{baseUrl}}/… now go to the mock.`,
      });
    } catch (e) {
      notify({ tone: "error", text: String(e) });
    }
  };
  const undocumented = routes.filter((r) => r.examples.length === 0);

  return (
    <section className="folder-editor mock-view">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb">
            Mock server
            <span className="crumb-sep">/</span>
          </span>
          <span className="crumb current">{mockName(server)}</span>
          <span className="pill">Running</span>
        </div>
        <button className="button" onClick={() => void mocks.stop(id)}>
          <Square size={13} />
          Stop
        </button>
      </div>
      <div className="panel-body">
        <div className="mock-address">
          <code>{server.url}</code>
          <button className="button small" onClick={copy}>
            <Copy size={13} />
            {copied ? "Copied" : "Copy"}
          </button>
          <button className="button small primary" onClick={() => void useInEnvironment()}>
            Send requests here
          </button>
        </div>
        <p className="hint">
          Answers with each request's saved examples, matched by method and path. Choose an example with an{" "}
          <code>x-mock-response-code: 404</code> or <code>x-mock-response-name</code> header; otherwise the first
          successful one is sent. Edits to examples apply at once. Browsers may call it from any origin.
          {server.delayMs > 0 && ` Every answer waits ${server.delayMs} ms.`}
        </p>

        <h3 className="mock-heading">
          Answers {routes.length} request{routes.length === 1 ? "" : "s"}
          {undocumented.length > 0 && (
            <span className="warning-text small"> · {undocumented.length} without examples answer 501</span>
          )}
        </h3>
        <table className="header-table mock-routes">
          <tbody>
            {routes.map((r) => (
              <tr key={r.path}>
                <td className="mock-method">
                  <MethodTag method={r.method} />
                </td>
                <td className="mono">{r.pattern}</td>
                <td>
                  <button className="link-button" onClick={() => void useStore.getState().openRequest(r.path)}>
                    {r.name}
                  </button>
                </td>
                <td className="muted small">{r.examples.length ? r.examples.join(", ") : "No examples"}</td>
              </tr>
            ))}
          </tbody>
        </table>

        <div className="toolbar spaced mock-log-head">
          <h3 className="mock-heading">Requests received</h3>
          {events.length > 0 && (
            <button className="icon-button" title="Clear" aria-label="Clear" onClick={() => mocks.clear(id)}>
              <Trash2 size={15} />
            </button>
          )}
        </div>
        {events.length === 0 ? (
          <p className="hint">
            Nothing yet. Try <code>curl {server.url}{routes[0]?.pattern.replace(/\{…\}/g, "1") ?? "/"}</code>
          </p>
        ) : (
          <table className="header-table mock-log">
            <tbody>
              {[...events].reverse().map((e, i) => (
                <tr key={i}>
                  <td className="muted mono small">{new Date(e.at).toLocaleTimeString()}</td>
                  <td className="mock-method">
                    <MethodTag method={e.method} />
                  </td>
                  <td className="mono">{e.path}</td>
                  <td className={`mono ${e.status < 300 ? "ok" : e.status < 400 ? "redirect" : "fail"}`}>{e.status}</td>
                  <td className="muted small">
                    {e.request ? `${e.request}${e.example ? ` › ${e.example}` : ""}` : "No match"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}
