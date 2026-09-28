# Changelog

All notable changes to Nexum will be documented here.

## Unreleased

### Added

- **Core**: task lifecycle, scheduler, resolver integration, and event collection. SQLite storage and restart recovery now support the running server; recovery persists the normalized `Queued` state of previously downloading, paused, or retrying tasks.
- **Engine**: in-memory and HTTP adapters. The HTTP adapter follows redirects, stages the response in a `.part` file, renames a complete download into place, reports progress after each written response chunk, and exposes cooperative pause/cancel control for server workers. The server-owned path now keeps a stable hidden partial file and atomic JSON sidecar, and resumes only a validator-matched `206 Partial Content` response after restart. The generic synchronous adapter remains without pause/resume capability.
- **Protocol**: JSON-RPC 2.0 task and server-info methods, a version identifier, credential fields, event envelope/buffer types, and `events.event` notifications for subscribed clients. `TaskView`, returned by `task.get` and `task.list`, includes persisted progress and an `error` field for the most recent transfer error. Credential enforcement remains unimplemented.
- **Server**: localhost, line-delimited TCP JSON-RPC with one thread per connection. It holds a data-directory lock, stores tasks in `data_dir/nexum.sqlite`, recovers them before listening, and fails startup on directory, lock, database, or recovery errors. Queueing an HTTP/HTTPS task, recovering queued work at startup, and completing or failing an active HTTP transfer trigger the server's automatic dispatcher. It claims eligible tasks up to the scheduler concurrency limit while skipping unsupported sources and overlapping destinations. `task.start` remains a manual kick that launches one HTTP/HTTPS worker and returns before transfer completion, then uses the same dispatcher to fill other slots. The server persists progress when a response has added 1 MiB or 250 ms have elapsed, then flushes the final snapshot before persisting `Completed`. Failures preserve the error in `TaskView` and requeue while the default three-retry budget remains; the dispatcher starts the retry when a slot is available, and after the budget is exhausted the task stays `Failed`. A new claim clears the previous error. Active HTTP workers now support chunk-boundary `task.pause`, same-process `task.resume`, and destructive `task.remove` cancellation; a blocking response read can delay pause until the 30-minute HTTP timeout, while remove waits up to 30 seconds before returning an error. Restart recovery resumes a validator-matched HTTP partial with `Range`/`If-Range`; missing or changed validators, a `200` response, invalid `Content-Range`, inconsistent length, or a response without a validator fall back to byte zero. Clients can use a dedicated `events.subscribe` TCP connection for sequenced task and scheduler notifications; the stream has no replay buffer, so clients refresh a full snapshot after connecting or reconnecting. Connection limits and required authentication are not enforced.
- **CLI**: task commands, a reusable TCP JSON-RPC client, saved server address and credential fields, server inspection, and RPC error formatting.
- **Desktop**: Tauri 2 + React task UI connected to the TCP server, with a server-address field, native destination picker, dedicated event subscription, debounced task refresh, and polling fallback.
- **Browser**: Manifest V3 context menu, link detection, and popup configuration. Its HTTP send request is not yet compatible with the TCP-only server.
- **Security**: credential, TLS, and rate-limit types; server-side authentication, TLS, and rate limiting remain unimplemented.
- **Plugin**: manifest, permission, capability, lifecycle-state, and provider-trait foundations. Core tracks plugin manifests but does not yet load provider engines or resolvers.
- **Media**: media and workflow models with simulated, in-memory probing and job execution; no real media processing is wired in.

### Documentation

- Synchronized README, architecture, development guides, and the consolidated development plan with the implemented repository state.
- Kept English and Simplified Chinese project documentation aligned.

### Fixed

- **CI**: removed redundant workflow configuration and repaired workspace compatibility issues so the main CI workflow passes formatting, checks, tests, and Clippy.
- **Branding**: refreshed Desktop, Browser Extension, and macOS icon assets and corrected invalid PNG/ICNS payloads.

## [0.1.0] - TBD

- Initial project structure and public development foundation.

For the Chinese version, see [CHANGELOG.zh-CN.md](CHANGELOG.zh-CN.md).
