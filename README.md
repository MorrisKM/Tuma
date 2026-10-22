# Tuma

*Tuma* is Swahili for "send". It's a fast, local-first API client: a lightweight alternative to Postman, built with
Rust and Tauri.

- **Local-first.** No account, no cloud, no telemetry.
- **Plain files.** Workspaces are folders of readable files you can commit to git.
- **Fast.** Native Rust networking and the system webview instead of a bundled browser.
- **Easy to switch.** Imports Postman collections and environments.

> **Status:** early development (milestone M0). See [docs/PLAN.md](docs/PLAN.md) for the roadmap.

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
crates/core/     Engine: HTTP, {{var}} templating, storage, importers (no UI)
app/src-tauri/   Tauri shell: thin commands that call core
app/src/         React + TypeScript UI
docs/            Plan and design notes
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
Unless you state otherwise, any contribution you submit for inclusion is dual licensed as above, without additional
terms or conditions.
