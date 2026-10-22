# Tuma

*Tuma* is Swahili for "send". It's a fast, local-first API client: a lightweight alternative to Postman, built with
Rust and Tauri.

- **Local-first.** No account, no cloud, no telemetry.
- **Plain files.** Workspaces are folders of readable files you can commit to git.
- **Fast.** Native Rust networking and the system webview instead of a bundled browser.
- **Easy to switch.** Imports Postman collections and environments, and OpenAPI / Swagger specs.
- **Built for AI agents.** Coding agents test the API they are building through Tuma's MCP server or CLI, and save
  passing requests as tests.

> **Status:** early development (milestone M3: the agent tool). See [docs/PLAN.md](docs/PLAN.md) for the roadmap.

## The `tuma` command

```bash
cargo install --path crates/cli

tuma send POST /users -d '{"name":"Ann"}' --var baseUrl=http://localhost:3000 \
  -x status=201 -x '$.id exists' --save "Users/create user"   # Save it as a test if the checks pass
tuma test                       # Rerun saved tests; exit code 1 if a check fails
tuma openapi discover --server http://localhost:8000 --import   # Find and import a running API's spec
tuma send GET /todos/7 -x schema                                  # Check the response against the spec
tuma check                      # Validate the workspace files
tuma mcp                        # MCP server for coding agents
```

Connect a coding agent, for example Claude Code: `claude mcp add tuma -- tuma mcp`. Agents can call localhost and
the hosts in the selected environment; add more with `--allow-host`, or stop them saving files with `--read-only`.
Secret values come from the OS keychain, or from `TUMA_SECRET_<NAME>` environment variables in CI, and are never
shown to agents.

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
