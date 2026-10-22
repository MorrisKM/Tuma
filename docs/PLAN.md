# Tuma plan

## Principles

1. **Local-first.** No account, no cloud, no telemetry. It works offline.
2. **Your data is plain files.** Workspaces are folders of readable files you can review and commit to git, so teams
   share collections through a repo.
3. **Fast by default.** Measurable targets, checked in CI (see the performance budget).
4. **Easy to leave Postman.** Postman collection and environment import from day one.

## Architecture

```
crates/
├── core/        Pure Rust, no UI: HTTP engine, {{var}} templating, storage, importers
└── cli/         Later: run saved requests and collections in CI using core
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
    ├── _folder.yaml               Folder-level auth, headers, variables
    └── notify.yaml                method, url, headers, body, auth
```

YAML because it reads well in diffs. Keys are written in a stable order so diffs stay clean.

## Milestones

| #      | What you get                                                                                                                                                                  | Done when                                         |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| **M0** | Scaffold, CI building Linux/macOS/Windows packages, lint and test setup                                                                                                       | A blank window launches on all 3 operating systems |
| **M1** Send | Method, URL, query params, headers, body (JSON, form, multipart, raw, binary file), auth (Bearer, Basic, API key), response viewer (pretty or raw, headers, timing, size), history | A real POST with headers and a JSON body works end to end |
| **M2** Organize | Collections and folders on disk, environments, `{{var}}` resolution (request → folder → collection → environment → global), keychain secrets, **Postman v2.1 import** | Real Postman exports import cleanly               |
| **M3** UX polish | Tabs, command palette (Ctrl+K), keyboard-first navigation, dark and light themes, search, cURL import/export, code snippets (curl, Python, JS)                           | It feels nicer than Postman for daily use         |
| **M4** Scripts | Pre-request and test scripts in an embedded JS engine (`rquickjs`) with a `pm.*`-compatible subset; collection runner                                                      | Imported Postman scripts run unchanged            |
| **M5** Protocols | GraphQL, WebSocket, SSE, gRPC, OAuth 2.0 flows, cookie jar, proxy, client certificates                                                                                  | As requested                                      |
| **M6** Release | Signed releases, auto-update, AppImage/.deb/.rpm, Homebrew, winget, Flathub                                                                                                | `v1.0`                                            |

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
