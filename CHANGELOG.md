# Changelog

All notable changes to Tuma. Versions follow [Semantic Versioning](https://semver.org/): the workspace file format
and the `tuma` command's options and `--json` output are the public interface.

## 1.0.0 (unreleased)

The first stable release.

### Requests

- HTTP with every body type (JSON, XML, text, form, multipart, binary file, GraphQL), timing breakdown, redirects,
  compressed responses, and a response viewer with pretty, raw and preview modes and a JSON path filter
- GraphQL with schema introspection, completion and a schema explorer
- WebSocket with saved messages and a message log; server-sent events shown live; gRPC (unary and every kind of
  streaming) by server reflection or `.proto` files
- Auth: Bearer, Basic, API key, OAuth 2.0 (every grant, PKCE, refresh, a token manager), JWT, Digest, AWS Signature v4
  and OAuth 1.0, inherited from folders and collections
- Proxies, client certificates, extra CA certificates and a cookie jar per workspace

### Workspaces

- Collections, folders and requests as plain YAML files in a folder you can commit, with JSON Schemas and
  `tuma check`
- Environments, folder, collection and global variables, dynamic variables, secrets in the OS keychain, and current
  values that scripts set kept on this machine only
- Import from Postman (collections, environments, globals), OpenAPI and Swagger (with discovery and sync), HAR and
  cURL; export to Postman and cURL, Python and JavaScript snippets

### Scripts and testing

- Postman's pre-request and test scripts (`pm.*`, the older `postman.*` and `tests[...]`), sandboxed, with lodash,
  moment, crypto-js, chai, ajv and tv4
- Checks without code (`expect:`), and contract checks against OpenAPI specs
- A collection runner with iterations, data files, `setNextRequest`, saved runs and JUnit export

### For terminals, CI and agents

- The `tuma` command: `send`, `run`, `test`, `check`, `import`, `export`, `openapi`, `grpc`, `ws`, `auth`, with
  `--json` output and exit codes for CI
- `tuma mcp`: an MCP server so coding agents can test the API they are building, with host rules and secrets masked

### App

- Tabs, a command palette (Ctrl+K), keyboard shortcuts, light and dark themes, history, and automatic updates
- A resizable sidebar with collections, environments and history (Ctrl+B hides it), the environment beside the tabs,
  and the request and response split by a handle you drag, one above the other or side by side
- A console (Ctrl+`) logging every request, collection run, WebSocket connection and gRPC call, each expanding to
  its network details and timing, request and response headers and bodies, and script output
