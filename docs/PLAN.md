# Tuma plan

## Principles

1. **Local-first.** No account, no cloud, no telemetry. It works offline.
2. **Your data is plain files.** Workspaces are folders of readable files you can review and commit to git, so teams
   share collections through a repo.
3. **Fast by default.** Measurable targets, checked in CI (see the performance budget).
4. **Easy to leave Postman.** Postman collection and environment import from day one.
5. **Built for AI agents.** Tuma is the tool a coding agent uses to test the API it is building: send a request, check
   the result, save it as a test. Agents never see secret values.

## Architecture

```
crates/
├── core/        Pure Rust, no UI: HTTP engine, {{var}} templating, storage, importers
└── cli/         `tuma` binary: run requests and collections in CI, and `tuma mcp` for AI agents
app/
├── src-tauri/   Thin Tauri shell: commands that call core
└── src/         React + TypeScript + Vite
```

- **Requests are sent from Rust** (`hyper`/`reqwest` with rustls), never from the webview's `fetch`: no CORS limits,
  full control of certificates, proxies and cookies, and a per-request timing breakdown (DNS, connect, TLS, time to
  first byte, download). The breakdown needs hooks at the hyper level; reqwest does not expose it.
- **`core` is separate from the UI**, so the CLI, the tests and any future front end share one engine.
- **Frontend:** React, TypeScript and Zustand; Tailwind with Radix primitives (shadcn style); CodeMirror 6 for editors;
  a virtualized viewer for large responses.
- **Secrets** live in the OS keychain (`keyring` crate), never in workspace files, so committing a workspace is safe.
- **Three web engines.** Tauri uses WebKitGTK (Linux), WebView2 (Windows) and WKWebView (macOS). The UI is tested on
  all three from the start; WebKitGTK has the most quirks.

## On-disk format

```
my-workspace/
├── workspace.yaml
├── environments/local.yaml        Variables; secret values are only keychain references
└── PCK HWR API/
    ├── _folder.yaml               Folder-level order, auth, headers, variables
    └── notify.yaml                method, url, headers, body, auth, expect (assertions)
```

YAML because it reads well in diffs. Keys are written in a stable order and defaults are left out, so diffs stay
clean. A file's name is the request's name. Each folder's `_folder.yaml` lists its children in display order, so
reordering changes one file. A request without `auth` inherits it from the nearest folder that sets one.

`{{variables}}` resolve from, highest priority first: the selected environment, the request's folders (nearest
first), the collection, then the workspace's globals. The environment wins over collection variables, as in Postman,
so a collection can carry defaults that each environment overrides.

## Milestones

| #      | What you get                                                                                                                                                                  | Done when                                         |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| **M0** | Scaffold, CI building Linux/macOS/Windows packages, lint and test setup                                                                                                       | A blank window launches on all 3 operating systems |
| **M1** Send | Method, URL, query params, headers, body (JSON, form, multipart, raw, binary file), auth (Bearer, Basic, API key), response viewer (pretty or raw, headers, timing, size), history | A real POST with headers and a JSON body works end to end |
| **M2** Organize | Collections and folders on disk, environments, `{{var}}` resolution (environment → folder → collection → global), keychain secrets, **Postman v2.1 import** | Real Postman exports import cleanly               |
| **M3** Agent tool | `tuma` CLI (`send`, `run`, `test`, `check`, `--json`); `tuma mcp` server; ad-hoc requests with assertions; `expect:` assertions in request files; OpenAPI import, discovery, sync and contract checks; JSON Schemas for workspace files | An agent builds an endpoint, tests it through MCP and saves a passing request as a test, with no setup |
| **M4** UX polish | Tabs, command palette (Ctrl+K), keyboard-first navigation, dark and light themes, search, cURL import/export, code snippets (curl, Python, JS)                           | It feels nicer than Postman for daily use         |
| **M5** Scripts | Pre-request and test scripts in an embedded JS engine (`rquickjs`) with a `pm.*`-compatible subset; collection runner; JUnit output for `tuma test` | Imported Postman scripts run unchanged |
| **M6** Protocols | GraphQL, WebSocket, SSE, gRPC, OAuth 2.0 flows, cookie jar, proxy, client certificates                                                                                  | As requested                                      |
| **M7** Release | Signed releases, auto-update, AppImage/.deb/.rpm, Homebrew, winget, Flathub                                                                                                | `v1.0`                                            |

## AI agents

Tuma has two jobs for coding agents (Claude Code, Cursor, Copilot and others). The main one is to be the tool an agent
uses to test the API it is building: change the code, restart the server, call the endpoint, read a short result, fix,
repeat, then save the passing request as a test. The second is to let agents read and edit the workspace like a
person.

### Testing loop

- **No setup.** `tuma mcp` and `tuma send` work in any repo. Requests to `localhost` need nothing; a `.tuma/` workspace
  is created the first time something is saved.
- **Ad-hoc requests.** `send` takes method, URL, headers, body and auth directly; there's no need to save a request
  first. A path such as `/users` resolves against the `baseUrl` variable, from the environment or a one-off `--var`.
- **Assertions in the same call.** Status, header values, JSON path checks (equals, exists, matches), response time,
  and "the response matches the OpenAPI schema for this operation". Each check returns pass or fail, with expected and
  actual values.
- **Short results.** Large bodies are cut off with the full size noted, `select` returns only part of a JSON body, and
  headers are summarized. This keeps the agent's context small.
- **Wait for the server.** `wait_for` polls a URL until it responds or times out, for use after a restart.
- **Save as a test.** `save_request` writes the request and its assertions to the workspace as YAML (an `expect:`
  block), so `tuma test` reruns them later and in CI.
- **Run the suite.** `run` executes a folder or collection and returns details only for failures.
- **Same thing from the CLI**, for agents without MCP:
  `tuma send POST /users --body '{"name":"Ann"}' --expect status=201 --expect '$.id exists' --json`.

### Specs and codebases

- **Find the spec.** Given a running server, Tuma tries the usual paths (`/openapi.json`, `/v3/api-docs`,
  `/swagger/v1/swagger.json`, `/swagger.json`, `/api-docs`); it also looks for `openapi.yaml` or `swagger.json` in the
  repo. This covers FastAPI, Spring (springdoc), ASP.NET (Swashbuckle), NestJS and other frameworks that generate a
  spec.
- **Keep it in sync.** A collection stays linked to its spec. A re-sync adds new endpoints and updates changed ones,
  keeps saved examples, scripts, auth and assertions, and flags removed endpoints instead of deleting them. Example
  bodies are generated from the schemas.
- **No spec.** For Express, Flask, Gin, Axum, Laravel and similar, the agent reads the routes and writes requests with
  `save_request`. A built-in route scanner for popular frameworks may come after v1.0.

### Workspace files

- The YAML format is documented, and a JSON Schema for each file type ships in the repo (`schemas/`) and is referenced
  from the files, so agents and editors can validate what they write. `tuma check` validates a whole workspace and
  reports errors with file and line.

### Guardrails and visibility

- Agents never see secrets. Tuma resolves `{{var}}` and keychain secrets itself, and masks secret values in everything
  it returns.
- By default an agent can send to `localhost` and to the hosts in the selected environment. Other hosts need
  `tuma mcp --allow-host`. Agents can save requests and set non-secret variables, which only touches the workspace's
  YAML files (reviewable in git); `tuma mcp --read-only` turns that off. Agents cannot set or read secrets.
- Agent requests appear live in the desktop app's history, marked as sent by an agent, so you can watch, replay and
  edit them.
- Settings show a ready-made MCP config to paste into Claude Code, Cursor or VS Code.

## Performance budget

- Cold start under 500 ms
- Under 150 MB RAM when idle
- A 5,000-request collection opens in under 1 s
- A 50 MB response renders without freezing the UI

## Testing

- **core:** unit tests, a local mock server for the HTTP engine, golden-file tests for importers. Fixtures are
  hand-made; never commit real exports, which carry secrets.
- **UI:** end-to-end tests with WebdriverIO and `tauri-driver` on Linux and Windows in CI.

## Open questions

- Which features Postman lacks that Tuma should have (to slot into the milestones).
- Feature-by-feature comparison with Postman and proposed milestone changes: see [POSTMAN-PARITY.md](POSTMAN-PARITY.md).
