# Development Guide

## Environment

Nexum is a Rust 2024-edition workspace with a Tauri 2 desktop client. The desktop frontend uses TypeScript and React; the browser extension is written in TypeScript.

- Rust stable (1.85 or newer), Cargo, `rustfmt`, and Clippy
- Node.js LTS and pnpm for the two frontend packages
- Tauri 2 system dependencies and the Tauri CLI when running or bundling the native desktop app

The Ubuntu CI job installs `libgtk-3-dev` and `libwebkit2gtk-4.1-dev` before checking the Rust workspace. Platform-specific Tauri prerequisites also apply to local builds.

## Workspace

The root `Cargo.toml` includes all crates below and `apps/desktop/src-tauri`. The browser extension is a separate frontend package.

```text
crates/
  core/ domain/ task/ scheduler/ storage/ resolver/
  protocol/ plugin/ security/ media/ engine/
apps/
  server/                 # nexum-server binary
  cli/                    # nexum-cli binary
  desktop/                # React/Vite and src-tauri/
  extension/              # Manifest V3 extension
```

Keep UI logic out of Core and engine-specific details out of the Domain Model. Add focused tests and update both language versions of affected documentation when behavior changes. Check [Contributing](../CONTRIBUTING.md) for changes that require an RFC.

## Rust Checks

From the repository root, run the same commands as `.github/workflows/ci.yml`:

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

`Cargo.lock` is committed because this workspace ships Server, CLI, and Desktop binaries; keep it updated when changing Rust dependencies. CI runs these Rust checks on Ubuntu. It does not currently build or check the TypeScript packages or produce release artifacts.

## Server and CLI

Start the local server in one terminal:

```bash
cargo run -p nexum-server -- --port 39100
```

It binds to `127.0.0.1:39100` by default and serves newline-delimited JSON-RPC 2.0 over TCP plus one-request HTTP `POST /jsonrpc` calls. Validated browser-extension origins receive CORS headers. Run the CLI in another terminal:

```bash
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
```

The built binaries are named `nexum-server` and `nexum-cli`. Use `cargo run -p nexum-server -- --help` and `cargo run -p nexum-cli -- --help` for current flags and commands. The CLI defaults to `127.0.0.1:39100`; put `--server ADDR` before `task` or `server` to override it.

The server stores task metadata and progress in `nexum.sqlite` under `--data-dir` (default `./data`, relative to the server's working directory). It creates the directory if needed and holds a lock on `nexum.lock` there until exit, so only one server can use that directory. It opens the database and recovers tasks before listening; directory, lock, database, or recovery errors stop startup. An explicitly supplied `--config` file that cannot be read also stops startup. On restart, previously downloading, paused, or retrying tasks become queued in memory and in SQLite. Once recovery and listening complete, eligible queued HTTP/HTTPS tasks are dispatched automatically; unsupported sources and blocked destinations remain queued. `--max-connections` limits active TCP connection handlers; a connection over the limit is accepted by the OS and immediately closed before request processing. Authentication is disabled by default. Set `require_auth=true` together with `auth_scheme=Bearer|ApiKey` and a non-empty `auth_token` to require credentials; `server.auth` reports the active scheme without the secret. An incomplete or unsupported credential configuration stops startup.
To enable the Server TLS listener, set both `tls_cert_path` and `tls_key_path` in the key-value config or pass `--tls-cert-path PATH --tls-key-path PATH`. The PEM certificate and private key are loaded before the listener accepts connections. TLS carries TCP JSON-RPC, `events.subscribe`, and HTTP `/jsonrpc` over the same stream; a TLS handshake failure closes the connection and never downgrades to plaintext. The CLI selects TLS with `--server tls://host:port`, loads the platform root store (or the `SSL_CERT_FILE`/`SSL_CERT_DIR` sources supported by `rustls-native-certs`), validates the server name, and sends credentials only after a verified handshake. Desktop and Browser client trust settings are still pending.

`task queue` persists a task and triggers the server dispatcher. It selects eligible queued HTTP/HTTPS tasks until the scheduler's concurrent-task limit or destination rules prevent another claim. `task start` remains a manual kick that selects one queued HTTP/HTTPS task and returns its ID after launching a worker; it then invokes the same dispatcher to fill other available slots. The worker reports progress after each response chunk; the server persists an intermediate snapshot after 1 MiB or 250 ms and flushes the final snapshot before marking the task `Completed`. Check `task list` or `task get ID` again for the current byte counts and completion; completion is asynchronous. The server rejects destinations inside its data directory, symbolic-link destinations, and overlapping active destinations. The worker writes to a hidden stable partial file in the destination directory and atomically maintains a JSON sidecar with the source, destination, validator, and expected length before renaming a complete response into place. A failed transfer keeps an existing destination, stores the failure text in the task's `error` view field, and is queued again while the retry policy allows; the dispatcher starts it automatically when a slot is available. The default policy allows three retries. A new attempt clears the previous error; the server restores the validated partial byte count after the claim when the sidecar matches. An active HTTP transfer can be paused at a response chunk boundary and resumed in the same server process; the worker keeps its temporary file and response open while paused. `task remove` cancels an active HTTP worker, waits up to 30 seconds for it to exit, removes the task, and leaves an existing destination untouched. If that wait times out, the request returns an error and the worker can remain blocked until the 30-minute HTTP request timeout; retry removal after it exits. A blocking response read can delay pause until the HTTP request timeout. After a restart, a stable partial file and sidecar are reused only when the source, destination, ETag or Last-Modified validator, expected length, and server-confirmed `206 Partial Content` range match. A `200` response, validator change or absence, malformed range, or length mismatch discards the partial response and starts from zero; a response without a validator cannot resume after restart. Magnet and local-file sources have no transfer engine yet.


## Authentication

The server accepts `Bearer` and `ApiKey` credentials. Configure authentication in a key-value file passed with `--config`:

```ini
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

The same values can be supplied as command-line overrides:

```bash
cargo run -p nexum-server -- \
  --require-auth true \
  --auth-scheme ApiKey \
  --auth-token replace-with-a-secret
```

`require_auth=true` requires a supported scheme and non-empty token; a missing pair, empty token, or unsupported scheme makes startup fail. When enabled, the server checks the request credential before dispatching every RPC and before accepting `events.subscribe`. Both the scheme and secret must match. Missing or invalid credentials return JSON-RPC error `-32001` with message `authentication required`. `server.auth` returns only the configured scheme, and server logs and debug output redact the token.

The CLI stores its credential as plain text in the same config directory as the default server address. Set or clear it with:

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- auth clear
```

`auth set` accepts only `Bearer` or `ApiKey` and rejects an empty token. `auth clear` clears both credential field values in the CLI config. The CLI may attach the saved credential to a verified `tls://` connection; for a bare plaintext address it first checks that the connected peer is loopback and refuses to send credentials otherwise. On macOS, first save a loopback Server address in Desktop Settings. Under General, select `Bearer` or `ApiKey`, enter the matching secret, and save it. The Tauri backend keeps both values in Keychain under that saved address, sends them with ordinary RPC and `events.subscribe`, and returns only the configured scheme to React when reading Keychain state. Saving or clearing the credential restarts the event subscription and refreshes the task snapshot. The secret field is write-only; clearing removes the Keychain item. Until the Desktop TLS client and trust controls are available, Desktop does not store or attach credentials for non-loopback addresses; unauthenticated RPC connections can still be attempted. Desktop on other platforms can connect without authentication, but saving and clearing credentials are unsupported. The Browser extension still has no credential setting.

## RPC Rate Limiting

Rate limiting is disabled by default. To enable it, set both the sustained requests per second and the token-bucket burst capacity in the server's key-value configuration file:

```ini
rate_limit_rps=10
rate_limit_burst=20
```

The equivalent command-line flags are:

```bash
cargo run -p nexum-server -- --rate-limit-rps 10 --rate-limit-burst 20
```

The two settings must be supplied together and have valid positive values; invalid or incomplete settings stop startup. An explicitly supplied `--config` file that cannot be read also stops startup. One process-wide token bucket is shared by parsed, authenticated requests on both line-delimited TCP and HTTP `POST /jsonrpc`, including `events.subscribe`. Authentication failures do not consume tokens. Server-generated event heartbeats and HTTP `OPTIONS` preflight requests do not consume tokens. An over-limit call with an `id` receives JSON-RPC error `-32002`; HTTP `/jsonrpc` still responds with status `200` and the error in its JSON body. A notification without an `id` is dropped without a JSON-RPC response; HTTP uses status `204`. The CLI performs a version request before its command, and the Desktop makes multiple calls at startup, so use a burst size comfortably above these client call sequences (the example uses 20).

## Desktop

Start the Vite frontend from `apps/desktop`:

```bash
cd apps/desktop
pnpm install
pnpm dev
```

Vite uses port `1420`. The frontend calls Tauri commands, Keychain-backed credential commands on macOS, and the Tauri dialog plugin, so testing the full application requires a running Nexum server and a native Tauri window. With the Tauri 2 CLI installed, keep Vite running and start `cargo tauri dev` from `apps/desktop` in another terminal. `pnpm build` runs the TypeScript compiler and Vite build; this is a separate check from Rust CI. To create a local macOS development app from `apps/desktop`, run `pnpm dlx @tauri-apps/cli@2 build --debug --bundles app`; the bundle is written to `target/debug/bundle/macos/Nexum.app` at the repository root. For a local ad-hoc signature, run `codesign --force --deep --sign - ../../target/debug/bundle/macos/Nexum.app` from `apps/desktop` after bundling. Distribution signing and notarization are still separate release work. Keep the Tauri JavaScript and Rust package minor versions aligned when updating dependencies.

The macOS-first information architecture, task states, add flow, settings, event subscription, polling fallback, and implementation slices are documented in [Desktop UI Design](DESKTOP_UI_DESIGN.md).

## Browser Extension

From `apps/extension`:

```bash
cd apps/extension
pnpm install
pnpm lint
pnpm build
```

The extension's `pnpm dev` watches and rebuilds files. Load `apps/extension` as the unpacked extension: its root `manifest.json` references assets under `dist/` and icons under `icons/`.

The server's HTTP bridge accepts one JSON-RPC request per connection at `/jsonrpc`, enforces the same authentication gate, requires `Content-Type: application/json`, returns CORS headers to validated browser-extension origins, and rejects web-page origins, chunked bodies, or bodies larger than 1 MiB. The extension's send flow calls `task.create` and then `task.queue`; it uses the saved `server` address, handles content-script messages, and resolves relative links before sending. Event subscriptions remain TCP-only, and the extension has no credential settings, so leave `require_auth` disabled for extension use.

For the Chinese version, see [DEVELOPMENT.zh-CN.md](DEVELOPMENT.zh-CN.md).
