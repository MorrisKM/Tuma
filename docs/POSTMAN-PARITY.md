# Postman feature parity

Research on what Postman (v11 and v12, as of October 2026) offers, and where each feature fits in Tuma. Sources are
Postman's docs at learning.postman.com and the March 2026 v12 launch post.

Each feature gets one of three verdicts:

- **Yes:** build it, in the milestone shown.
- **Local:** build a local-first version of a cloud feature (no account, no server).
- **No:** skip it. It needs Postman's cloud or breaks a principle in [PLAN.md](PLAN.md).

## 1. Sending requests

| Feature                                                                                         | Tuma         |
| ----------------------------------------------------------------------------------------------- | ------------ |
| Methods, including custom verbs                                                                 | Yes, M1      |
| Query and path params (`/users/:id`), bulk edit                                                 | Yes, M1      |
| Headers, auto-generated headers you can see and turn off                                        | Yes, M1      |
| Body: none, form-data (text and files), x-www-form-urlencoded, raw (JSON/XML/HTML/text/JS), binary | Yes, M1   |
| Body: GraphQL (query and variables)                                                             | Yes, M6      |
| Per-request settings: SSL verification, redirects, encoding, HTTP version, timeouts             | Yes, M1      |
| Request descriptions (Markdown docs)                                                            | Yes, M2      |
| Saved examples (request and response pairs per request)                                         | Yes, M2      |

## 2. Authorization

Postman resolves auth through **inherit from parent** (request, folder, collection). Supported types:

| Type                                                            | Tuma    |
| --------------------------------------------------------------- | ------- |
| No auth, inherit, API key (header or query), Bearer, Basic      | Yes, M1 |
| JWT bearer (signs a token from a payload: HS, RS, ES, PS)       | Yes, M6 |
| Digest                                                          | Yes, M6 |
| OAuth 2.0: auth code, auth code with PKCE, implicit, password, client credentials; token refresh; token manager | Yes, M6 |
| OAuth 1.0                                                       | Yes, M6 |
| AWS Signature v4                                                | Yes, M6 |
| NTLM, Hawk, Akamai EdgeGrid, ASAP (Atlassian)                   | Yes, after v1.0 |

The current Postman docs page only lists the first five types; the others now have their own pages (Digest and OAuth 2.0
confirmed). Hawk, NTLM, Akamai EdgeGrid and ASAP come from the long-standing docs.

## 3. Protocols

| Protocol                                                                    | Tuma            |
| --------------------------------------------------------------------------- | --------------- |
| HTTP                                                                        | Yes, M1         |
| GraphQL: schema introspection, schema explorer, query builder, autocomplete | Yes, M6         |
| WebSocket: raw messages, saved messages, message log                        | Yes, M6         |
| Server-sent events                                                          | Yes, M6         |
| gRPC: unary, client, server and bidirectional streaming; server reflection or `.proto` import | Yes, M6 |
| Socket.IO                                                                   | Yes, after v1.0 |
| MQTT                                                                        | Yes, after v1.0 |
| MCP client (call tools, read resources and prompts on an MCP server)        | Yes, after v1.0 |
| AI requests (OpenAI, Anthropic, Google and OpenAI-compatible providers)     | Yes, after v1.0 |
| SOAP (v11 listed it; v12 dropped it; in practice it's XML over HTTP)        | Covered by HTTP |

## 4. Variables, environments and secrets

| Feature                                                                                        | Tuma        |
| ---------------------------------------------------------------------------------------------- | ----------- |
| Scopes: global, collection, environment, data (runner), local; `{{var}}` everywhere            | Yes, M2     |
| Initial and current values (shared vs. this machine only)                                      | Yes, M2     |
| Dynamic variables (`{{$guid}}`, `{{$timestamp}}`, `{{$randomEmail}}` and other Faker ones)     | Yes, M2     |
| Variable autocomplete and hover to see the resolved value                                      | Yes, M4     |
| Vault secrets (`{{vault:name}}`), external vaults (1Password, AWS Secrets Manager, Azure Key Vault, HashiCorp Vault) | Local: OS keychain M2; external vaults after v1.0 |

## 5. Responses

| Feature                                                                              | Tuma    |
| ------------------------------------------------------------------------------------ | ------- |
| Pretty, raw and preview (HTML render, images, PDF) views                             | Yes, M1 |
| Status, time, size; timing breakdown (DNS, TCP, TLS, first byte, download)           | Yes, M1 |
| Response headers and cookies                                                         | Yes, M1 |
| Search inside a response, JSON path filtering                                        | Yes, M4 |
| Save response to file, save as example                                               | Yes, M2 |
| Visualizer (`pm.visualizer` renders an HTML template from response data)             | Yes, M5 |
| Test results tab                                                                     | Yes, M5 |

## 6. Scripts and testing

| Feature                                                                                         | Tuma            |
| ----------------------------------------------------------------------------------------------- | --------------- |
| Pre-request and post-response scripts at request, folder and collection level                  | Yes, M5         |
| Assertions without code (`expect:` in request files); contract checks against OpenAPI          | Tuma extra, M3  |
| `pm.*` API: `pm.test`, `pm.expect` (Chai), `pm.response`, `pm.request`, `pm.environment`/`collectionVariables`/`globals`/`variables`, `pm.sendRequest`, `pm.cookies`, `pm.info`, `pm.iterationData`, `pm.execution` (skip request, set next request), `pm.visualizer`; the older `postman.*`, `tests[...]`, `responseBody` API | Yes, M5; `pm.cookies.jar()` not yet |
| Variables set by scripts are "current values": on this machine only, never in the files        | Yes, M5         |
| `pm.vault`, `pm.state`                                                                          | `pm.vault.get` reads secret variables, M5; setting vault values and `pm.state` not yet |
| Bundled libraries (lodash, moment, crypto-js, ajv, xml2js, cheerio, tv4 and others)             | lodash, moment, crypto-js, chai, ajv, tv4, uuid, atob/btoa and `xml2Json`, M5; cheerio, xml2js and others not yet |
| `pm.require`: shared script packages (team Package Library, or npm/jsr)                         | Local: a scripts folder in the workspace; npm later |
| Snippets and AI help writing tests                                                              | Snippets M5; AI no |
| Console (logs, raw request and response sent over the wire)                                     | Yes, M1         |

## 7. Collection runner and automation

| Feature                                                                               | Tuma                         |
| ------------------------------------------------------------------------------------- | ---------------------------- |
| Run a collection or folder: order, iterations, delay, data file (CSV or JSON)         | Yes, M5                      |
| Persisted results, export of results                                                  | Yes, M5: the last 50 runs per workspace; JSON and JUnit export |
| Postman CLI / Newman: run collections in CI, JUnit and HTML reporters                 | Yes, `tuma` CLI M3; JUnit M5; HTML not yet |
| Performance testing: virtual users, ramp-up, latency and error charts                 | Local: in the runner, after v1.0 |
| Scheduled runs and Monitors (run from Postman's cloud on a schedule)                  | No: the CLI plus cron or CI does this |

## 8. Collections, workspace and git

| Feature                                                                                         | Tuma                    |
| ----------------------------------------------------------------------------------------------- | ----------------------- |
| Collections, folders, drag and drop, duplicate, folder-level auth, scripts and variables        | Yes, M2                 |
| History                                                                                         | Yes, M1                 |
| Tabs, search, keyboard shortcuts                                                                | Yes, M4                 |
| Native Git (v12): collections stored as Collection v3 YAML under `postman/`, branch switching, Local View and Cloud View | Already the core design |
| Workspace documents (Markdown files in the workspace)                                           | Yes, M2                 |

Native Git is the most important finding: Postman v12 now stores collections as YAML files in a git repo, which was
Tuma's main selling point. Tuma still differs: no account, no cloud copy, and no `workspace push`. Tuma should also
**import Collection v3 YAML** as well as v2.1 JSON.

## 9. Import, export and code

| Feature                                                                                         | Tuma            |
| ----------------------------------------------------------------------------------------------- | --------------- |
| Import: Postman v2.1, cURL                                                                      | Yes, M2 and M4  |
| Import: Postman Collection v3 YAML                                                              | Yes, M2         |
| Import: OpenAPI 3 and Swagger 2 (with discovery and sync), HAR                                 | OpenAPI M3, HAR M4 |
| Import: Insomnia, Bruno, GraphQL schema, WSDL, RAML                                             | Yes, after v1.0 |
| Export: Postman v2.1, cURL                                                                      | Yes, M4         |
| Code snippets: curl, Python, JS (fetch/axios), Go, Rust, Java, C#, PHP, Ruby, Swift, Kotlin and others | 3 languages M4, more later |

## 10. Network

| Feature                                                                                           | Tuma            |
| ------------------------------------------------------------------------------------------------- | --------------- |
| Cookie jar with a domain allowlist, cookie manager UI                                             | Yes, M6         |
| Proxy: system, custom, auth, bypass list, env vars                                                | Yes, M6         |
| Client certificates (PFX, or CRT and KEY) per host; custom CA certificates                        | Yes, M6         |
| Capture traffic: built-in proxy, Interceptor browser extension                                    | After v1.0      |

## 11. API design, mocks and docs

| Feature                                                                                         | Tuma                    |
| ----------------------------------------------------------------------------------------------- | ----------------------- |
| Mock servers: from examples, with simulated latency, errors and rate limits; local and CI mocks in v12 | Local: `tuma mock` serving saved examples, after v1.0 |
| API specs editor (OpenAPI, AsyncAPI, protobuf) with linting; generate a collection from a spec  | Generation M3; editor after v1.0 |
| Published API documentation (hosted pages)                                                      | Local: export static HTML, after v1.0 |
| SDK generation (including Kotlin, added April 2026)                                             | No              |

## 12. Cloud, collaboration and AI

| Feature                                                                                         | Tuma |
| ----------------------------------------------------------------------------------------------- | ---- |
| Accounts, team workspaces, roles, comments, forking, pull requests, Public API Network          | No: git does sharing and review |
| Cloud sync between devices                                                                      | No: the workspace is a git repo |
| Flows (visual, no-code workflows with logic, loops and AI blocks)                               | No for now |
| API Catalog, Insights (production observability), Application Inventory                         | No |
| Postman AI agent (writes requests, tests and docs across assets)                                | Local: no built-in agent; your own coding agent uses Tuma through `tuma mcp` and the CLI, M3 |
| Built-in terminal and code editor (v12)                                                         | No |

## Changes to the plan

1. **M2:** add Collection v3 YAML import, saved examples, dynamic variables and initial vs. current values.
2. **M4:** add HAR import, response search and JSON filtering, variable hover and autocomplete.
3. **M5:** the `pm.*` subset must include `pm.execution`, `pm.sendRequest`, `pm.cookies` and the bundled libraries,
   or imported scripts will not "run unchanged". Add JUnit output to `tuma test`.
4. **M6:** list the OAuth 2.0 grant types, Digest, OAuth 1.0, AWS SigV4 and JWT bearer explicitly; add SSE.
5. **After v1.0:** Socket.IO, MQTT, MCP client, local mock server, performance runs, traffic capture, more importers.
6. **AI agents:** a new M3 makes Tuma the agent's testing tool: `tuma mcp`, `tuma send` with assertions, OpenAPI
   discovery and sync, and JSON Schemas for workspace files. Later milestones move up by one. See "AI agents" in
   [PLAN.md](PLAN.md).
7. **Positioning:** "plain files in git" is no longer unique, now that Postman has Native Git. Lead with "no account,
   offline, fast and open source" instead.

## Sources

- [Postman authorization types](https://learning.postman.com/docs/sending-requests/authorization/authorization-types)
- [OAuth 2.0 in Postman](https://learning.postman.com/docs/use/send-requests/authorization/oauth-20)
- [Other API protocols in Postman](https://learning.postman.com/docs/use/send-requests/protocols/protocols)
- [Request basics](https://learning.postman.com/docs/use/send-requests/create-requests/request-basics)
- [Collection Runner](https://learning.postman.com/docs/tests-and-scripts/running-collections/intro-to-collection-runs)
- [Postman Sandbox API reference](https://learning.postman.com/docs/tests-and-scripts/write-scripts/postman-sandbox-reference/overview)
- [Native Git: develop locally](https://learning.postman.com/latest-v-12/docs/use/native-git/develop-locally)
- [Native Git: sync](https://learning.postman.com/latest-v-12/docs/use/native-git/sync)
- [Postman Flows reference](https://learning.postman.com/latest-v-12/docs/postman-flows/reference/overview-action-blocks)
- [New capabilities, March 2026](https://blog.postman.com/new-capabilities-march-2026/)
- [August 2026 feature spotlight](https://learning.postman.com/docs/getting-started/whats-new/2026/8/1)
- [Postman plans](https://learning.postman.com/docs/billing/about-plans)
