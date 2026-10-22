# Tuma

*Tuma* is Swahili for "send". It's a fast, local-first API client: a lightweight alternative to Postman, built with
Rust and Tauri.

- **Local-first.** No account, no cloud, no telemetry.
- **Plain files.** Workspaces are folders of readable files you can commit to git.
- **Fast.** Native Rust networking and the system webview instead of a bundled browser.
- **Easy to switch.** Imports Postman, Insomnia and Bruno exports, OpenAPI / Swagger specs, HAR recordings, cURL
  commands and plain JSON environments; exports collections back to Postman format and workspaces as .zip files.
- **Load and security checks.** Performance tests with latency percentiles and passive security checks, exported as
  HTML reports; fields encrypted and decrypted for APIs that need it.
- **Docs and mocks from your requests.** Generates OpenAPI, Markdown or HTML docs from a collection, and runs a mock
  server that answers with the responses you saved as examples.
- **Keyboard-first.** Ctrl+K finds any request or command; Ctrl+Enter sends, Ctrl+S saves, Ctrl+T opens a tab.
- **More than REST.** GraphQL (with introspection and a schema explorer), WebSocket, server-sent events and gRPC
  (by reflection or `.proto` files), plus OAuth 2.0, JWT, Digest, AWS Signature and OAuth 1.0 auth, proxies, client
  certificates and a cookie jar.
- **Runs your Postman scripts.** Pre-request and test scripts (`pm.test`, `pm.expect`, `pm.sendRequest` and the rest)
  run in a sandboxed JavaScript engine, in the app, the runner and CI.
- **Built for AI agents.** Coding agents test the API they are building through Tuma's MCP server or CLI, and save
  passing requests as tests.

> **Status:** version 1.0.0 is being prepared for release. See [CHANGELOG.md](CHANGELOG.md) for what's in it and [docs/PLAN.md](docs/PLAN.md) for the roadmap.

## Install

Download the app for your system from the [latest release](https://github.com/MorrisKM/Tuma/releases/latest):

| System  | Download                                                                                     |
| ------- | -------------------------------------------------------------------------------------------- |
| Linux   | `.deb` (Debian, Ubuntu), `.rpm` (Fedora, openSUSE) or `.AppImage` (anything), for x64 or arm64 |
| macOS   | `.dmg` for Apple Silicon (`aarch64`) or Intel (`x64`)                                         |
| Windows | `-setup.exe` (or `.msi`)                                                                      |

Or with a package manager:

```bash
brew install --cask morriskm/tuma/tuma-app   # macOS: the app and the tuma command
brew install morriskm/tuma/tuma              # macOS and Linux: the tuma command only
winget install MorrisKM.Tuma                 # Windows
```

Every package includes the `tuma` command. The app updates itself; check under Settings → Updates.

## The `tuma` command

```bash
cargo install --path crates/cli   # or use the one that comes with the app

tuma send POST /users -d '{"name":"Ann"}' --var baseUrl=http://localhost:3000 \
  -x status=201 -x '$.id exists' --save "Users/create user"   # Save it as a test if the checks pass
tuma test                       # Rerun saved tests (checks and test scripts); exit code 1 if one fails
tuma run "Users API" --data users.csv --junit results.xml        # A collection run, once per row, for CI
tuma openapi discover --server http://localhost:8000 --import   # Find and import a running API's spec
tuma send GET /todos/7 -x schema                                  # Check the response against the spec
tuma grpc grpc://localhost:50051 shop.v1.Shop/GetItem -d '{"id": 7}'   # gRPC, methods by reflection
tuma ws wss://example.com/socket -m '{"type":"ping"}'                  # WebSocket: send, print what comes back
tuma auth login "Users API"     # OAuth 2.0: get a token for a collection's auth (browser grants open a browser)
tuma docs "Users API" -o openapi.yaml   # API docs: OpenAPI, Markdown or HTML (from the file extension)
tuma mock "Users API" --port 4010       # A mock server answering with the saved examples
tuma perf "Users API" -u 20 -d 60 --max-p95 300 --report perf.html   # A load test, failing CI when too slow
tuma check                      # Validate the workspace files
tuma mcp                        # MCP server for coding agents
```

Connect a coding agent, for example Claude Code: `claude mcp add tuma -- tuma mcp`. Agents can call localhost and
the hosts in the selected environment; add more with `--allow-host`, or stop them saving files with `--read-only`.
Secret values come from the OS keychain, or from `TUMA_SECRET_<NAME>` environment variables in CI, and are never
shown to agents.

Proxies, client certificates, extra CA certificates and cookies are settings of the machine (in the app's Settings,
also used by the CLI), not of the workspace; passwords, passphrases and OAuth 2.0 tokens are kept in the OS keychain.

Scripts can't read files or reach the network except with `pm.sendRequest`, which follows the same host rules as the
request. Values they set with `pm.environment.set` and friends are kept on this machine, in Tuma's data folder, not in
the workspace files, so a login token never ends up in git.

## Development

Prerequisites: Rust (stable), Node.js 20+, pnpm, and the
[Tauri system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```bash
cd app
pnpm install
pnpm tauri dev      # Run the app with hot reload
pnpm tauri build    # Build installers for your OS
```

From the repo root:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Layout

```
crates/core/     Engine: HTTP, {{var}} templating, storage, checks, importers, OpenAPI (no UI)
crates/cli/      The tuma command and its MCP server
app/src-tauri/   Tauri shell: thin commands that call core
schemas/         JSON Schemas of the workspace files
app/src/         React + TypeScript UI
docs/            Plan and design notes
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
Unless you state otherwise, any contribution you submit for inclusion is dual licensed as above, without additional
terms or conditions.
