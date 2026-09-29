# Nexum

[English](README.md) · [简体中文](README.zh-CN.md)

> A Rust workspace for download task management and client integrations.

Nexum is under active development. Its runnable path is a local TCP JSON-RPC server for creating and controlling tasks. The server stores tasks in SQLite. Queuing an HTTP/HTTPS task makes it eligible for automatic dispatch to a background worker when a scheduler slot and destination are available.

[![CI](https://github.com/liveait/Nexum/actions/workflows/ci.yml/badge.svg)](https://github.com/liveait/Nexum/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Current status

The CLI and early Tauri desktop client can call the local server; the desktop client requires the server to run separately. The server opens `nexum.sqlite` under its data directory (default `./data`) and recovers stored tasks on startup. Queuing a supported HTTP/HTTPS task, restarting the server with a queued task, or completing/failing an active HTTP transfer causes the server to fill available scheduler slots automatically. An interrupted HTTP response can resume after restart when its stable partial file, sidecar, and ETag or Last-Modified validator still match a server-confirmed range. `task.start` remains a manual kick and compatibility method for starting one queued HTTP/HTTPS task, after which the same dispatcher fills other available slots. The Desktop client now subscribes to a dedicated TCP event stream for debounced live task refreshes and falls back to its configured polling policy when that stream is unavailable. Magnet and local-file sources can be created but have no transfer path yet. The browser extension sends task.create and task.queue through the server's loopback HTTP `/jsonrpc` bridge; event subscriptions remain TCP-only.

The server can enforce Bearer or ApiKey authentication when `require_auth` is enabled and can apply optional, process-wide RPC rate limiting. TLS, executable plugins, and real media processing are not active in the running server. The CLI saves credentials in its config file; the macOS Desktop saves a credential for each Server address in Keychain and can connect to an authenticated loopback Server. The Browser extension has no credential setting yet. See [Architecture](docs/ARCHITECTURE.md) for the code-level boundaries and current call paths, and the [Development Plan](docs/DEVELOPMENT_PLAN.md) for remaining integration work.

## Run the local task flow

Requirements: Rust with edition 2024 support (Rust 1.85 or newer) and Cargo. From the repository root, start the server in one terminal:

```bash
cargo run -p nexum-server
```

In another terminal, use the CLI:

```bash
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
cargo run -p nexum-cli -- task list
```

To protect the server, provide a complete authentication configuration. The scheme and token must be supplied together, and the server accepts `Bearer` and `ApiKey`:

```ini
# server.conf
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

Start with `cargo run -p nexum-server -- --config server.conf`, then save the same credential for the CLI (the token is stored as plain text in the CLI config directory):

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- auth clear
```

Missing, invalid, or mismatched credentials return JSON-RPC error `-32001` (`authentication required`). `server.auth` reports the configured scheme without returning the secret. If `require_auth=true` is set without a valid scheme and non-empty token, the server refuses to start. Authentication applies to ordinary RPC requests and `events.subscribe`. On macOS, save the matching scheme and secret under Desktop Settings → General for the current saved loopback Server address; the Desktop attaches that credential to both request types. Its secret field is write-only, and clearing the credential removes it from Keychain. Credential storage and sending for non-loopback addresses await TLS support.

RPC rate limiting is disabled by default. Set both `--rate-limit-rps` and `--rate-limit-burst` on the server, or both `rate_limit_rps` and `rate_limit_burst` in its configuration file, to enable one shared token bucket for TCP and HTTP `/jsonrpc`. Excess calls with an `id` receive JSON-RPC error `-32002`; HTTP replies keep status `200`. Notifications without an `id` are dropped without a JSON-RPC response. Invalid or incomplete rate-limit settings and an unreadable file explicitly passed with `--config` stop server startup. See the [Development Guide](docs/DEVELOPMENT.md) for examples and counting rules.

`task.queue` returns after the task is persisted and the server has attempted to fill available HTTP worker slots. `task.start` remains available as a manual kick and returns after launching one HTTP worker. Workers report progress after each response chunk; the server persists an intermediate snapshot when at least 1 MiB has arrived or 250 ms have elapsed since the previous write, then flushes the final snapshot before marking the task `Completed`. On success it writes `./example.html` and records the final byte count; `task list` may show `Downloading` until then. The `task.get` and `task.list` views expose the persisted progress and an `error` field containing the most recent transfer error. The destination is replaced only after the full response is staged in a hidden stable partial file beside it; the sidecar records the source, destination, validator, and expected length. A failed transfer preserves an existing destination, logs the failure, stores the error on the task, and requeues it while the retry policy allows; the server automatically dispatches that retry when a slot is available. The default policy permits three retries. Claiming a new attempt resets the persisted snapshot and clears the previous error; the server then restores validated partial progress when available. Active HTTP transfers can be paused at a response chunk boundary and resumed in the same server process; the temporary file and open response are retained while paused. `task.remove` cancels an active HTTP worker, removes the task, and leaves the existing destination untouched. `task.pause` can wait for a blocking response read until the 30-minute HTTP request timeout. `task.remove` waits up to 30 seconds for the worker; if it times out, it returns an error while the worker can remain blocked until that HTTP timeout, and the removal can be retried after the worker exits. After a restart, an interrupted HTTP response resumes with `Range` and `If-Range` only when the sidecar validator matches and the server returns a matching `206 Partial Content`. A `200` response, changed or missing validator, invalid `Content-Range`, or inconsistent length discards the partial response and starts from zero; a response without ETag or Last-Modified therefore starts from zero after interruption. The server binds to `127.0.0.1:39100` by default; use `--port PORT` on the server or `--server ADDR` on the CLI to change the connection. Authentication is disabled by default. When enabled, the server compares both the configured scheme and secret before dispatching any RPC; it does not log the secret. See the [Development Guide](docs/DEVELOPMENT.md) for client setup, authentication configuration, all Rust checks, and platform dependencies.

## Documentation and contribution

- [Architecture](docs/ARCHITECTURE.md)
- [Development Guide](docs/DEVELOPMENT.md)
- [Development Plan](docs/DEVELOPMENT_PLAN.md)
- [Desktop UI Design](docs/DESKTOP_UI_DESIGN.md)
- [Changelog](CHANGELOG.md)
- [Architecture Decisions](docs/decisions/README.md)
- [Contributing](CONTRIBUTING.md)
- [Governance](GOVERNANCE.md)
- [Chinese README](README.zh-CN.md)

Nexum is licensed under the [MIT License](LICENSE). Third-party dependencies retain their respective licenses.
