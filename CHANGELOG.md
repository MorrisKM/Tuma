# Changelog

All notable changes to Tuma. Versions follow [Semantic Versioning](https://semver.org/): the workspace file format
and the `tuma` command's options and `--json` output are the public interface.

## 1.2.0 (2026-10-27)

### Added

- Compare responses: the response panel's Compare tab shows two of the tab's last ten responses side by side (or
  inline), with what changed marked as in a diff tool, plus their status, time, size, environment and request. JSON
  is compared pretty-printed with its keys sorted, and keys or headers that change on every call can be ignored.
- Compare environments: send the request once in each chosen environment and compare what came back.
- Header suggestions: a request's or folder's Headers table offers common header names as you type or click, and
  values for the header in the row, such as content types for Content-Type or `Bearer {{token}}` for Authorization.
- Compare text, below Mock servers in the sidebar: paste or drop two texts (JSON, XML, HTML, plain text or anything
  else) and see what differs, side by side as in a diff tool. Both sides can be edited; Format JSON pretty-prints both
  with their keys sorted and can leave out keys that always differ.
- JSON tools, below Compare text: paste JSON to validate it as you type, with the first mistake explained in plain
  words ("Trailing comma: remove the comma before ]") and marked where it is, then format it (2 spaces, 4 or tabs)
  or minify it onto one line, optionally with keys sorted.
  Repair fixes broken JSON and says what it changed: trailing and missing commas, single quotes, unquoted keys,
  comments, Python and JavaScript values (True, None, undefined, NaN), line breaks in strings, cut-off endings, text
  around the JSON, and several JSON Lines values at once. Numbers keep every digit.

### Changed

- A parameter, header or other field with a value too long for it wraps when clicked and grows to show the whole
  value, up to about eight lines, then scrolls. It goes back to one line when you leave it.

## 1.1.1 (2026-10-27)

### Added

- Pasting a cURL command copied with "Auth and scripts" brings its pre-request scripts back into the request's
  Scripts tab. Scripts from a collection or folder come each under its label.

### Fixed

- The method list's arrow no longer disappears when the list is clicked.

## 1.1.0 (2026-10-26)

### Added

- Choose HTTP, WebSocket or gRPC at the start of the request bar; the URL's scheme follows, keeping TLS.
- URLs in a response body and in header values are links: click one to open a new request to it.
- Copy as code has an "Auth and scripts" toggle. On, the code has the request's auth and what its pre-request
  scripts change, with the scripts above it as comments; the scripts run, but nothing is sent or saved. Off, it is
  the request as written, with no auth and no Authorization headers. OAuth 2.0 and JWT tokens are written out too.
- Settings → Appearance: the theme, a palette that tints the app (Harbor, Grove) and an accent color for outlines,
  focus rings and buttons. Settings are grouped in cards, with a list of sections to jump to.

### Fixed

- The method list shows every method, not only the one already chosen; Custom… takes any other method.

## 1.0.1 (2026-10-23)

### Added

- Drag requests, folders and collections in the sidebar to arrange them, or to move them into another folder or
  collection. Collections keep their order in `workspace.yaml`.

### Fixed

- Multipart bodies sent with a `Content-Type: multipart/form-data` header of their own (as Postman exports have) now
  include the boundary, without which servers couldn't read the parts.
- Relative file paths in multipart and binary bodies are read from the workspace folder, not from wherever Tuma was
  started, so they work in the app and on other computers. Files picked inside the project are saved that way.
- Undo (Ctrl+Z) in the URL bar, body, script and GraphQL editors no longer erases the text or brings back another
  tab's; each request has its own history, and the URL bar has one at all.

## 1.0.0 (2026-10-23)

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
- Encryption of chosen request body fields and decryption of response fields: AES-GCM or AES-CBC, RSA-OAEP (SHA-1
  as node-forge does, or SHA-256) with PEM keys, and hybrid responses whose AES key and IV come RSA-encrypted with the
  data; keys from secret variables

### Scripts and testing

- Postman's pre-request and test scripts (`pm.*`, the older `postman.*` and `tests[...]`), sandboxed, with lodash,
  moment, crypto-js, chai, ajv and tv4
- Checks without code (`expect:`), and contract checks against OpenAPI specs
- A collection runner with iterations, data files, `setNextRequest`, saved runs and JUnit export

### For terminals, CI and agents

- The `tuma` command: `send`, `run`, `test`, `check`, `import`, `export`, `docs`, `mock`, `perf`, `openapi`, `grpc`,
  `ws`, `auth`, with `--json` output and exit codes for CI
- `tuma mcp`: an MCP server so coding agents can test the API they are building, write its scripts and tests, and
  document it (describe requests, save examples, generate docs) and load-test it (`run_performance_test`, capped at
  50 users and 60 seconds unless raised with `--max-load-users` and `--max-load-seconds`), with host rules and
  secrets masked; the app's Scripts tab writes the prompt for them

### App

- Tabs, a command palette (Ctrl+K), keyboard shortcuts, light and dark themes, history, and automatic updates
- A resizable sidebar with collections, environments and history (Ctrl+B hides it), the environment beside the tabs,
  and the request and response split by a handle you drag, one above the other or side by side
- A console (Ctrl+`) logging every request, collection run, WebSocket connection and gRPC call, each expanding to
  its network details and timing, request and response headers and bodies, and script output
