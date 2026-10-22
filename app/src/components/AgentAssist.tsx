import { Bot } from "lucide-react";
import { useState } from "react";
import { useStore } from "../store";

/** The coding agent that used Tuma most recently, from history, if any did in the last day. */
function lastAgent(history: { source?: string | null; sentAt: number }[]): { name: string; minutes: number } | null {
  const entry = history.find((h) => h.source?.startsWith("mcp"));
  if (!entry) return null;
  const minutes = Math.round((Date.now() - entry.sentAt) / 60000);
  if (minutes > 24 * 60) return null;
  return { name: entry.source!.slice(4) || "An agent", minutes };
}

const ago = (m: number) => (m < 1 ? "just now" : m < 60 ? `${m} min ago` : `${Math.round(m / 60)} h ago`);

/** Asks the connected coding agent to write scripts and tests: you describe them, it writes them over MCP. */
export function AgentAssist({ path, scope }: { path: string | null; scope: "request" | "folder" | "collection" }) {
  const history = useStore((s) => s.history);
  const [goal, setGoal] = useState("");
  const [copied, setCopied] = useState(false);
  const agent = lastAgent(history);

  if (!path) {
    return (
      <div className="agent-assist muted small">
        <Bot size={15} /> Save the request first; then a coding agent can write its scripts and tests for you.
      </div>
    );
  }
  const name = path.replace(/\.yaml$/, "");
  const prompt = (goal: string) =>
    scope === "request"
      ? [
          `Using the Tuma MCP tools, write scripts and tests for the saved request "${name}".`,
          `What I want: ${goal || "tests that check the status, the important response fields and their types, and save values later requests need"}.`,
          `1. read_request "${name}", then run_saved "${name}" to send it and see a real response.`,
          "2. Write the scripts with set_scripts: pre-request for setup, test for pm.test()/pm.expect() checks, in Postman's pm.* API.",
          `3. run_saved "${name}" and fix the scripts until every test passes.`,
          "Keep secrets as {{variables}}; never write their values.",
        ].join("\n")
      : [
          `Using the Tuma MCP tools, add automated tests to every request in the ${scope} "${name}".`,
          `What I want: ${goal || "for each request, tests for the status, the important fields and their types; chain values (such as ids and tokens) between requests with pm.environment.set"}.`,
          `1. list_requests, then read_request each request in "${name}" and send it to see real responses.`,
          `2. Write each request's test script with set_scripts; put shared setup (such as logging in) on "${name}" itself with set_scripts, which runs for every request in it.`,
          `3. run_saved "${name}" and fix until everything passes.`,
          "Keep secrets as {{variables}}; never write their values.",
        ].join("\n");

  const copy = () =>
    void navigator.clipboard.writeText(prompt(goal.trim())).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    });

  return (
    <div className="agent-assist">
      <Bot size={16} className={agent ? "accent" : "muted"} />
      <input
        value={goal}
        placeholder={
          scope === "request"
            ? "Ask your agent: check the user is created and keep its id…"
            : "Ask your agent: test every endpoint, logging in first…"
        }
        aria-label="What the agent should write"
        onChange={(e) => setGoal(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && copy()}
      />
      <button className="button small primary" onClick={copy} title="Copy a prompt to paste into your coding agent">
        {copied ? "Copied: paste it into your agent" : "Copy prompt"}
      </button>
      <span className="agent-status muted small">
        {agent ? (
          <>
            <span className="live-dot" /> {agent.name}, {ago(agent.minutes)}
          </>
        ) : (
          <button className="link-button" onClick={() => useStore.setState({ view: { kind: "agents" } })}>
            Connect an agent
          </button>
        )}
      </span>
    </div>
  );
}
