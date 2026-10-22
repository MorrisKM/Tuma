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
- Workspaces created from the app in a workspaces folder (Documents/Tuma Workspaces by default), a workspace menu
  to switch between them, and workspaces imported from a folder or a .zip and exported as a .zip
- Import from Postman (collections with their examples, environments, globals), Insomnia, Bruno, OpenAPI and Swagger
  (with discovery and sync), HAR, cURL, and JSON files of names and values as environments; export to Postman and
  cURL, Python and JavaScript snippets
- Duplicate requests, folders and collections; rename anything by clicking its name
- A cookie manager: see, add, edit and delete cookies by domain, clear a domain or everything
- Responses saved as examples on requests
- Comments (`//` and `/* */`) in JSON bodies, removed before sending; Ctrl+/ toggles them

### Docs and mocks

- API docs generated from requests, folders, collections or the whole workspace: OpenAPI 3.1 (YAML or JSON) with
  schemas inferred from examples, Markdown, or a standalone HTML page; in the app, with `tuma docs`, or by a coding
  agent through MCP
- Mock servers that answer with saved examples, matched by method and path, in the app or with `tuma mock`

### Performance and security

- Performance tests: virtual users, a duration or iterations, ramp-up and think time; throughput, error rate and
  latency percentiles per request and per second, live charts, pass/fail limits, and reports as HTML or JSON; in the
  app or with `tuma perf`
- Passive security checks of the responses a test receives: HTTPS, security headers, cookie flags, CORS, information
  that leaks, secrets in URLs
- Encryption of chosen request body fields and decryption of response fields (AES-GCM or AES-CBC, the key from a
  secret variable)

### Scripts and testing

- Postman's pre-request and test scripts (`pm.*`, the older `postman.*` and `tests[...]`), sandboxed, with lodash,
  moment, crypto-js, chai, ajv and tv4
- Checks without code (`expect:`), and contract checks against OpenAPI specs
- A collection runner with iterations, data files, `setNextRequest`, saved runs and JUnit export

### For terminals, CI and agents

- The `tuma` command: `send`, `run`, `test`, `check`, `import`, `export`, `docs`, `mock`, `perf`, `openapi`, `grpc`,
  `ws`, `auth`, with `--json` output and exit codes for CI
- `tuma mcp`: an MCP server so coding agents can test the API they are building, write its scripts and tests, and
  document it (describe requests, save examples, generate docs), with host rules and secrets masked; the app's Scripts
  tab writes the prompt for them

### App

- Tabs, a command palette (Ctrl+K), keyboard shortcuts, light and dark themes, history, and automatic updates
- A resizable sidebar with collections, environments and history (Ctrl+B hides it), the environment beside the tabs,
  and the request and response split by a handle you drag, one above the other or side by side
- A console (Ctrl+`) logging every request, collection run, WebSocket connection and gRPC call, each expanding to
  its network details and timing, request and response headers and bodies, and script output
