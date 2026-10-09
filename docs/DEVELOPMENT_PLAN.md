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
- [x] Enforce optional configured RPC rate limiting for TCP and HTTP `/jsonrpc`
- [x] Enforce configured TLS on the Server listener, CLI transport, Desktop RPC/event stream, and Browser HTTPS bridge

### Phase 7 - Server and CLI

- [x] Loopback TCP server using line-delimited JSON-RPC
- [x] CLI TCP client with task-control, address configuration, and server inspection commands
- [x] CLI TLS client with explicit `tls://host:port` addresses, system-root hostname verification, and credential gating
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
- [x] Store per-server Bearer/ApiKey credentials in macOS Keychain, attach them to RPC and event subscriptions, and allow saving/clearing them in Desktop Settings; scope accounts by transport identity, allow verified TLS endpoints, and restrict plaintext credentials to loopback peers
- [x] Add keyboard navigation, accessible labels, focus management, and reduced-motion behavior
- [x] Follow the macOS light/dark appearance with readable Desktop palettes
- [x] Add a persisted System/English/Simplified Chinese language choice and localized visible and accessibility-facing Desktop strings
- [ ] Capture light/dark screenshots and complete native macOS visual, bilingual, and VoiceOver release checks
  - [x] Capture native debug-app Downloads, Settings, and Add Download states in both appearances; see [Release Process](RELEASE.md#native-macos-visual-evidence).
  - [x] Verify immediate in-app English/Simplified Chinese switching, accessible names, and English persistence after restart in the native debug app.
  - [ ] Verify macOS per-app language override, Reduce Motion, VoiceOver reading, and the final release bundle.
- [ ] Add manual theme and accent-color options

### Phase 9 - Browser Integration

- [x] Manifest V3 extension shell, link context menu, and downloadable-link badge heuristic
- [x] Popup field for storing one server address
- [x] Connect send-to-Nexum to the loopback HTTP `/jsonrpc` bridge; the bridge accepts CORS POST requests and applies the RPC authentication gate
- [x] Use the saved `server` address in the background script and queue created tasks
- [x] Handle the content script's send message, resolve absolute links, and cover the bridge with an end-to-end task-creation test
- [x] Map an explicit `tls://host:port` address to the browser's HTTPS `/jsonrpc` bridge; keep bare `host:port` on the HTTP compatibility path and rely on browser trust without downgrade
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

1. Complete the remaining server/client contracts: durable event-delivery semantics and configured TLS.
   - [x] Define v1 reconnect and dropped-event behavior: the Desktop ignores duplicate/stale sequences, treats a forward gap or disconnect as stale, reconnects, and refreshes a full snapshot; the server still has no replay buffer.
   - [ ] Add a durable replay buffer and an explicit after-sequence subscription when reliable event delivery is required.
   - [x] Add optional process-wide RPC rate limiting with one shared token bucket for TCP and HTTP `/jsonrpc`. Count parsed, authenticated requests, including event subscriptions; exclude heartbeats and HTTP preflight. Return JSON-RPC `-32002` for excess calls with an `id` and drop excess notifications without a response. Keep limiting disabled by default and reject invalid or incomplete configuration at startup.
   - [x] Add per-server Desktop credential settings backed by macOS Keychain; apply credentials to RPC and event subscriptions, and restart the stream after credential changes.
   - [ ] Add Browser credential settings after defining an appropriate storage and pairing flow.

### TLS implementation plan (transport slices and failure-path coverage in place)

- [x] Define one explicit transport syntax: bare `host:port` stays plaintext for compatibility, while `tls://host:port` selects TLS; never fall back from a certificate or handshake failure to anonymous plaintext.
- [x] Add all-or-nothing Server `tls_cert_path`/`tls_key_path` validation and load PEM material before listening; use the secured stream for TCP JSON-RPC, `events.subscribe`, and the HTTP `/jsonrpc` bridge.
- [x] Add CLI and Desktop client trust handling with platform system roots and hostname verification; reject chain or trust failures without sending credentials.
- [ ] Add an explicit CA bundle or certificate pin option without introducing an insecure bypass.
- [x] Wire the CLI TLS RPC client with system-root and hostname verification, preserving the existing plaintext loopback path while TLS is disabled.
- [x] Wire Desktop RPC/event stream with system-root TLS verification, preserving the existing plaintext loopback path while TLS is disabled.
- [x] Wire the Browser HTTPS bridge in a separate slice: map `tls://host:port` to browser HTTPS, preserve the existing plaintext loopback path, and do not retry TLS failures over HTTP.
- [x] Include transport identity in Desktop credential scoping; permit credentials on verified TLS endpoints and keep plaintext credentials restricted to actual loopback peers.
- [x] Add generated-certificate integration coverage for a successful TLS handshake, plaintext rejection, TCP RPC, HTTPS `/jsonrpc` `OPTIONS`/`POST`, validated browser-extension CORS, web-page Origin rejection, configured authentication over TCP/HTTPS, and authenticated `events.subscribe`; generate certificates and private keys at test runtime.
- [x] Extend generated-certificate coverage to invalid or incomplete TLS configuration and trust/hostname handshake failures. Test CLI and Desktop credential gating for RPC and event subscriptions, and verify that the CLI does not retry plaintext after a TLS failure; keep private keys out of the repository.

The staged transport decision and its non-goals are recorded in [ADR 0005](decisions/0005-tls-transport.md).
2. The Desktop's system light/dark palettes and in-app English/Simplified Chinese switching have native debug-app evidence. Complete macOS per-app language, VoiceOver, Reduce Motion, and release-bundle checks. Keyboard navigation, accessible labels, focus management, and reduced-motion behavior are implemented but still need the full native release check.
3. Connect plugin providers and enforce their declared permissions.
4. Replace simulated media operations with real processing, then expose automation and remote-device workflows.

## 3. Current Focus

Phases 0-9 have varying levels of scaffolding and library coverage. The running server persists tasks in SQLite, recovers them, and automatically dispatches eligible HTTP/HTTPS work after queueing, startup recovery, and worker completion or failure. `task.start` remains a manual kick and compatibility method. The server persists throttled intermediate progress, exposes the latest transfer error through task views, and records final progress and completion after a successful transfer. Active server HTTP transfers support cooperative chunk-boundary pause, same-process resume, and destructive remove cancellation; a blocking response read can delay `task.pause` until the 30-minute HTTP timeout, while `task.remove` returns after a 30-second worker wait if it cannot stop sooner. The server now preserves validated partial HTTP responses across process restarts. It resumes only a matching sidecar and `206 Partial Content`; invalid or unvalidated responses are discarded and downloaded from byte zero. Task and Scheduler events now leave Core through the server event pump and a dedicated `events.subscribe` TCP stream; Desktop refreshes a full task snapshot after debounced notifications and falls back to its configured polling policy when the stream is unavailable. The server also accepts one-request CORS HTTP `/jsonrpc` calls through the Browser bridge, which creates and queues tasks through the same dispatcher and authentication gate. Optional process-wide rate limiting now covers authenticated TCP and HTTP RPC calls. Server TLS can now be enabled with paired PEM certificate and private-key paths, with the same secured stream serving TCP, event subscriptions, and the HTTP bridge; the CLI and Desktop use verified `tls://host:port` connections, while the Browser extension maps `tls://host:port` to browser HTTPS and uses the browser trust store. Generated-certificate tests now cover successful TLS TCP/HTTPS requests, plaintext rejection, browser-extension CORS allow/reject behavior, configured authentication over TCP/HTTPS, authenticated event subscriptions, invalid or incomplete Server TLS material, and trust/hostname handshake failures. CLI and Desktop tests cover credential gating after failed connections; CLI tests also verify that a TLS failure does not retry plaintext. Magnet and local-file transfers remain unsupported. Plugin and media crates contain more than data types, but their provider callbacks and real processing are not integrated into the product path. The macOS Desktop information architecture is documented in [Desktop UI Design](DESKTOP_UI_DESIGN.md); the React surface now has the sidebar/list/inspector shell, Settings page, Add Download sheet with a native destination chooser, explicit RPC errors, adaptive event-driven refresh, persisted Server settings, and Keychain-backed credentials for a saved loopback or TLS Server. Keyboard navigation, accessible labels, focus management, reduced-motion behavior, system light/dark palettes, and a persisted System/English/Simplified Chinese language choice are in place. Native macOS visual and bilingual release checks, manual theme/accent options, and explicit Desktop CA/pinning controls remain outstanding.
