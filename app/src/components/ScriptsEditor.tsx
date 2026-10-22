import { useState } from "react";
import type { Scripts } from "../types";
import { CodeEditor } from "./CodeEditor";

type Which = "preRequest" | "test";

interface Snippet {
  label: string;
  code: string;
}

// Common scripts, in Postman's pm.* API, to insert with a click.
const SNIPPETS: Record<Which, Snippet[]> = {
  preRequest: [
    { label: "Set a variable", code: `pm.variables.set("timestamp", Date.now());` },
    { label: "Add a header", code: `pm.request.headers.add({ key: "X-Request-Id", value: require("uuid").v4() });` },
    {
      label: "Log in first and keep the token",
      code: `pm.sendRequest({
  url: pm.environment.get("baseUrl") + "/login",
  method: "POST",
  header: { "Content-Type": "application/json" },
  body: { mode: "raw", raw: JSON.stringify({ user: pm.environment.get("user") }) },
}, (err, res) => {
  pm.environment.set("token", res.json().token);
});`,
    },
    {
      label: "Sign the request (HMAC)",
      code: `const signature = CryptoJS.HmacSHA256(pm.request.url.toString(), pm.environment.get("secret"));
pm.request.headers.upsert({ key: "X-Signature", value: signature.toString(CryptoJS.enc.Base64) });`,
    },
    { label: "Skip this request", code: `if (!pm.environment.get("token")) {\n  pm.execution.skipRequest();\n}` },
  ],
  test: [
    { label: "Status code is 200", code: `pm.test("Status code is 200", () => {\n  pm.response.to.have.status(200);\n});` },
    {
      label: "Response time is below 500 ms",
      code: `pm.test("Response time is below 500 ms", () => {\n  pm.expect(pm.response.responseTime).to.be.below(500);\n});`,
    },
    {
      label: "JSON value check",
      code: `pm.test("Has an id", () => {\n  const json = pm.response.json();\n  pm.expect(json.id).to.exist;\n});`,
    },
    {
      label: "Body contains a string",
      code: `pm.test("Body mentions success", () => {\n  pm.expect(pm.response.text()).to.include("success");\n});`,
    },
    {
      label: "Header is present",
      code: `pm.test("Content-Type is JSON", () => {\n  pm.response.to.have.header("Content-Type");\n  pm.expect(pm.response.headers.get("Content-Type")).to.include("json");\n});`,
    },
    {
      label: "Matches a JSON schema",
      code: `const schema = {\n  type: "object",\n  required: ["id"],\n  properties: { id: { type: "integer" } },\n};\npm.test("Matches the schema", () => {\n  pm.response.to.have.jsonSchema(schema);\n});`,
    },
    { label: "Save a value for later requests", code: `pm.environment.set("userId", pm.response.json().id);` },
    { label: "Go to another request next (runs)", code: `pm.execution.setNextRequest("Request name");` },
    {
      label: "Visualize as a table",
      code: `pm.visualizer.set(\`
<table>
  <tr><th>Name</th><th>Value</th></tr>
  {{#each rows}}<tr><td>{{name}}</td><td>{{value}}</td></tr>{{/each}}
</table>\`, { rows: Object.entries(pm.response.json()).map(([name, value]) => ({ name, value: JSON.stringify(value) })) });`,
    },
  ],
};

/** Pre-request and post-response scripts, for a request, folder or collection. */
export function ScriptsEditor({
  scripts,
  onChange,
  scope,
}: {
  scripts: Scripts;
  onChange: (scripts: Scripts) => void;
  /** Where the scripts run, for the hint: "request", "folder" or "collection". */
  scope: "request" | "folder" | "collection";
}) {
  const [which, setWhich] = useState<Which>("test");
  const value = scripts[which] ?? "";
  const set = (code: string) => onChange({ ...scripts, [which]: code });

  return (
    <div className="scripts-editor">
      <div className="toolbar spaced">
        <div className="segmented">
          <button className={which === "preRequest" ? "active" : ""} onClick={() => setWhich("preRequest")}>
            Pre-request{scripts.preRequest?.trim() ? " ●" : ""}
          </button>
          <button className={which === "test" ? "active" : ""} onClick={() => setWhich("test")}>
            Post-response{scripts.test?.trim() ? " ●" : ""}
          </button>
        </div>
        <span className="muted small">
          {which === "preRequest"
            ? "Runs before the request is sent: change it, set variables."
            : "Runs after the response arrives: pm.test(), pm.expect(), save values."}
          {scope !== "request" && ` Runs for every request in this ${scope}.`}
        </span>
      </div>
      <div className="scripts-body">
        <div className="editor-frame">
          <CodeEditor
            key={which}
            value={value}
            onChange={set}
            language="javascript"
            placeholder={which === "preRequest" ? "// pm.variables.set(...)" : "// pm.test(\"Status code is 200\", () => pm.response.to.have.status(200));"}
          />
        </div>
        <aside className="snippets">
          <h4>Snippets</h4>
          {SNIPPETS[which].map((s) => (
            <button
              key={s.label}
              className="link-button"
              title={s.code}
              onClick={() => set(value.trim() ? `${value.replace(/\s+$/, "")}\n\n${s.code}\n` : `${s.code}\n`)}
            >
              {s.label}
            </button>
          ))}
          <p className="hint">
            Postman's pm.* API. require() works for lodash, moment, crypto-js, chai, ajv, tv4 and uuid.
          </p>
        </aside>
      </div>
    </div>
  );
}
