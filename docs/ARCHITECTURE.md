# Architecture

This document describes the behavior present in the repository. A type or trait in a crate does not imply that the server or clients use it yet.

## Runtime Boundary

```text
CLI ───────────────┐
Desktop (Tauri) ───┼── line-delimited JSON-RPC 2.0 / TCP
                   ▼
              Local server
              /                    \
      RpcDispatcher       HTTP dispatch coordinator
              │                    │              \
              └────────────► Core/Scheduler   HTTP worker ── HttpEngine ── destination
                                      ▲              │
                                      └── progress/final result ─┘
                                      │
                              event pump / EventHub
                                      │
                         events.subscribe TCP stream
                                      ▼
                              Desktop event client
                 ┌─────┼───────────┐
  TaskService   Scheduler   TaskRepository
       │            │              │
  Task events   Queue/events   SQLite (server)
                    │           InMemory (injectable)
                    ▼
              EngineRegistry (Core library API)
             /              \
       InMemoryEngine     HttpEngine

Browser extension ── HTTP(S) POST /jsonrpc + CORS (same listener)
```

The server listens on `127.0.0.1:39100` by default. Its address is always bound to loopback in the current implementation; `--port` changes only the port. The CLI can connect to a configured bare TCP address or select TLS with `tls://host:port`, and the desktop UI accepts a server address. The same listener accepts line-delimited TCP JSON-RPC and one-request HTTP(S) `POST /jsonrpc` calls. HTTP requests require `Content-Type: application/json` and a bounded `Content-Length`; chunked requests and event subscriptions remain unsupported over HTTP(S). CORS responses are emitted only for validated Chrome, Firefox, or Safari extension origins; web-page origins are rejected before RPC dispatch.

The Server also has a `--managed` entry point that the Desktop does not call yet. It requires an explicit absolute data directory, validates versions and a Bearer secret from framed private stdin, then listens on an ephemeral loopback port. stdout reports one secret-free readiness frame. stdin closure or a shutdown command ends the process while leaving partial files for existing recovery. Shared Server state retains the data-directory lock so it is not released while a worker still holds the database. See [ADR 0006](decisions/0006-managed-macos-server.md) for this contract and the pending App supervisor.

## Crates and Responsibilities

| Crate | Implemented role |
| --- | --- |
| `domain` | Task ID, source, destination, and progress value types. |
| `task` | In-memory task service, validated state transitions, and task events. |
| `scheduler` | Explicit queue operations, priority, concurrency limit, retry policy, bandwidth-policy calculation, and scheduler events. It does not perform network I/O; the server dispatch coordinator uses its claims and limits to start compatible HTTP work. |
| `storage` | `TaskRepository` with in-memory and SQLite implementations. SQLite has a schema version and migration. |
| `resolver` | HTTP/HTTPS, magnet, and local-source classification and validation through a registry. |
| `engine` | Adapter/registry APIs, simulated in-memory engine, and blocking HTTP GET engine. |
| `core` | Coordinates task, scheduler, resolver, engine, repository, and plugin manager state. |
| `security` | Credential, TLS configuration, rate-limit, and credential-store types. |
| `protocol` | JSON-RPC request/response dispatch, task/server methods, error objects, and event envelopes. |
| `plugin` | Manifest/permission/capability types, provider traits, and a plugin-manager state machine. |
| `media` | Media and workflow data types, dependency ordering, and simulated in-memory jobs. |

## Task Flow

`Core::create_task` validates the source through `ResolverRegistry`, creates a `DownloadTask`, and writes it to the injected repository. `queue_task` adds it to the scheduler. The server dispatch coordinator runs after `task.queue`, after startup recovery, and after an HTTP worker completes or fails. It repeatedly selects the highest-priority queued HTTP/HTTPS task with a permitted, non-overlapping destination, claims a scheduler slot, resets its progress, clears the previous transfer error, and spawns the worker outside the Core mutex until no slot or eligible task remains. `task.start` remains a manual kick and compatibility method: it claims one queued HTTP/HTTPS task, returns its ID before transfer completion, and then invokes the same fill loop. Magnet and local-file tasks remain queued because no transfer engine supports them; they do not block compatible HTTP/HTTPS tasks behind them. `Core::start_next` remains an in-memory-engine default for library callers; `start_next_with_engine("http")` is also a library API.

The task state machine permits:

```text
Created     → Queued
Queued      → Downloading
Downloading → Paused | Completed | Failed
Paused      → Queued | Downloading
Completed   → Queued
Failed      → Retrying → Queued
```

The scheduler enforces its concurrency count when a task is claimed or resumed. `HttpEngine` reports a progress snapshot after each response chunk. The server persists an intermediate snapshot when at least 1 MiB has arrived since the previous write or 250 ms have elapsed, and always flushes the final snapshot before marking success `Completed`. On transfer failure, the server stores the latest error, while the scheduler requeues the task while the default three-retry budget remains; the dispatch coordinator starts the retry when a slot is available. A new claim clears the previous error and resets the task snapshot before the server restores any validated partial progress. After the retry budget is exhausted, the task stays `Failed`. Bandwidth policy currently calculates limits; the HTTP transfer does not apply them. Task and scheduler events are collected in Core, drained by a server event pump, and published to subscribed clients.

## Engines and Sources

The resolver accepts HTTP/HTTPS URLs, magnet URIs containing an `xt=urn:btih:` parameter, and existing local paths; it does not validate the magnet hash itself. The server routes only HTTP/HTTPS to a transfer engine. It rejects destinations inside the server data directory, symbolic-link destinations, and concurrent active transfers targeting the same canonical path. `HttpEngine` performs a blocking GET, follows up to five redirects, has a 10-second connection timeout and a 30-minute request timeout, and reports progress after each written response chunk. The server-owned HTTP worker keeps one hidden stable partial file beside each destination and an atomic JSON sidecar containing the source, destination, ETag or Last-Modified validator, and expected length. After restart it sends `Range` and `If-Range` only for matching metadata, and accepts only a matching `206 Partial Content` range. A `200` response, changed or missing validator, malformed `Content-Range`, or inconsistent length removes the partial response and starts a full download. A response without a validator may finish in the current process but cannot resume after restart. Validated partial data survives ordinary transfer errors and process exit; successful commit, cancellation, and task removal attempt to clean it up. The completed partial file is synced and exposed at the destination atomically without replacing an existing path; an existing destination fails the task. The server's direct HTTP worker path supports cooperative pause at a response chunk boundary, same-process resume, and destructive remove cancellation. Pause retains the partial file and response while the worker waits; remove cancels the worker, drops the partial file and sidecar, removes the task, and leaves an existing destination in place. A blocking response read can delay `task.pause` until the 30-minute request timeout. `task.remove` waits up to 30 seconds for the worker and returns an error if it has not exited; the worker can remain blocked until the HTTP timeout, after which removal can be retried. The generic `HttpEngine` adapter remains synchronous and advertises no pause/resume capability. The in-memory engine simulates lifecycle operations and does not transfer bytes. There is no magnet or local-file transfer engine.

Commit uses the platform's atomic no-clobber rename (`renamex_np` on macOS, `renameat2` on Linux, and `MoveFileExW` without replacement on Windows). If that operation is unavailable, the engine tries an atomic hard link. If the filesystem supports neither operation, commit fails without exposing an incomplete destination.

The cleanup of partial files and sidecars after success, cancellation, or removal is best effort. If the process exits after publishing the complete destination but before persisting `Completed`, recovery requeues the task and the no-clobber check reports an existing destination. The complete file remains intact; task-state reconciliation for that window is still open.

## Persistence and Recovery

`Core<R>` accepts a `TaskRepository`. The server locks `data_dir/nexum.lock` for its lifetime, opens `data_dir/nexum.sqlite` (default `./data/nexum.sqlite`), and calls `Core::recover` before accepting connections. A second server using the same data directory cannot start. The SQLite repository persists task metadata, progress snapshots, and the most recent transfer error. Recovery keeps created, completed, and failed tasks in their stored states; queued tasks remain queued, while previously downloading, paused, or retrying tasks are reset to `Queued` in memory and SQLite. The scheduler queue is rebuilt at normal priority; prior priority, order, and retry counts are not persisted. After recovery and listener startup, the dispatch coordinator automatically starts eligible queued HTTP/HTTPS tasks; unsupported sources or blocked destinations remain queued. A restarted HTTP transfer restores its validated partial byte count before dispatch. It resumes from that byte only when the HTTP validator and `206` range match; otherwise the partial response is discarded and the transfer starts from byte zero. Directory creation, lock acquisition, database opening, and recovery failures stop server startup.

## Protocol and Clients

The server reads one JSON-RPC request per TCP line and writes a response line for requests with IDs. It handles `task.start` and active HTTP control before the dispatcher; the dispatcher supports `task.get`, `task.list`, `task.create`, `task.queue`, `task.start`, `task.pause`, `task.resume`, `task.remove`, `server.version`, and `server.auth`. A successful `task.queue` dispatches eligible HTTP/HTTPS work after the Core operation; `task.start` remains available to manually kick one queued task. For an active server HTTP worker, `task.pause` waits for a chunk-boundary acknowledgement and persists `Paused`, `task.resume` wakes the same worker when a scheduler slot is available, and `task.remove` cancels the worker, waits up to 30 seconds for it to exit, and deletes the task after that wait succeeds. A timed-out removal returns an error and can be retried after the worker exits. `task.get` and `task.list` return `TaskView` values with the current persisted progress and an `error` field for the most recent transfer error; the field is cleared when a new attempt is claimed. There is no general task-update method. The protocol has a V1 version field and a version-inspection method, but no negotiated feature set or version enforcement beyond JSON-RPC `2.0` envelope validation. A client can open a dedicated TCP connection, send `events.subscribe`, receive an acknowledgement, and then receive `events.event` JSON-RPC notifications. Non-heartbeat notifications carry incremental task or scheduler data and have monotonically increasing server sequences; a heartbeat is sent every 15 seconds, reuses the current sequence, and does not represent a task update. The per-subscriber queue is bounded; a full queue disconnects that subscriber. There is no replay buffer: clients treat a disconnect or forward sequence gap as stale state, reconnect, and fetch a full snapshot; duplicate or stale sequence values on one connection are ignored.

The CLI uses the TCP protocol for task control and server inspection. A bare `host:port` keeps the plaintext compatibility path; `tls://host:port` creates a rustls client using the platform root store (including the `SSL_CERT_FILE`/`SSL_CERT_DIR` sources supported by `rustls-native-certs`) and validates the server name before any RPC or credential is sent. The Tauri 2 + React desktop app uses Tauri commands as a TCP JSON-RPC client, with a Motrix-style sidebar, Downloads filters, task Inspector, Add Download sheet, Settings, Notifications, and planned-capability pages. It opens a dedicated event subscription through a Tauri background thread, tracks event sequences per connection, ignores duplicate or stale notifications, reconnects on a forward gap, refreshes the task snapshot after debounced task or scheduler notifications, and uses the configured five-second idle or one-second active polling fallback when the stream is disconnected. A reconnect or sequence gap schedules a full snapshot; manual refresh policy disables the polling fallback. It pauses polling when the window is hidden and persists the Server address, refresh policy, and language choice through Tauri. For a loopback Server, each new download defaults to this Mac's system Downloads folder; the user can use the native folder picker and preview the destination assembled from the editable suggested file name. For a remote Server, the user enters an absolute folder path on the Server host; the App does not use this Mac's folder picker or local file-existence check. General settings show the active saved Server's configured credential scheme and allow saving or clearing its secret; those changes restart the event subscription and refresh the task snapshot. Desktop keyboard navigation, shortcuts, modal focus management, accessible labels, and reduced-motion behavior are implemented. CSS follows the system light/dark preference through `prefers-color-scheme`; React uses a typed English/Simplified Chinese message catalog for Desktop-owned visible and accessibility-facing text. The saved language choice can follow the first preferred language reported by the WebView (Chinese resolves to Simplified Chinese, other languages to English) or override it; older settings default to following the system. This presentation choice does not alter protocol values or Server requests, and raw Server/OS diagnostics remain verbatim. Manual theme and accent-color options remain planned. The Manifest V3 browser extension uses the HTTP bridge for `task.create` and `task.queue`, reads the popup's `server` storage key, maps bare addresses to HTTP and explicit `tls://host:port` addresses to browser HTTPS, handles link-badge messages, and resolves relative links before sending. The browser performs normal certificate and hostname validation, and the extension does not retry a failed TLS request over HTTP. It has no device selection or credential settings, so an authenticated server rejects its requests.

Protocol requests can carry one `Credential` value (`Bearer` or `ApiKey`). When `require_auth` is enabled, the server validates the request credential before any dispatcher method or `events.subscribe` handling. The configured scheme and secret must match exactly; missing, invalid, or mismatched credentials return JSON-RPC error `-32001` with message `authentication required`. `require_auth=true` requires a supported `auth_scheme` and non-empty `auth_token`; an incomplete or unsupported pair stops startup. `server.auth` reports the active scheme (`none`, `Bearer`, or `ApiKey`) without exposing the secret. The CLI stores `default_auth_scheme` and `default_auth_token` as plain text in its config. It attaches them to verified `tls://` RPC requests, while plaintext credentials are allowed only after the connected peer is confirmed to be loopback. On macOS, the Tauri backend stores the Desktop credential's scheme and secret under the active saved Server address in Keychain and attaches it to ordinary RPC and `events.subscribe`; the Keychain status command returns only the configured scheme, so React never reads back the saved secret. Desktop uses the system root store for `tls://` connections, validates the server name before sending credentials, and scopes Keychain accounts by transport identity; plaintext credential transport remains loopback-only. On other platforms, Desktop can connect without authentication but credential save and clear are unsupported. The server never logs credential secrets. Its `max_connections` setting limits active TCP connection handlers and closes excess connections before request processing. Server TLS can be enabled with paired `tls_cert_path` and `tls_key_path` settings; the listener loads the PEM material before accepting connections and carries TCP JSON-RPC, `events.subscribe`, and HTTP `/jsonrpc` over the same rustls stream. The staged TLS transport contract and rollout boundaries are recorded in [ADR 0005](decisions/0005-tls-transport.md).

Optional RPC rate limiting uses one process-wide token bucket shared by line-delimited TCP and HTTP `POST /jsonrpc`. It is disabled by default; the server requires both `--rate-limit-rps` and `--rate-limit-burst`, or both `rate_limit_rps` and `rate_limit_burst` in its config file, with valid positive values. Invalid or incomplete settings stop startup, as does an unreadable file explicitly supplied with `--config`. After parsing and authentication, every request, including `events.subscribe`, consumes a token. Authentication failures, server-generated event heartbeats, and HTTP `OPTIONS` preflight do not consume tokens. When the bucket has no token, calls with an `id` receive JSON-RPC error `-32002`; HTTP `/jsonrpc` keeps status `200` and carries that error in the response body. Notifications without an `id` are dropped without a JSON-RPC response.

## Extension and Media Boundaries

`PluginManager` registers manifests and tracks load/start/stop states, and the crate defines `EngineProvider` and `ResolverProvider` traits. Core does not call plugin implementations or register their adapters; `init_plugins` only advances manager state. Permission declarations are metadata, not an enforced sandbox. Dynamic loading and executable plugin integration are not present.

The media crate defines probe, manifest, track, segment, mux, pipeline, job, workflow, and MCP request types. Workflow dependency ordering and in-memory job bookkeeping are implemented. `MediaProcessor::probe` returns default metadata, while job execution fabricates a completed result; no file probing, manifest parsing, segment processing, muxing, external automation endpoint, or MCP integration is wired up.

For the Chinese version, see [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md).
