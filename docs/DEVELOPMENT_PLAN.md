# Nexum Development Plan

This plan starts from the code as of October 9, 2026, and orders work around a standalone macOS downloader first, followed by download capabilities and ecosystem integration. The comparison uses the [official Motrix feature list](https://github.com/agalwood/Motrix/blob/main/README.zh-CN.md) and its [v2.0.0-beta.46 release](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46). It is a feature-gap reference, not a claim that we tested every Motrix feature or a commitment to copy every implementation or release on a fixed date.

“Present” means a code path is wired, “needs validation” means an implementation lacks a check of the stated artifact, and “planned” means there is no usable end-to-end implementation. A capability is delivered only after its milestone acceptance criteria pass. See [Architecture](ARCHITECTURE.md) for current call paths, [Release Process](RELEASE.md) for build, screenshot, and release steps, and [ADR 0005](decisions/0005-tls-transport.md) for the completed TLS decision.

## 1. Current baseline and Motrix gaps

| Area | Nexum today | Remaining gap |
| --- | --- | --- |
| Tasks and HTTP/HTTPS | The server stores and recovers tasks in SQLite. Queueing, concurrency slots, retries, progress, pause, and removal are wired. Single-connection HTTP downloads use staged files and cross-restart resume validated by ETag/Last-Modified. | Real speed and ETA, limits applied to transfers, prompt pause and worker cancellation on removal during a blocked read, and safe multi-connection downloads of one file. Responses without validators restart from byte zero. |
| macOS Desktop | The native Tauri app has a task list, Add Download, Settings, Keychain credentials, light/dark appearance, and English/Chinese UI; debug-app screenshots exist. | The server must still be started separately. The release app built with the pinned Tauri CLI still needs its own launch, final screenshots, actual VoiceOver speech, and Reduce Motion checks. |
| Security and protocol | TCP/HTTP JSON-RPC, event subscriptions, optional authentication, RPC rate limiting, and verified TLS are wired. | Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks can follow concrete needs; completed TLS wiring is no longer a near-term workstream. |
| FTP and BT/magnet | The magnet resolver only checks for the presence of an `xt=urn:btih:` parameter; local-file sources are only validated; the server dispatches only HTTP/HTTPS workers. FTP has no transfer path. | Real FTP, torrent, and magnet downloads with control, recovery, and integrity checks. The synchronous general Engine Adapter cannot directly provide the long-running control already implemented in the server HTTP worker. |
| Browser and system integration | The extension can submit links through HTTP/HTTPS `/jsonrpc`. | The extension has no credential setting and writes destinations to `/tmp/nexum-*`. It lacks locked release dependencies, browser validation, and desktop integration such as menu bar, notifications, and URL handling. |
| Distribution and extensibility | CI checks Rust and builds an unsigned macOS app. Plugin, Media, and Automation have foundation models. | No distribution signing, notarization, install/upgrade validation, or automated release. Windows/Linux installers are unvalidated. Plugin providers, real media processing, and automation are not wired into the product path. |

## 2. Delivery milestones

The order expresses dependencies and priority, not dates. M0–M3 form the first releasable macOS HTTP downloader path; M4–M7 close the core Motrix capability gaps; M8–M9 follow once those paths are stable. HTTP engine work and distribution preparation may overlap, but M3 release validation must use the final actual artifact.

### M0 — Validate the existing release app natively

- **Work:** Launch the release app built with the pinned Tauri CLI on its own. Capture light, dark, and narrow-window evidence using the [Release Process](RELEASE.md#native-macos-visual-evidence). Check English/Chinese, keyboard focus, actual VoiceOver speech, and indeterminate progress under Reduce Motion in the real app.
- **Acceptance:** The same candidate app connects to a local server, completes a known HTTP file, and shows the right state; the native checks have recorded results. Debug-app screenshots do not stand in for release-artifact evidence. Fix and repeat any failing check.

### M1 — A double-click-ready macOS HTTP app

- **Design first:** Document how the Desktop bundles, starts, probes, reuses, and stops a local server; how it distinguishes an app-managed instance from a user-configured external server; and the stable data directory, default download directory, old-data migration, port-conflict, and local-authentication policies. Preserve external server connections under Settings.
- **Implementation slices:** Bundle the server with the app and manage its lifecycle. Put the task database in a stable application data directory while preserving destination-side partial files and atomic completion. Derive a safe filename from the URL/trusted response metadata, generate task IDs, handle name collisions, and let users override the destination. Provide a controlled default download directory, protect the managed local connection, and show actionable failure/recovery states.
- **Acceptance:** In a clean macOS user environment, double-clicking the app and pasting a URL downloads to the default directory without a separate terminal or manually entering a task ID/destination path. Tasks and files survive quit, relaunch, abnormal interruption, and upgrade. A busy port or failed server startup is diagnosable; the app neither connects to nor stops an unknown process. External server mode still works.

### M2 — HTTP quality needed for the first release

- **Work:** Compute speed from actual transferred bytes; show ETA only when the length is trustworthy. Define global/per-task limit settings and pass Scheduler-calculated limits to workers. Make `task.pause` and `task.remove` (which cancels the worker internally) respond promptly during blocked reads while preserving the existing destination and recoverable partial files.
- **Acceptance:** Controlled local servers cover slow responses, disconnects, changed validators, invalid `Content-Range`, competing tasks, retries, and destination conflicts. Measured throughput over a sustained window respects configured limits; pause and removal each finish within 5 seconds against a local stalled-response fixture. Successful files have the correct hashes; interruption, failure, and removal do not overwrite an existing good destination.

### M3 — Installable and trusted macOS distribution

- **Work:** Pin build inputs and versions, produce an installer for each claimed architecture, and complete Developer ID signing, notarization, stapling, Gatekeeper, install/upgrade/uninstall, and clean-machine smoke checks. Release workflow and update policy must match the actual support scope.
- **Acceptance:** A user installs the release artifact and launches it by double-clicking, without development tools or a terminal, then completes M1–M2 HTTP downloads. System signing and notarization checks pass; upgrade preserves task data. Release notes state version, hashes, supported macOS/CPU architectures, and unfinished FTP/BT/extension capabilities. Keep procedures and evidence in the [Release Process](RELEASE.md).

### M4 — Multi-connection HTTP downloads of one file

- **Work:** Once the single-connection path is stable, add fallback-capable `Range` segments, persistent segment-level state, and atomic completion. Keep concurrency, pause, retry, and cross-restart resume coverage.
- **Acceptance:** Controlled servers serve valid `Range`, reject `Range`, change validators, and return abnormal segment lengths. Single- and multi-connection results have identical hashes. Unsupported segments or failed validation fall back safely without overwriting an existing good destination.

### M5 — General transfer runtime contract and FTP

- **Design first:** Define capability declarations, asynchronous start/progress, pause/cancel, retry, recovery metadata, errors, and server dispatch for long-running engines. The current `EngineAdapter::start` returns synchronously while real HTTP control lives in a separate server worker. Move HTTP through the new contract without regressing M2–M4 before adding another protocol. Decide the FTP/FTPS and credential support scope explicitly.
- **Acceptance:** A real FTP source can be created, queued, downloaded, paused, resumed, canceled, and recovered after restart from CLI and Desktop; restart safely from byte zero when the server does not support resume. Authentication failures, disconnects, and destination conflicts have visible errors; completed files can be verified. HTTP regression tests remain green.

### M6 — Torrent and magnet downloads

- **Work:** Build on M5 with torrent metadata, magnet metadata retrieval, file selection, Tracker/Peer management, piece verification, task persistence, and recovery. Decide seeding, ratio, and network-permission policies.
- **Acceptance:** Controlled torrent and magnet fixtures download selected files through CLI and Desktop. Piece verification and final hashes remain correct after pause, restart, and failure retry. Tracker state and errors are visible; unimplemented seeding must not be advertised as supported.

### M7 — Browser and macOS system integration

- **Dependency:** Build on M1's controlled default download directory and protected local connection, then design browser pairing in this milestone. The extension must not construct arbitrary server-side file paths.
- **Work:** Add trusted pairing, credential storage, destination selection or a server-managed default directory to the extension. Commit a lockfile, include the extension in CI, and package/test each browser that is claimed as supported. Add macOS menu-bar controls, notifications, open file/folder actions, download URL handling, and defined quit behavior. Handle `magnet:` and `.torrent` system associations after M6 is complete.
- **Acceptance:** From an installed app with authentication enabled, a browser submits a link to the expected directory and shows its task result; unauthorized origins cannot create tasks. Each claimed browser and system entry point has its own end-to-end evidence.

### M8 — Windows/Linux delivery

- **Dependency:** Port M1's local server lifecycle and M3's release flow, and provide platform equivalents for M7's system integration; macOS Keychain assumptions cannot carry over unchanged.
- **Acceptance:** Each claimed platform separately passes native checks for credential storage, paths, installers, upgrades, system integration, and HTTP/FTP/BT downloads. An unvalidated platform remains a development target.

### M9 — Executable plugin ecosystem (later product decision)

- **Work:** Once built-in transfer paths are stable, decide the plugin SDK, loading/unloading, version compatibility, permission isolation, and failure boundaries. Wire the existing providers instead of only changing PluginManager state. A plugin marketplace is a separate product decision.
- **Acceptance:** At least one real Engine or Resolver plugin passes end-to-end installation, invocation, permission denial, fault isolation, and removal before plugin support is claimed.

## 3. Scope and immediate next step

Motrix features such as automatic Tracker lists, UPnP/NAT-PMP, upload limits, launch at login, a statistics Dashboard, and remote/headless control need separate scope decisions; this plan does not count them as committed M0–M7 deliverables. Media/Automation, AI/MCP, remote devices, and manual themes/accent colors are also outside the first macOS downloader release gate. Whether local-file sources need an import/transfer path is a separate product decision. Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks should be scheduled for concrete multi-client or remote-connection needs. Clients currently recover from a disconnected event stream by refreshing a full task snapshot; [ADR 0005](decisions/0005-tls-transport.md) records the implemented TLS boundary. Existing data models or page placeholders alone do not make these features usable.

**Next work package:** Close M0's native check of the release app built with the pinned Tauri CLI while writing M1's architecture decision for server ownership, data storage, and local security. Then implement and test the first app-managed server end-to-end slice. On milestone completion, update both plan languages, the README current-status section, and actual evidence in the [Release Process](RELEASE.md).
