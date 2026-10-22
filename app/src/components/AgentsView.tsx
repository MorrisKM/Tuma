import { useEffect, useState } from "react";
import { agentSetup } from "../api";
import type { AgentSetup } from "../types";

/** How to let a coding agent use Tuma over MCP. */
export function AgentsView() {
  const [setup, setSetup] = useState<AgentSetup | null>(null);
  useEffect(() => {
    agentSetup().then(setSetup);
  }, []);
  if (!setup) return null;

  const command = setup.cli ?? "tuma";
  const json = JSON.stringify({ mcpServers: { tuma: { command, args: ["mcp"] } } }, null, 2);

  return (
    <section className="folder-editor">
      <div className="top-bar">
        <div className="crumbs">
          <span className="crumb current">Connect an agent</span>
        </div>
      </div>
      <div className="panel-body agents">
        <p>
          Coding agents (Claude Code, Cursor, VS Code and others) can send requests through Tuma, check the responses
          and save passing requests as tests, while they build your API. Their requests show up in History, marked
          "agent". They never see secret values.
        </p>

        <h3>1. Install the tuma command</h3>
        {setup.cli ? (
          <p className="ok">
            Installed: <code>{setup.cli}</code>
          </p>
        ) : (
          <>
            <p className="muted">Not found on your PATH yet. From a clone of the Tuma repository:</p>
            <Snippet text="cargo install --path crates/cli" />
          </>
        )}

        <h3>2. Add it to your agent</h3>
        <p className="muted">Claude Code, run in your project{setup.project ? ` (${setup.project})` : ""}:</p>
        <Snippet text={`claude mcp add tuma -- ${command} mcp`} />
        <p className="muted">
          Cursor (<code>.cursor/mcp.json</code>), VS Code (<code>.vscode/mcp.json</code>, with <code>servers</code>{" "}
          instead of <code>mcpServers</code>) and other clients:
        </p>
        <Snippet text={json} />

        <h3>Options</h3>
        <ul className="options">
          <li>
            <code>--env local</code>: the environment to use when the agent doesn't name one.
          </li>
          <li>
            <code>--allow-host api.example.com</code>: agents may only call localhost and the hosts in the selected
            environment unless you allow more.
          </li>
          <li>
            <code>--read-only</code>: don't let agents save requests or change variables.
          </li>
          <li>
            <code>--max-load-users 50</code> and <code>--max-load-seconds 60</code>: the most an agent's performance test
            may use (the defaults). Agents load-test only the hosts they may call.
          </li>
        </ul>
        <p className="hint">The same is available without MCP: run tuma --help.</p>
      </div>
    </section>
  );
}

function Snippet({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="snippet">
      <pre>{text}</pre>
      <button
        className="link-button"
        onClick={() =>
          navigator.clipboard.writeText(text).then(
            () => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            },
            () => {},
          )
        }
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}
