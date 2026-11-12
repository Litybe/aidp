# Gateway HTTP API

`aidp gateway serve` runs a resident daemon (default `http://127.0.0.1:3001`) that owns every
session, project, automation and preset — see `crates/aidp-gateway/src/http.rs`. The viewer
proxies `/api/*` to it verbatim; the daemon accepts each path with or without the `/api` prefix, so
every route below works both as `GET /api/sessions` and `GET /sessions`. The one exception is a
daemon started with `--ui-dir`: there a `GET` or `HEAD` outside `/api` belongs to the web UI (see
[Web UI](#web-ui----ui-dir)), so API reads must use the `/api` prefix.

State lives under `$AIDP_GATEWAY_DIR`, or `~/.aidp/gateway` when that variable is unset. Inside it:
`sessions/` (one `<id>.json` meta file plus one `<id>.events.jsonl` per session), `projects.json`,
`automations.json`, `presets.json`, `automation-runs.jsonl` (the Historique log), `queue.json` (the
persisted queue entries, keyed by `projectId`), `rate-limit.jsonl` (the append-only rate-limit
change log behind `GET /api/usage/rate-limit`). The daemon seeds no default automation.
`GET /api/health` reports this directory's total size by walking it recursively.

`GET /api/gateway/status` reports `vault` as an absolute, normalized path, or `null` when the vault is
disabled (`AIDP_VAULT` set to `off`, `none`, `0`, `false` or `-`, case-insensitive) or unresolved. The
daemon resolves it from `AIDP_VAULT`, then `~/.aidp/config.json`, without auto-detection. While it is
`null`, `/api/dispatcher/plan` answers mode `no-vault` and `/api/tickets` answers `503`.

## Conventions

### Host integration

`GET /api/gateway/status` reports `capabilities.catalog`, `capabilities.executionIdentity`, `capabilities.restrictedLaunchPolicy`, and `capabilities.headlessLaunchPolicy`.
Hosts must check restricted launch support before relying on permission or MCP isolation fields; older gateways can ignore unknown JSON fields.
Check `headlessLaunchPolicy` before sending `headless`: an older gateway ignores the field and relays permission prompts.
A supported contract does not imply every provider accepts every policy. Unsupported combinations fail before launching an agent.

| Route | Result |
|---|---|
| `GET /api/catalog/templates` | Parsed templates with `stageCount`, `inputCount`, `source` and `inputs` (see below) |
| `GET /api/catalog/templates/:id` | `{parsed, raw}` for one template |
| `POST /api/catalog/templates` | Validate and create `{id, content}`; existing identifiers return `409` |
| `PUT /api/catalog/templates/:id` | Validate and replace an existing template from `{content}` |
| `POST /api/catalog/templates/validate` | `{valid, errors}` for `{content}`, without writing |
| `GET /api/catalog/flows` | Parsed flow definitions |
| `GET /api/catalog/flows/:id` | `{parsed, raw}` for one flow |

Each template of `GET /api/catalog/templates` carries `inputs`, an object keyed by input name whose values are `{ required: bool, kind: string, values: string[], default: any|null, description: string }`; a front builds the launch form from it. `required: true` with no `default` must be given to start the template.

Catalog routes use the configured skill root and the normal gateway access gate.
Identifiers cannot escape that root. Template schema and dependency cycles are checked before writing.
Collections are limited to 256 files and four MiB of source YAML; individual documents retain the one MiB pipeline limit.

### Execution identities

`POST /api/flows` accepts an optional `executionKey` independently of `ticket.key`.
The execution key owns flow state and worktrees; the original ticket key still names the ticket note and its pull request.
With an execution key, repeating the same normalized launch request returns the recorded execution, including terminal executions.
Reusing that key with another request returns `409`. Omitting it preserves the existing ticket-key behavior.

`POST /api/runs` accepts an optional `clientRequestId` with the same retry contract, including after a gateway restart.
Generate it once per launch intent and retain it after network errors.

Flow `policy` accepts `autonomous`, `autoApproveGates`, `provider`, `permissionMode`, `strictMcpConfig`, `mcpConfigId`, `mcpServers`, `deniedShellCommands`, `deniedMcpTools`, `headless`, and `executor`.
Native runs accept the same restriction fields at the top level.
An explicit permission mode never falls back to a more permissive launch mode.
Policies are persisted with the execution, so they must not contain credentials. Nonempty MCP server definitions currently fail before persistence.

`headless: true` launches every session of the execution without a permission prompt: flow steps, native runs and their stage sessions, retries and resumes.
An absent or `default` permission mode becomes `dontAsk`. Other modes are kept, and aidp never switches to `bypassPermissions`.
Claude starts without `--permission-prompt-tool`: a tool call outside the Claude settings allowlist (`permissions.allow`), an edit included, is denied at once and the agent continues.
The host owns that allowlist: it grants what the pipelines need (reads, edits, builds, tests, `aidp`, `git commit`), never `git push`.
The host's deny rules and `--strict-mcp-config` are unchanged. Native stage sessions also get the vault root as an `--add-dir`, next to the run directory.

aidp adds its own deny rules to every headless Claude session, each as `Bash(<prefix>:*)` and `Bash(rtk <prefix>:*)`.
A deny rule wins over an allow rule, so the host can grant `Bash(aidp:*)` and an agent still cannot run these verbs:

| Denied prefix | Why the agent never runs it |
|---|---|
| `aidp flow` | Flow actions such as `git-push` and `pr-create`, and the human steps `approve` and `verdict` |
| `aidp gateway` | A session it starts carries none of these rules |
| `aidp gh pr-create`, `aidp gh pr-ready`, `aidp gh pr-merge` | The flow opens, readies and merges the PR |
| `aidp gh pr-update-branch` | It writes to the PR branch |
| `aidp gh pr-fix` | It starts a fix session that commits |

A prefix rule misses a global option placed before the verb, such as `aidp -v flow git-push`.
So every headless session, whatever its provider, also gets `AIDP_HEADLESS_SESSION=1`, and `aidp` refuses these verbs there with exit 4, wherever the global options sit.
Flow actions run outside the agent sessions and are unaffected.
These guards narrow the risk without removing it: a build or test command the allowlist grants runs code the agent can edit.

### JSON conventions

- Every request and response body is camelCase JSON. Rust fields are `snake_case`; `#[serde(rename_all = "camelCase")]` on each struct does the translation.
- An absent optional field is omitted from the response rather than sent as `null` (`#[serde(skip_serializing_if = "Option::is_none")]` throughout). Don't rely on a key being present just because the type says `Option`.
- Errors are `{"error": "<message>", "code": "<slug>"}` at the matching HTTP status code — see [Error responses](#error-responses) for the status table.
- CORS is restricted to `AIDP_ALLOWED_ORIGINS` since **LOC-26**: the exact `Origin` is echoed only when allowed, never `*` (see [Access policy — LOC-26](#access-policy--loc-26)). `Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS` is unchanged. Since **Lot 5a** every route also passes one auth gate before dispatch — see [Auth and read-only mode](#auth-and-read-only-mode--lot-5a) below. This Lot is the revisit the earlier localhost-only posture called for.
- `OPTIONS` on any path returns `204` with no body and the CORS headers the LOC-26 rules allow (preflight), and always passes the auth gate.

## Error responses

Every route's error goes through `ApiError`, a typed enum mapping to a status and a body of
`{"error": <message>, "code": <slug>}`:

| Variant | Status | `code` |
|---|---|---|
| `NotFound` | 404 | `not_found` |
| `Conflict` | 409 | `conflict` |
| `Unauthorized` | 401 | `unauthorized` |
| `Forbidden` / read-only mode / bad origin | 403 | `forbidden` |
| `Config` | 503 | `config` |
| `ShuttingDown` | 503 | `shutting_down` |
| `Upstream` | 502 | `upstream` |
| `Io` (a `std::io::Error` anywhere in the error chain) | 500 | `io` |
| untyped internal fallback | 500 | `internal` |
| request body over the size cap | 413 | `payload_too_large` |
| untyped validation | 400 | `bad_request` |

Every error response — typed or untyped — is built by the same envelope constructor
(`err`/`api_err` in `crates/aidp-gateway/src/http.rs`), so `code` is always present, never just
on the typed cases above.

A session launch whose `aidp` binary cannot be resolved is also a `Config` 503, refused before
anything is queued or spawned — see [Spawning claude](#spawning-claude).

## Startup and shutdown lifecycle

### In-process entry point

The gateway exposes `aidp_gateway::http::serve_on(listener: std::net::TcpListener, config: HttpConfig) -> anyhow::Result<ServeHandle>` for in-process hosting (used by integration tests). Configuration now arrives as a struct rather than being read from the process environment inside the daemon:

```rust
pub struct HttpConfig {
    pub state_dir: PathBuf,
    pub vault_root: PathBuf,
    pub hook_token: String,
    pub ui_token: String,
    pub read_only: bool,
}
```

`serve_on()` returns a `ServeHandle`:

```rust
impl ServeHandle {
    pub fn port(&self) -> u16;
    pub fn stop(mut self);
}
```

The `vault_root` parameter — not the ambient `AIDP_VAULT` environment variable or `~/.aidp/config.json` — decides whether the dispatcher takes its `no-vault` path. This allows tests to control the vault location independently of the test machine's own setup.

### Environment-based wrapper

`serve(bind: IpAddr, port: u16, state_dir: Option<PathBuf>, ui_dir: Option<PathBuf>)` is a thin wrapper. It builds `HttpConfig` from the process environment (`AIDP_HOOK_TOKEN`, `AIDP_UI_TOKEN`, `AIDP_READ_ONLY`, and vault path via `vault_path()` in `crate::util`), binds to the given address, delegates to `serve_on()` and blocks on incoming requests. `AIDP_MAX_BODY_BYTES` is the exception: `serve_on()` reads it itself, so an embedded or test daemon gets the same body bound as the CLI one (see [Request body bound — LOC-25](#request-body-bound--loc-25)); `serve()` only logs the value at startup when the variable is set. The CLI surface, the startup messages and every HTTP route remain unchanged from the caller's perspective.

#### Spawning claude

Three environment variables shape how the daemon launches a session's `claude` child:

| Variable | Effect |
|---|---|
| `AIDP_CLAUDE_BIN` | **LOC-24.** Path of the `claude` executable to use instead of PATH resolution. Must name an existing file. When unset and `claude` is not on PATH, session start fails with an error naming `AIDP_CLAUDE_BIN`. Also the test seam until LOC-32's `ProcessRunner`. |
| `AIDP_ALLOW_API_KEY` | Unless set to `1`, `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` are stripped from the child environment, so the gateway rides the CLI's OAuth login. |
| `AIDP_BIN` | **LOC-68.** Absolute path of the `aidp` binary the child `claude` must use. When set and non-empty it must name an existing file and is passed through unchanged, never overwritten. When unset, the gateway uses its own executable (`std::env::current_exe()`, the same path `GET /api/gateway/status` reports as `aidpBin`). |

The gateway resolves `aidp` once per launch, in `SessionManager::start`. The child receives
`AIDP_BIN=<that path>` and a `PATH` whose first entry is that binary's directory, followed by the
inherited `PATH` (joined with the platform separator). A session therefore runs the same `aidp` as
the daemon, even when no `aidp` is on the user's `PATH`. If the binary cannot be found (an invalid
user `AIDP_BIN`, or no current executable), the launch is refused before anything is queued or
spawned: `SessionError::Config`, answered as `503 {"error": "<message naming the path and
AIDP_BIN>", "code": "config"}`. This covers every launch path (`POST /api/sessions`, presets,
automations, the ticket dispatcher, queue replay), since all go through `start()`. A binary deleted
after launch still fails inside the session with exit 127.

No shell is involved on any platform. Before LOC-24 the daemon ran `cmd.exe /C claude` on Windows;
that hop is removed. `spawn_claude` resolves `claude` with the shell-free PATH x PATHEXT resolver
`aidp_integrations::resolve_program` and spawns the resolved program directly, so Rust's `.bat`/`.cmd`
argument escaping (the CVE-2024-24576 fix, Rust >= 1.77.2; toolchain pinned to 1.95.0) applies to
every argument. On Windows the direct child is therefore the npm shim `claude.cmd`, which spawns
`node`; the `taskkill /T /F /PID` cleanup is unchanged.

### Shutdown contract

`ServeHandle::stop()` initiates graceful shutdown:

1. Sets a shared `Shutdown` flag visible to the five resident loops (automations scheduler, automations bus consumer, dispatcher tick, dispatcher bus consumer, runs watcher).
2. Calls `SessionManager::shutdown()` — its first production caller — to drain the session index and emit a final bus event.
3. Unblocks the `tiny_http::Server` so the accept loop can join.
4. Returns in under 2 seconds.

Each of the five loops polls the `Shutdown` flag on every iteration and exits cleanly when the flag is set, rather than waiting for a timeout or being force-closed. In-flight HTTP requests are allowed to finish rather than being aborted mid-stream.

**Ctrl-C.** `aidp gateway serve` installs a Ctrl-C handler (the `ctrlc` crate) and waits for either
that signal or the accept thread to exit, checking once a second. On a signal it logs
`[gateway] stop requested — shutting down` and calls `ServeHandle::stop()`, so an interactive stop
runs the same graceful path as a test harness. If the handler cannot be installed, the daemon logs
the error and keeps running.

**Queued launches are not started during shutdown.** `SessionManager::shutdown()` first sets a
`shutting_down` flag. A session that ends after that point, including one the shutdown itself
kills, no longer replays the next launch queued for its project.

Once shutdown starts, `POST /api/sessions` and the resume route (`POST
/api/sessions/:id/resume`) both answer `503 gateway is shutting down` instead of admitting a new
launch. `shutdown()` takes the same admission gate `start()` holds across spawn and registration, so
a session admitted concurrently with `shutdown()` is always fully registered — and so always killed —
before the gate is released; there is no window where a session starts unseen.

The consequence for testing: a test that forgets `ServeHandle::stop()` leaves the accept thread alive and blocks test teardown. A test that calls `stop()` but then expects further requests to that handle will hang — `stop()` is one-way.

### Testing

The daemon's start/stop contract is exercised by integration tests in `crates/aidp-gateway/tests/http_contract.rs` and primitives pinned in `tests/util_primitives.rs`. Two standing constraints apply when extending them:

1. `GET /api/health` is off-limits from a test binary. The endpoint re-executes `<current_exe> doctor --format json-compact`, which in a test binary resolves to the test binary itself (which has no `doctor` subcommand). Each probe is bounded at 5s, both probes run concurrently and their results are cached for 30s, so a cold health check costs at most one 5s timeout. Integration tests verify the route exists; in-process harnesses omit it to keep the test budget under 60s.

2. No assertion may depend on a response body's size from SSE endpoints (e.g., `GET /api/sessions/:id/stream`). The event stream pads each batch with `pad_sse_batch` to smooth out timing variations; response sizes are not load-bearing and vary per batch. Assertions must extract and compare individual event payloads instead.

## Web UI — `--ui-dir`

`aidp gateway serve --ui-dir <dir>` serves a static web UI build (the `dist/` of `aidp-studio`) on
`/` from the same port as the API; without the flag, `AIDP_UI_DIR` gives the directory. The daemon
refuses to start when the directory does not exist and logs a warning when it holds no
`index.html`. Without either, nothing changes: `GET /` answers `404 {"error": "not found"}` like
any unknown route. In-process hosts pass the directory as `RuntimeEnvironment::ui_dir` to
`serve_on_with_environment` (`crates/aidp-gateway/src/static_ui.rs`).

Only `GET` and `HEAD` requests outside `/api` reach the UI; `/api` and every path under it always
reach the API routes, and other methods keep their API behaviour. For those requests:

| Request | Answer |
|---|---|
| A file of the directory (`/assets/index-3f9a1c.js`, `/favicon.svg`) | `200`, the file |
| `/`, or a path without a known extension (`/board`, `/board/GH-12`) | `200`, `index.html` (client-side router) |
| A missing file with a known extension (`/assets/gone.js`) | `404 {"error": "not found"}` |
| `..`, a segment of dots, an encoded `/` or `\`, a `:` or NUL, a bad `%` escape | `404 {"error": "not found"}` |
| A path that resolves outside the directory (symbolic link included) | `404 {"error": "not found"}` |

`Content-Type` comes from the extension (`html`, `js`, `mjs`, `css`, `json`, `map`, `webmanifest`,
`txt`, `svg`, `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `ico`, `woff`, `woff2`, `ttf`, `otf`,
`wasm`; `application/octet-stream` otherwise). Files under `assets/`, whose names Vite hashes, get
`Cache-Control: public, max-age=31536000, immutable`; `index.html` and every other file get
`no-cache`. Every answer carries `X-Content-Type-Options: nosniff` and `Vary: Accept-Encoding`.

**Precompressed files.** The daemon never compresses on the fly. When the build wrote
`<file>.br` or `<file>.gz` next to a file and `Accept-Encoding` names `br` or `gzip` (not with
`q=0`; `*` is ignored), it sends that variant with `Content-Encoding`, Brotli first. The content type
and cache policy stay those of the original file, which must exist.

The access gate of the next section runs first: a loopback caller gets the UI without a token, an
off-loopback caller needs `AIDP_UI_TOKEN` like for any route. A browser sends an `Origin` header on
`POST`. The daemon always accepts its own origin (`http://127.0.0.1:<port>`,
`http://localhost:<port>`, `http://[::1]:<port>` for the port it listens on), so a UI served with
`--ui-dir` needs no setting. In development, the Vite server (`npm run dev`, `http://127.0.0.1:5173`)
proxies `/api` with `changeOrigin: false` and never rewrites `Origin`: start the daemon with
`AIDP_ALLOWED_ORIGINS=http://127.0.0.1:5173`.

## Auth and read-only mode — Lot 5a

`App` carries two new fields: `ui_token` (from `AIDP_UI_TOKEN`) and `read_only` (from
`AIDP_READ_ONLY`, true when it is `1`, `true`, `yes` or `on`). One gate runs in `http.rs::handle()`
**before** `route()` dispatches, so it covers every current and future route without each new match
arm having to remember it. Rules, evaluated in order:

1. `OPTIONS` always passes (preflight).
2. Any path whose first segment is `hooks` is exempt from both the UI gate and read-only mode, and
   keeps its own fail-closed `AIDP_HOOK_TOKEN` bearer check. An inbound GitHub webhook is inherently
   not a loopback call, so a blanket "401 off-loopback" rule would break it.
3. In read-only mode, `POST` / `PUT` / `PATCH` / `DELETE` outside `/api/hooks` return
   `403 {"error": "read-only mode: …"}`. `PATCH` is included deliberately —
   `PATCH /api/sessions/:id` mutates.
4. A non-loopback caller must present a valid `AIDP_UI_TOKEN` or gets `401 {"error": "unauthorized"}`.
   An **unset** `AIDP_UI_TOKEN` therefore makes every route 401 for off-loopback callers.
5. A loopback caller always passes, by design: the viewer proxies `/api/*` with no token until
   Lot 5b's login page exists, and `aidp gateway <cmd>` is itself a loopback client. A peer address
   `tiny_http` reports as `None` counts as non-loopback, not loopback.

The token is read from `Authorization: Bearer <t>`, else `X-Aidp-Token`. The `?token=` query
fallback was removed by LOC-26 (see [Access policy — LOC-26](#access-policy--loc-26)). It is
compared with `util::ct_eq` (constant time). `ui_token` and `hook_token` are **two distinct
secrets**: neither satisfies the other's check.

Startup prints a warning when `AIDP_UI_TOKEN` is unset, mirroring the existing `AIDP_HOOK_TOKEN`
warning, and a second warning when read-only mode is on. The CLI client (`client.rs`) sends
`Authorization: Bearer $AIDP_UI_TOKEN` on every request when that variable is set.

## Request body bound — LOC-25

`http.rs::handle()` bounds what it reads from a request before it routes it. The limit is
`MAX_BODY_BYTES` (`pub const`, `8 * 1024 * 1024` = 8 MiB, aligned on `ledger::MAX_SCAN_BYTES`).
`AIDP_MAX_BODY_BYTES` overrides it with a positive integer byte count; an unset, invalid or zero
value falls back to 8 MiB. The daemon reads the variable once, at startup, inside `serve_on()`.
Rules, evaluated in order:

1. The [auth and read-only gate](#auth-and-read-only-mode--lot-5a) runs first, before a single body
   byte is read. A `401` or `403` therefore buffers nothing: the unread body is drained through a
   fixed 8 KiB buffer and never allocated.
2. When the `Content-Length` header exceeds the limit, the daemon answers
   `413 {"error":"request body too large: limit is <N> bytes"}` at once. This shortcut applies to
   **every** method, `GET` included.
3. The body is read through `Read::take(limit + 1)`. When the bytes actually received exceed the
   limit, the daemon answers the same `413`. This bounded read is the reference bound, not rule 2:
   with `Transfer-Encoding: chunked`, `tiny_http` ignores `Content-Length`.
4. A non-empty body that is not valid JSON gets `400 {"error":"invalid JSON body"}`, before routing.
   An **empty** body still counts as `{}`, so its responses are byte-identical to what they were
   before LOC-25 — for example `POST /api/sessions` with no body still answers
   ``400 {"error":"invalid body: missing field `projectId`"}``.

Every `413` drains the remainder through the fixed buffer before responding, because `tiny_http`'s
`EqualReader::drop` would otherwise allocate the whole unread remainder in one `Vec`. The daemon's
memory cost per request is therefore bounded by the limit. Before LOC-25, a 400 MiB body raised the
process peak to 811.6 MB even when rejected `403`, and a `GET /api/gateway/status` announcing
`Content-Length: 419430400` was answered `200` while the peak rose from ~10 MB to 409.8 MB. An
invalid body likewise became `{}` without a word, so `POST /api/sessions` reported
`missing field projectId` for a field that was present.

`urldecode` is total: it parses percent-escapes on raw bytes and no longer panics on a `%` followed
by a multi-byte character. That panic was never reachable over HTTP — `tiny_http` rejects any
non-ASCII request line by closing the connection without a response. That closure is dependency
behaviour, left unchanged and characterised by a test.

Out of scope: the access policy itself (`Origin` / CSRF, the `?token=` fallback, webhook signature,
the CORS origin) is LOC-26, documented in [Access policy — LOC-26](#access-policy--loc-26); the thread cap, bounded channels and network timeouts are LOC-29. LOC-25
bounds the memory cost of **one** request, not the number of requests.

## Access policy — LOC-26

LOC-26 closes the browser-facing holes left by Lot 5a: cross-site request forgery through the
loopback exemption, a wildcard CORS origin, and tokens leaking through URLs.

**Allowed origins.** `AIDP_ALLOWED_ORIGINS` (comma-separated, default empty) is carried by
`HttpConfig.allowed_origins`. The daemon adds its own origins for the port it really listens on
(`http://127.0.0.1:<port>`, `http://localhost:<port>`, `http://[::1]:<port>`); no wildcard, no
`null`, no other port.

- A `POST`, `PUT`, `PATCH` or `DELETE` whose `Origin` header is not in that list gets `403`, even
  from loopback. With an empty list only the daemon's own origins are accepted.
- A request with no `Origin` header (the `aidp gateway` CLI, `curl`) is unaffected.

**LAN access (phone on the local network).** `aidp gateway serve --bind 0.0.0.0 --ui-dir <build>`
with `AIDP_UI_TOKEN` set.

- The daemon refuses to start on a non-loopback `--bind` when `AIDP_UI_TOKEN` is empty or shorter
  than 32 characters.
- Its own origins also include `http://<ip>:<port>` for the bound address, or, when listening on
  `0.0.0.0` / `::`, for the source address the OS picks for an outbound IPv4 and IPv6 route (the
  machine's LAN address). Any other interface address must be listed in `AIDP_ALLOWED_ORIGINS`.
- The static front (`--ui-dir`: `index.html`, `assets/*`, favicon, `GET`/`HEAD` outside `/api`) is
  served without a token, off-loopback too: it holds no data. Every `/api/*` route still needs the
  token off-loopback: `Authorization: Bearer <token>` or `X-Aidp-Token: <token>`, never the query
  string.
- `EventSource` cannot send headers: `POST /api/events/token` (authenticated) returns
  `{ "token": "...", "expiresIn": <seconds> }`, a single-use token to pass as
  `GET /api/events?es=<token>`.
- Windows firewall: allow `aidp.exe` on the **Private** profile only, never Public or Domain.

**CORS.** `Access-Control-Allow-Origin: *` is gone.

- An allowed `Origin` is echoed exactly, with `Vary: Origin`.
- Any other `Origin` gets no `Access-Control-Allow-Origin`. `OPTIONS` from such an origin answers
  `204` with no CORS headers at all — the Fetch spec's way to refuse a preflight, not a `403`.
- A request with no `Origin` still receives `Access-Control-Allow-Methods` and
  `Access-Control-Allow-Headers`.

**Tokens come from headers only.** Both the general gate and `POST /api/hooks/:id` read a token
only from `Authorization: Bearer <t>` or `X-Aidp-Token`. The `?token=` fallback is removed — a
breaking change for any script or bookmark that relied on it.

**SSE tokens.** `EventSource` cannot set headers, so an off-loopback browser exchanges its token
first:

- `POST /api/events/token` (header-authenticated) returns a single-use token valid 30 s.
- `GET /api/events?es=<token>` consumes it; reusing a token gets `401`.

Loopback callers still need no token, so this only matters off-loopback.

**Webhook signature.** `POST /api/hooks/:id` is GitHub-compatible: a
`X-Hub-Signature-256: sha256=<HMAC-SHA256(AIDP_HOOK_TOKEN, raw body)>` header is verified in
constant time, and a present but invalid signature gets `401`. Without a signature,
`Authorization: Bearer <AIDP_HOOK_TOKEN>` still works.

## Concurrency and resource bounds — LOC-29

LOC-25 bounds the memory one request can cost. LOC-29 bounds how many requests, stream frames and
bus events the daemon holds at once, puts a timeout on every outbound HTTP call, and keeps a panic in
a resident loop from killing that loop.

**In-flight request ceiling.** The accept thread counts the requests being handled. At the ceiling it
answers the next request itself, without spawning a handler thread:

```
503 {"error":"too many requests in flight"}
```

- The ceiling is `MAX_INFLIGHT_REQUESTS` (`pub const`, `256`). `AIDP_MAX_INFLIGHT_REQUESTS`
  overrides it with a positive integer; an unset, invalid or zero value falls back to `256`. The
  daemon reads the variable once, at startup, inside `serve_on()`, and `serve()` logs a valid
  override or the fallback.
- This `503` is not LOC-25's `413`: the `413` rejects one request that is too large, the `503`
  rejects a request because too many others are running. The `503` carries no CORS headers and
  does not drain the body, to keep the saturated path cheap.
- An SSE connection (`GET /api/events`, `GET /api/sessions/:id/stream`) holds its slot for as long
  as it stays open.
- A slot is released when its handler thread finishes, even if the handler panics.

**Bounded bus subscriptions.** Each bus subscriber gets a queue of `SUBSCRIBER_CAPACITY` (`1024`)
events. `emit()` never blocks: when a subscriber's queue is full, that one event is dropped for that
subscriber only, the subscriber stays registered, and `Bus::dropped_events()` counts the drop. Other
subscribers and the 500-event `Bus::recent` ring are unaffected. A subscriber whose receiver is gone is
removed at the next emit.

**Bounded SSE streams.** Both SSE endpoints buffer at most `SSE_CHANNEL_CAPACITY` (`256`) frames for
the client. When the buffer is full, the daemon retries every 50 ms for up to `SSE_STALL_GRACE`
(5 s). If the client still has not read, the daemon counts it as stalled: it emits one `overflow`
bus event (see [Bus events](#bus-events)) and closes the stream. The client must reconnect;
`GET /api/events?since=<seq>` replays up to 100 recent events from the ring.

**Outbound HTTP timeouts.** Every outbound call goes through a shared agent with explicit connect,
read and write timeouts, so a server that accepts the connection and never answers can no longer
hold a thread forever. The agents live in `aidp-integrations/src/net.rs` (one per integration) and
`aidp-gateway/src/net.rs` (the gateway's own).

| Caller | Connect | Read | Write |
|---|---|---|---|
| GitHub images (`GET /api/github/asset`) | 10 s | 30 s | 10 s |
| Gateway agent: automation webhook actions, automation notifications, the `aidp gateway` CLI client | 10 s | 30 s | 30 s |

**Runs watcher negative cache.** The runs watcher (`runs::start_watcher`, the source of
`session.run`) polls every 5 s. A running session whose vault scan finds no run is not scanned again
on every poll: the next scan waits 10 s after the first miss, doubles after each further miss, and is
capped at 300 s. A scan that finds the run clears the entry.

**Resident loops survive a panic.** The five resident loops — `automations.scheduler`,
`automations.bus`, `dispatcher.timer`, `dispatcher.bus` and `runs.watcher` — run each iteration inside
`catch_unwind` (`threads::guarded`). A panic ends that iteration, is logged as
`[gateway] resident loop <name> panicked: <message>` and counted, and the loop goes on. The
shutdown checks stay outside the guard. `GET /api/health` reports the live loops and the panic count
under `threads` (see [`GET /api/health`](#get-apihealth)).

## The session record

Every pipeline run is one `SessionRecord` (`sessions.rs`). Fields marked **Lot 1a** are new in this
Lot; every other field predates it.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Gateway-assigned session id. |
| `projectId` | string | Registered project this session runs in. |
| `projectName` | string | Human label of that project, snapshotted at start. |
| `cwd` | string | Absolute path the `claude` process was spawned in. |
| `engine` | string | `v7` \| `v4` \| `prompt`. |
| `templateId` | string? | Pipeline template id (engine `v7`/`v4` only). |
| `target` | string? | Pipeline target (e.g. a ticket id). |
| `inputs` | object? | Decided pipeline inputs, `key: value`. |
| `prompt` | string | The exact prompt handed to `claude -p` (built from template+inputs, or the raw prompt for engine `prompt`). |
| `permissionMode` | string | `default` \| `acceptEdits` \| `bypassPermissions` \| `plan`. |
| `model` | string? | Requested model, or the model the CLI reports once the session starts. |
| `effort` | string? | Requested reasoning effort. |
| `maxBudgetUsd` | number? | Per-session cost cap passed to the CLI. |
| `status` | string | `running` \| `done` \| `failed` \| `cancelled` \| `orphaned`. |
| `startedAt` / `endedAt` | string / string? | ISO timestamps. |
| `exitCode` | number? | Process exit code once ended. |
| `claudeSessionId` | string? | Claude Code's own session id — what `--resume` needs. |
| `resumedFrom` | string? | Gateway session id this one was resumed from. |
| `costUsd` | number? | The CLI's own cost estimate (informational on a Max subscription). |
| `turns` | number | Completed turns. |
| `lastText` | string? | Last 400 characters of assistant text or turn result — a cheap sidebar preview. |
| `pendingQuestion` | object? | `{toolUseId, questions}` when an `AskUserQuestion` is awaiting a reply. |
| `origin` | object | `{kind, ref?, label?}` — who/what started this session (`ui`, `api`, `automation`, `preset`, …). |
| `automationId` / `automationRunId` | string? | Set when an automation launched this session. |
| `eventCount` | number | Number of events recorded so far. |
| `autoClose` | bool | End the process after the first completed turn with no pending question. |
| `usage` | object? | Cumulative `{input, output, cacheRead, cacheCreate}` token counts. |
| `title` | string? | **Lot 1a.** User-editable label shown in the viewer sidebar. |
| `pinned` | bool | **Lot 1a.** Default `false`. |
| `archived` | bool | **Lot 1a.** Default `false`. |
| `unread` | bool | **Lot 1a.** Default `false`. |
| `startSha` | string? | **Lot 2a.** `git rev-parse HEAD` of the session's `cwd`, captured at start. Omitted from the JSON when absent — every session started before Lot 2a has no `startSha`, and neither does a session whose `cwd` is not a git repository. |
| `contextFrom` | string? | **Lot 6a.** Gateway session id this session's prompt was chained from, when the launch requested it. Absent on every record written before Lot 6a. |
| `chainDepth` | number? | **Lot 6a.** Number of chained hand-offs behind this session; absent/omitted when the session was not chained, starts at 1 for a direct chain (parent depth + 1). Absent on every record written before Lot 6a. |
| `headless` | bool? | `true` when the session never relays a permission prompt; omitted otherwise. A resume of the session inherits it. |

**Compatibility rule.** Every field is either required-since-day-one or carries a serde default
(`#[serde(default)]` / `Option`), so a JSON file written before this Lot still deserializes into a
valid `SessionRecord` — the four new fields simply come back `None`/`false`. This is not incidental:
`SessionManager::load` drops a session file from the index **entirely** if it fails to parse (a
malformed or genuinely incompatible record is silently invisible, not partially loaded), so adding a
required field without a default would make every pre-Lot session vanish from the API on the next
daemon restart. `sessions.rs`'s own test (`old_record_json_still_deserializes`) pins a real 17-key
legacy record for this reason. This is why `startSha` (Lot 2a) is `Option` with a serde default
rather than a required field.

## Inbox endpoint — Lot 2a

### `GET /api/inbox`

Aggregates everything awaiting a human across sessions, PR attention signals and automation runs into one
urgency-ordered list (`inbox.rs::build_inbox`, pure — no I/O, unit-tested from fixtures). Query
parameter `limit=N` truncates the result **after** sorting, so the most urgent entries always
survive truncation.

Each entry (`InboxEntry`):

| Field | Type | Meaning |
|---|---|---|
| `kind` | string | `gate` \| `pr.attention` \| `failure` \| `skippedAutomation` \| `runningSession` \| `flow.human`. |
| `action` | string | `reply` \| `launch` \| `open` \| `dismiss`. |
| `title` | string | |
| `detail` | string? | Omitted when absent. |
| `sessionId` | string? | Omitted when absent. |
| `fingerprint` | string? | Omitted when absent. |
| `automationId` | string? | Omitted when absent. |
| `projectId` | string? | Omitted when absent. |
| `projectName` | string? | Omitted when absent. |
| `at` | string | ISO-8601, the instant this entry became relevant. Used for the tie-break. |
| `urgency` | number | `0` = most urgent. |

Five kinds, most urgent first:

| urgency | kind | source | action | endpoint the action calls |
|---|---|---|---|---|
| 0 | `gate` | a `running` session with a `pendingQuestion` | `reply` | `POST /api/sessions/:id/reply` |
| 1 | `pr.attention` | an unread [PR attention](#pr-attention--lot-28a) signal that notified | `open` | (viewer-side, no call) |
| 2 | `failure` | a `failed` session ended within the last 24 h | `open` | (viewer-side, no call) |
| 3 | `skippedAutomation` | an automation whose latest run is `skipped` and which has ≥ 2 `skipped` runs in the last 24 h | `dismiss` | (viewer-side, no call) |
| 4 | `runningSession` | a `running` session with no pending question | `open` | (viewer-side, no call) |

The waiting `human` steps of flows follow, as `flow.human` entries.

Ties within the same urgency band are broken by `at`, most recent first. A `gate` entry is never
also counted as a `runningSession` — a running session with a pending question surfaces once, as
`gate`. The recurrence rule for `skippedAutomation` is exact and was an open product question until
this Lot: group runs by `automationId`, take the latest by `startedAt`; if that latest run's
`status` is not `skipped`, no entry. Otherwise count how many runs for that automation are
`skipped` **and** started within the 24 h window before that latest run's own `startedAt`; fewer
than 2 in that window yields no entry, 2 or more yields exactly one entry (never one per run).

## Session endpoints

### `GET /api/sessions`

List and search sessions. Query parameters (all optional, all combinable):

| Param | Meaning |
|---|---|
| `status` | Exact match on `status`. |
| `projectId` | Exact match on `projectId`. |
| `q` | **Lot 1a.** Case-insensitive substring match over `title`, `templateId`, `target` and `projectName` — deliberately **not** `projectId` (an opaque id nobody types). Empty or whitespace-only matches everything. |
| `limit` | Default 200. |

Results are sorted newest-first by `startedAt`.

```bash
curl "http://127.0.0.1:3001/api/sessions?q=nightly&status=done&limit=20"
```

**Search coverage is thin on pre-Lot records.** `q` scans four fields — `title`, `templateId`,
`target`, and `projectName` — but three of the four are `Option<String>` carrying
`#[serde(default)]`, so they are frequently absent from session records written before this Lot;
only `projectName` is a plain required `String` always populated at session start. That asymmetry
means the search can degrade, on an existing corpus, to `projectName` alone — a user typing a
remembered phrase gets nothing back and reasonably concludes the feature is broken, when the code is
doing exactly what it says. `prompt` is a required `String` and the only free-text field guaranteed
non-empty on those older records, and it is deliberately not searched: this is an assumed trade-off,
not an oversight, since a multi-kilobyte prompt's substring matches would be dominated by boilerplate
and return everything while meaning nothing. A client that needs to search free text should filter
client-side instead, over `prompt` and/or the truncated `lastText` preview from the listed records.

### `POST /api/sessions`

Start a session. Body is a `StartRequest` (`projectId` required; either `templateId` for engine
`v7`/`v4` or `prompt` for engine `prompt`). If `origin` is omitted the daemon defaults it to
`{kind: "api"}` (autonomous — see `autoClose` below); the viewer's own Launch page sends
`{kind: "ui"}`. Returns the started `SessionRecord`. `404` if `projectId` does not resolve to a
registered project, `400` for any other validation failure (e.g. missing `templateId`/`prompt`).

**Engine and response shape.** `engine` is `native` (default for a `templateId` launch), `workflow` (legacy alias `v7`, the Claude session drives `workflow.js`) or `prompt`. With a template and the native engine, the daemon creates a native run and the response carries `mode: "native"` and `runId` beside the session fields; `autonomous` is an optional boolean (default false for viewer launches). With `engine: "workflow"` the response is the started `SessionRecord` as before. Presets and automations accept the same `engine` and `autonomous` fields. See [native-execution.md](native-execution.md).

**Input validation — LOC-24.** `SessionManager::start` checks these fields before queueing and
before any spawn; an invalid value returns `400 {"error": "..."}`, and for the two enums the error
names the allowed values:

- `permissionMode` — one of `default | acceptEdits | bypassPermissions | plan`.
- `effort` — one of `low | medium | high | xhigh | max`.
- `model` — a character allow-list, not an enum: `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`.
  `claude-sonnet-5` and `claude-opus-4-1` pass; `&`, `|`, `^`, `<`, `>`, `"`, `%`, spaces and
  newlines are rejected.
- `provider` — optional agent provider id overriding the config default for this launch; an id
  unknown to the providers config fails the launch with `400`.
- `resumeFrom` — the stored claude session id it resumes is checked against the same allow-list.
- `503 {"code": "config"}` — the `aidp` binary cannot be resolved (invalid `AIDP_BIN`, or no
  current executable). The launch is refused before anything is queued or spawned; see
  [Spawning claude](#spawning-claude).

Dispatcher-started sessions go through the same validation (`<vault>/TODO/config.yaml`
`dispatcher.model` / `dispatcher.permission_mode`): a typo there fails the launch loudly instead of
being normalised. This is deliberate.
The body also accepts an optional `contextFrom` (**Lot 6a**) — see
[Launch chaining — Lot 6a](#launch-chaining--lot-6a) below for what the gateway does with it.

Two more `StartRequest` fields, both from **Lot 9a**'s dispatcher:

- `autoApproveGates: string[]` — gate names a fully autonomous session may answer itself instead of
  waiting for a human. Exported to the child process as `AIDP_AUTO_APPROVE_GATES`
  (comma-separated).
- `inputsFile: string` — absolute path to an ASCII-escaped JSON file holding the run's inputs.
  Mentioned in the session prompt so the driver passes `--inputs-file` instead of inlining the
  inputs.

Neither field lands on `SessionRecord` — see [The session record](#the-session-record) above; the
record gains no field for this Lot. A dispatcher-started session is instead identified by the same
`origin` shape every other launch path already uses: `origin.kind: "dispatcher"` with
`origin.ref: "<TICKET-KEY>"`. Nothing new is persisted on disk, and nothing new appears in the
viewer.

**Documentation gap, noted rather than fixed here:** this document describes `SessionRecord`
field-by-field but has never carried an equivalent field table for `StartRequest` itself —
`resumeFrom` (`sessions.rs`) has been a real request field with zero documentation since before
this Lot, and `priority` only got documented (see [Queue endpoints — Lot 4a](#queue-endpoints--lot-4a))
because that Lot added a queue section. `contextFrom` is documented here in the same prose style
as its neighbours; a full `StartRequest` field table remains a separate, not-yet-done improvement.

#### Shared writer group: `shared_writer_group`

`StartRequest` also accepts the optional string `shared_writer_group`. Sessions that carry the same
value share one workspace and may write at the same time; the native executor sets it to the run id
for the tasks of a `plan_tasks` stage. Without it, writing sessions stay serialized per workspace.

#### Free sessions: `workspace`, `title`, `external`

`StartRequest` also accepts:

- `workspace` — `{"kind", "ticket"?, "name"?, "path"?}`, resolved to a directory before admission.
  `kind` is one of `project` (the registered checkout), `ticket` (the worktree of `ticket`, which must
  already exist), `scratch` (a throwaway worktree named `name`, matching `^[a-z0-9][a-z0-9-]{0,40}$`,
  created on branch `scratch/<name>` under `<vault>/Pipelines/<repo>/scratch-<name>/.worktree`) or
  `path` (an absolute, existing directory without `..`). The legacy `worktree` field still works and
  creates the ticket worktree itself; as a `kind` value, `worktree` is an alias of `ticket`.
- `title` — session title; defaults to the first prompt line cut to 60 characters.
- `external` — with `resumeFrom`, resumes a Claude Code transcript id found on disk instead of a gateway
  session (see below).

A session started with the `prompt` engine from the UI has `kind: "free"` in the session summary.

Errors: `400` for a missing ticket/name/path, an invalid scratch name, a relative or missing path;
`403` for a path containing `..` or outside the allowed roots; `409` for a ticket with no worktree or a
branch checked out elsewhere; `503` when ticket or scratch is requested with no vault.

**`AIDP_ALLOWED_ROOTS`.** Roots a `path` workspace may live under, separated like `PATH` (`;` on
Windows, `:` elsewhere). When unset or empty, the roots are every registered project path plus the
vault root. It is read from the daemon's environment on each resolution.

**Scratch lifecycle.** A scratch worktree is reused by name. Archiving a scratch session releases it:
unsaved work is saved under `refs/aidp-wip/scratch-<name>-<sha12>`, then the worktree is removed.

**External resume.** `{"resumeFrom": "<transcript id>", "external": true}` looks the id up among the
project's Claude transcripts (see `GET /api/projects/:id/claude-sessions`) and spawns `--resume <id>`.
`400` if `resumeFrom` is missing, `404` if the id is unknown.

**Read-only plan lane.** Sessions with `permissionMode: "plan"` are admitted on a separate lane that does
not count against writers: up to 3 run at once (`read-only-ceiling`), even beside a writer on the same
workspace. A writer launch on a busy workspace is queued with reason `workspace-busy`; a plan launch over
the ceiling is queued with reason `read-only-ceiling`. The reason is the `reason` field of the queue entry.

### `PUT /api/projects/:id`

Sets the test checkout of a project: body `{ "testCheckout": "<absolute path>" }`; `null` or a blank string clears it.
Answers `200` with the project, `400` for a relative path or a non-string value, `404` for an unknown project.
A project with a `testCheckout` lists it in `GET /api/projects` (`"testCheckout": "C:/UNITY_PROJECT"`); without one the
field is absent. `workspace: project` flows (the test lane), their sessions and `GET /api/unity/status` use it instead
of `path`. CLI: `aidp gateway projects test-checkout <id> <path>`, `aidp gateway projects test-checkout <id> --clear`.

### `POST /api/projects/discover`

Body `{roots?: string[]}`. Registers every git repository directly under each root (by default the
folders holding the registered checkouts), then the vault repos whose working copy is found. Idempotent.
Answers `200 {roots, scanned, added: Project[], already, unresolved: string[]}`.

### `GET /api/projects/:id/claude-sessions`

Claude Code transcripts on disk for the project, as
`[{"id", "firstPrompt", "modified"}]`. `404` for an unknown project. Feeds the `external` resume above.

### `GET /api/sessions/:id`

The current `SessionRecord`. `404` on an unknown id.

### `GET /api/sessions/:id/events`

The full event list recorded so far (live in-memory buffer while running, the persisted JSONL
otherwise). `404` on an unknown id.

### `GET /api/sessions/:id/stream`

Server-Sent Events: replays every event recorded so far, then follows live, ending with
`{"type": "done", "status": ..., "exitCode": ...}` once the session finishes. `404` on an unknown id.

### `POST /api/sessions/:id/reply`

Body is `{"text": "...", "attachments"?: [{"mediaType": "image/png", "dataBase64": "..."}]}` for a free message, or `{"answers": {"<question text>": {"answer": "...", "note?": "..."}}}` to resolve a pending `AskUserQuestion`. `409` if the session isn't running, isn't found, or the body has neither `text` nor `answers`.

`attachments` are images: `mediaType` is one of `image/png`, `image/jpeg`, `image/gif`, `image/webp`, and each
decoded image is at most 5 MiB. An invalid type, bad base64 or an oversize image is `400` and nothing is
written to the session. A reply with images is sent as a text block plus one image block per attachment.

### `POST /api/sessions/:id/interrupt`

Interrupts the current turn without ending the session (stream-json control request). `409` if not running.

### `POST /api/sessions/:id/cancel`

Kills the process tree and marks the session `cancelled`. `409` if not running.

### `POST /api/sessions/:id/resume`

Starts a **new** session that resumes the given one's `claudeSessionId` (`--resume`). Body:
`{"prompt"?, "origin"?, "autoClose"?, "permissionMode"?, "maxBudgetUsd"?}` — every field optional;
`prompt` defaults to `"Continue the pipeline from where it stopped."`. `404` if the parent session
is unknown; `400` if `start()` fails for any other reason (e.g. the parent never produced a
`claudeSessionId`).

### `PATCH /api/sessions/:id` — Lot 1a

Updates session metadata. Body may carry any of `title`, `pinned`, `archived`, `unread` — only the
keys present are applied, and unknown keys are ignored without error (permissive, matching the
existing automations `PUT` precedent). `"title": null` or `"title": ""` clears the title. Returns
the full updated `SessionRecord`. `404` on an unknown id. Emits `session.updated`.

```bash
curl -X PATCH http://127.0.0.1:3001/api/sessions/SESSION_ID \
  -H "Content-Type: application/json" \
  -d '{"title": "Nightly regression run", "pinned": true}'
```

### `GET /api/sessions/:id/export.md` — Lot 1a

Renders the session's transcript as standalone Markdown (`sessions.rs::render_markdown`): an H1
(the title when set, else `Session <id>`), a metadata block, then one section per event kind (text,
thinking, tool calls/results, questions/answers, errors, …). Reads from the live event buffer for a
running session and from the persisted JSONL otherwise. Answers `text/markdown; charset=utf-8`.
`404` on an unknown id.

```bash
curl http://127.0.0.1:3001/api/sessions/SESSION_ID/export.md
```

**Treat this response as untrusted — download-only for now.** `render_markdown` interpolates event
content straight into the document with no escaping of any kind, and that content is not the
daemon's own: it is tool output, web pages and files an agent fetched, and raw prompts. Its code
fences are hardcoded to three backticks at every render site, and nothing guards against a backtick
run inside the content, so a transcript that quotes a fenced code block terminates the fence early
and everything after it escapes the code context. Any client that renders this export as HTML — a
preview pane, a Markdown-to-HTML pipeline, a chat message — therefore inherits a stored-injection
surface. Until the renderer escapes content and computes fence lengths (security finding SEC-1 of
the Lot 1a review, major and non-blocking, tracked as its own remediation task), serve this route as
a file download only; a client that must render it should treat the entire document as hostile
input, not merely escape HTML characters and call it done.

### `GET /api/sessions/:id/changes — Lot 2a`

Git/GitHub state for the session's `cwd` (`changes.rs`). `404` on an unknown id.

Query parameters:

| Param | Meaning |
|---|---|
| `file` | Repo-relative path. Returns that one file's full `git diff` as `text/plain` instead of the JSON aggregate. `404` when the path is unknown or rejected (rejected: containing `..`, a NUL byte, or absolute). |
| `pr` | `pr=0` skips the `gh pr view` lookup. |

JSON response shape:

| Field | Type | Meaning |
|---|---|---|
| `sessionId` | string | |
| `cwd` | string | |
| `startSha` | string? | The session's recorded `startSha`. Omitted when the session has none. |
| `baseShaMissing` | bool? | Present and `true` **only** when the recorded `startSha` no longer resolves as a commit in the cwd (rebase, squash, recreated branch). Absent otherwise — never `false`. |
| `branch` | string? | `git rev-parse --abbrev-ref HEAD`. Omitted when unknown. |
| `files[]` | array | `ChangedFile` entries (below). |
| `insertions` | number | Sum over `files[]`. |
| `deletions` | number | Sum over `files[]`. |
| `pr` | object? | Omitted when the current branch has no PR. `{number, title, state, url, isDraft, statusCheckRollup}` — `statusCheckRollup` is the raw value from `gh pr view --json`, unsummarized. |

Each `ChangedFile`:

| Field | Type | Meaning |
|---|---|---|
| `path` | string | |
| `status` | string | The two RAW `git status --porcelain` characters, leading space included and significant (` M`, `M `, `A `, `??`, `D `, `R `, or `"  "` — two spaces — below). |
| `origPath` | string? | Set on a rename. Omitted otherwise. |
| `insertions` | number? | Omitted when unavailable. |
| `deletions` | number? | Omitted when unavailable. |

A file present in the committed diff (against `startSha`) but clean in the worktree carries status
`"  "` (two spaces) — distinguishing it from a worktree change, which always carries one of the
single-character porcelain codes in its first or second position.

**Degraded contract on a stale `startSha`.** A rebase, squash or recreated branch makes the
recorded `startSha` unreachable in the cwd. The endpoint then answers **200**, not 500 and not an
error body, with `baseShaMissing: true` and `files` limited to the worktree status (`git status
--porcelain`, which needs no base sha) — no diff stats against the missing base.

`git` and `gh` are both run with the **session's** `cwd` as their working directory, never the
daemon's — the daemon serves several projects at once. `pr` is the only network call on this route,
made via `gh pr view` in the session's cwd, and it is never invoked on any per-turn path.

### `DELETE /api/sessions/:id`

Cancels the session if still running, then removes its meta file and events JSONL. `404` on an unknown id.

### `GET /api/sessions/:id/run` — Lot 3a

Resolves and returns the vault pipeline run this session is driving (`runs.rs`). `404`
(`"session not found"`) on an unknown session id; `404` (`"no vault run linked to this session"`)
when the session resolves to no run at all.

**The link is an inference, not a foreign key.** The daemon never receives the CLI's
`workflow_run_id` — nothing is persisted on the session record naming its run. Instead, on every
request, the route walks `<vault>/Pipelines/*/*/*/run-state.json` and matches a session to a run by
exact `workspace_root` == session `cwd`, then picks the candidate whose `started_at` is nearest the
session's own `startedAt` (a run started up to 120s before the session, or up to 30 minutes after
it; further than that, no match). The response's `match.by` and `match.deltaSeconds` say which
criterion fired and how far apart the two timestamps were. Because nothing is stored, a run that is
replayed or moved to a different `run-state.json` mtime is simply re-matched on the next request
instead of going silently stale.

Response body:

| Field | Type | Meaning |
|---|---|---|
| `runId` | string | |
| `pipelineId` | string | |
| `repo` | string | |
| `feature` | string | |
| `runDir` | string | Absolute path to the run's directory in the vault. |
| `workspaceRoot` | string | |
| `startedAt` | string | ISO timestamp, from the run's own `run-state.json`. |
| `workflowRunId` | string? | `null` when the run-state file has none. |
| `status` | string | `pending` \| `running` \| `paused` \| `done` \| `failed` (see the normalisation rule below; `paused` when a gate is pending or when `workflow-result.json` reports an `awaiting_gate` stop whose anchor `run-state.json` never recorded, `failed` when the last workflow status was `failed`). |
| `currentGate` | object? | `{stage, anchorPersisted: true}` when `run-state.json` has a pending gate. `{stage, anchorPersisted: false, workflowRunId, resumeCommand}` when the run stopped at a gate but `run-state.json` has neither a pending gate nor a Workflow run id: the gate is read from `workflow-result.json`, and `resumeCommand` is `aidp run record-resume --run <runId> --from-result <runDir>/workflow-result.json`. Otherwise `null`. |
| `stageCount` | number | |
| `doneCount` | number | Stages with wire status `done`. |
| `stages[]` | array | One entry per stage (below). |
| `sessionId` | string | The session id passed in. |
| `match` | object | `{by: "workspace_root+started_at", deltaSeconds}` — how the run was matched (see above). |

Each `stages[]` entry:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | |
| `status` | string | Wire status — see the table below. |
| `startedAt` | string? | |
| `endedAt` | string? | |
| `durationMs` | number? | |
| `tokensUsed` | number? | |
| `stageDir` | string? | |

**Stage-status normalisation** (`runs::stage_status`) — on-disk value to wire value:

| On disk | On the wire |
|---|---|
| `pending` | `pending` |
| `in_progress` | `running` |
| `success` | `done` |
| `cancel` | `cancelled` |
| `skipped` | `skipped` |
| anything else | `pending` |

## Preset endpoints — Lot 1a

A preset (`presets.rs`) is a saved session-start configuration a project/template/target combo can
be replayed from later, by the CLI or the viewer.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Assigned on create, stable across updates. |
| `name` | string | **Required.** |
| `projectId` | string | **Required.** |
| `templateId` | string | **Required.** |
| `target` | string? | Pipeline target. |
| `inputs` | object? | Decided pipeline inputs. |
| `permissionMode` | string? | Falls back to `"default"` at start time when absent. |
| `model` | string? | |
| `effort` | string? | |
| `createdAt` / `updatedAt` | string | ISO timestamps. |

### `GET /api/presets`

List every saved preset.

### `POST /api/presets`

Upsert: a non-empty `id` in the body updates that preset in place and preserves its `createdAt`
(only `updatedAt` changes); an absent or empty `id` creates a new one. `400` with a per-field
message (`"name is required"`, `"projectId is required"`, `"templateId is required"`, checked in
that order) when a required field is missing. Emits `preset.saved`.

```bash
curl -X POST http://127.0.0.1:3001/api/presets \
  -H "Content-Type: application/json" \
  -d '{"name": "Nightly dev", "projectId": "proj-1", "templateId": "dev", "target": "LOC-1"}'
```

### `DELETE /api/presets/:id`

`404` (`"preset not found"`) if the id doesn't exist.

```bash
curl -X DELETE http://127.0.0.1:3001/api/presets/PRESET_ID
```

### `POST /api/presets/:id/start`

Empty body. Starts a session from the preset (pure composition over `POST /api/sessions`) and
returns the started `SessionRecord`. `404` if the preset id is unknown; `400` if the underlying
start fails (e.g. the preset's `projectId` no longer resolves to a registered project).

```bash
curl -X POST http://127.0.0.1:3001/api/presets/PRESET_ID/start
```

Two launch decisions worth knowing:

- The started session's `origin` is `{"kind": "preset", "ref": "<presetId>", "label": "<preset name>"}`.
- Because `autoClose` derives from `origin.kind != "ui"`, a preset-started session is **autonomous**: it closes after its first completed turn with no pending question, exactly like an automation-launched session.

## Usage endpoints — Lot 4a

### `GET /api/usage`

Aggregates token/cost usage across sessions (`usage.rs`, a pure function over the session records —
`SessionManager::search` is untouched by this Lot). Query parameters:

| Param | Meaning |
|---|---|
| `from` | Inclusive lower bound on `startedAt`. ISO-8601, or a bare `YYYY-MM-DD` which is expanded to midnight UTC. |
| `to` | Exclusive upper bound on `startedAt`. Same forms as `from`. |
| `groupBy` | `day` \| `project` \| `template` \| `origin` \| `ticket` \| `model` \| `stage` \| `route`, default `day`. An unknown value is `400`. |

The response is an **object**, not a bare array — it carries running totals alongside the buckets:

```json
{
  "from": "2026-09-01T00:00:00.000Z",
  "to": "2026-09-10T00:00:00.000Z",
  "groupBy": "day",
  "totals": { "key": "total", "inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheCreateTokens": 0, "totalTokens": 0, "estimatedCostUsd": 0.0, "sessions": 0, "turns": 0 },
  "buckets": [ { "key": "2026-09-09", "inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheCreateTokens": 0, "totalTokens": 0, "estimatedCostUsd": 0.0, "sessions": 0, "turns": 0 } ]
}
```

`totals` and each entry of `buckets[]` share one shape (`UsageBucket`): `key`, `inputTokens`,
`outputTokens`, `cacheReadTokens`, `cacheCreateTokens`, `totalTokens`, `estimatedCostUsd`,
`sessions`, `turns`. `totals.key` is always the literal `"total"`.

The bucket `key` depends on `groupBy`:

| `groupBy` | `key` | Notes |
|---|---|---|
| `day` | `YYYY-MM-DD` of the session's `startedAt` | |
| `project` | the session's `projectId` | Also carries a `label` field with the project name. |
| `template` | the session's `templateId` | The literal `"none"` for a free-prompt (`engine: "prompt"`) session. |
| `origin` | the session's `origin.kind` | |
| `ticket` | the session's `origin.ref`, which is the ticket key (`GH-<n>`) for the sessions of a flow | The literal `"unknown"` for a session with no `origin.ref`. |
| `model` | the session's `model` | The literal `"unknown"` when the session pinned none. |
| `stage` | the stage name of the pipeline runs of the tickets that have a session in range | Read from `usage` in each `run-state.json` under `<vault>/Pipelines/*/<ticket>/*/`; a stage bucket counts stage rows, not sessions. |
| `route` | the route of the ticket (`code`, `art`, `map`, `doc`) | The `route` field of the ticket note, else a `route:<name>` label, else guessed from the template id; `"unknown"` otherwise. |

Ordering: `day` buckets sort ascending by key; every other grouping sorts by `totalTokens`
descending, then by key ascending. A session with no `usage` still counts as one session and still
adds its `turns` to the bucket it falls into — it just contributes zero to every token field.
`estimatedCostUsd` is the sum of the per-session `costUsd` the CLI itself reported: an estimate,
informational on a subscription, and there is no price table anywhere in the daemon (see
Decisions).

### `GET /api/tickets/:key/cost`

The token cost of one ticket (`ticket_cost.rs`). The key is the `origin.ref` of the ticket's flow
sessions, which is the ticket key (`GH-<n>`) and not the flow execution key. `404` when no session
and no ticket note carries the key. Never read-only restricted (a `GET`).

```json
{
  "key": "GH-12",
  "route": "code",
  "totals": { "key": "total", "input": 300000, "output": 100000, "cacheRead": 700000, "cacheCreate": 100000, "totalTokens": 1200000, "sessions": 4 },
  "byFlow":  [ { "key": "dev-unity", "input": 0, "output": 0, "cacheRead": 0, "cacheCreate": 0, "totalTokens": 0, "sessions": 0 } ],
  "byStep":  [ { "key": "implement", "totalTokens": 0 } ],
  "byStage": [ { "key": "plan", "totalTokens": 0 } ],
  "byModel": [ { "key": "opus", "totalTokens": 0 } ],
  "windowShare": { "fiveHour": 0.04, "sevenDay": 0.01 },
  "budget":   { "limitTokens": 3000000, "usedTokens": 1200000, "raisedTo": null, "exceeded": false },
  "pixellab": { "generations": 4, "limit": 10, "spent": 2.5, "runs": [] }
}
```

Every `by*` row has the same shape: `key`, `input`, `output`, `cacheRead`, `cacheCreate`,
`totalTokens`, `sessions`, sorted by `totalTokens` descending. `byFlow` groups the sessions by their
pipeline template, `byStep` by the flow step (`origin.label`). `byStage` and the model of `byModel`
come from the stage usage of the ticket's runs in the vault; without run states `byModel` falls back
to the model of each session. `windowShare` is the part of the 5-hour and 7-day windows the ticket
used: the utilization of the window times the ticket's share of the tokens of the sessions started
inside that window (an estimate; `null` when no rate-limit snapshot is known). `budget` is the
per-ticket ceiling (see the Ticket budget section) and `pixellab` the credits the ticket's art stages
recorded (see the Pixellab section); both are omitted when the vault is not configured.

The report footer of the cost contract is built from this report by
`aidp_gateway::ticket_cost::ticket_footer(&TicketCost) -> String`
(`Coût : 1,2 M tokens (in 0,3 M / out 0,1 M / cache 0,8 M), 4 % de la fenêtre 5 h`; thousands
instead of millions below one million tokens; the window clause is left out when `windowShare.fiveHour`
is `null`), and for one run by `aidp_gateway::ticket_cost::run_footer(&[StageRow], Option<f64>)`.
`aidp cost ticket <key> --format footer` prints it.

### `GET /api/tickets/costs[?keys=GH-1,GH-2]`

The dollars and the tokens of every ticket in one call, for a board that shows a cost on each card
(`ticket_cost::ticket_totals`, computed from the sessions in memory in a single pass, no vault
walk). The body is an object keyed by ticket key:

```json
{
  "GH-12": { "costUsd": 1.84, "totals": { "totalTokens": 1200000 } },
  "GH-13": { "costUsd": null, "totals": { "totalTokens": 0 } }
}
```

`costUsd` and `totals.totalTokens` are the numbers of `GET /api/tickets/:key/cost` for the same key
(`costUsd` is `null`, not omitted, while no session of the ticket has a price). Without `keys` the
answer holds every ticket with a session and every ticket note (archived ones included); with `keys`
(comma-separated) it holds only those, and a key no session and no note knows is left out instead
of answering `404`. Never read-only restricted (a `GET`).

### Read latency

The cost, usage and Pixellab routes (`/api/usage`, `/api/tickets/:key/cost`, `/api/pixellab/balance`)
read `vault_index`, an in-memory index of `<vault>/Pipelines/<repo>/<ticket>/<run>/`: per run its
route, its stage rows and its Pixellab usage and balance. The daemon builds it at startup on a
background thread, then refreshes it incrementally on a background thread when a read finds it older
than 15 s (a run is read again only when its `run-state.json` or its directory changed) and on
`run.started`, `run.stage_changed` and `run.ended`. The ticket ceilings (`budget.per_ticket`,
`budget.pixellab.per_ticket`) read the runs of the ticket again before they decide.

The index is persisted in SQLite, `<gateway dir>/memory/vault-index.sqlite` (next to `memory.db`, WAL,
`PRAGMA user_version` checked: another version drops and rebuilds it). One row per run keyed by its
directory (repo, ticket, run id, route, the `run-state.json` size and mtime and the directory mtime
that decide staleness, Pixellab usage, newest balance) and one row per stage row. At startup the
daemon loads it at once, so the routes answer from it without walking the vault, then runs the
incremental refresh in the background: it stats each run, reads again only the changed ones, and
writes the changed runs and deletes the vanished ones in one transaction. Reads stay on the
in-memory index; SQLite is only its persistence. The run files remain the source of truth (the
`aidp` CLI writes them from other processes) and the database is a cache: if it cannot be opened it
is deleted and recreated, and if that fails the daemon logs a warning and keeps the in-memory
behaviour. Response shapes do not change.

`GET /api/worktrees`, `GET /api/tickets/:key/pr`, `GET /api/catalog/templates|flows[/:id]`,
`GET /api/memory` and `GET /api/unity/status` are served stale-while-revalidate (`swr_cache.rs`): the
last answer comes back at once and, once older than its TTL (worktrees 20 s, ticket pull request
30 s, catalogue 30 s, memory 10 s, Unity processes 5 s), a background thread computes the next one.
Only the first read after startup waits for `git`, `gh` or the disk. A write (`POST /api/worktrees/*`,
a catalogue or memory write, a memory event, a PR webhook) drops the cached answers, and
`?refresh=1` on `GET /api/tickets/:key/pr` reads again now. `GET /api/prs` keeps its inbox cache
(120 s TTL, persisted snapshot served stale); the GitHub quota handling is unchanged, no route asks
`gh` more often than before.

### `GET /api/cost/anomalies`

Flags the tickets whose token total is an outlier among all tickets with a session (modified z-score
over median and MAD, the same threshold as `aidp cost anomaly`). Takes `from` and `to` like
`/api/usage`.

```json
{ "status": "ok", "n": 5, "median": 105.0, "mad": 5.0, "anyFlagged": true,
  "points": [ { "key": "GH-5", "totalTokens": 9000, "modifiedZ": 801.2, "flagged": true } ] }
```

`status` is `insufficient_data` (with an empty `points`) below three tickets. A zero spread flags
nothing.

### `GET /api/usage/rate-limit`

`?limit=` (default 200). Returns a **bare array** of rate-limit snapshots, newest first, each the
same shape as the `rateLimit` object of `GET /api/gateway/status`:

```json
[
  { "status": "ok", "isUsingOverage": false, "fiveHour": { "utilization": 0.42, "resetsAt": 1757500000 }, "sevenDay": { "utilization": 0.18, "resetsAt": 1757900000 }, "observedAt": 1757500000000 }
]
```

`resetsAt` (inside `fiveHour`/`sevenDay`) is UNIX **seconds**; `observedAt` is UNIX
**milliseconds**. A line is appended to `rate-limit.jsonl` only when a window's `utilization` or the
top-level `status` actually **changes** since the previous snapshot — this is a change log, not a
periodic sample, so the interval between two entries carries no timing information of its own.

## Token budget endpoints and gates (lot P5)

### `GET /api/pixellab/balance`

The last Pixellab balance an art stage recorded in its `pixellab-usage.json` (`balanceAfter`, else
`balanceBefore`; `pixellab_usage.rs`). The daemon never calls Pixellab: no secret lives on its side.
A `GET`, allowed under `AIDP_READ_ONLY`.

```json
{ "balance": 41.5, "raw": { "credits": 41.5 }, "recordedAt": "2026-11-06T10:00:00Z",
  "runId": "run-1", "minReserve": 5.0, "belowReserve": false }
```

`balance`, `raw`, `recordedAt` and `runId` are `null` until a stage recorded one. The scan of the
vault is remembered 20 seconds. `balance` is the first numeric value found under `credits`, `credit`,
`balance`, `remaining`, `usd`, `amount`, `total` (up to three object levels deep) in the raw
`get_balance` answer.

### Ticket ceiling (`budget.per_ticket`)

`<vault>/TODO/config.yaml`:

```yaml
budget:
  per_ticket:          # token ceiling of one ticket, by route (a number, `3M`, `1.5M`, `500k`; 0 = none)
    code: 3M
    art: 1.5M
    map: 2M
    doc: 0.5M
    default: 2M        # a ticket whose route is unknown
  pixellab:
    min_reserve: 0     # no `art` launch while the last known balance is below this value (0 = off)
    per_ticket: 10     # Pixellab generations one ticket may use (0 = none)
```

Before the flow engine starts a pipeline step, it totals the tokens of the ticket's sessions
(`origin.ref`, input + output + cache) and its Pixellab generations (`ticket_ceiling.rs`). Past a
ceiling the step goes `waiting_human` with reason `ticket-budget` and an inbox entry "Plafond atteint
pour GH-12 : tokens 3100000 / 3000000. Continuer ?" with the choices `approve` and `reject`; the bus
publishes `ticket.budget_exceeded` (`key`, `ticket`, `step`, `what`, `used`, `limit`) next to the usual
`flow.step_changed` and `flow.waiting_you`. `approve` raises the ceiling of that ticket only (what it
already used plus one more allowance, kept in `<vault>/TODO/.state/ticket-budget.json`) and starts the
step. `reject` escalates the run: the ticket becomes `blocked` with `blocked_reason: budget`. Manual
requests are never held. The ceilings and what the ticket used are in `budget` and `pixellab` of
`GET /api/tickets/:key/cost`.

### Dispatcher launch gates

`dispatcher.night` and `dispatcher.stop_at_7d_percent` plan the launches around the rate-limit
windows; `budget.pixellab.min_reserve` guards the `art` route. A held ticket stays queued and shows in
`skipped` of the plan as `budget:<reason>`; a manual start is never held.

```yaml
dispatcher:
  night:
    enabled: false
    start: "22:00"            # local time; end earlier than start wraps past midnight
    end: "07:00"
    heavy_after_reset: true   # heavy runs start only just after a 5 h window reset...
    heavy_window_minutes: 30  # ...within this many minutes
  stop_at_7d_percent: 85      # no launch once the 7-day window reaches 85 %
```

A run is heavy when one of its stages resolves to `opus` (template, then the `models:` policy). With
`night.enabled`, a heavy run waits for the night window and the minutes after a 5 h reset;
other runs launch as usual. `stop_at_7d_percent` turns an open budget verdict into a wait on the
7-day window until it resets. The `art` route waits while the last known Pixellab balance is under
`budget.pixellab.min_reserve`.

### Model policy (`models:`)

```yaml
models:
  code: { report: haiku, review: sonnet }
  art:  { review: sonnet }
```

`<route>: { <stageId>: haiku|sonnet|opus }` in `config.yaml`, applied when a pipeline is compiled
(`model_policy.rs`): `aidp run start --stage-model STAGE=TIER` first, then the policy of the ticket's
route, then the template's `model:`. The stage ids are `prepare`, `plan`, `implement`, `repro`,
`review`, `verdict`, `fix`, `test`, `report`, `brief`, `generate`, `animate`, `import`, `graph`,
`rooms`, `write`; the routes `code`, `art`, `map`, `doc`.

## Queue endpoints — Lot 4a

The daemon runs at most one session per registered project, keyed on `projectId`; a launch that
arrives while that project is busy becomes a queue entry instead of a session, and the
highest-priority entry starts automatically when the running session ends.

A queue entry:

```json
{ "id": "q_1", "projectId": "proj-1", "projectName": "pixelhood", "templateId": "dev", "target": "LOC-1", "engine": "v7", "priority": 0, "status": "queued", "queuedAt": "2026-09-10T12:00:00.000Z", "origin": { "kind": "api" } }
```

`templateId` and `target` are omitted when absent, same as on a `SessionRecord`.

### `POST /api/sessions`

Unchanged request shape, but the response now returns **either** a session record **or** a queue
entry — always `200` in both cases, distinguishable by `status == "queued"`. Both shapes carry
`id`, `projectId`, `projectName`, `templateId`, `target`, which is what existing clients already
read, so a client that ignores `status` keeps working for the non-queued path unchanged. The
request body accepts a new optional `priority` (integer, default `0`); it is carried on the queue
entry when the launch is queued, and is never stored on the session record when it is not.

### `GET /api/queue`

Optional `?projectId=`. Bare array of queue entries in dispatch order: `priority` descending, then
`queuedAt` ascending.

### `POST /api/queue/:id/promote`

Makes the entry the next one out for its project by raising its priority above every sibling entry
for that project. Returns the updated entry. `404` (`"queue entry not found"`) on an unknown id.

### `POST /api/queue/:id/cancel`

Drops the entry. Returns `{"ok": true}`. Same `404` (`"queue entry not found"`) on an unknown id.

### `GET /api/gateway/status`

`capabilities.safeHumanRejection` certifies that rejection stops progression, Retry asks again and Skip cannot bypass the rejection.
`capabilities.externalActions` certifies the [external executor](#external-executor): `GET /api/flows/:key/external` and `POST /api/flows/:key/external/:jobId/result`.
`capabilities.externalAgents` certifies the [agent jobs](#agent-jobs-run-by-a-host): `policy.externalAgents`, the `agent` items of the external listing, and the claim, heartbeat and result routes for them.
The first two are `true` from runtime `7.5.11`, `externalActions` from runtime `7.5.12`, `externalAgents` from runtime `7.5.16`. Hosts must check the actual pinned gateway; missing flags are unsupported.


Gains `sessions.queued` — the total queue depth across every project — alongside the existing
`sessions.running` and `sessions.total`.

## Tickets endpoints — Lot 9a

| Route | Role |
|---|---|
| `GET /api/tickets?status=&service=&source=` | Bare array of public ticket frontmatter (no body), in dequeue order. |
| `GET /api/tickets/:key` | `{ ticket, body }`; `ticket` carries `flowSummary` as in the list. |
| `GET /api/tickets?status=&service=&source=` | Bare array of public ticket frontmatter (no body), in dequeue order. Each element also carries `githubNumber` and `url` (null for a local ticket); `labels` (GitHub labels), `milestone` (`{title, number?, due_on?}`), `route` (`code`, `art`, `map`, `doc`) and `flow` are frontmatter fields. `phase` (`dev`, `to-test`, `testing`) and `branch` come from `autopilot`. The tickets in phase `to-test` appear in the order the test lane takes them (manual promotion first, then the oldest), each in a position a `to-test` ticket already had. |
| `GET /api/tickets/:key` | `{ ticket, body }`; `ticket` has the same fields as a list element. |
| `POST /api/tickets/sync?source=github` | Mirrors the open issues of `github.repos` now: the work of `aidp ticket sync`. Optional `repo=owner/name`, `limit=N`, `dry_run=1`. `200` the report `{source, repos, created, updated, cancelled, unrouted, triaged, unchanged, labels_pushed, errors, labels_seen, dry_run, duration_ms}`; `400` an unknown `source` or no configured repository; `503` no vault. The daemon also runs it every `github.sync_minutes` and writes the state labels back after each dispatcher tick. |
| `POST /api/tickets` | Create a local ticket — the body of `aidp ticket new`. |
| `POST /api/tickets/:key/start` | `{ force? }` — manual start: sets `manual_priority`, queues the ticket, ticks. |
| `POST /api/tickets/:key/cancel` / `/route` | The CLI verbs of the same names, over HTTP. |
| `POST /api/tickets/:key/status` | `{ status, phase?, reason?, force? }` — a manual transition, journaled with actor `human`. `phase` (`dev`, `to-test`, `testing`) also sets the ticket phase and, for a GitHub mirror, the label (`aidp:to-test`, `aidp:testing`): "send to test". A `status` equal to the current one with a `phase` only sets the phase. `400` an unknown `phase`. `200 {key, from, to, path, phase?}`. `409` when the transition is not allowed from the current status (the message lists the allowed targets), or when it leaves `in-progress` without `force`. |
| `POST /api/tickets/:key/force` | `{ target, reason? }` — a forced move to any board column, whatever the current status: see [tickets.md](tickets.md#forced-move). `target` is `ready`, `review`, `to-test`, `testing`, `merge`, `blocked` or `done`. Aborts the active flow, writes status, phase and state label, then for `review`, `testing` and `merge` restarts the flow of the route that declares that entry. `200 {key, target, from, to, phase?, aborted, flow?, entry?}`. `400` an unknown target, `404` an unknown ticket, `409` no flow of the route declares the entry, `503` no flow engine. |
| `POST /api/tickets/:key/adopt` | `{ pr }` (URL or number) — attaches an existing PR to a `todo` or `queued` ticket and moves it to `in-review` without a session. `200` with the updated ticket, `404` unknown ticket, `409` ticket in another status. |
| `POST /api/tickets/:key/flow` | `{ flow?, settings? }` — starts the default flow `perso-delivery` (or the named flow) for the ticket, through the same start as `POST /api/flows`. It does not read the dispatcher pause state. `201` with the run, `404` unknown ticket, `409` the ticket already has a flow run that is not finished (the body carries it under `existing`), `503` flows unavailable. |
| `GET /api/tickets/:key/flow` | `200` the run of the ticket, as `GET /api/flows/:key`. `404` no run. |

Each ticket of `GET /api/tickets` (and `ticket` of the detail) carries `flowSummary`: the most recent
non-terminal flow whose ticket key is the ticket's, or `null` (also when flows are unavailable).
Shape: `{ flowKey, status, phase, stepId, iteration, startedAt }` where `status` is the flow status
(`running`, `paused`, `pausing`, `waiting_human`, `waiting_external`), `stepId` the current step
(`null` when none), `phase` and `iteration` those of that step (`null` and `0` without one), and
`startedAt` the flow start (`null` when unknown).

`POST /api/tickets/:key/start` also accepts `service`, `template` and `track`; `aidp ticket start --service|--template|--track` sends them and the daemon applies them before it queues the ticket. `aidp gateway serve --bind` takes an IP address (`IpAddr`), not a host name, and gateway commands default to `--format json-pretty`.

#### Manual launch

`POST /api/tickets/:key/start` body `{ service?, flow?, template?, track?, force? }`:

- `flow`: id of a flow of `GET /api/catalog/flows`; it replaces the flow of the routing for this ticket (persisted on the note). An unknown id answers `400 { error: "unknown flow `x`", code }` and nothing changes.
- `template`: pipeline only. Without `flow`, the ticket's routed flow is dropped and the pipeline runs alone.
- `track`: `quick`, `standard` or `deep`; `service`: an existing vault service.
- `force: true`: a manual launch. A `blocked` ticket (`not-ready`, `needs-refine`, `needs-routing`, ...) is queued anyway, and the dispatcher pause and a budget wait are waived for this one ticket. The slot ceiling and the one session per project rule still apply (`409 start refused: no-slot` / `project-busy`). Without `force`, a paused dispatcher answers `423`.
- Success: `200` with the dispatcher tick report `{ launched: [key], skipped, slotsUsed, slotsMax, ... }` plus, when they exist, `flowKey` (the flow run driving the ticket, equal to the ticket key) and `runId` (the native run of a pipeline launch). The ticket becomes `in-progress` and its GitHub state label follows through the usual sync.

`POST /api/runs/start` body `{ project?, template, inputs?, track? }` starts a native run of a pipeline with no ticket, as `POST /api/runs` does, and answers `202 { runId }` (no `sessionId`: a native run owns its stage sessions). `project` is an id or a name; when omitted the only registered project is used (`400` when several are registered, `404` when it is unknown). `inputs` is an object checked against the template's declared inputs (defaults applied, required and allowed values enforced); `track` is stored as the `track` input. `400 { error, code }` for a missing or unknown `template`, a fragment, `inputs` that is not an object, an invalid `track`, or a missing required input. The usual `run.started` event is published on `/api/events`.

A manual transition (`/status`, `/cancel`) publishes `ticket.status` with `{key, status, from, to,
reason, actor}`, like the dispatcher's own transitions; a ticket moved to `cancelled` then has
its worktree saved to a ref and removed.

Every mutating route above passes the same Lot 5a gate
as the rest of the API: it needs a valid `AIDP_UI_TOKEN` from an off-loopback caller and answers
`403` under `AIDP_READ_ONLY` (see [Auth and read-only mode](#auth-and-read-only-mode--lot-5a)).
`GET /api/tickets` and every other list route in this Lot return a **bare array**, matching the
existing convention (see [Conventions](#conventions)); errors go through `ApiError` at the matching
status code; and no route in this Lot returns `201` — a successful create answers `200`, like every
other route in this API.

## Dispatcher endpoints — Lot 9a

| Route | Role |
|---|---|
| `GET /api/dispatcher` | `.state/dispatcher.json` — as of **Lot 10a** its `budget` object carries the persisted verdict from the last tick; the verdict lives in the state file itself now, rather than being injected by the handler. |
| `GET /api/dispatcher/budget` | **Lot 10a.** The token-budget verdict explainer: both rolling windows (utilization and reset instant), the thresholds in force, the reserve per run (and whether it is learned or the static config default), the count of dispatcher sessions not yet reflected in the last usage snapshot, the persisted verdict, and the per-template learned-reserve stats. A `GET`, so it is allowed under `AIDP_READ_ONLY` — unlike `pause`/`resume`/`tick`. Served by `Dispatcher::budget_json` (`dispatcher.rs`). |
| `GET /api/dispatcher/plan` | Dry-run plan; launches nothing. |
| `POST /api/dispatcher/pause` / `/resume` / `/tick` | Control. `pause` and `resume` write `dispatcher.paused` in `TODO/config.yaml`; see below. |

Same auth and read-only rules as above: the three `GET` routes are unaffected by `AIDP_READ_ONLY`;
`POST /api/dispatcher/pause`, `/resume` and `/tick` are mutating and therefore `403` in read-only
mode; all six routes need a valid `AIDP_UI_TOKEN` from an off-loopback caller. Errors are
`{"error": ...}`; no route returns `201`.

**`POST /api/dispatcher/pause` sets the paused flag.** The body is `{"paused": boolean, "actor"?: string}`.
`actor` defaults to `"api"`; the viewer sends `"viewer-rollback"`.

| Status | When |
|---|---|
| `200` | Body `{"paused": boolean}`: the effective state after the write. A call that changes nothing answers `200` without writing. |
| `400` | The body is absent, the JSON is invalid, or `paused` is not a boolean. Nothing changes. |
| `500` | `TODO/config.yaml` cannot be written. |

**`POST /api/dispatcher/resume`** takes no body and answers the same `200` with `{"paused": false}`.

`GET /api/dispatcher` exposes a root boolean `paused`. It is fresh immediately after a write.

**`POST /api/dispatcher/tick` runs the local tick.** The route calls `Dispatcher::tick_local("api")`:
it plans and launches, but skips the network housekeeping (the `gh pr view` merge probe, at most
every 10 minutes, and the native-run probe). Only the timer-driven tick runs that housekeeping, at its configured rhythm, so calling this
route never delays or cancels it.

**Busy tick.** Only one tick runs at a time. Any tick — timer, bus-triggered, `POST
/api/dispatcher/tick` or a manual ticket start — that arrives while another tick is in flight
returns at once, without waiting:

```json
{ "mode": "busy", "lastReason": "api", "launched": [], "skipped": [] }
```

A busy tick touches neither the store nor `.state/dispatcher.json` and emits no `dispatcher.tick`
event. A bus-triggered tick dropped this way is picked up by the next timer tick.

`GET /api/dispatcher` returns `.state/dispatcher.json` verbatim — as of Lot 10a that includes a
persisted `budget` object (`aidp_tickets::dispatcher_state::BudgetState`), empty (`"verdict": ""`)
until the dispatcher's first budget-aware tick:

```json
{ "mode": "normal", "paused": false, "waitUntil": null, "lastTickAt": "2026-09-14T09:08:09Z", "lastReason": "timer",
  "slots": { "used": 2, "max": 3, "testUsed": 1, "testMax": 1 },
  "running": [ { "key": "LOC-1", "sessionId": "s-1", "project": "pixelhood", "startedAt": "2026-09-14T09:00:00Z" } ],
  "lastPlan": { "launched": ["LOC-1"], "skipped": [ { "key": "LOC-2", "reason": "project-busy" } ] },
  "budget": { "verdict": "allow", "maxLaunches": 3, "why": "", "utilization5h": 0.5, "utilization7d": 0.31,
    "limit5h": 0.8, "limit7d": 0.9, "reserve5h": 0.1, "reserve7d": 0.02, "unreflected": 0,
    "resetsAt5h": "2026-09-14T14:35:00.000Z", "waitWindow": null, "observedAt": "2026-09-14T09:07:40.000Z" } }
```

`lastPlan.skipped[].reason` is one of the following wire strings: `project-busy`, `no-slot`,
`needs-routing`, `no-template`, `template-without-demand-mode`, `budget:<detail>`, `paused`,
`start-failed`, `waiting-on:<KEY>[,<KEY>...]` (unfinished blockers listed by a GitHub mirror, in its body or in GitHub's dependencies;
the ticket stays `todo` and a manual start bypasses it). Each entry also carries an optional `detail` string with the reason's specifics —
populated for `start-failed` with the launch error, and left empty for reasons the wire string
already states in full.

`GET /api/dispatcher/budget` wraps the same persisted verdict plus live context that `dispatcher.json`
does not carry — the live rate-limit snapshot, which reserve is actually in force, and the learned
per-template history:

```json
{
  "verdict": { "verdict": "wait", "maxLaunches": 0, "why": "fenêtre 5 h à 87% ≥ 80%", "utilization5h": 0.87,
    "utilization7d": 0.31, "limit5h": 0.8, "limit7d": 0.9, "reserve5h": 0.1, "reserve7d": 0.02,
    "unreflected": 1, "resetsAt5h": "2026-09-14T14:35:00.000Z", "waitWindow": "5h",
    "observedAt": "2026-09-14T09:07:40.000Z" },
  "live": { "status": "ok", "isUsingOverage": false,
    "fiveHour": { "utilization": 0.87, "resetsAt": "2026-09-14T14:35:00.000Z" },
    "sevenDay": { "utilization": 0.31, "resetsAt": "2026-09-18T09:07:40.000Z" },
    "observedAt": "2026-09-14T09:07:40.000Z" },
  "reserves": { "fiveHour": 0.1, "sevenDay": 0.02, "learned": true },
  "unreflected": 1,
  "stats": { "templates": { "dev-unity": { "samples": 7, "p50_delta_5h": 0.06, "p75_delta_5h": 0.09,
    "p75_delta_7d": 0.015, "deltas_5h": [0.05, 0.08], "deltas_7d": [0.01, 0.02] } },
    "updated_at": "2026-09-14T08:50:00.000Z" }
}
```

`verdict` is the same `BudgetState` object `GET /api/dispatcher`'s `budget` key carries — it is read
from the dispatcher's in-memory state, never recomputed, so the two routes never disagree. `live` is
`null` when the gateway has no rate-limit snapshot yet. `reserves.learned` answers whether the config
allows learned reserves at all (`budget.learn` in `config.yaml`), not which specific per-template
value fed the last tick's decision — that provenance is not itself persisted. `stats` is
`aidp_tickets::budget::BudgetStats` serialized as-is: snake_case keys, the same shape written to
`.state/budget-stats.json`, unlike every other key on this page.

## Health endpoints — Lot 5a

### `GET /api/health`

One-shot rollup of daemon health for a diagnostics/status page. Not exempt from the auth gate — a
gated health endpoint cannot itself diagnose a token misconfiguration from off-loopback, but the
operator checking it is expected to be on loopback.

```json
{
  "ok": true,
  "doctor": { "ok": false, "checks": 20, "failing": ["templates"], "error": null },
  "hooks": { "webhooksEnabled": true, "hooksInstalled": true },
  "aidpBinary": {
    "ok": false,
    "path": "D:/AIDP/bin/aidp.exe",
    "modifiedAt": "<rfc3339>",
    "runningPath": "D:/AIDP/target/release/aidp.exe",
    "runningModifiedAt": "<rfc3339>",
    "issues": ["mismatch", "older_than_running"]
  },
  "providers": [{ "id": "claude", "program": "claude", "found": true, "version": "<string or null>" }],
  "claudeLogin": { "state": "ok|expired|unknown", "source": "probe|turn_end|none", "detail": "…", "checkedAt": "<iso>" },
  "stateDir": { "path": "…", "bytes": 123456, "files": 42 },
  "sessions": { "running": 1, "orphaned": 3 },
  "auth": { "uiTokenConfigured": true, "readOnly": false },
  "threads": {
    "residentLoops": { "automations.bus": 1, "automations.scheduler": 1, "dispatcher.bus": 1, "dispatcher.timer": 1, "runs.watcher": 1 },
    "residentTotal": 5,
    "loopPanics": 0,
    "requestsInFlight": 1,
    "requestsCeiling": 256
  }
}
```

| Field | Type | Meaning |
|---|---|---|
| `ok` | bool | |
| `doctor` | object | `{ok, checks, failing[], error}` — parsed from shelling out to `<current_exe> doctor --format json-compact`, whose stdout is still parsed even though `aidp doctor` exits `6` (negative verdict, see [exit codes](exit-codes.md)) whenever a check fails. `fn doctor` is private to the bin crate, so an in-process call is impossible. |
| `aidpBinary` | object | `{ok, path, modifiedAt, runningPath, runningModifiedAt, issues[]}`: the first `aidp` on `PATH` compared with the running daemon binary. `modifiedAt` and `runningModifiedAt` are RFC 3339 strings or `null`; `path` is `null` when no `aidp` is on `PATH`. `issues` holds the slugs `absent`, `target_build`, `mismatch`, `older_than_running` and `older_than_sources`, the same ones the `aidp_on_path` doctor check reports. Distinct from the `aidpBin` string of the status payload, which is unchanged. A failing `aidp_on_path` already makes `doctor.ok` false, so the top-level `ok` needs no extra term. |
| `providers` | array | One row per configured agent provider: `{id, program, found, version}`. `found` is false when the program is not on `PATH`; `version` is the `--version` output or `null`. See [providers.md](providers.md). |
| `hooks.webhooksEnabled` | bool | `AIDP_HOOK_TOKEN` is set. |
| `hooks.hooksInstalled` | bool | Read from doctor's own `hooks_installed` check. |
| `claudeLogin` | object | Resolved by probing `claude auth status` and, failing that, by scanning the 20 most recent sessions for the newest `turn_end` event with `isError: true` whose text looks like an authentication failure. An inconclusive probe yields `"unknown"`, never `"expired"`. |
| `stateDir.bytes` / `stateDir.files` | number | A recursive walk of the state directory, not a sum of known filenames. |
| `sessions.running` / `sessions.orphaned` | number | Live count and count of `running`-at-boot records not yet cleaned up. |
| `auth.uiTokenConfigured` | bool | Whether `AIDP_UI_TOKEN` is set. |
| `auth.readOnly` | bool | Whether `AIDP_READ_ONLY` is on. |
| `threads.residentLoops` | object | Live instance count per resident loop name (see [Concurrency and resource bounds — LOC-29](#concurrency-and-resource-bounds--loc-29)). A loop that has exited is absent. |
| `threads.residentTotal` | number | Sum of `residentLoops`. |
| `threads.loopPanics` | number | Resident-loop iterations that panicked since the process started. |
| `threads.requestsInFlight` | number | HTTP requests being handled, this one and open SSE streams included. |
| `threads.requestsCeiling` | number | The in-flight ceiling in force (`256`, or `AIDP_MAX_INFLIGHT_REQUESTS`). |

**Probe cost.** The `doctor` summary, the `aidpBinary` block and the `claude auth status` probe result
are each cached for 30 s, so repeated calls within that window re-run neither process. On a cold cache
the probes run concurrently (`aidpBinary` adds no latency), so a call costs at most one probe timeout (5 s), not two. A probe still running at its
timeout has its child process killed and reaped, and the request goes on without it. Each cache is
single-flight: concurrent misses trigger exactly one probe, and any other caller either gets the
stale value right away (when one exists) or waits for that one result, rather than each spawning its
own `aidp doctor` / `claude auth status` child. The
`turn_end` fallback of `claudeLogin` is not cached and reads the sessions on every call.

### `POST /api/health/cleanup`

Purges every session left `orphaned` (a `running` record found at boot) and returns
`{"ok": true, "count": n, "purged": [ids]}`, emitting a `sessions.purged` bus event. It is a
mutating verb, so read-only mode refuses it.

```bash
curl -X POST http://127.0.0.1:3001/api/health/cleanup
```

## PR workspace holder

A run or flow action working on a pull request branch records itself in
`<vault>/Pipelines/<repo>/PR-<n>/workspace-holder.json`, so other writers can tell the branch is busy.

| Field | Meaning |
|---|---|
| `stage` | The workflow stage or action name holding the branch. |
| `run_id` | The native run id, absent for a flow action. |
| `session_id` | The session holding the branch, absent when a run holds it. |
| `acquired_at` | ISO 8601 UTC instant the holder took the branch. |

**Which input names the PR.** The first of `pr_number`, `pr`, `target` that parses: a JSON number, a
numeric string, `#123`, or a GitHub URL ending in `/pull/123`. A value that does not parse (a ticket
key in `target`) is skipped. A run or action with none of them records nothing.

**Lifecycle.** Written when a native run starts or a flow action takes the workspace lock, rewritten
when the run changes stage, and removed when the run ends (done, failed, paused or cancelled) or
the action releases the lock.

**Effect on the fix route.** `POST /api/prs/fix` answers `409` while a live holder exists for the
PR. A holder whose run is dead is purged and the fix proceeds.

## Worktrees — Lot 17a

Each ticket the dispatcher launches, or that is started by hand, works in its own git worktree
instead of the project checkout. Your checkout stays yours, and several tickets of one project run
in parallel.

**Layout.** `<vault>/Pipelines/<repo>/<KEY>/.worktree`, on branch `feat/<KEY>`. `<repo>` is the
project's `repo_segment`. The path comes from `worktrees::worktree_path`, the directory name from
`WORKTREE_DIR` (`.worktree`). A run started from the worktree files its artefacts under the right
`Pipelines/<repo>/`.

**Creation order** (`worktrees::ensure`):

1. `git fetch origin`.
2. A worktree already registered at that path is reused (`reused: true`).
3. An existing local branch `feat/<KEY>` is attached with `git worktree add <path> feat/<KEY>`.
4. Otherwise `git worktree add -b feat/<KEY> <path> origin/<default>`. The default branch is the
   target of `refs/remotes/origin/HEAD`, then `master`, then `main`.

**Refusal.** When `feat/<KEY>` is already checked out in another worktree, for example your
checkout, nothing is created. The start fails with the slug `branch-checked-out-elsewhere`, and
the dispatcher skips the ticket with that reason.

**Copied local files.** After a creation (not a reuse), untracked files matching
`dispatcher.worktree_copy` are copied from the project checkout, never overwriting. Default:
`.claude/settings.local.json` and `.aidp/*.json`. A wildcard is honoured in the last path segment
only. Set the list in `<vault>/TODO/config.yaml` (see [tickets](tickets.md#worktrees-and-the-cap)).

**Sessions.**

- `StartRequest.worktree` (optional string, a ticket key): `start()` prepares that ticket's
  worktree and runs the session in it. It is ignored for `ui` launches. With no vault configured,
  the session starts in the checkout with the warning `worktree-unavailable: no vault`.
- `SessionRecord.workspace` is `{kind: "project" | "worktree", path, branch}`. `branch` is `null`
  for a checkout. Sessions saved before this lot have no `workspace`; they count as the checkout at
  their `cwd`.
- The dispatcher sets `worktree` on every launch it makes.

**Admission.** One live session per workspace, not per project. A second session aimed at the same
worktree waits in the queue. A session in a worktree blocks neither the checkout nor the other
worktrees of the project. `ui` sessions stay on the checkout and keep the old rule there. A queue
entry saved without a workspace key waits on its project id.

**Events.**

| Event | Payload | Notes |
|---|---|---|
| `worktree.created` | `{key, project, path, branch, reused}` | A worktree is ready for a session; `reused` is true when it already existed. |
| `worktree.removed` | `{key, salvagedRef?}` | Deleted after a merge, a cancel or a sweep. `salvagedRef` is set when unsaved work was pinned first. |
| `worktree.remove_failed` | `{key, path, error, attempts}` | Deletion still failed after 3 attempts. The next sweep retries. |

**Routes.**

- `GET /api/worktrees[?project=<id>]` returns rows `{key, project, path, branch, exists, modified,
  prunable}`. `modified` counts changed or untracked, non-ignored paths. `prunable` is true when
  the owning ticket is `done`, `cancelled` or unknown.
- `GET /api/tickets/:key/worktree` returns `{path, branch, exists, modified}`, and `404` for an
  unknown ticket.
- Both answer `503` when no vault resolves.

**Cleanup.** A merged PR (the `probe_reviews` probe or a `pr.merged` event) or a cancelled ticket
saves any unsaved work, then removes the worktree and its branch (see
[tickets](tickets.md#worktree-lifecycle)). The daemon also sweeps every `.worktree` under
`<vault>/Pipelines/*/*/` at startup and on `POST /api/health/cleanup`, removing those whose ticket
is `done`, `cancelled` or unknown. The cleanup response gains `worktrees: {removed[], failed[]}`.
A failed removal appears in `GET /api/health` under `worktreesRemoveFailed`, as rows
`{key, path, error, attempts}`.

CLI: `aidp gateway worktrees list [--project <id>]` and `aidp gateway worktrees prune [--dry-run]`.
`prune` posts to `/api/health/cleanup`; `--dry-run` only lists the prunable rows.

## Launch chaining — Lot 6a

`contextFrom` is an optional field on `POST /api/sessions`'s `StartRequest` body and on an
automation's `Action::Launch`. Its value is a **gateway session id** — not a vault run id.

On start, the gateway resolves that session id to its vault run using the same
`workspace_root` + `started_at` match `GET /api/sessions/:id/run` already uses, then appends a
`## Contexte du run précédent` block to the new session's prompt: each stage's `summary` from
`run-state.json`, plus the artefacts `study.md`, `plan.md`, `context.md` and `exploration.md`
wherever the stage directories hold them. The block is budgeted: 8 000 characters per document,
24 000 characters for the whole block, tail-truncated with `[... tronqué]` when a limit is hit.

**Resolution happens at start time, not enqueue time.** A launch that waits in the per-project
queue (see [Queue endpoints — Lot 4a](#queue-endpoints--lot-4a)) reads the parent run's artefacts
fresh when it is finally dispatched, not when it was queued — so a slow-moving queue never chains
in stale context.

**Graceful degradation.** An unknown session id, an unresolvable run, a run still parked on a
`pending_gate`, or an empty resulting block all mean the new session simply launches with no
context block. None of these is an error.

**Anti-loop, loud.** The gateway walks the parent session's `contextFrom` links outward to
reconstruct its persisted ancestry, then refuses the launch if that ancestry contains the same
session id twice, or if the resulting depth would exceed `AIDP_MAX_CHAIN_DEPTH` (default `3`): the
start call fails rather than launching unchained. In practice the first guard catches a corrupted
or manually edited chain of session records that loops back on itself, since the new session's own
id is freshly minted and cannot already appear in that ancestry. This is the one case in this
section that is *not* graceful degradation — a would-be loop or an over-deep chain is a
configuration mistake worth surfacing, not silently dropping the context and proceeding.

`AIDP_MAX_CHAIN_DEPTH` is read from the environment inside this check, once per launch — changing
it takes effect on the next chained start and never requires restarting the daemon.

**Inherited budget.** When `contextFrom` is set and the request gives no `maxBudgetUsd`, the new
session inherits the parent session's cap. An explicit `maxBudgetUsd` on the request always wins
over inheritance. Inheriting only ever copies the **cap** — it never decrements a shared budget
envelope by the parent's `costUsd`, since `costUsd` is the CLI's own estimate and is informational
only on a subscription.

**Session record.** `contextFrom` and `chainDepth` are recorded on the started session — see
[The session record](#the-session-record) above. Both are absent on every `SessionRecord` written
before Lot 6a.

**`Action::Launch` only.** `contextFrom` is offered on an automation's `Action::Launch`, not on
`Action::Prompt` — a prompt action replies into an already-running session, so there is no new
session to inject a context block into.

**CLI.** `aidp gateway start --context-from <sessionId>` sets `contextFrom` on the same
`StartRequest` body described above. The CLI does no resolution of its own — it only forwards the
flag's value as-is, and the gateway resolves and refuses exactly as documented here.

## Flows — Lot 29a

The ticket workflow engine; see [flows.md](flows.md). One flow run per ticket `KEY`, stored under `<vault>/Pipelines/<repo>/<KEY>/flow/`. Errors use the [error body](#error-responses).

| Route | Body | Answer |
|---|---|---|
| `GET /api/flows?status=&service=` | none | `200` list of run summaries. |
| `GET /api/flows/:key` | none | `200` the run: frozen definition, steps with status, outputs, iterations, timestamps, phase, and the current step, plus `repo`, the open `inbox` entries, `sessions` (effect id `<step>:<iteration>:start_pipeline` → native run id or session id, live while the step runs) and `pr` (`{number, url, repository}` from the newest `pr.json` of the run, `null` before). `404` unknown key. |
| `GET /api/flows/:key/journal` | none | `200` journal lines (step, from, to, reason, actor, time). `404` unknown key. |
| `POST /api/flows` | `{ticket, flow?, settings?}` | `201` the new run. `409` when the ticket already has an active run (body carries the existing run). `404` unknown ticket or flow. |
| `POST /api/flows/:key/pause` | none | `200`. The current step finishes, then the run stops. |
| `POST /api/flows/:key/resume` | none | `200`. |
| `POST /api/flows/:key/abort` | none | `200`. Cancels the running session; worktree and PR are kept. |
| `POST /api/flows/:key/steps/:id/retry` | none | `200`. `409` when the step is not retryable. |
| `POST /api/flows/:key/steps/:id/skip` | none | `200`. |
| `POST /api/flows/:key/steps/:id/approve` | none | `200`. `409` when the step is not `waiting_human`. |
| `POST /api/flows/:key/steps/:id/reject` | none | `200`. Escalates with reason `rejected` and actor detail; following steps do not run. `409` when the step is not `waiting_human`. |
| `POST /api/flows/:key/steps/:id/click` | none | `200`. `409` when the step is not `waiting_human`. |
| `POST /api/flows/:key/steps/:id/verdict` | `{ok, report?}` | `200`. `409` when the step is not `waiting_human`. |
| `GET /api/flows/:key/external` | none | `200` `{key, jobs}`: the [external jobs](#external-executor) the run waits for, oldest first. `404` unknown key. |
| `POST /api/flows/:key/external/:jobId/result` | `{outcome, outputs?, reason?, state?, actor?}` | `200` the run, plus `duplicate: true` on an idempotent replay. `400`, `404` and `409` as in [External executor](#external-executor). A job id that starts with `agent:` takes the [agent result](#agent-jobs-run-by-a-host) body instead. |
| `POST /api/flows/:key/external/:jobId/claim` | `{actor?, hostId?}` | `200` the claimed agent job with its lease. `404` and `409` as in [Agent jobs](#agent-jobs-run-by-a-host). |
| `POST /api/flows/:key/external/:jobId/heartbeat` | `{claimToken}` | `200` `{leaseExpiresAt, cancelRequested}`. `404` and `409` as in [Agent jobs](#agent-jobs-run-by-a-host). |
| `POST /api/flows/:key/manual` | `{action, with?}` or `{pipeline, with?, publish?}` | `200`. Runs one extra action or pipeline. `publish` defaults to `false`; non-booleans return `400`. A `done` pipeline with `publish: true` emits `git.push`. |

A rejected human gate remains current and escalated, including expanded block IDs. Retry creates a fresh inbox entry; Skip returns `409`.
See [rejected gates](flows.md#rejected-gates).

Manual launches preserve the run's execution policy. Busy launches are not queued: the journal records `manual-<n> not started: workspace busy`. See [manual publishing and refusals](flows.md#manual-publishing-and-refusals) for completion and publication journal entries.

### External executor

`policy.executor` is `internal` (the default when absent) or `external`. It is chosen at launch, persisted with the execution, and never changes during a run.
A retried `POST /api/flows` with the same `executionKey` and another executor returns `409`.
Check `capabilities.externalActions` before sending `external`: an older gateway ignores the field and runs every action itself.

In `external` mode the gateway starts no action or probe subprocess. Agent steps, human gates and the in-process `noop`, `fail` and `timer` steps run as before, unless `policy.externalAgents` hands the agent sessions to the host too.
Every other registered action or probe becomes a job the host executes, then answers. The flow keeps its order, conditions, loops, gates and escalation.
See [flows.md](flows.md#external-executor) for the names and the outputs each one returns.

`GET /api/flows/:key/external` answers the pending jobs. Nothing is listed while the run is paused, pausing, aborted or terminal.
A job id is an effect id such as `publish@dev/git_push:1:run_action`: percent-encode it in the path.

```json
{
  "key": "LOC-12",
  "jobs": [{
    "id": "publish@dev/git_push:1:run_action",
    "kind": "action",
    "step": "publish@dev/git_push",
    "name": "git.push",
    "attempt": 1,
    "manual": false,
    "writes": true,
    "with": { "branch": "feat/LOC-12" },
    "ticketKey": "LOC-12",
    "local": false,
    "repo": "pixelhood",
    "workspace": "D:/AIDP/Vault/Pipelines/pixelhood/LOC-12/.worktree",
    "repoConfig": { "caps": { "worktrees": 3 } },
    "createdAt": "2026-10-16T12:00:00Z"
  }]
}
```

A probe job has `"kind": "probe"` and three more fields: `startedMs` (the wait start, gateway clock), `nowMs` (the gateway clock at listing time) and `state` (the memory of the last `pending` result, `null` at first).
`with` holds the step's `with` plus the placeholders the gateway resolved without calling GitHub, so `sha` can be stale: read the live head.

`POST /api/flows/:key/external/:jobId/result` takes:

```json
{ "outcome": "ok", "outputs": { "branch": "feat/LOC-12", "sha": "4f2c9e1", "changed": true }, "reason": null, "state": null, "actor": "host" }
```

| Job kind | `outcome` | Effect |
|---|---|---|
| action | `ok` | The step is `done` with `outputs`. |
| action | `skipped` | The step is `skipped`; `reason` is required. |
| action | `waiting_external` | The step waits and the job stays listed; `outputs` replace the step outputs. |
| action | `technical_error` | The step is `failed` with `reason`. |
| action | `business_error` | The step is `escalated` with `reason`. |
| probe | `satisfied` | The wait ends with `outputs`. |
| probe | `pending` | The wait goes on; `state` is stored and returned on the next listing. |

| Status | Cause |
|---|---|
| `200` | The run view. `duplicate: true` marks a replay of a result already applied (same job id and same digest, the last 64 are kept). |
| `400` | Unknown `outcome`, `skipped` without `reason`, or `outputs` that is not a JSON object. |
| `404` | Unknown run or job id. |
| `409` | A job the run no longer expects, a paused or aborted run, an outcome of the wrong kind, or the same job id with a different result. Drop the job. |

A manual action (`POST /api/flows/:key/manual` with `action`) in `external` mode is listed as a job too, under its manual effect id such as `manual-1:0:run_action`, and its result goes through the same route.

#### Agent jobs run by a host

`policy.externalAgents: true` hands the agent sessions of the flow's pipeline steps to the host. It needs `executor: "external"`, otherwise `POST /api/flows` answers `400`.
Check `capabilities.externalAgents` first: an older gateway ignores the field and launches the sessions itself.
The gateway starts the pipeline run as before and launches no session. It publishes one job per stage, plan task, shard or merge session, and waits for the host. The reply check, the one repair, the scope guard, loops and gates stay in the gateway.

The listing of `GET /api/flows/:key/external` also holds the pending agent jobs of the flow's runs. They have `"kind": "agent"`, an empty `with`, `step` set to the flow step that started the run, and an `agent` object.
A claimed job is not listed. A job id is `agent:<runId>:<seq>`: percent-encode the `:` in the path.

```json
{
  "id": "agent:20261018-101500-a1b2:3",
  "kind": "agent",
  "step": "dev",
  "name": "implement#T2",
  "attempt": 1,
  "manual": false,
  "writes": true,
  "with": {},
  "ticketKey": "LOC-12",
  "local": false,
  "repo": "pixelhood",
  "workspace": "D:/AIDP/Vault/Pipelines/pixelhood/LOC-12/.worktree",
  "createdAt": "2026-10-18T10:15:03Z",
  "agent": {
    "runId": "20261018-101500-a1b2",
    "stage": "implement",
    "label": "implement#T2",
    "jobKind": "initial",
    "resumed": false,
    "provider": "claude",
    "tier": "sonnet",
    "model": null,
    "effort": "high",
    "cwd": "D:/AIDP/Vault/Pipelines/pixelhood/LOC-12/20261018-101500-a1b2/tasks-iso/T2"
  }
}
```

| Route | Answer |
|---|---|
| `POST /api/flows/:key/external/:jobId/claim` | `200` `{claimToken, leaseSeconds, leaseExpiresAt, resumed, job}`: the full job with the prompt and every session parameter. A re-claim after an expired lease has `resumed: true` and the resumption note at the end of `job.prompt.text`. |
| `POST /api/flows/:key/external/:jobId/heartbeat` | `200` `{leaseExpiresAt, cancelRequested}`. Stop the session when `cancelRequested` is `true`. |
| `POST /api/flows/:key/external/:jobId/result` | `200` `{jobId, accepted, duplicate, next, repairJobId, reason}`. `next` is `settled`, `repair` (a repair job is published, named in `repairJobId`) or `ignored` (the run was cancelled, `accepted: false`). |

The result body names the outcome and the session:

```json
{
  "outcome": "completed",
  "reason": null,
  "actor": "host",
  "agent": {
    "claimToken": "9f2c4d7e1b0a4c58a3d6e2f1b7c90a11",
    "sessionId": "host:3f2a9c1e5b7d4e0f8a6b2c4d6e8f0a1b",
    "provider": "claude",
    "resolvedModel": "claude-sonnet-5-5",
    "durationMs": 412345,
    "exitCode": 0,
    "lastMessage": "...",
    "replyFile": "D:/AIDP/Vault/Pipelines/.../implement/tasks/T2/reply.md",
    "retryAfterSeconds": null,
    "usage": { "inputTokens": 12000, "outputTokens": 3400, "cacheReadTokens": 90000, "cacheWriteTokens": 5000, "costUsd": 0.42, "toolCalls": 31, "mcpCalls": 4, "complete": true }
  }
}
```

| `outcome` | Verdict |
|---|---|
| `completed` | The gateway reads and validates the reply file: accepted, repaired once through a second job, or failed with `reply rejected after one repair`. |
| `failed` | The row fails with `session failed: <reason>`. No repair. |
| `timeout` | The row fails with `session timed out: <reason>`. |
| `rate_limited` | The row fails with `session rate limited: <reason>`, plus the retry delay when `retryAfterSeconds` is sent. |
| `cancelled` | The row ends cancelled. |

From runtime `7.5.17`, a headless job carries `job.permissions.guidance`: how the session should read a denial. Add it to the session's instructions; it is omitted when the session is not headless.

A reply output whose path is missing is read again with its decoded JSON escapes (``, `	`, `
`, `
`, ``) turned back into backslashes, and kept when that path exists. This covers a Windows path such as `C:\araud` written with single backslashes.

The same claim token and the same result answer `duplicate: true`. The gateway prices `usage` by the tier of `resolvedModel` and falls back to `costUsd`.

| Lease rule | Behavior |
|---|---|
| Lease | A claim lasts 120 seconds. Renew it every 30 seconds. |
| Expired lease | The job returns to the listing with `resumed: true`. The old token stops working: a heartbeat or result with it answers `409`, even before the gateway swept the job. A second expiry fails the job with `host session interrupted twice`. |
| Claim timeout | A pending job that nobody claims for 30 minutes fails with `no host claimed the job`. |
| Session timeout | A claimed job older than `timeoutMinutes` plus ten minutes fails with `host session exceeded its timeout`. |
| Cancel | Open jobs become `cancelled`. The next heartbeat answers `cancelRequested: true`. A result without the claim token answers `409`; a result with it answers `200`, `accepted: false`, `next: "ignored"`. |
| Paused flow | New jobs are not listed and a claim answers `409`. A running job continues: its heartbeat and result are accepted. |
| Writer safety | While a host holds a claim, the listing withholds the writer manual actions of the flow. |

| Status | Cause |
|---|---|
| `404` | Unknown flow, unknown job, or a job of a run the flow did not start. |
| `409` | A claim of a job that is claimed, done or cancelled, or of a paused flow. A heartbeat or result with a wrong or expired token. A different result for a settled job. A run that does not use external agents. |

Status codes common to the routes:

- `404` for an unknown `key` or step `id`.
- `409` for a second active run on a ticket, and for a human decision on a step that is not `waiting_human`. The actor and time of a decision are journaled.
- `403` for every `POST` when `AIDP_READ_ONLY` is set; the `GET` routes stay served.

Path segments are percent-decoded one by one: a runtime step id travels encoded, `steps/publish%40dev%2Fgit_push/retry` targets `publish@dev/git_push`.

Events published (payloads below, also listed in [Bus events](#bus-events)):

| Event | Payload |
|---|---|
| `flow.started` | `{key, flowId, version}` |
| `flow.step_changed` | `{key, stepId, phase, from, to, reason?}` |
| `flow.waiting_you` | `{key, stepId, kind, inboxId?}` |
| `flow.ended` | `{key, status}` where `status` is `done`, `failed`, `escalated` or `aborted`. |

## PR endpoints — Lot 12a

Read-only. Nothing here writes to GitHub. The same facts and verdict come from the CLI verb
`aidp gh pr-status [<pr>] [--repo owner/name]`.

### `GET /api/prs/status?repo=&number=[&refresh=1]`

Returns the merge facts and the gate verdict of one pull request. `repo` is `owner/name`. The
response is cached per `(repo, number)` for 20 seconds; `refresh=1` bypasses that cache. The base
branch rules are cached separately for 1 hour; a degraded branch-rules read is not cached, so the
next request retries it. The query and the gate verdicts live in
`crates/aidp-integrations/src/pr.rs`, shared with the CLI. Fields are camelCase:

```json
{
  "repo": "owner/name", "number": 123, "url": "...", "title": "...",
  "state": "open", "isDraft": false,
  "head": { "ref": "feat/KEY-1", "sha": "..." }, "base": { "ref": "master" },
  "author": "...", "viewer": "...",
  "review": { "decision": "approved", "source": "derived",
              "reviewers": [{ "login": "...", "state": "APPROVED", "at": "...", "commitSha": "..." }],
              "submitted": [{ "login": "...", "state": "COMMENTED", "at": "...", "commitSha": "..." }], "requested": ["..."] },
  "checks": { "verdict": "pending",
              "runs": [{ "name": "build", "status": "IN_PROGRESS", "conclusion": null, "url": "..." }] },
  "mergeable": "mergeable", "behindBy": 3,
  "threads": { "unresolved": 2, "actionable": 2, "actionableIds": ["thread-id:comment-id"], "requireResolution": false },
  "gate": { "ok": false, "reason": "checks are still running",
            "items": [{ "id": "approved", "ok": true },
                      { "id": "checks", "ok": false, "reason": "checks are still running" }] },
  "degraded": [], "readAt": "2026-09-28T14:00:00Z"
}
```

- `checks.verdict` is `none`, `pending`, `passed` or `failed`. Only the latest run per check name
  counts. `pending` wins over `failed`.
- `review.source` is `github` when GitHub supplies `reviewDecision`, and `derived` when the daemon
  computes it from each reviewer's latest opinion.
- `behindBy` is absent when the PR is not open.
- `gate.reason` is an English sentence for the CLI: the sentence of the first failing item. Clients
  translate from the item ids. `gate.items[].reason` is present on every item that carries a
  sentence; a passing item usually has none, except `approved` when no approval rule applies.
- The `approved` item follows this rule. A non-empty GitHub `reviewDecision` is authoritative:
  `APPROVED` passes and `REVIEW_REQUIRED` fails. `CHANGES_REQUESTED` blocks through
  `no_changes_requested`, which is always evaluated first. When GitHub returns no decision and the
  base branch rules are readable, approval is required only when a `pull_request` rule has
  `required_approving_review_count` above 0 (the maximum over the rules). The number of approving
  reviewers is compared to it, and a failure reads like "this PR has 0 of 1 required approvals".
  When the rules are unreadable (403, or a repository without protection such as a private
  repository on a free account), no approval is required and the item passes with the reason
  "no approval rule on this repository". `degraded` still lists `rules`, because the read failed.
- The squad listing's `mergeable` signal follows the same rule, using GitHub's `reviewDecision`
  only. The squad listing computes the gate with the required approval count unknown: it reads no
  branch rules. A PR whose `reviewDecision` is null therefore passes the `approved` item there even
  when the branch rules require approvals, and `aidp gh pr-merge` may then refuse (exit 4). The per-PR
  status endpoint reads the rules and is authoritative.
- `gate.items[].id` is one of `open`, `not_draft`, `no_changes_requested`, `approved`, `checks`,
  `no_conflict`, `threads`. The gate returns the first blocking reason in that order.
  `no_conflict` blocks only on `CONFLICTING`; `UNKNOWN` does not block. `threads` counts
  unresolved conversations only when the base branch requires thread resolution.
- A payload the daemon cannot classify yields `"unknown"` for `state`, `mergeable` and
  `review.decision`, and `"UNKNOWN"` for a check run `status` or `conclusion` and for a reviewer
  `state`. Clients treat an `unknown` `mergeable` as non-blocking, as the gate does, and render the
  other unknown values as neutral.
- `review.submitted` includes submitted `COMMENTED` reviews. `review.reviewers` retains the latest opinionated reviews used by the approval gate.
- `threads.actionable` and `actionableIds` describe pending responses from paginated GraphQL threads. Their values are `null` when thread data is incomplete.
  See [automatic review corrections](flows.md#automatic-review-corrections) for the response rules and fresh-read probes.
- `degraded[]` names the reads that failed or came back incomplete. An unreadable `checks` or
  `review` read blocks the gate. An unreadable `threads` or `rules` read does not block it.

### `GET /api/tickets/:key/pr`

Resolves the ticket's pull request (`autopilot.pr` when set, else the PR of branch `feat/<KEY>` in
the ticket's project) and returns the same body as `/api/prs/status`. When none exists the answer is
`404` with `{"error": "...", "code": "...", "reason": "no-pull-request"}`. The answer
is also `404` when no project is registered for the ticket's service.

### Cache invalidation and the `pr.status` event

`POST /api/hooks/:id` reads the `X-GitHub-Event` header. For `pull_request`, `pull_request_review`,
`pull_request_review_comment`, `check_run`, `check_suite` and `status` deliveries that concern a PR,
the daemon drops that PR's cache entry and publishes `pr.status` with payload
`{"repo": "owner/name", "number": 123}`. This happens whether or not an automation matches the hook
id. Automation handling of the delivery is unchanged.

A `status` delivery names a commit, not a PR: the daemon invalidates every cached PR of the
repository whose head sha matches and emits one `pr.status` per invalidated PR.

| Event | Payload | Notes |
|---|---|---|
| `pr.status` | `{repo, number}` | A GitHub delivery changed this PR. Clients refetch `/api/prs/status`. |

The status route is the one per-request GitHub read in the daemon. The 20 s cache bounds it; see
`docs/adr/loc-55-pr-status-gate.md`.

## PR inbox — Lot 25a

Lists the open PRs of the whole squad, sorted into lanes. Reads go to GitHub; the only local write is
the marks file. The same data comes from `aidp gh squad-repos` and `aidp gh squad-prs`.

### Configuration

The `squad` key of `~/.aidp/config.json`:

```json
{ "squad": { "name": "...", "github_team": "owner/team-slug",
             "extra_repos": [], "exclude_repos": [],
             "bot_authors": ["dependabot[bot]", "renovate[bot]"] } }
```

The defaults are `name` = `squad`, `github_team` = `""` (no team) and `bot_authors` =
`["dependabot[bot]", "renovate[bot]"]`. With a team, the repo list is the team's non-archived repos,
plus `extra_repos`, minus `exclude_repos`, cached for 24 hours. Without a team, it is `extra_repos`
plus the registered repositories, minus `exclude_repos`, and no `gh api` call is made. Any author
ending in `[bot]` also counts as a bot.

### `GET /api/prs?lane=&repo=&author=&bots=&q=`

All filters are optional. `bots` is `hide`, `show` or `only`. `q` is a free-text match. Fields are
camelCase:

```json
{ "prs": [ { "repo": "owner/name", "number": 123, "title": "...", "lane": "needs_action",
             "reviewState": "stale", "flow": { "key": "LOC-1", "phase": "implement" } } ],
  "lanes": { "needs_action": 2, "re_review": 1 },
  "fetchedAt": "2026-10-01T14:00:00Z", "degraded": [] }
```

- Lane ids, in priority order: `needs_action`, `re_review`, `first_review`, `discussion`, `reviewed`,
  `drafts`. A PR sits in exactly one of them. `mine` is an extra lane holding every PR the viewer
  authored, whatever its main lane.
- `reviewState` is the viewer's own review: `none`, `commented`, `approved`, `changes_requested`,
  `stale` (approved or reviewed on an older head sha) or `dismissed`.
- `flow` is present only when a ticket flow tracks the PR (Lot 29a index); otherwise it is absent.
- `degraded[]` entries carry a `kind`: `team` when the team repos cannot be read (the token lacks
  the `read:org` scope; the message says to grant it, and the list falls back to `extra_repos` plus
  the daemon's registered projects), and `batch` when a read failed (the entry names the repos it
  covered; the other reads are still served). When the failed read had a cached answer, those pull
  requests are still listed and the message ends with `(showing the last known state)`.

The fetch is incremental, so a refresh costs a few GraphQL points instead of hundreds:

1. A light listing reads every repo, 5 per query: number, `updatedAt`, head sha, draft, mergeable,
   review decision and the checks rollup. GitHub prices a query by its declared bounds, so this
   listing stays cheap whatever the number of pull requests.
2. Only the pull requests that are new, or whose `updatedAt` or head sha changed, are read in full,
   20 per query. The others keep their cached details, with the listing's draft, mergeable, review
   decision and checks.

Up to 6 `gh` calls run at once. GitHub answers HTTP 502, 503 or 504 (or a cut-short body) to a
query it finds too heavy: such a batch is read again in halves, and a single repo or pull request
is retried once.

### `POST /api/prs/refresh`

Refetches, or waits for the fetch already running and reuses it, then publishes `pr.inbox_changed`.
Answers with the same body as `GET /api/prs`; when the fetch fails, the cached inbox is listed.

### `POST /api/prs/marks`

Body `{repo, number, seen?, ignored?}`. Marks live in `~/.aidp/gateway/pr-marks.json`, keyed
`"<owner/repo>#<n>"` with `{seen_sha, seen_at, ignored}`. A PR marked seen is recorded at its current
head sha; when the head moves, the PR is unseen again. `ignored` is not reset by a push. Refuses
with `403` under `AIDP_READ_ONLY`.

### Cache and the `pr.inbox_changed` event

The inbox is cached for 2 minutes. A `POST /api/hooks/:id` delivery of `pull_request*`,
`pull_request_review*`, `check_suite` or `issue_comment` marks the cache stale, as does a write on
a pull request and a marks change. A stale inbox is still listed at once, and `GET /api/prs` then
refreshes it in the background. Each change publishes `pr.inbox_changed` with an empty payload;
clients refetch `GET /api/prs`.

One fetch runs at a time: a reader that arrives during a fetch waits for it and reuses its result.
Every fetch is saved to `~/.aidp/gateway/squad/squad-prs.json`. After a restart the daemon lists
that snapshot at once, as stale, and refreshes it incrementally.

### `GET /api/github/rate-limit[?refresh=1]`

The GitHub quotas of the `gh` user, read through `gh api rate_limit` (a call GitHub does not count)
and cached for 30 seconds. `refresh=1` reads them again. `502` when `gh` cannot answer.

```json
{ "graphql": { "limit": 5000, "remaining": 4890, "used": 110, "resetAt": "2026-10-09T09:30:00Z" },
  "core": { "limit": 5000, "remaining": 4988, "used": 12, "resetAt": "2026-10-09T10:00:00Z" },
  "fetchedAt": "2026-10-09T09:00:00Z" }
```

A quota GitHub leaves out is `null`.

### `GET /api/github/asset?url=<url>[&repo=<owner/name>]`

An image or video of a pull request or comment, read on the viewer's behalf: a private
repository's attachments answer a login page to a browser that is not signed in to GitHub.

- `url` must be HTTPS on a GitHub host: `github.com` (`/user-attachments/...`,
  `/<owner>/<repo>/assets/...`, `/<owner>/<repo>/raw/...`), `raw.githubusercontent.com`,
  `private-user-images`, `user-images`, `objects`, `avatars` or `camo.githubusercontent.com`.
  Anything else is `400`.
- A `github.com` file needs `repo`: GitHub's Markdown API, called in that repo's context, gives
  its short-lived signed address, which is then read without a token. A raw file is read with
  the `gh` token. The other hosts are read without one.
- Only `image/*` and `video/*` answers are served, up to 30 MB, with `Cache-Control: private,
  max-age=86400`, `X-Content-Type-Options: nosniff` and a sandbox `Content-Security-Policy`, so an
  SVG opened alone runs no script. A failed read is `502`.

| Event | Payload | Notes |
|---|---|---|
| `pr.inbox_changed` | `{}` | The inbox changed. Clients refetch `/api/prs`. |

## PR attention — Lot 28a

The daemon turns PR changes into notifiable signals. It computes them on a squad PR refresh and on
a GitHub webhook delivery. Signals are never delivered outside the daemon.

### Signals

For PRs you authored: `new_comment`, `changes_requested`, `conflict`, `mergeable`. For other
people's PRs: `review_requested`, raised when the PR requests your review.

- `new_comment` ignores your own comments, bot comments and comments carrying an aidp marker.
  `last_seen` is the latest of your seen mark (Lot 25a) and your own last comment.
- `mergeable` fires once per head sha, and only when the merge gate (`/api/prs/status`) is green.

### Fingerprint and store

Fingerprint: `<owner/repo>#<n>:<signal>:<head_sha>[:<comment id or reviewer>]`. The store is
`~/.aidp/gateway/pr-attention.json`, keyed by fingerprint with `{firstSeen, notified, read}`. Dedup
reads this file, so it survives a daemon restart. Each newly notified signal creates an inbox entry
of kind `pr.attention` and publishes the bus event `pr.attention`.

### `GET /api/prs/attention?unread=`

`unread=true` keeps only unread items. Answers `{items}`.

### `POST /api/prs/attention/read`

Body `{fingerprints: [...]}` or `{all: true}`. Answers `{count}`, the number of items marked read.
Answers `400` when the body has neither. Emits `pr.attention_read`. Refuses with `403` under
`AIDP_READ_ONLY`.

### Configuration

The `notifications.pr` key of `~/.aidp/config.json` holds one boolean per signal (`new_comment`,
`changes_requested`, `conflict`, `mergeable`, `review_requested`). All default to `true`. Disabling a signal stops new entries and keeps the existing ones.

| Event | Payload | Notes |
|---|---|---|
| `pr.attention` | `{fingerprint, repo, number, signal, firstSeen}` | A new signal was notified. |
| `pr.attention_read` | `{fingerprints}` | Items were marked read. |

## PR detail — Lot 26a

The viewer reads a PR's detail and writes review comments through the daemon. Reads go through
`gh api` (REST first). Every mutating route refuses with `403` under `AIDP_READ_ONLY`. The transport
choices are in `docs/adr/loc-81-pr-detail-transport.md`. All routes take `repo` (`owner/name`) and
`number`.

### Read routes

| Route | Answer |
|---|---|
| `GET /api/prs/detail?repo=&number=` | PR header, body, reviews, issue comments, review threads and the merge gate. |
| `GET /api/prs/files?repo=&number=&page=` | `{page, perPage, hasMore, files[], sinceLastReview[]}`. Each file carries status, additions, deletions, blob sha and `viewed` (true only while its blob equals the one marked). `sinceLastReview` lists the marked paths of the page whose blob changed since the mark. |
| `GET /api/prs/file-diff?repo=&number=&path=` | The patch of one file, with the line numbers that accept a review comment. |

### Comment routes

| Route | Body | Bus event |
|---|---|---|
| `POST /api/prs/comments` | `{repo, number, body}` | `pr.comment_created` |
| `PATCH /api/prs/comments/:id` | `{repo, number, body}` | `pr.comment_updated` |
| `DELETE /api/prs/comments/:id` | `{repo, number}` | `pr.comment_deleted` |
| `POST /api/prs/review-comments` | `{repo, number, path, line, side, body, inReplyTo?}` | `pr.comment_created` |
| `PATCH /api/prs/review-comments/:id` | `{repo, number, body}` | `pr.comment_updated` |
| `DELETE /api/prs/review-comments/:id` | `{repo, number}` | `pr.comment_deleted` |

`/api/prs/comments` handles conversation comments, `/api/prs/review-comments` handles comments
anchored to a diff line. The literal route `/api/prs/comments/state`
(Lot 14a) keeps precedence over `/api/prs/comments/:id`; an id is numeric, so they never collide.
Payloads are `{repo, number, commentId}`.

### Drafts, review and viewed

| Route | Body | Notes |
|---|---|---|
| `GET /api/prs/drafts?repo=&number=` | | The pending comments saved for the PR. |
| `PUT /api/prs/drafts` | `{repo, number, drafts[]}` | Replaces the whole list. |
| `POST /api/prs/review` | `{repo, number, event, body?, comments[]}` | Submits one review. Publishes `pr.review_submitted`. |
| `POST /api/prs/viewed` | `{repo, number, path, blobSha, viewed}` | Marks a file viewed or not. |

`event` is `COMMENT`, `APPROVE` or `REQUEST_CHANGES`. The review and its inline comments go in a
single request, so a review is never half-submitted. On success the daemon clears the PR's drafts.

### Stores

- `~/.aidp/gateway/pr-drafts.json`: drafts per PR. Drafts never reach GitHub until `review` succeeds.
- `~/.aidp/gateway/pr-viewed.json`: viewed marks per PR, keyed on the file path and its blob sha. A
  file whose blob sha changed after a push is shown as not viewed again.

### Statuses

- `400`: invalid body, or a line outside the diff (`off-diff line`).
- `403`: the comment belongs to someone else, or `AIDP_READ_ONLY` is set.
- `404`: unknown repo, PR, comment or file.
- `409`: `own-pr`. GitHub refuses `APPROVE` and `REQUEST_CHANGES` on your own PR.
- `502`: GitHub or `gh` failed.

### Cache invalidation

Every mutation drops the PR's status cache entry (Lot 12a) and the inbox cache (Lot 25a), then
publishes its event, so open clients refetch.

| Event | Payload |
|---|---|
| `pr.comment_created` | `{repo, number, commentId}` |
| `pr.comment_updated` | `{repo, number, commentId}` |
| `pr.comment_deleted` | `{repo, number, commentId}` |
| `pr.review_submitted` | `{repo, number, event, commentCount}` |

## PR actions — Lot 13a

Five mutating routes act on a PR. Each refuses with `403` under `AIDP_READ_ONLY`, drops the PR's
status cache entry and publishes its bus event.

| Route | Body | Bus event |
|---|---|---|
| `POST /api/prs/ready` | `{repo, number, draft}` | `pr.ready_changed` |
| `POST /api/prs/update-branch` | `{repo, number}` | `pr.branch_updated` |
| `POST /api/prs/merge` | `{repo, number, expectedHeadSha}` | `pr.merged` |
| `POST /api/prs/threads/reply` | `{repo, number, threadId, body}` | `pr.thread_replied` |
| `POST /api/prs/threads/resolve` | `{repo, number, threadIds[], unresolve?}` | `pr.threads_resolved` |

`merge` recomputes the gate uncached and refuses while it is closed. It squashes with
`--match-head-commit expectedHeadSha`, then deletes the remote branch through the git refs API.
A merge moves the linked ticket to `done` with actor `human` and reason `merged-from-viewer`; the
merge probe does not replay that transition.

## PR comments — Lot 14a

The state route lets the viewer triage a PR's review comments; a selection is fixed through
[PR fix](#pr-fix) (`POST /api/prs/fix`, or `aidp gh pr-fix`). It refuses mutating calls with `403` under `AIDP_READ_ONLY`. Nothing is written to GitHub.

### Triage state

`GET /api/prs/comments/state?repo=&number=` returns `{points, handed}` for the PR.
`POST /api/prs/comments/state` with `{repo, number, pointId, state}` records the triage state of one
point. The state lives in `<gateway dir>/prs/<owner>__<name>__<n>.json`, shaped
`{points, handed}`: `points` maps a point id (`<commentId>#<i>`) to its state, `handed` lists the
point ids already handed to a fix session.

## PR fix

`POST /api/prs/fix` starts one autonomous session that fixes the selected review threads of a PR,
replies on them and pushes to the PR head branch. It refuses mutating calls with `403` under
`AIDP_READ_ONLY`.

Body: `{repo, number, threads, points, note?, model?, effort?}`. `threads` holds thread node ids,
`points` holds point ids (`<commentId>#<i>`), `note` is free text for the session.

Response: `{session_id, workspace, workspace_kind, threads, dropped_threads}`. `dropped_threads` are
selected threads already resolved on GitHub; the session does not see them.

Refusals use the standard `{error, code}` body:

- `403` when the PR is not yours (`not-mine`).
- `409` when the PR is closed or merged.
- `409` when the workspace is busy; the error names the holder session or workflow stage.

### Workspace

The ticket worktree is used when the head branch is `feat/<KEY>` or `fix/<KEY>` and that worktree
exists. Otherwise the session runs in `<vault>/Pipelines/<repo>/PR-<n>/.worktree`, removed after a
successful push and kept otherwise (`worktree_kept` on the event).

### Session settings

- Prompt: the engine prompt built from the recipe (see below).
- Origin: `pr-fix`, ref `owner/repo#n`.
- Profile: autonomous. `git push` is restricted to the PR head branch, with no force.
- `autoClose` is on. The default model is `sonnet`.

### Marks

When a fix session starts, `pr-marks.json` records under the PR's entry:

- `fix_session`: the session id, and `fix_at`: the launch time (RFC 3339).
- `threads`: an object keyed by review thread id. Each value carries `fix_session` (the session id)
  and `at` (RFC 3339, the same instant as `fix_at`). Only selected review threads appear there,
  never points, and the map is omitted when empty.

```json
{
  "owner/repo#42": {
    "fix_session": "sess-1",
    "fix_at": "2026-10-08T14:16:31Z",
    "threads": {
      "PRRT_kwDOA": { "fix_session": "sess-1", "at": "2026-10-08T14:16:31Z" }
    }
  }
}
```

The selected thread and point ids are also added to the `handed` list of the PR comment state, which
gives their verdict `addressed: indirect`.

### Event `pr.fix_session`

Published twice per session, with `phase` `started` then `ended`. Payload: `{repo, number,
sessionId, phase, threads, fixed?, needsReviewer?, assumptions?, pushedSha?, worktreeKept?, error?,
summaryMissing, unanswered?, replyCheck}`. The optional fields are present on `ended`;
`summaryMissing` is true when the session closed without its final summary line.

On `ended` the gateway re-reads the PR review threads on GitHub. A chosen thread is answered when it
gained at least one comment by the PR author since the session started. `unanswered` lists the chosen
thread ids without such a reply, whatever the session reported. `replyCheck` is `not_checked` on
`started`, then on `ended` one of `all_answered`, `unanswered` or `unverified` (GitHub could not be
read, so `unanswered` is absent).

### Recipe

The prompt comes from `templates/recipes/fix-pr-comments.md`. The gateway fills these placeholders
in one pass: `{{repo}}`, `{{pr_number}}`, `{{head_branch}}` (the only push target), `{{threads}}`
(id, file, line, diff hunk, conversation), `{{points}}`, `{{note}}` (`(none)` when absent) and
`{{summary_prefix}}`. The session ends with one line: the prefix followed by
`{"fixed":[ids],"needs_reviewer":[ids],"not_relevant":[ids],"pushed_sha":"...","assumptions":["..."]}`.
The gateway parses it to fill the `ended` event.

### Known limit

Thread comments are read with `comments(first:50)` and no pagination: a longer thread is truncated.

## Runs natifs

The daemon executes a v7 run itself, one stage per session, with no Claude Workflow session. See `docs/native-execution.md` and `docs/adr/loc-92-native-executor.md`. Mutating routes refuse with `403` under `AIDP_READ_ONLY`.

| Route | Body | Answer |
|---|---|---|
| `POST /api/runs` | `{projectId, templateId, inputs, target?, workspace?, provider?, providerByRole?, autonomous?, autoApproveGates?, vault?, noVault?}` | `201 {runId}`. `workspace` is the absolute directory the run works in (the project checkout when absent); a relative one is `400`. `providerByRole` maps agent roles to provider ids, before falling back to `provider`. |
| `GET /api/runs/:id` | | The run: status, stages, current gate |
| `GET /api/runs/:id/files` | | `200` bare array `[{ path, size, kind, stage }]` of every regular file under the run directory, recursive. `path` is relative to the run directory and `/`-separated, `kind` is `md`, `json`, `png`, `gif` or `other` (by extension), `stage` is the first directory of the path (`""` for a file at the root). Sorted by `stage` then `path`, capped at 2 000 entries; symbolic links are skipped. `404` unknown run. |
| `GET /api/runs/:id/files/*path` | | `200` the raw file, read-only. `Content-Type` from the UI MIME table (plus `text/markdown` for `.md`, `text/plain` for `.yaml`/`.yml`), `Cache-Control: no-cache`, `X-Content-Type-Options: nosniff`. `404` for a missing file, a directory, `..`, an encoded separator, a drive mark, or a path crossing a symbolic link; `413` above 20 MiB. |
| `POST /api/runs/:id/cancel` | | `200 {ok: true}` |
| `POST /api/runs/:id/stages/:stage/retry` | | `200 {ok: true}`, stage relaunched |
| `POST /api/runs/:id/stages/:stage/skip` | | `200 {ok: true}`, stage skipped |
| `POST /api/runs/:id/stages/:stage/approve` | | `200 {ok: true}`, gate approved |

`autoApproveGates` is a list of stage names, or `*` for every declared gate. An autonomous run pre-approves every gate.

`vault` is the vault root that holds the run directory, instead of the vault the daemon resolves (`AIDP_VAULT`, then `~/.aidp/config.json`). `noVault: true` keeps the run directory in the workspace, under `.aidp/pipelines/`, whatever vault is configured. Both are kept in the run's `request`, so a resumed run goes back to the same place.

`:id` and `:stage` are percent-decoded, as for the flow routes. The stage verbs take a stage name: a plan task row (`implement#t1`) is acted on through its stage (`implement`).

`GET /api/runs/:id` also returns:

| Field | Type | Meaning |
|---|---|---|
| `autonomous` | boolean | Immutable flag set at launch. In an autonomous run no stage asks a question. |
| `assumptions` | array | Decisions taken without asking, aggregated from every stage. Each entry has the choice, the reason, the rejected alternative and `auto_answered` (true when a `blocked` reply or `ask_user_questions[]` was answered by the retry). |

A stage blocked twice in an autonomous run fails with reason `blocked-twice`.

Errors use the standard `{error, code}` body: `400` for an invalid body or unknown template, `404` for an unknown run, project or stage, `409` when the workspace is busy or the stage is not in a state the verb accepts, `503` with `code: config` when the `aidp` or `claude` binary is missing.

Events, forwarded over `GET /api/events`: `run.started`, `run.stage_changed`, `run.waiting_gate`, `run.ended`. Each payload carries the `runId`; `run.stage_changed` and `run.waiting_gate` also carry the `stage`.

CLI: `aidp run exec <template> --target <id> --inputs-file <path> [--foreground]` posts to `POST /api/runs`. The global `--vault` and `--no-vault` become `vault` and `noVault`; `--workspace-root` and `--vault` are made absolute against the current directory first, since the daemon does not share it.

## Bus events

Every fact the daemon produces is also published on its internal event bus and forwarded over
`GET /api/events` (SSE) — see `bus.rs`. Event-triggered automations subscribe with `on: [...]`
patterns that support a `*` wildcard and a `<prefix>.*` prefix match (`automations.rs::pattern_matches`),
so an automation on `session.*` starts seeing `session.updated` the moment this Lot ships it.

| Event | Payload | Notes |
|---|---|---|
| `session.started` | session summary | Emitted when a session is created. |
| `session.updated` | session summary | **Lot 1a.** Emitted by `PATCH /api/sessions/:id`. Same summary shape as `session.started`, which as of this Lot always includes `title`, `pinned`, `archived`, `unread`. |
| `session.question` | summary + `questions` | An `AskUserQuestion` is pending. |
| `session.answered` | session summary | The pending question was answered. |
| `session.turn_end` | summary + `isError`, `costUsd`, `usage` | One turn completed. |
| `session.changes` | `{sessionId, fileCount}` | **Lot 2a.** Emitted at every `turn_end`; `fileCount` is the number of entries in `git status --porcelain` for the session's `cwd`. Emitted even when `fileCount` is `0`, so a subscriber can distinguish "nothing changed" from "no event". Computed on a detached thread — it never slows the session transport. **New event name matching the existing `session.*` glob**: any automation subscribed to `session.*` starts firing on it too, and will see its trigger rate rise. |
| `session.run` | `{sessionId, runId, status, stage, stageStatus, doneCount, stageCount, currentGate}` | **Lot 3a.** Fires only for a `running` session that resolves to a vault run (`runs::resolve_run`, same workspace_root/started_at inference as `GET /api/sessions/:id/run`), and only when its stages or `currentGate` actually changed since the last poll — not on every tick. `stage`/`stageStatus` are the name/status of the first stage currently `running`, or both absent when none is. Computed on a detached thread (`runs::start_watcher`): a 5-second poll of the run-state file's mtime per running session, so it never slows the session transport. |
| `session.workflow_progress` | `{sessionId, runId, phase, agent, label, status, ts}` | **`runId` is the Workflow run id** (e.g. `wf_b0f2b915-acb`) — the same string `run-state.json` stores as `workflow_run_id` and that a resume passes as `resumeFromRunId`, prefix included. `agent` is the Workflow runtime's agent id; `label` and `phase` are the stage label and phase when the journal carries them, `null` otherwise — and they frequently are null: measured over 280 real runs, only 262 of 2425 `started` journal lines carry `label`/`phase`, and a hand-authored Workflow script (not an aidp run) has no aidp-shaped phase at all, so a consumer must render a null phase, not assume one. `status` is `started`, or the sub-agent reply's `status` on completion (`completed` / `needs_review` / `blocked`), or the literal string `result` when the journal's result payload carries no status string, or `failed`; the sub-agent's full reply envelope is deliberately **not** forwarded — real result lines run 800-4500 bytes. The watcher starts at the Workflow tool result, not the tool call (which returns in about 0.1s), and stops at session end; a resume that re-launches the same run id does not start a second watcher. Computed on a detached thread, a 2-second incremental read of the run's `journal.jsonl`, so it never slows the session transport — the same discipline as `session.changes` and `session.run`. **New event name matching the existing `session.*` glob**: any automation subscribed to `session.*` (see `automations.rs::pattern_matches`) starts firing on it too, and will see its trigger rate rise — about 17 events per pipeline run, spread over tens of minutes, more than `session.run`, far less than a per-line firehose, and not enough to meaningfully shorten the 500-entry `Bus::recent` catch-up ring (`bus.rs`). |
| `session.ended` | summary + `exitCode`, `costUsd`, `turns`, `lastText`, `usage` | The process exited. |
| `gateway.rate_limit` | `{status, fiveHour, sevenDay, isUsingOverage}` | Subscription utilization changed. |
| `preset.saved` | `{presetId, name}` | **Lot 1a.** Emitted by `POST /api/presets` (create or update). |
| `automation.saved` | `{automationId, name, enabled}` | An automation was created/updated. |
| `automation.run` | `{automationId, name, runId, status, reason, sessionId?, note?}` | An automation fired. |
| `automation.error` | `{automationId, name, error}` | An automation failed to run or deliver. |
| `automation.skipped` | `{automationId, name, trigger, skipped}` | The scheduler found a due automation whose previous scheduled run is still in flight and did not start a second one. `trigger` is `cron`; `skipped` is `"already running"`. See [Automation endpoints](#automation-endpoints--lot-3a). |
| `overflow` | `{stream, sessionId, capacity}` | An SSE client stopped reading and its stream was closed (see [Concurrency and resource bounds — LOC-29](#concurrency-and-resource-bounds--loc-29)). `stream` is `events` (`GET /api/events`, `sessionId` is `null`) or `session` (`GET /api/sessions/:id/stream`); `capacity` is the frame buffer size, `256`. Emitted once per closed stream. |
| `webhook.received` | `{automationId, name}` | `POST /api/hooks/:id` matched an automation. |
| `session.queued` | `{queueId, projectId, projectName, templateId, target, priority, queuedAt, origin}` | **Lot 4a.** Emitted when `POST /api/sessions` enqueues instead of starting. |
| `session.dequeued` | `{queueId, projectId, reason}` | **Lot 4a.** `reason` is `started` \| `cancelled` \| `failed`. Carries `sessionId` when `reason` is `started` and `error` when it is `failed`. |
| `sessions.purged` | `{count, sessionIds}` | **Lot 5a.** Emitted by `POST /api/health/cleanup`. |
| `ticket.created` | ticket record | **Lot 9a.** Emitted when `POST /api/tickets` creates a local ticket. |
| `ticket.updated` | ticket record | **Lot 9a.** Emitted when a ticket's stored fields change. |
| `ticket.status` | `{key, from, to, reason, actor}` | **Lot 9a.** Emitted on every status transition. |
| `ticket.started` | `{key, sessionId, project}` | **Lot 9a.** Emitted when the dispatcher launches a session for the ticket. |
| `ticket.reviewed` | `{key, pr}` | **Lot 9a.** Emitted once the ticket's PR is ready for review. |
| `ticket.done` | `{key}` | **Lot 9a.** Emitted when the ticket reaches a terminal done state. |
| `dispatcher.tick` | `{launched, skipped, slotsUsed, slotsMax}` | **Lot 9a.** Emitted after every dispatch pass. |
| `dispatcher.paused_changed` | `{paused, actor, at}` | Emitted by `POST /api/dispatcher/pause` and `/resume` only when the paused state effectively changes. `at` is ISO 8601 UTC. Bus only: no messaging notification. |
| `dispatcher.resumed` | `{reason}` | **Lot 10a.** Emitted when the dispatcher leaves a budget wait (`mode` going `"waiting"` -> not-`"waiting"` on a tick), with `reason: "budget"` — **not** emitted by `POST /api/dispatcher/resume`, which emits `dispatcher.paused_changed` instead. The name was already documented for the HTTP route and is reused here rather than replaced, disambiguated by `reason`; see [Decisions and non-goals](#decisions-and-non-goals). |
| `dispatcher.waiting` | `{until, window, utilization, limit}` | **Lot 10a.** Emitted once per entry into waiting mode — a normal/paused -> waiting transition — never on a tick that was already waiting. `until` and `window` (`"5h"` \| `"7d"`) are `null` on a `paused` verdict with no resume instant to report; `until` is RFC3339, `utilization`/`limit` are the raw fractions (e.g. `0.87`). |
| `worktree.created` | `{key, project, path, branch, reused}` | **Lot 17a.** See [Worktrees](#worktrees--lot-17a). |
| `worktree.removed` | `{key, salvagedRef?}` | **Lot 17a.** |
| `worktree.remove_failed` | `{key, path, error, attempts}` | **Lot 17a.** |
| `flow.started` | `{key, flowId, version}` | **Lot 29a.** See [Flows](#flows--lot-29a). |
| `flow.step_changed` | `{key, stepId, phase, from, to, reason?}` | **Lot 29a.** Every step status transition. |
| `flow.waiting_you` | `{key, stepId, kind, inboxId?}` | **Lot 29a.** A step waits for a human. |
| `flow.ended` | `{key, status}` | **Lot 29a.** |
| `unity.lock_changed` | `{resource, held, holder, waiting}` | The Unity editor lock changed hands or its queue changed. See [Unity editor lock and status](#unity-editor-lock-and-status). |
| `doctor.incident` | the incident | An incident of the pipeline doctor opened, started its investigation or closed. See [Pipeline doctor](#pipeline-doctor). |
| `memory.proposed` / `memory.changed` / `knowledge.changed` / `rag.reindexed` / `memory.model` | see [Project memory and RAG](#project-memory-and-rag) | Memory proposals, memory changes, knowledge note changes, finished re-indexes and the state of the embedding model. |

Because an `Event` trigger already subscribes with `on: [...]` glob patterns (see above), an
automation can notify on `ticket.reviewed` through the existing generic webhook action with no new
mechanism — the same path any other `session.*` automation already uses.

Session summary shape (`SessionManager::summary`): `sessionId`, `projectId`, `projectName`,
`engine`, `templateId`, `target`, `status`, `origin`, `automationId`, `automationRunId`, `title`,
`pinned`, `archived`, `unread`.

## Automation endpoints — Lot 3a

**The scheduler never overlaps two runs of one automation.** When a scheduled automation comes
due while its previous scheduled run is still in flight, the scheduler starts nothing and emits
`automation.skipped` with `skipped: "already running"` (see [Bus events](#bus-events)); the next due
time is computed as usual. The in-flight mark is released when the run ends, even on a panic. For a
`Launch`/`Prompt` action, "the run ends" means the session it started ends (or, for a launch queued
behind another running session on the same project, until that queued entry is cancelled or fails to
start) — the automation stays in flight for the session's whole lifetime, not merely until it is
launched or queued. Manual runs (`POST /api/automations/:id/run`) are not guarded: they can overlap a
scheduled run or each other.

### `POST /api/automations/:id/run`

Fires an automation for real, right now, outside its own schedule (`http.rs:587-600`). `404`
(`"automation not found"`) on an unknown id.

It runs `run_automation(id, "ui", {"payload": <body>})` and returns the resulting `AutomationRun` (`200`),
or `400` on a start failure (e.g. `launch`/`prompt` whose `projectId` no longer resolves).

```bash
curl -X POST http://127.0.0.1:3001/api/automations/AUTOMATION_ID/run \
  -H "Content-Type: application/json" -d '{}'
```

### `POST /api/automations/:id/test`

The dry-run (`AutomationEngine::test_automation`). Replays the trigger against a supplied payload
and reports what *would* happen — it executes nothing and persists nothing:

- No session is started, no webhook HTTP request is sent.
- No `AutomationRun` is created, no line is appended to `automation-runs.jsonl`, no `automation.run`
  / `automation.error` bus event is emitted. A test run would otherwise pollute the Historique view
  (`GET /api/automations/:id/runs`) and the 24h `skippedAutomation` aggregation in the inbox.
- `run_count`, `last_run_at`, `last_status` on the automation itself are untouched.

`404` (`"automation not found"`) on an unknown id.

Body: `{"event"?: {"type", "data"}, ...}` — for an `Event` trigger, `event.type`/`event.data` are
what `event_trigger_matches` is evaluated against; any other keys are available to templating as
`payload`.

Response:

| Field | Type | Meaning |
|---|---|---|
| `automationId` | string | |
| `name` | string | |
| `dryRun` | bool | Always `true`. |
| `wouldRun` | bool | `false` once `filter` rejects or `guardrail` blocks; `true` otherwise. |
| `status` | string | `would-run` \| `filtered` \| `skipped`. |
| `trace[]` | array | See below. |

```bash
curl -X POST http://127.0.0.1:3001/api/automations/AUTOMATION_ID/test \
  -H "Content-Type: application/json" \
  -d '{"event": {"type": "session.ended", "data": {"status": "done"}}}'
```

### `GET /api/automations/:id/runs`

The last 50 `AutomationRun`s for one automation, newest first (`list_runs(Some(id), 50)`).

### `GET /api/automation-runs`

The last N `AutomationRun`s across every automation, newest first. `limit` query param, default 50.
Both routes read the same `automation-runs.jsonl` (append-only; the latest line per run id wins) and
feed the same Historique view.

### `AutomationRun`

| Field | Type | Meaning |
|---|---|---|
| `id` | string | |
| `automationId` | string | |
| `automationName` | string | Snapshotted at run start. |
| `reason` | string | `cron` \| `webhook` \| `event` \| `ui` — what triggered this run. |
| `startedAt` | string | |
| `endedAt` | string? | Omitted while the run is still `started`. |
| `status` | string | `started` \| `done` \| `failed` \| `cancelled` \| `skipped`. |
| `sessionId` | string? | Set once a `launch`/`prompt` action starts a session. Omitted for `webhook`/`none` actions and until the session exists. |
| `error` | string? | Omitted when absent. |
| `note` | string? | Omitted when absent (e.g. the utilization-guardrail message on a `skipped` run). |
| `trace` | `TraceStep[]` | Omitted (not `[]`) when empty — every run recorded before this Lot has no `trace` key at all, and a run still `started` may legitimately have an empty one. |

### `TraceStep`

| Field | Type | Meaning |
|---|---|---|
| `step` | string | One of `trigger` \| `filter` \| `guardrail` \| `action` \| `delivery`. |
| `outcome` | string | One of `matched` \| `rejected` \| `passed` \| `blocked` \| `rendered` \| `planned` \| `skipped` \| `na`. |
| `detail` | string? | Omitted when absent. |
| `data` | value | Omitted when `null`. |
| `at` | string | ISO timestamp. |

### Trace semantics

`trace[]` always appears in this order — `trigger` → `filter` → `guardrail` → `action` →
`delivery` — produced by the same helpers (`event_trigger_matches`, `render_action_trace`,
`delivery_trace`) on both the real run path (`run_automation`) and the dry-run
(`test_automation`), so the two can never drift apart.

Three things a consumer must get right:

- **The guardrail step is conditional on the action type.** The 5-hour utilization guardrail is
  only evaluated for `launch` and `prompt` actions (`outcome: "passed"` or `"blocked"`, with the
  blocking window/threshold string in both `detail` and, on a real run, the sibling
  `AutomationRun.note`). For `webhook` and `noop` actions the guardrail step reports
  `outcome: "na"` — that is a real, meaningful state, not a truncated trace.
- **A filtered-out event leaves no run at all.** The trigger filter (an `Event` trigger's `on` /
  `filter`) is evaluated *before* any `AutomationRun` exists, so on a real run `trace[]` can never
  explain why an automation did **not** fire — there is no run to attach that explanation to.
  `POST /:id/test` is the only place a `filter` step with `outcome: "rejected"` is observable. A
  `Webhook` trigger has no filter concept at all (`outcome: "na"`, detail `"webhook triggers have no
  filter"`); every other non-`Event` trigger reports the same `na` with a generic detail.
- **`trace` is absent on every run recorded before this Lot**, and absent is a valid state — so is
  an empty array, on a run still `status: "started"` whose trace hasn't been assigned yet.

## CLI verbs

`aidp gateway presets <verb>` (`client.rs`) — thin HTTP clients for the routes above:

```bash
aidp gateway presets list                 # GET /api/presets, one row per preset
aidp gateway presets add preset.json      # POST /api/presets — upserts; JSON file, not inline flags
aidp gateway presets rm PRESET_ID         # DELETE /api/presets/:id
aidp gateway presets start PRESET_ID      # POST /api/presets/:id/start
```

There is no `PUT` route for presets: `add` always `POST`s, and the server decides create-vs-update
from whether the JSON file's `id` is present and non-empty.

`aidp gateway queue <verb>` — Lot 4a:

```bash
aidp gateway queue list                   # GET /api/queue, one row per queued launch
aidp gateway queue list --project-id ID   # GET /api/queue?projectId=ID
aidp gateway queue promote QUEUE_ID       # POST /api/queue/:id/promote
aidp gateway queue cancel QUEUE_ID        # POST /api/queue/:id/cancel
```

There is deliberately no `queue add`: enqueueing only ever happens implicitly, through
`POST /api/sessions` when the target project is already busy.

`aidp gateway dispatcher <verb>` — Lot 9a:

```bash
aidp gateway dispatcher status [--url] [--format]   # GET /api/dispatcher
aidp gateway dispatcher pause  [--url] [--format]   # POST /api/dispatcher/pause, sends {"paused": true}
aidp gateway dispatcher resume [--url] [--format]   # POST /api/dispatcher/resume
aidp gateway dispatcher tick   [--url] [--format]   # POST /api/dispatcher/tick
```

There is deliberately no `aidp gateway dispatcher budget` in this Lot. The budget explainer is
reached through `aidp ticket budget [--url] [--format]` — served, like `aidp ticket start` and
`aidp ticket dispatch`, by `aidp-cli`'s gateway client rather than `aidp-tickets` (which stays
HTTP-free) — or directly through `GET /api/dispatcher/budget`.

`aidp ticket start` and `aidp ticket dispatch` also reach the daemon — through `aidp-cli`, because
`aidp-tickets` deliberately carries no HTTP client of its own. `aidp ticket plan` is the exception:
it answers locally with an empty running set and never calls the daemon.

`aidp gh <verb>` — local review loop:

```bash
aidp gh review-post <repo> <n> --findings <file> --pass <n> --run <id>
aidp gh thread-reopen <threadId> --body-file -
```

`review-post` submits one `COMMENT` review: one inline comment per in-diff finding, and the
out-of-diff findings in the review body with their location. Each comment starts with the marker
`<!-- aidp:finding id=<fid> run=<run_id> pass=<n> -->`; a finding whose marker already exists for
the same run and pass is skipped, so a retry posts nothing twice. It prints
`{posted, review_id, threads[{finding_id, thread_id, path, line}]}`.

`thread-reopen` replies to the thread with the body read from the file (or stdin for `-`), then
unresolves it. It prints `{thread_id, replied, unresolved}`.

## Decisions and non-goals

- **No shell spawns `claude` (LOC-24).** `cmd /C` is never used, on any platform, so batch
  escaping covers every argument. `model` is guarded by a character allow-list rather than an enum,
  so new model ids need no code change. De-duplicating the three PATH resolvers and a
  `ProcessRunner` seam are LOC-32/LOC-36, not this Lot.

- **`GET /api/health` is deliberately not exercised in in-process tests.** It shells out to `<current_exe> doctor`, which in a test binary is the test binary itself, and costs up to ~5s on a cold cache. Integration tests that verify the health route exist; in-process harnesses omit that route.
- **A session fixture missing a non-`serde(default)` field is silently dropped by `SessionManager::load`.** The loader deserializes to `Option<SessionRecord>`, and an incomplete fixture rounds to `None` — the entire session file is skipped without error or warning. Fixtures used in tests must be complete and structurally valid, and the test must assert the fixture's presence before relying on it.
- **`unread` defaults to `false` and `PATCH` is its only writer in this Lot.** Auto-marking a
  session unread when new output arrives (and clearing it on view) is a viewer-side policy left to
  Lot 1b — the gateway only stores and returns the flag.
- **`permissionMode` is optional on a preset.** `Presets::start` passes it through unchanged to
  `StartRequest`, and `SessionManager::start` itself falls back to `"default"` when it's absent —
  the same fallback every other launch path already relies on.
- **`PATCH /api/sessions/:id` is permissive, not strict.** Unknown keys are silently ignored rather
  than rejected, matching the existing `PUT /api/automations/:id` shallow-merge precedent — the
  alternative (400 on any unrecognized key) would make the route brittle against a viewer sending a
  superset of fields.
- **Three `http.rs` plumbing repairs landed in this Lot, all pre-existing gaps rather than new
  behavior:** `PATCH` bodies are now actually read (`method_has_body` covers `Post`/`Put`/`Patch`,
  previously only `Post`); `PATCH` is now in the CORS `Access-Control-Allow-Methods` list; and a
  `Reply::Text` variant now lets a route answer a non-JSON content type (used by `export.md`) without
  reworking `Reply::Stream` or the SSE path.
- **`aidp-gateway` does not depend on `aidp-core` for Lot 2a.** `aidp_core::gitlog::changed()`
  returns bare paths with no per-file status and no insertion/deletion counts, and its `run_git`
  helper is private, so `changes.rs` carries its own local `git` shell-out instead. The duplication
  is deliberate; the crate stays standalone.
- **`aidp-integrations::gh` was left untouched.** It has no working-directory parameter (only
  `--repo`), and its `current_branch()` runs in the *daemon's* cwd, not the session's — so this Lot
  shells out to `gh` locally with the session's cwd instead. Adding a `cwd` parameter to the shared
  helper would change public signatures called elsewhere in the workspace; that is a separate piece
  of work, not this Lot's.
- **`session.changes` is computed off the hot path.** `gh` is never invoked per turn — only on
  `GET /api/sessions/:id/changes` itself.
- **The CLI verbs `aidp gateway inbox` / `aidp gateway changes` are not part of this Lot.** The
  acceptance criteria ask for the HTTP contract, Rust tests and documentation; CLI clients for these
  two routes remain a follow-up.
- **`GET /api/sessions/:id/run` has no foreign key and stores nothing on the session record.** The
  run link is recomputed per request from `workspace_root` + `started_at`; nothing is written back
  to `SessionRecord`, so there is nothing to keep in sync when a run is replayed or moved.
- **`POST /api/automations/:id/test` persists nothing** — no `AutomationRun`, no JSONL line, no bus
  event, no change to the automation's `run_count`/`last_run_at`/`last_status` — by design, so
  exercising a trigger never pollutes the Historique view or the inbox's `skippedAutomation`
  aggregation.
- **The dry-run evaluates the real utilization window, on purpose.** `test_automation` calls the
  same `utilization_block` as a real run, so the same call can answer `skipped` on one invocation
  and not the next depending on the live 5h budget. This makes the dry-run non-deterministic, which
  is accepted deliberately: an honest trace against the real guardrail is worth more than a
  reproducible one that lies about whether the automation would actually fire.
- **A dry-run's planned delivery, for a `launch`/`prompt` action, is hypothetical.** `test_automation`
  never starts a session, so its `delivery` trace step is built on the sessionless branch
  (`outcome_for_delivery` derived from `wouldRun`/`status` alone) — it cannot see the real delivery
  outcome, which for those two action kinds is only settled asynchronously later, off
  `session.ended` / `session.question`.
- **No CLI verb was added for automations in this Lot.** `POST /:id/run`, `POST /:id/test`,
  `GET /:id/runs` and `GET /automation-runs` are HTTP-only; the CLI verbs section above is
  unchanged.
- **A queued launch is not a session.** No `SessionRecord`, no id in the session index, and above
  all nothing in the live map — `running_count()` stays honest and no existing consumer that
  filters on `status` had to be audited. The consequence for the viewer is that a queued launch
  appears in `GET /api/queue`, not in `GET /api/sessions`.
- **The queue lives inside `SessionManager`, not on the HTTP `App`.** `start()` stays the single
  entry point and decides under one admission mutex, so there is no check-then-spawn race between
  the route and the manager.
- **`.aidp/run-in-flight` is deliberately not reused.** That file marker guards a CLI pipeline run
  and belongs to a different process model; driving the daemon from it would create two sources of
  truth. The one-session-per-repo rule is re-implemented natively.
- **The dollar estimate needs no price table.** `SessionRecord.costUsd` already comes from the
  CLI's own `total_cost_usd`; the aggregate sums it. The crate stays standalone — no dependency on
  `aidp-core`, no vendored model prices.
- **`GET /api/usage` filters by date in the aggregation, not in `SessionManager::search`.** `search`
  was left untouched and the usage module stays a pure function over the records.
- **BEHAVIOUR CHANGE: the daemon now restores the rate-limit window at boot.** Until this Lot,
  `rate_limit` was in memory only and initialised to `None`, so `utilization_block` returned `None`
  (= no block) and every automation launched between a daemon restart and the first
  `rate_limit_event` ran with NO utilization guard, whatever the real level of the 5h window.
  `SessionManager::load` now seeds it from the newest persisted snapshot whose 5h window has not
  yet reset. This is a deliberate repair, not a side effect of persisting the history: the guard now
  fires in a case where it previously never could.
- **The automatic dequeue is not re-checked against the 5h utilization guard.** The guard applies at
  launch time; a dequeue is the continuation of an already-admitted launch.
- **An automation whose launch is queued records a `queueId` instead of a
  `sessionId`.** Its delivery (which keys off `session.ended` by session id) is therefore not wired
  for that run — a known limitation of this Lot, not an oversight.
- **The auth gate is one central check in `handle()`, not a per-route guard.** `route()` grows a
  new match arm every Lot; a per-route check would eventually be forgotten on a new one.
- **Loopback stays open on purpose.** The viewer is still tokenless and `aidp gateway <cmd>` is
  still a loopback client, so `AIDP_UI_TOKEN` protects an *exposed* daemon, not a local one — it is
  not, and is not meant to be, a general authentication layer.
- **`ui_token` and `hook_token` stay two distinct secrets.** Neither satisfies the other's check;
  rotating one never silently weakens the other.
- **`/api/hooks` is exempt from read-only mode**, and this is a known limitation, not an oversight:
  a webhook-triggered launch is a Rust-internal call inside the daemon, not a second HTTP request,
  so **read-only mode does not stop a GitHub push from starting a session.**
- **`aidp doctor` is a shell-out, deliberately.** The gateway crate has no `aidp-core` dependency,
  and `fn doctor` is private to the bin crate, so an in-process call is impossible without either
  dependency.
- **`GET /api/health` is gated even though that limits its own usefulness as a diagnostic.** A
  misconfigured `AIDP_UI_TOKEN` makes the health check itself 401 from off-loopback — accepted
  because the operator checking it is expected to be on loopback.
- **No gateway section exists yet in `USER-GUIDE.md` or `README.md`.** Creating one is out of this
  Lot's scope; `docs/gateway-api.md` remains the only contract document.
- **A chain that would revisit its own ancestry, or exceed `AIDP_MAX_CHAIN_DEPTH` (default 3), is
  refused loudly.** The start call fails outright rather than launching the session unchained —
  silently dropping the context here would hide a real configuration mistake behind an
  indistinguishable "just didn't chain" session.
- **Every other `contextFrom` failure mode degrades gracefully instead.** An unknown session id, an
  unresolvable run, a run still parked on a `pending_gate`, or an empty resulting block all launch
  the session with no context block and no error — chaining is an enhancement to the prompt, not a
  precondition for the session to start.
- **`maxBudgetUsd` inheritance copies the cap, not a shared envelope.** A chained launch with no
  explicit `maxBudgetUsd` inherits the parent session's cap value; an explicit request value always
  overrides it. The parent's `costUsd` is never subtracted from anything — it is the CLI's own cost
  estimate, informational only on a subscription, not a wallet to draw down.
- **`contextFrom` is offered on `Action::Launch` only, not on `Action::Prompt`.** A prompt action
  replies into a session that is already running; chaining injects context into a session's
  *opening* prompt, which only a launch has.
- **The dispatcher never writes `queue.json`.** Its only write on that file is cancelling its own
  entry after losing an admission race — everything else about the queue stays `queue::admit`'s job
  (see [Queue endpoints — Lot 4a](#queue-endpoints--lot-4a)).
- **The dispatcher does not reimplement one-session-per-project.** `queue::admit` already owns that
  rule; the dispatcher composes on top of it instead of duplicating it.
- **The slot ceiling counts only `origin.kind == "dispatcher"` sessions.** A UI session still
  occupies its project — one session per project stands regardless of origin — but it does not
  consume a dispatcher slot; the two counters are deliberately separate.
- **Two `Verdict` types, on purpose.** `aidp_tickets::plan::Verdict { max_launches, reason }` is the
  flat contract `plan::plan` consumes; `aidp_tickets::budget::Verdict` is the rich four-variant enum
  (`Allow`, `Conservative`, `Wait`, `Paused`) the budget guard produces. The gateway adapts the
  second into the first (`to_plan_verdict` in `dispatcher.rs`). They are not unified because
  `aidp-tickets` must stay free of any dependency on `aidp-core` and `aidp-gateway`, and because
  `plan::Verdict`'s shape is already asserted by `tests/plan_dispatch.rs`.
- **`dispatcher.resumed` reused rather than renamed.** The budget wait reuses the event name the
  docs already promised (`POST /api/dispatcher/resume`), disambiguated by a `reason` field in the
  payload, rather than minting `dispatcher.budget_resumed`. The consequence is recorded honestly
  above: `POST /api/dispatcher/resume` emits `dispatcher.paused_changed`, so a consumer seeing
  `dispatcher.resumed` can only be seeing a budget resume.
- **`utilization_block` is a separate mechanism, untouched by this Lot.** The automations engine's
  own usage guard (threshold `0.9` on the 5h window by default, a blocked run reported as
  `skipped`) lives on a separate code path from the dispatcher's budget guard and this Lot leaves it
  as-is. Structurally, the project never works around a rate limit by switching to a billed API key
  — the budget guard is the whole answer to running out of window, not a stopgap until one is
  bought.
- **The journal file is the source of truth for progress, not the `stream-json` stream and not
  `run-state.json`.** Three candidates were measured before choosing. The `stream-json` stream does
  carry signal during a background Workflow run — `system` messages with subtype `task_started` and
  `background_tasks_changed`, and `{"type":"tool_progress","heartbeat":true,...}` lines landing in
  the session's generic `raw` event arm — but none of them names a phase, an agent or a run, and the
  gateway's `system` handler keeps only `subtype`/`sessionId`/`model`/`tools`. The stream gives
  liveness, never progress — present but insufficient, not absent. `run-state.json` moves only at
  pause and at end, since the emitted Workflow script never re-saves state while it runs, so the
  existing `session.run` poll cannot see inside a running stage; it stays as-is and is unaffected by
  this event. `journal.jsonl` is written incrementally: it was observed holding a `started` line for
  a stage that was still running, with its `result` line not yet present, and its full line
  vocabulary is `launched`, `started`, `result`, `failed`. The run's transcript directory is named
  outright in the Workflow tool result, so the gateway resolves it by parsing that text — no
  workspace-path encoding and no directory globbing — and as a consequence `aidp-gateway` still does
  not depend on `aidp-core`, and did not need to duplicate `tokens.rs`'s path logic either.

## Pipeline doctor

The daemon investigates a broken flow or run by itself; see [doctor.md](doctor.md). Both routes are
reads, allowed in read-only mode, and answer `503` when the daemon has no flow engine (no vault).

| Route | Answer |
|---|---|
| `GET /api/flows/:key/doctor` | `200 {key, enabled, incidents}`: the incidents of flow or ticket `key`, newest first. `404` when the key has no flow run and no incident. |
| `GET /api/doctor` | `200 {settings, incidents}`: the `doctor:` settings in force and every incident, oldest first. |

An incident:

```json
{ "id": "GH-214/implement@1", "key": "GH-214", "ticket": "GH-214", "trigger": "flow_failed",
  "step": "implement", "iteration": 1, "runId": "20261109-101500-a1b2",
  "detail": "implement failed (pipeline-failed): pipeline run failed",
  "status": "fixed", "detectedAt": "…", "updatedAt": "…",
  "investigationRunId": "20261109-101630-c3d4", "investigatedMs": 1794241107819,
  "bundle": "D:/Vault/Pipelines/pixelhood/GH-214/doctor/GH-214_implement_1",
  "diagnosis": { "category": "false_failure", "cause": "…", "evidence": [], "confidence": 0.85,
                 "action": "retry_step", "params": {"step": "implement"}, "needs_human": false,
                 "summary": "…" },
  "outcome": { "action": "retry_step", "idempotencyKey": "GH-214/implement@1:action:1", "ok": true,
               "detail": "retried `implement`", "at": "…" },
  "actions": [ { "action": "retry_step", "idempotencyKey": "GH-214/implement@1:action:1", "ok": true,
                 "detail": "retried `implement`", "at": "…" } ],
  "live": true, "activityMs": 1794241107819,
  "note": "retry_step : retried `implement`", "commented": true }
```

`outcome` is the latest action, `actions` all of them (3 at most, one idempotency key per attempt).
`live` and `activityMs` order the incidents (live flows first, newest first). `nextAttemptMs` is the
earliest time a passing fault is resumed again; `prUrl` the pull request opened for a harness patch.

`trigger`: `flow_failed`, `flow_escalated`, `run_paused`, `step_stuck`, `waiting_too_long`, `bugs_not_converging`. New incident fields: `sharedPattern`, `affected[]`, `rearmedMs`.
`status`: `detected`, `investigating`, `fixed`, `action_failed`, `needs_human`, `refused`,
`no_action` (also an obsolete incident, note `obsolete: …`), `investigation_failed`, `capped`.
`diagnosis.action` is one of `retry_step`, `requeue_to_test`, `requeue_dev`,
`abort_dead_flow_and_requeue`, `update_pr_branch`, `release_unity_lock`, `create_bug_ticket`,
`open_harness_pr`, `restore_test_infra`, `none`. New categories: `test_infra_regression`, `branch_behind_main`, `shared_harness_failure`, `real_defect`. `diagnosis` keeps the snake_case keys of
`diagnosis.json`. Optional fields are absent until set.

### Event `doctor.incident`

Published with the incident above when it opens, when its investigation starts, and when it
closes (or its note changes while it waits for budget).

## Unity editor lock and status

Both routes work on one registered project: `?project=<id or name>` (or `"project"` in the POST body). Without it, the only registered project is used; with several, the only one holding `ProjectSettings/ProjectVersion.txt`; otherwise HTTP 400. An unknown project is 404.

### `GET /api/unity/status[?project=]`

```json
{
  "project": "p1",
  "editor": { "running": true, "pid": 21156, "projectPath": "D:/CLAUDE_UNITY/mixandjam-celestemovement",
              "heartbeat": { "at": "2026-11-06T16:00:02.120Z", "state": "idle", "activeScene": "Assets/Tests/Playtest/Scenes/PlaytestArena.unity", "dirty": false, "playtest": null, "compileFailed": false, "staleAfterSeconds": 15, "lastError": null, "lastGuard": { "at": "...", "steps": ["OpenEditScene", "RefreshAssets"], "skipped": null, "activeScene": "..." } } },
  "lock": {
    "held": true,
    "holder": { "flowKey": "GH-214", "ticketKey": "GH-214", "stepId": "implement", "since": "2026-11-06T16:00:00.000Z" },
    "waiting": ["GH-215"]
  },
  "lastCompile": { "id": "...", "ok": true, "errors": [], "warnings": 3, "durationMs": 8400, "unityVersion": "6000.6.7f1", "finishedAt": "..." },
  "lastPlaytest": { "id": "issue-214-ca2", "passed": true, "finishedAt": "..." }
}
```

- `editor.running` is true when a `Unity.exe` process has `-projectpath <path>` on its command line equal to the project's test checkout (`testCheckout`), else its path; `editor.projectPath` is that path, and the results below are read under it.
- Case and `/` against `\` are ignored, as is a trailing separator. When no editor runs, `pid` is `null`. The process list is read with PowerShell (`Get-CimInstance Win32_Process`) on Windows and `ps` elsewhere, and cached for 5 seconds.
- `editor.heartbeat` is `<project>/.aidp/editor/heartbeat.json` as the playtest package writes it every two seconds (`state`: `idle`, `compiling`, `importing`, `compile-failed`, `entering-play`, `playing`, `paused`, `reloading`, `quitting`), `null` when the file is absent. An `at` older than `staleAfterSeconds` with `editor.running` true and a state other than `reloading` means the editor's main thread is stuck (dialog box, freeze); `aidp flow unity-refresh` reports it as `blocked` on that basis.
- `lock.holder` is `null` when the lock is free; `lock.waiting` lists the run keys queued, first served first.
- `lastCompile` is the newest `*.json` under `<project>/.aidp/compile/results/`, as written by the compile gate. `lastPlaytest` is `{id, passed, finishedAt}` from the newest `*.json` under `<project>/.aidp/playtest/results/`; `finishedAt` is the file's own field when present, else its write time. Both are `null` when there is no readable result.

### `POST /api/unity/lock/release[?project=]`

Frees the `unity-editor:<project id>` lock whoever holds it, for a lock whose holder is gone. Mutating: refused in read-only mode. Body: optional `{ "project": "<id or name>" }`. Response:

```json
{ "ok": true, "released": true, "resource": "unity-editor:p1", "holder": { "flowKey": "GH-214", "ticketKey": "GH-214", "stepId": "implement", "since": "..." } }
```

`released` is `false` and `holder` `null` when nobody held it. The release is logged (`warn`, `unity lock released by hand`) and journaled on the holder's flow run. Returns 503 when the daemon has no flow engine. The next waiter is woken at once.

### Event `unity.lock_changed`

Published on every change of the holder or of the waiters:

```json
{ "resource": "unity-editor:p1", "held": true, "holder": { "flowKey": "...", "ticketKey": "...", "stepId": "...", "since": "..." }, "waiting": ["..."] }
```

`holder` is `null` when `held` is false.

## Settings — `/api/settings/{routing,budget,models}`

The three routes read and write `TODO/routing.yaml` and `TODO/config.yaml` of the vault as JSON. **Unlike the rest of the API, the JSON keys are the YAML keys, `snake_case`**, so the page and the files speak the same names; only the reply flags (`reloadRequired`, `changed`) are camelCase. A missing file reads as the defaults; `503 config` when the daemon has no vault.

`PUT` takes a partial body. Every value is checked first: the keys and types against the daemon's own config types, the ranges, the times, the regular expressions and the route, track and tier names. Refused values are reported together, nothing is written, and the answer is `400`:

```json
{ "error": "invalid settings", "code": "bad_request",
  "errors": [ { "path": "budget.per_ticket.code", "message": "must be a token count: a number, or text such as 3M or 500k" } ] }
```

`path` is dotted, with `[i]` for a list index (`rules[2].match.summary_regex`, `repos.pixelhood-art.route`, `models.code.report`). A file that exists but is not valid YAML answers `409 conflict` and is left alone.

On success the answer is `200` with the same body as `GET`, plus `"reloadRequired": false` and `"changed"` (`false` when the body changed nothing: the file is then not rewritten). `reloadRequired` is always `false`: the dispatcher rereads `config.yaml` on every tick, the model policy is frozen into each run when it starts, and the routing file is read by each sync. A change publishes the SSE event `settings.changed` with `{ "section": "routing" | "budget" | "models" }`. `PUT` is refused in read-only mode (`403`).

The write is atomic (a `.settings-tmp` file in the same directory, then a rename) and serialized with the other settings writes. `budget` edits patch the lines of `config.yaml` in place, so comments, order and the other sections survive; `routing` and `models` replace only the sections the body carries (`defaults`, `rules`, `repos` / the `models:` block), keep the rest of the file, and lose the comments inside a replaced section. A section whose content is unchanged is not touched.

### `GET /api/settings/budget`

```json
{
  "budget": {
    "max_utilization_5h": 0.8, "max_utilization_7d": 0.9,
    "reserve_per_run_5h": 0.1, "reserve_per_run_7d": 0.02,
    "unknown_policy": "conservative", "learn": true,
    "per_ticket": { "code": 3000000, "art": 1500000, "map": 2000000, "doc": 500000, "default": 2000000 },
    "pixellab": { "min_reserve": 0, "per_ticket": 10 }
  },
  "dispatcher": {
    "slots": 3,
    "test_slots": 1,
    "bug_slots": 1,
    "stop_at_7d_percent": null,
    "night": { "enabled": false, "start": "22:00", "end": "07:00", "heavy_after_reset": true, "heavy_window_minutes": 30 }
  }
}
```

Token amounts are integers on read. `PUT` accepts an integer or text such as `"3M"`, `"1.5M"`, `"500k"` (written as typed). `0` disables a ceiling. `max_utilization_*` are fractions in `(0, 1]`; `reserve_per_run_*` fractions in `[0, 1]`; `unknown_policy` is `conservative`, `allow` or `block`; `slots` is `1..32`; `stop_at_7d_percent` is a percentage in `(0, 100]` (`85` = 85 %), `null` removes the key; `night.start` and `night.end` are local `HH:MM`; `heavy_window_minutes` is `1..300`; `pixellab.per_ticket` is a whole number. Other `dispatcher` and `budget` keys (for instance `rate_limit_patterns`) stay hand-edited.

### `GET /api/settings/models`

```json
{
  "models": { "code": { "report": "haiku", "review": "sonnet" }, "art": { "review": "sonnet" } },
  "routes": ["code", "art", "map", "doc"],
  "stages": ["prepare", "plan", "implement", "repro", "review", "verdict", "fix", "test", "report", "brief", "generate", "animate", "import", "graph", "rooms", "write"],
  "tiers": ["haiku", "sonnet", "opus", "fable"]
}
```

`routes`, `stages` and `tiers` list the usual words and are read-only. `PUT` body: `{ "models": { "<route>": { "<stageId>": "<tier>" } } }`; it replaces the whole policy (an empty object or `null` removes the `models:` block). The route must be one of `routes`, a stage id any `[A-Za-z0-9_-]+` name, the tier one of `tiers`.

### `GET /api/settings/routing`

```json
{
  "defaults": { "track": "standard" },
  "rules": [
    { "match": { "labels": ["pixelart", "art", "pixel:*"] }, "service": "pixelhood-art" },
    { "match": { "keywords": ["sprite", "tileset"] }, "service": "pixelhood-art" }
  ],
  "repos": {
    "pixelhood-art": { "route": "art", "flow": "game-art" },
    "pixelhood-bug": { "route": "code", "flow": "game-code", "track": "bug" }
  }
}
```

Keys that are unset are omitted. A rule's `match` takes `component`, `label`, `epic`, `key_prefix`, `summary_regex`, `issuetype` (texts) and `labels`, `keywords` (non-empty lists of texts); it must set at least one. A rule names its target as `service`; `route` is accepted as a synonym on `PUT` but never both, and the answer always says `service`. A `repos` entry takes `template`, `flow` (non-empty texts), `track` (`quick`, `standard`, `deep`, `bug`) and `route` (`code`, `art`, `map`, `doc`). `defaults` takes `track`. A `PUT` section replaces the stored one as a whole: send the full `rules` list or the full `repos` map. Unlike the tolerant loader, which drops a bad rule with a warning, `PUT` refuses it.

## Runtime MCP configurations

`POST /api/runtime/mcp-configs` requires the UI token, including loopback calls.
The body is `{ "id": "host", "servers": [], "environment": {} }`; `servers` accepts named
`stdio` definitions (`command`, `args`, `env`) and `http` definitions (`url`, `headers`).
The response is `{ "id": "host" }`. Identifiers are public strings of up to 128 ASCII letters,
digits, underscores, dots or hyphens. Upserting an identifier atomically replaces its configuration;
already launched sessions retain their original snapshot. There is no API to read definitions.

Pass `mcpConfigId` in session and native-run requests or flow `policy`. Definitions and environment
values remain in runtime memory; persisted requests, status and flow state carry only the identifier.
An unavailable reference refuses the launch instead of loading ambient MCP configuration.
An in-process host (`serve_on`) passes `initial_mcp_configurations` before flows and native runs
recover; hosts must supply their named configurations again after a restart.
Gateway status advertises `capabilities.mcpConfigurationReferences: true`.


## Project memory and RAG

Backed by `aidp-memory` (see [memory.md](memory.md)). One SQLite store is opened on first use under
the state directory (`memory/memory.db`) and shared by every request, with a single writer
connection. The project is the registered project: `?project=<id or name>` (or `project` in a JSON
body), else the only registered one, else the only Unity one; with several candidates the answer is
`400`. The memory key is the project name. Writes (`POST`, `PATCH`) are refused in read-only mode.
Errors use the usual `{error, code}` body: `404` unknown memory or project, `409` refused
transition (approving an archived memory, merging into a dead one), `400` invalid input.

The memory object is the C10 one, camelCase: `id`, `project`, `kind`, `title`, `text`,
`scope {route?, paths, tags}`, `refs [{type, id}]`, `confidence`, `evidence`, `status`, `supersedes`,
`createdAt`, `updatedAt`, `lastUsedAt`, `uses`.

| Route | Body | Answer |
|---|---|---|
| `GET /api/memory?status=&kind=&q=` | | `[memory]` in id order |
| `GET /api/memory/:id` | | `memory` |
| `POST /api/memory` | `{kind, title, text, scope?, refs?, confidence?, status? proposed\|active, supersedes?}` | `201 {memory, deduplicated, activated}` (`200` when it grew an existing memory) |
| `PATCH /api/memory/:id` | any of `kind, title, text, confidence`, and `scope {route, paths, tags}` (or `route`, `paths`, `tags` at top level) | `memory` |
| `POST /api/memory/:id/approve` and `/archive` | | `memory` |
| `POST /api/memory/:id/merge` | `{into}` | the surviving memory |
| `GET /api/rag/search?q=&source=&budget=&project=` (also `k`, `path`, `domain`, `agentType`, `lexical`) | | `{project, query, usedTokens, budgetTokens, dropped, hits}` |
| `POST /api/rag/reindex` | `{project?, source? "bible,doc,...", full?}` | `202 {ok, started, project}` |

A hit is `{id, source, path, lines: "L10-L24", citation: "path:L10-L24", title, score, via, text}`;
`budget` is in tokens (characters / 4, default 1500). Search is built like the CLI one: hybrid (BM25
fused with cosine, `via` is `bm25`, `vector`, `hybrid` or `graph`) when the embedding model is
running in the daemon (see [Embedding model service](#embedding-model-service)), BM25 otherwise; `lexical=1` forces BM25. The vault glossary
(`Knowledge/_glossary.md`) widens the query. `domain` (comma separated knowledge domains, `400` for an
unknown one) and `agentType` filter the `knowledge` hits as `rag search --domain/--agent-type` do; without
`domain`, the domains of the agent type apply. Re-indexes embed new and changed chunks when the model is
installed. `POST /api/rag/reindex` runs on a background
thread, one re-index at a time (`409` while one runs or waits); the result arrives as `rag.reindexed`.

### Ingestion at the end of a run

When a native run ends (`run.ended`, any terminal status), every `memory-proposals.json` under its
run folder (6 levels deep) is ingested as `proposed` memories through the de-duplication of the
manager. References are completed from the run: the ticket (`issue`, `214` becomes `GH-214`), the
pull request input (`pr`) and the run id (`run`). A file is ingested once: a ledger
(`memory/ingested.json`, keyed by path and content hash) skips it afterwards, and a modified file is
ingested again. A refused file is logged (`warn`) and never fails the run.

### Knowledge proposals

Every `knowledge-proposals.json` under the run folder (same depth, same ledger, same references rules
for the `run:<id>` source) is reconciled with the knowledge notes of the vault (see
[memory.md](memory.md#knowledge-proposals)) when a vault is configured: a note is added, merged into an
existing one as an evidence line, parked in `<vault>/Knowledge/_inbox/`, rewritten or superseded. The
file is marked ingested even when refused (its content cannot improve by retrying); refused files and
proposals are logged. `knowledge.changed` is published when a note changed.

### After a run

On `run.ended`, after the ingestion, the daemon refreshes the vectors of the memories of the project and
of `_global`, runs `memory gc`, then queues a re-index of the `run`, `ticket` and `knowledge` sources of the
project (the `_global` knowledge notes and the embedding of new chunks included) on the same single
worker (`rag.reindexed` carries `"trigger": "run"`).

### Catch-up at start

At start the daemon scans `<vault>/Pipelines/<repo>/` (4 levels) of every registered project for
`native-run.json` records with an `endedAt` in the last 30 days (200 runs at most, newest first), ingests
each run exactly as above (the ledger skips what was already ingested) and queues one re-index per
project (`"trigger": "catch-up"`). It runs on its own thread; nothing waits for it.

### Re-index after a merge

A successful `gh.merge` flow action publishes `flow.merged` (`{key, repo}`); the daemon then
re-indexes the `code`, `doc` and `bible` sources of the project whose repository matches, on the
background thread (queued behind a running re-index; the pass only reads files whose size or mtime
changed). `rag.reindexed` carries `"trigger": "merge"`.

### Embedding model service

The embedding model (`potion-multilingual-128M`, about 0.5 GB of RAM once loaded) exists as one
instance on the machine, owned by the daemon. The CLI (`rag index|search`, `memory context`), the
compile-time knowledge blocks of `aidp run` and `workflow emit` and the `SubagentStart` hook never load
it: they embed through `POST /api/embed` when the model is running and search with BM25 plus the glossary
otherwise. The daemon uses the same instance for its own search, ingestion and native runs.

The state is `stopped`, `loading` (a background thread reads the files), `running` or `error` (the load
failed, `error` holds the message). Stopping drops the model and frees its memory, also while it loads.
`autostart` is saved in `<state dir>/memory/model.json` (`{"autostart": bool}`); it defaults to true when
the model is installed, and the daemon loads the model at start when it is true. Errors use the flat
`{error, code}` body.

| Route | Body | Answer |
|---|---|---|
| `GET /api/memory/model` | | `200` status body |
| `POST /api/memory/model/start` | | `202` status body (`loading`, or `running` when already); `409` code `model_missing` when the files are not installed |
| `POST /api/memory/model/stop` | | `200` status body (`stopped`) |
| `PATCH /api/memory/model` | `{autostart}` | `200` status body; `400` on a bad body |
| `POST /api/memory/model/embed-pending` | | `202 {queued: true}`; `409` code `model_stopped` when the model is not running |
| `POST /api/embed` | `{texts: [..]}` | `200 {model, dim, vectors: [[f32..]]}`; `409` code `model_stopped`; `400` code `batch_too_large` over 256 texts |

The status body is `{model, installed, state, autostart, dim, ramBytes, loadedAt, error, chunks: {total,
embedded, stale}, path}`: `dim`, `ramBytes`, `loadedAt` and `error` are `null` when unknown. `ramBytes` is
an estimate, the size of `model.safetensors`, set while the model is `loading` or `running`. `loadedAt`
is RFC 3339. `chunks.embedded` counts the vectors of this model, `stale` those of another model, `total`
every chunk. `embed-pending` queues, on the re-index worker, the embedding of every chunk that has no
vector of the running model, in every project; the result arrives as `rag.reindexed` with `trigger:
"embed-pending"`. A daemon started with the model already installed loads it in the background: chunks
indexed before it finished loading get their vectors with `embed-pending`. In read-only mode `POST
/api/embed` still answers (it changes nothing); start, stop, patch and embed-pending are refused with `403`.

### Events

| Event | Data |
|---|---|
| `memory.proposed` | `{project, runId, files, created, deduplicated, activated, rejected}` after an ingestion; `{project, ids, created}` after `POST /api/memory` of a proposal |
| `memory.changed` | `{id, project, status, action}`, `action` in `added`, `edited`, `approved`, `archived`, `merged` |
| `knowledge.changed` | `{project, runId, added, merged, inboxed, updated, superseded, rejected}` after the ingestion of `knowledge-proposals.json` files that changed a note |
| `memory.model` | the status body of `GET /api/memory/model`, on every state change of the model and on a change of `autostart` |
| `rag.reindexed` | `{ok, project, trigger: "api"|"merge"|"run"|"catch-up"|"embed-pending", indexed, updated, removed, unchanged, chunks, embedded, durationMs}`;|"merge"\|"run"\|"catch-up", indexed, updated, removed, unchanged, chunks, durationMs}`; on failure `{ok: false, project, trigger, error}` |
| `flow.merged` | `{key, repo}`, internal trigger of the post-merge re-index |
