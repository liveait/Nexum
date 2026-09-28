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
                 ┌─────┼───────────┐
  TaskService   Scheduler   TaskRepository
       │            │              │
  Task events   Queue/events   SQLite (server)
                    │           InMemory (injectable)
                    ▼
              EngineRegistry (Core library API)
             /              \
       InMemoryEngine     HttpEngine

Browser extension ── HTTP POST /jsonrpc (no matching server endpoint yet)
```

The server listens on `127.0.0.1:39100` by default. Its address is always bound to loopback in the current implementation; `--port` changes only the port. The CLI can connect to a configured TCP address, and the desktop UI accepts a server address. The browser extension's HTTP request is incompatible with the current TCP-only server.

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

The scheduler enforces its concurrency count when a task is claimed or resumed. `HttpEngine` reports a progress snapshot after each response chunk. The server persists an intermediate snapshot when at least 1 MiB has arrived since the previous write or 250 ms have elapsed, and always flushes the final snapshot before marking success `Completed`. On transfer failure, the server stores the latest error, while the scheduler requeues the task while the default three-retry budget remains; the dispatch coordinator starts the retry when a slot is available. A new claim clears the previous error and resets the task snapshot before the server restores any validated partial progress. After the retry budget is exhausted, the task stays `Failed`. Bandwidth policy currently calculates limits; the HTTP transfer does not apply them. Task and scheduler events are collected in Core and can be drained, but the server does not publish them to clients.

## Engines and Sources

The resolver accepts HTTP/HTTPS URLs, magnet URIs containing an `xt=urn:btih:` parameter, and existing local paths; it does not validate the magnet hash itself. The server routes only HTTP/HTTPS to a transfer engine. It rejects destinations inside the server data directory, symbolic-link destinations, and concurrent active transfers targeting the same canonical path. `HttpEngine` performs a blocking GET, follows up to five redirects, has a 10-second connection timeout and a 30-minute request timeout, and reports progress after each written response chunk. The server-owned HTTP worker keeps one hidden stable partial file beside each destination and an atomic JSON sidecar containing the source, destination, ETag or Last-Modified validator, and expected length. After restart it sends `Range` and `If-Range` only for matching metadata, and accepts only a matching `206 Partial Content` range. A `200` response, changed or missing validator, malformed `Content-Range`, or inconsistent length removes the partial response and starts a full download. A response without a validator may finish in the current process but cannot resume after restart. Validated partial data survives ordinary transfer errors and process exit; successful commit, cancellation, and task removal clean it up. The destination is synced and replaced only after the complete response is received. The server's direct HTTP worker path supports cooperative pause at a response chunk boundary, same-process resume, and destructive remove cancellation. Pause retains the partial file and response while the worker waits; remove cancels the worker, drops the partial file and sidecar, removes the task, and leaves an existing destination in place. A blocking response read can delay `task.pause` until the 30-minute request timeout. `task.remove` waits up to 30 seconds for the worker and returns an error if it has not exited; the worker can remain blocked until the HTTP timeout, after which removal can be retried. The generic `HttpEngine` adapter remains synchronous and advertises no pause/resume capability. The in-memory engine simulates lifecycle operations and does not transfer bytes. There is no magnet or local-file transfer engine.

## Persistence and Recovery

`Core<R>` accepts a `TaskRepository`. The server locks `data_dir/nexum.lock` for its lifetime, opens `data_dir/nexum.sqlite` (default `./data/nexum.sqlite`), and calls `Core::recover` before accepting connections. A second server using the same data directory cannot start. The SQLite repository persists task metadata, progress snapshots, and the most recent transfer error. Recovery keeps created, completed, and failed tasks in their stored states; queued tasks remain queued, while previously downloading, paused, or retrying tasks are reset to `Queued` in memory and SQLite. The scheduler queue is rebuilt at normal priority; prior priority, order, and retry counts are not persisted. After recovery and listener startup, the dispatch coordinator automatically starts eligible queued HTTP/HTTPS tasks; unsupported sources or blocked destinations remain queued. A restarted HTTP transfer restores its validated partial byte count before dispatch. It resumes from that byte only when the HTTP validator and `206` range match; otherwise the partial response is discarded and the transfer starts from byte zero. Directory creation, lock acquisition, database opening, and recovery failures stop server startup.

## Protocol and Clients

The server reads one JSON-RPC request per TCP line and writes a response line for requests with IDs. It handles `task.start` and active HTTP control before the dispatcher; the dispatcher supports `task.get`, `task.list`, `task.create`, `task.queue`, `task.start`, `task.pause`, `task.resume`, `task.remove`, `server.version`, and `server.auth`. A successful `task.queue` dispatches eligible HTTP/HTTPS work after the Core operation; `task.start` remains available to manually kick one queued task. For an active server HTTP worker, `task.pause` waits for a chunk-boundary acknowledgement and persists `Paused`, `task.resume` wakes the same worker when a scheduler slot is available, and `task.remove` cancels the worker, waits up to 30 seconds for it to exit, and deletes the task after that wait succeeds. A timed-out removal returns an error and can be retried after the worker exits. `task.get` and `task.list` return `TaskView` values with the current persisted progress and an `error` field for the most recent transfer error; the field is cleared when a new attempt is claimed. There is no general task-update method. The protocol has a V1 version field and a version-inspection method, but no negotiated feature set or version enforcement beyond JSON-RPC `2.0` envelope validation. Event-envelope conversion and buffering exist in the crate, without a server subscription or stream.

The CLI uses the TCP protocol for task control and server inspection. The Tauri 2 + React desktop app uses Tauri commands as a TCP JSON-RPC client, with task list, add, and control views. It refreshes after actions or server-address changes, without periodic polling or pushed events; its server address is held only in component state. The Manifest V3 browser extension contains context-menu and link-badge UI, but its send flow posts HTTP to `/jsonrpc` and has no compatible endpoint. Its popup writes the `server` storage key while its background script reads `address`, so the saved address is ignored. It has no device selection.

Credentials can be carried in protocol requests, and `server.auth` reports only `none`. The server does not validate credentials. Its `require_auth` and `max_connections` configuration values are parsed but not enforced. TLS and rate-limit types are not wired into the server.

## Extension and Media Boundaries

`PluginManager` registers manifests and tracks load/start/stop states, and the crate defines `EngineProvider` and `ResolverProvider` traits. Core does not call plugin implementations or register their adapters; `init_plugins` only advances manager state. Permission declarations are metadata, not an enforced sandbox. Dynamic loading and executable plugin integration are not present.

The media crate defines probe, manifest, track, segment, mux, pipeline, job, workflow, and MCP request types. Workflow dependency ordering and in-memory job bookkeeping are implemented. `MediaProcessor::probe` returns default metadata, while job execution fabricates a completed result; no file probing, manifest parsing, segment processing, muxing, external automation endpoint, or MCP integration is wired up.

For the Chinese version, see [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md).
