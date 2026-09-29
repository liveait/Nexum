# Nexum Development Plan

This plan distinguishes code-level foundations from an end-to-end feature available through the current server and clients. Checked items are present in the repository; unchecked items still need implementation or wiring. Planned work describes direction, not a fixed release schedule. See [Architecture](ARCHITECTURE.md) for current call paths and [Contributing](../CONTRIBUTING.md) for engineering and submission guidance.

## 1. Milestones

### Phase 0 - Project Foundation

- [x] Repository structure, MIT license, and contribution/governance documents
- [x] English and Simplified Chinese documentation
- [x] Rust workspace with server, CLI, desktop Tauri crate, and core crates
- [x] GitHub CI workflow configured for formatting, workspace check/test, and Clippy

### Phase 1 - Domain and Task Core

- [x] Domain value types and download task model
- [x] Validated task state machine
- [x] In-memory task service and task events
- [x] Unit tests for task lifecycle and transitions

### Phase 2 - Scheduler

- [x] Priority queue and concurrent-task limit
- [x] Retry policy and pause/resume operations
- [x] Bandwidth-policy interface and limit calculation
- [x] Scheduler events and controlled unit tests
- [ ] Apply calculated bandwidth limits to transfers
- [x] Server-side dispatcher automatically fills available HTTP worker slots for queued work and retries; Scheduler remains transport-agnostic

### Phase 3 - Storage

- [x] `TaskRepository` and in-memory implementation
- [x] SQLite task metadata/progress repository with schema version and migration
- [x] `Core::recover` to rebuild queued work and persist normalized recovery states
- [x] Open SQLite and call recovery in the server startup path
- [x] Verify task continuity across a real server restart

### Phase 4 - Resolver

- [x] Resolver request/result/error model and registry
- [x] HTTP/HTTPS validation, magnet `xt=urn:btih:` parameter presence check, and existing-local-path validation
- [x] Resolver tests and Core task-creation validation
- [x] Route eligible queued HTTP/HTTPS tasks through the server dispatcher after `task.queue`, startup recovery, or worker completion/failure
- [ ] Route magnet and local-file sources to compatible transfer engines
- [ ] Add actual magnet and local-source transfer paths

### Phase 5 - Engine Adapter

- [x] Adapter capabilities, task mapping, and engine registry
- [x] Simulated in-memory engine and blocking HTTP GET engine with redirect following
- [x] Controlled engine tests
- [x] Run the HTTP engine from the server dispatcher; CLI and Desktop use the same RPC and may still call `task.start` as a manual kick
- [x] Stage HTTP downloads in a `.part` file and rename only a complete response to the destination
- [x] Add incremental progress reporting and persistence for real transfers
- [x] Add cooperative cancellation and same-process pause/resume behavior for server HTTP transfers
- [x] Add cross-restart HTTP resume with a stable destination-side partial file, atomic sidecar metadata, ETag/Last-Modified validation, and `Range`/`If-Range` requests; restart from zero when the validator or response range does not match

### Phase 6 - Nexum Protocol and Security

- [x] JSON-RPC 2.0 request/response and error objects
- [x] Task create/get/list/queue/start/pause/resume/remove and server inspection methods
- [x] V1 version type, request field, and `server.version` method
- [x] Task/scheduler event envelopes and buffering
- [x] Credential, TLS, and rate-limit types with protocol/security unit tests
- [ ] Enforce protocol compatibility beyond envelope validation
- [x] Publish events through a dedicated TCP subscription and expose them to clients; reconnects begin with a full task snapshot because the stream has no replay buffer
- [x] Validate and enforce configured Bearer/ApiKey credentials for RPC requests and event subscriptions
- [ ] Enforce configured TLS and rate limiting

### Phase 7 - Server and CLI

- [x] Loopback TCP server using line-delimited JSON-RPC
- [x] CLI TCP client with task-control, address configuration, and server inspection commands
- [x] Server CLI flags and key-value configuration parsing
- [x] Enforce `max_connections` at TCP admission; excess connections close before request processing
- [x] Apply `require_auth` with `auth_scheme` and `auth_token` before RPC dispatch and event subscription
- [x] Use `data_dir` for SQLite persistence and restart recovery
- [x] Make normal `task.start` launch a real HTTP/HTTPS download for supported sources
- [x] Persist transfer errors and expose them through task views instead of only server logs
- [x] Automatically dispatch queued retries and newly queued HTTP/HTTPS work when a scheduler slot is available

### Phase 8 - Desktop

- [x] Tauri 2 + React application and TCP JSON-RPC command bridge
- [x] Task list, add, queue/start, pause/resume, and remove UI
- [x] Editable server address and refresh after actions/address changes
- [x] Document the macOS-first sidebar, task list, inspector, add sheet, and settings flows ([Desktop UI Design](DESKTOP_UI_DESIGN.md))
- [x] Replace the prototype tabs with the documented sidebar/list/inspector shell
- [x] Add an Add Download sheet that creates and queues a task
- [x] Add a native destination chooser to the Add Download sheet through the Tauri dialog plugin
- [x] Add a Tauri event subscription with debounced task refreshes and retain periodic polling as a disconnected-stream fallback
- [x] Persist server address and refresh policy through Tauri commands
- [x] Surface connection/action failures without discarding the last successful task list
- [ ] Add keyboard navigation, accessibility labels, system appearance, and English/Simplified Chinese UI strings

### Phase 9 - Browser Integration

- [x] Manifest V3 extension shell, link context menu, and downloadable-link badge heuristic
- [x] Popup field for storing one server address
- [ ] Connect send-to-Nexum to a supported transport; the extension posts HTTP `/jsonrpc` while the server only speaks TCP
- [ ] Use the saved `server` address in the background script; it currently reads an `address` field instead
- [ ] Handle the content script's send message and complete an end-to-end task-creation test
- [ ] Add device selection if multi-device delivery is still a product requirement

### Phase 10 - Extensibility

- [x] Plugin manifest, permission, and capability data models
- [x] `PluginManager` state transitions and tests
- [x] `EngineProvider` and `ResolverProvider` traits
- [ ] Invoke plugin lifecycle implementations and load executable plugin entries
- [ ] Register plugin-provided engines/resolvers with Core; current initialization only changes manager state
- [ ] Enforce permissions and define a stable SDK/runtime contract

### Phase 11 - Media and Automation

- [x] Foundational media, job, workflow, and MCP request types
- [x] Workflow dependency ordering and simulated in-memory job API
- [ ] Probe real media and parse manifests
- [ ] Implement track selection, segment scheduling, muxing, and post-processing
- [ ] Execute and persist real jobs/workflows instead of fabricating completed results
- [ ] Expose an external automation endpoint and integrate AI/MCP where required
- [ ] Implement remote device management

## 2. Delivery Order From Current Code

1. Complete the remaining server/client contracts: durable event-delivery semantics, configured TLS/rate limiting, and a browser-compatible endpoint or bridge.
   - Specify reconnect, dropped-event, and replay behavior for the bounded event stream.
   - Decide whether the browser uses an HTTP JSON-RPC endpoint or a TCP bridge, then connect the extension send flow.
   - Keep authentication settings aligned across clients as Desktop credential storage is added.
2. Finish the Desktop release layer: keyboard navigation, accessibility labels, reduced-motion/system appearance behavior, and English/Simplified Chinese strings.
3. Connect plugin providers and enforce their declared permissions.
4. Replace simulated media operations with real processing, then expose automation and remote-device workflows.

## 3. Current Focus

Phases 0-9 have varying levels of scaffolding and library coverage. The running server persists tasks in SQLite, recovers them, and automatically dispatches eligible HTTP/HTTPS work after queueing, startup recovery, and worker completion or failure. `task.start` remains a manual kick and compatibility method. The server persists throttled intermediate progress, exposes the latest transfer error through task views, and records final progress and completion after a successful transfer. Active server HTTP transfers support cooperative chunk-boundary pause, same-process resume, and destructive remove cancellation; a blocking response read can delay `task.pause` until the 30-minute HTTP timeout, while `task.remove` returns after a 30-second worker wait if it cannot stop sooner. The server now preserves validated partial HTTP responses across process restarts. It resumes only a matching sidecar and `206 Partial Content`; invalid or unvalidated responses are discarded and downloaded from byte zero. Task and Scheduler events now leave Core through the server event pump and a dedicated `events.subscribe` TCP stream; Desktop refreshes a full task snapshot after debounced notifications and falls back to its configured polling policy when the stream is unavailable. Magnet and local-file transfers remain unsupported. Plugin and media crates contain more than data types, but their provider callbacks and real processing are not integrated into the product path. The macOS Desktop information architecture is documented in [Desktop UI Design](DESKTOP_UI_DESIGN.md); the React surface now has the sidebar/list/inspector shell, Settings page, Add Download sheet with a native destination chooser, explicit RPC errors, adaptive event-driven refresh, and persisted Server settings. Keyboard/accessibility polish, localization, Desktop credential settings, TLS/rate limiting, and browser transport remain outstanding.
