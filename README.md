# Nexum

> 用于下载任务管理与客户端集成的 Rust Workspace。<br>
> A Rust workspace for download task management and client integrations.

Nexum 仍在开发中。目前可运行的主线是用于创建和管理任务的本地 TCP JSON-RPC 服务。Server 使用 SQLite 保存任务。排队 HTTP/HTTPS 任务后，只要有可用的 Scheduler 槽位和目标路径，就会自动派发到后台 Worker。

Nexum is under active development. Its runnable path is a local TCP JSON-RPC server for creating and controlling tasks. The server stores tasks in SQLite. Queuing an HTTP/HTTPS task makes it eligible for automatic dispatch to a background worker when a scheduler slot and destination are available.

[![CI](https://github.com/liveait/Nexum/actions/workflows/ci.yml/badge.svg)](https://github.com/liveait/Nexum/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

<a id="current-status"></a>
<a id="当前状态"></a>

## 当前状态 / Current status

macOS Tauri App 默认托管随包 Server：Rust 后端校验可执行文件，在随机且强制鉴权的回环端点启动它，并将任务保存在稳定的 App 数据目录。设置也可选择带显式地址和 Keychain 凭据的外部 Server。

The macOS Tauri app defaults to an App-managed, bundled Server: the Rust backend verifies its executable, starts it on a random authenticated loopback endpoint, and stores its tasks in the stable application data directory. Settings can instead select an external Server with an explicit address and Keychain credentials.

CLI 继续连接单独启动的 Server。托管接线及原生验收边界见 [ADR 0006](docs/decisions/0006-managed-macos-server.md)。

The CLI continues to connect to a separately started Server. Managed startup and native acceptance boundaries are recorded in [ADR 0006](docs/decisions/0006-managed-macos-server.md).

Server 在数据目录（默认 `./data`）下打开 `nexum.sqlite`，启动时恢复已保存的任务。排队支持的 HTTP/HTTPS 任务、带排队任务重启 Server，或活动 HTTP 传输完成/失败后，Server 都会自动填充可用的 Scheduler 槽位。

The server opens `nexum.sqlite` under its data directory (default `./data`) and recovers stored tasks on startup. Queuing a supported HTTP/HTTPS task, restarting the server with a queued task, or completing/failing an active HTTP transfer causes the server to fill available scheduler slots automatically.

跨重启续传合约要求稳定的部分文件、sidecar 及 ETag 或 Last-Modified 校验器与服务端确认的范围匹配；已审查的新路径仍须对照该合约复验。`task.start` 仍可作为手动 kick 和兼容接口，用于启动一个排队的 HTTP/HTTPS 任务，之后同一派发器会继续填充其他可用槽位。

The restart-resume contract requires the stable partial file, sidecar, and ETag or Last-Modified validator to match a server-confirmed range; the reviewed new path still needs verification against this contract. `task.start` remains a manual kick and compatibility method for starting one queued HTTP/HTTPS task, after which the same dispatcher fills other available slots.

Desktop 现在通过独立 TCP 事件流订阅进行去抖的实时任务刷新；事件流不可用时按已配置的刷新策略回退到轮询。Magnet 和本地文件来源可以创建任务，但尚无传输路径。

The Desktop client now subscribes to a dedicated TCP event stream for debounced live task refreshes and falls back to its configured polling policy when that stream is unavailable. Magnet and local-file sources can be created but have no transfer path yet.

Browser Extension 现在根据保存的地址通过 Server HTTP 或 HTTPS `/jsonrpc` 桥接发送 `task.create` 和 `task.queue`；事件订阅仍只走 TCP。

The browser extension sends `task.create` and `task.queue` through the server's HTTP or HTTPS `/jsonrpc` bridge according to its saved address; event subscriptions remain TCP-only.

Server 可以在启用 `require_auth` 时执行 Bearer 或 ApiKey 认证，也可以启用可选的进程级 RPC 限流。配置成对的 PEM 证书和私钥路径后，Server 可以启用 TLS。

The server can enforce Bearer or ApiKey authentication when `require_auth` is enabled and can apply optional, process-wide RPC rate limiting. Server TLS is available when paired PEM certificate and private-key paths are configured.

CLI 和 macOS Desktop 都可使用 `tls://host:port` 地址选择 TLS，校验 Server 名称和系统根证书链，并且只在握手成功后发送凭据。Browser Extension 会将同样的 `tls://host:port` 映射到浏览器 HTTPS，并使用浏览器正常的信任库；它仍没有凭据设置。

The CLI and macOS Desktop can select TLS with a `tls://host:port` address, validate the server name and system root chain, and only send credentials after the handshake succeeds. The Browser extension maps the same `tls://host:port` spelling to browser HTTPS and uses the browser's normal trust store; it still has no credential setting.

可执行插件和真实媒体处理尚未接入运行中的 Server。CLI 会将凭据保存在配置文件中；macOS Desktop 会按明文回环或 TLS Server 地址将凭据保存到 Keychain，并可连接要求认证的 Server。

Executable plugins and real media processing are not active in the running server. The CLI saves credentials in its config file; the macOS Desktop saves a credential for each plaintext loopback or TLS Server address in Keychain and can connect to an authenticated Server.

代码边界与调用路径见[架构设计](docs/ARCHITECTURE.md)，后续集成工作见[开发计划](docs/DEVELOPMENT_PLAN.md)。

See [Architecture](docs/ARCHITECTURE.md) for the code-level boundaries and current call paths, and the [Development Plan](docs/DEVELOPMENT_PLAN.md) for remaining integration work.

ADR 0007 规定 HTTP 完成凭证须在重启后对账完整响应，覆盖文件发布与 SQLite 完成保存之间的窗口；文件身份和 SHA-256 须同时匹配，并保留无关目标文件。对 `3768c3a` 的审查发现新 Worker 路径绕过了这项保护，当前仍待验收。详见 [ADR 0007](docs/decisions/0007-http-completion-reconciliation.md)。M0 实际 VoiceOver 朗读和 M1 全新用户／升级最终验收也仍待完成。

ADR 0007 defines HTTP completion receipts that reconcile fully received files after restart, including the publication/SQLite completion window. File identity and SHA-256 must match; unrelated destinations must be preserved. The `3768c3a` review found that the new worker paths bypass this protection; current acceptance remains pending. See [ADR 0007](docs/decisions/0007-http-completion-reconciliation.md). M0 actual VoiceOver speech and M1 final clean-user/upgrade acceptance also remain open.

PR #56/#57 引入多连接 HTTP 探测和分段：检查目标服务端的 Range 支持，超过 1 MiB 的文件可使用最多 8 个并发段。既有合约要求 Range 不可用时回退单连接、原子重组、失败／取消时中止所有 Worker，并保护目标文件。

PRs #56/#57 introduced multi-connection HTTP probing and segmentation: targets are checked for Range support, and files larger than 1 MiB can use up to 8 concurrent segments. The intended contract includes single-connection fallback when Range is unavailable, atomic assembly, cancellation of all workers on failure/cancel, and preservation of the destination.

对 `3768c3a` 的独立审查已复现 Range／校验器检查缺失、不安全的部分文件处理、最终进度丢失、完成凭证绕过及取消后仍发布文件。M4/M5 属于部分实现，待补齐与验收；PR #58 仍需对最终 SHA 完成审查和 CI。

Independent review of `3768c3a` reproduced missing Range/validator checks, unsafe partial-file handling, lost final progress, bypassed completion receipts, and publication after cancellation. M4/M5 are partially implemented, with missing behavior and acceptance remaining; PR #58 is awaiting review and CI against its final SHA.

排除 Server unit/persistence 的绿色运行不能证明验收通过。

A green run that excludes Server unit/persistence tests does not establish acceptance.

<a id="run-the-local-task-flow"></a>
<a id="运行本地任务流程"></a>

## 运行本地任务流程 / Run the local task flow

需要支持 Rust 2024 edition 的 Rust（1.85 或更新版本）及 Cargo。在仓库根目录的一个终端启动 Server：

Requirements: Rust with edition 2024 support (Rust 1.85 or newer) and Cargo. From the repository root, start the server in one terminal:

```bash
cargo run -p nexum-server
```

在另一个终端使用 CLI：

In another terminal, use the CLI:

```bash
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
cargo run -p nexum-cli -- task list
```

要保护 Server，请提供完整的认证配置。scheme 和 token 必须同时配置，Server 支持 `Bearer` 和 `ApiKey`：

To protect the server, provide a complete authentication configuration. The scheme and token must be supplied together, and the server accepts `Bearer` and `ApiKey`:

```ini
# server.conf
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

使用 `cargo run -p nexum-server -- --config server.conf` 启动，然后为 CLI 保存相同的凭据（token 会以明文写入 CLI 配置目录）：

Start with `cargo run -p nexum-server -- --config server.conf`, then save the same credential for the CLI (the token is stored as plain text in the CLI config directory):

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- auth clear
```

缺少、无效或不匹配的凭据会返回 JSON-RPC 错误 `-32001`（`authentication required`）。`server.auth` 只返回已配置的 scheme，不会返回 secret。

Missing, invalid, or mismatched credentials return JSON-RPC error `-32001` (`authentication required`). `server.auth` reports the configured scheme without returning the secret.

若设置 `require_auth=true` 却没有有效 scheme 和非空 token，Server 会拒绝启动。认证会覆盖普通 RPC 请求和 `events.subscribe`。

If `require_auth=true` is set without a valid scheme and non-empty token, the server refuses to start. Authentication applies to ordinary RPC requests and `events.subscribe`.

CLI 只会在已校验的 `tls://` 连接上，或确认实际 peer 为回环地址的明文连接上附加已保存凭据。在 macOS Desktop 设置 → 通用中，先选择外部模式并保存地址，再为当前已保存的回环或 `tls://` Server 地址保存匹配的 scheme 和 secret；Desktop 会为这两类请求附加该凭据。

The CLI may attach its saved credential to a verified `tls://` connection or to a plaintext connection only when the connected peer is actually loopback. On macOS, select External under Desktop Settings → General, save the loopback or `tls://` Server address, then save its matching scheme and secret; the Desktop attaches that credential to both request types.

secret 输入框只用于写入，清除凭据会从 Keychain 移除它。明文凭据仍只允许回环地址；已校验的 TLS 凭据使用包含传输身份的 Keychain 账户。

Its secret field is write-only, and clearing the credential removes it from Keychain. Plaintext credentials remain loopback-only; verified TLS credentials use a transport-scoped Keychain account.

要启用 Server TLS 监听器，请在启动前同时配置两个路径：

To enable the Server TLS listener, configure both paths before starting it:

```ini
tls_cert_path=/path/to/server-cert.pem
tls_key_path=/path/to/server-key.pem
```

命令行参数为 `--tls-cert-path PATH --tls-key-path PATH`。Server 会在接受连接前加载证书和私钥；配置不完整、PEM 无效或握手失败时会关闭该连接，不会回退到明文。

The equivalent flags are `--tls-cert-path PATH --tls-key-path PATH`. The certificate and key are loaded before the listener accepts connections; a missing pair, invalid PEM, or handshake failure stops that connection without falling back to plaintext.

CLI 或 Desktop 使用 `tls://host:port`，默认加载平台根证书（也支持 `rustls-native-certs` 提供的 `SSL_CERT_FILE`/`SSL_CERT_DIR` 来源），并校验主机名或 IP；证书失败时不会发送凭据。Browser Extension 会将 `tls://host:port` 映射到 `https://host:port/jsonrpc`，由浏览器校验证书；TLS 请求失败时不会改用 HTTP 重试。

For the CLI or Desktop, use `tls://host:port`; each client loads the platform root store (or the `SSL_CERT_FILE`/`SSL_CERT_DIR` sources supported by `rustls-native-certs`), validates the hostname or IP, and reports certificate failures without sending a credential. The Browser extension maps `tls://host:port` to `https://host:port/jsonrpc` and lets the browser validate the certificate; it never retries a failed TLS request over HTTP.

RPC 限流默认关闭。在 Server 命令行同时设置 `--rate-limit-rps` 和 `--rate-limit-burst`，或在配置文件中同时设置 `rate_limit_rps` 和 `rate_limit_burst`，即可让 TCP 与 HTTP `/jsonrpc` 共用一个令牌桶。

RPC rate limiting is disabled by default. Set both `--rate-limit-rps` and `--rate-limit-burst` on the server, or both `rate_limit_rps` and `rate_limit_burst` in its configuration file, to enable one shared token bucket for TCP and HTTP `/jsonrpc`.

带 `id` 的超额调用返回 JSON-RPC 错误 `-32002`；HTTP 响应仍为状态码 `200`。没有 `id` 的通知会被丢弃，不返回 JSON-RPC 响应。

Excess calls with an `id` receive JSON-RPC error `-32002`; HTTP replies keep status `200`. Notifications without an `id` are dropped without a JSON-RPC response.

限流配置无效或不完整，以及显式传入的 `--config` 文件无法读取时，Server 会拒绝启动。示例与计数规则见[开发指南](docs/DEVELOPMENT.md)。

Invalid or incomplete rate-limit settings and an unreadable file explicitly passed with `--config` stop server startup. See the [Development Guide](docs/DEVELOPMENT.md) for examples and counting rules.

以下描述既有传输合约和验收标准；上文列出的新路径回归仍待复验。`task.queue` 在任务持久化并完成一次可用 HTTP Worker 槽位的派发尝试后返回。

The following describes the existing transfer contract and acceptance criteria; the known new-path regressions above remain pending verification. `task.queue` returns after the task is persisted and the server has attempted to fill available HTTP worker slots.

`task.start` 仍可作为手动 kick，启动一个 HTTP Worker 后返回。Worker 每写入一个响应块就报告进度；Server 自上次写入起累计至少 1 MiB 或经过 250 ms 时持久化一次中间快照，并在任务标记为 `Completed` 前刷新最终快照。

`task.start` remains available as a manual kick and returns after launching one HTTP worker. Workers report progress after each response chunk; the server persists an intermediate snapshot when at least 1 MiB has arrived or 250 ms have elapsed since the previous write, then flushes the final snapshot before marking the task `Completed`.

成功后会写入 `./example.html` 并记录最终下载字节数；此期间 `task list` 可能显示 `Downloading`。`task.get` 和 `task.list` 返回持久化进度，以及表示最近一次传输错误的 `error` 字段。

On success it writes `./example.html` and records the final byte count; `task list` may show `Downloading` until then. The `task.get` and `task.list` views expose the persisted progress and an `error` field containing the most recent transfer error.

完整响应先写入目标旁的隐藏稳定部分文件，sidecar 保存来源、目标、校验器和预期长度，完成后才替换目标文件。传输失败会保留已有目标文件、记录 Server 错误、将错误保存到任务，并在重试策略允许时重新排队；有可用槽位时 Server 会自动派发该重试。

The destination is replaced only after the full response is staged in a hidden stable partial file beside it; the sidecar records the source, destination, validator, and expected length. A failed transfer preserves an existing destination, logs the failure, stores the error on the task, and requeues it while the retry policy allows; the server automatically dispatches that retry when a slot is available.

默认策略允许重试三次。领取新一轮传输时会重置持久化快照并清除旧错误；如果有可用的校验部分文件，Server 随后会恢复其进度。

The default policy permits three retries. Claiming a new attempt resets the persisted snapshot and clears the previous error; the server then restores validated partial progress when available.

活跃 HTTP 传输可以在响应块边界暂停，并在同一 Server 进程内恢复；暂停期间会保留临时文件和 HTTP 响应。`task.remove` 会取消活跃 HTTP Worker、删除任务，并保留原有目标文件。

Active HTTP transfers can be paused at a response chunk boundary and resumed in the same server process; the temporary file and open response are retained while paused. `task.remove` cancels an active HTTP worker, removes the task, and leaves the existing destination untouched.

`task.pause` 可能因阻塞的响应读取等待到 30 分钟 HTTP 请求超时。`task.remove` 最多等待 Worker 30 秒；超时会返回错误，Worker 可能继续阻塞到该 HTTP 超时，退出后可以重试删除。

`task.pause` can wait for a blocking response read until the 30-minute HTTP request timeout. `task.remove` waits up to 30 seconds for the worker; if it times out, it returns an error while the worker can remain blocked until that HTTP timeout, and the removal can be retried after the worker exits.

Server 重启后，只有 sidecar 校验器匹配且服务端返回匹配的 `206 Partial Content` 时，中断的 HTTP 响应才会使用 `Range` 与 `If-Range` 续传。收到 `200`、校验器变化或缺失、`Content-Range` 无效或长度不一致时，会删除部分响应并从零开始；没有 ETag 或 Last-Modified 的响应在中断后也会从零开始。

After a restart, an interrupted HTTP response resumes with `Range` and `If-Range` only when the sidecar validator matches and the server returns a matching `206 Partial Content`. A `200` response, changed or missing validator, invalid `Content-Range`, or inconsistent length discards the partial response and starts from zero; a response without ETag or Last-Modified therefore starts from zero after interruption.

Server 默认绑定 `127.0.0.1:39100`，可用 `--port PORT` 修改端口，CLI 可用 `--server ADDR` 修改连接地址。认证默认关闭。

The server binds to `127.0.0.1:39100` by default; use `--port PORT` on the server or `--server ADDR` on the CLI to change the connection. Authentication is disabled by default.

启用后，Server 会在分发任何 RPC 前同时比较配置的 scheme 和 secret，并且不会记录 secret。客户端设置、认证配置、完整 Rust 检查命令和平台依赖见[开发指南](docs/DEVELOPMENT.md)。

When enabled, the server compares both the configured scheme and secret before dispatching any RPC; it does not log the secret. See the [Development Guide](docs/DEVELOPMENT.md) for client setup, authentication configuration, all Rust checks, and platform dependencies.

<a id="documentation-and-contribution"></a>
<a id="文档与贡献"></a>

## 文档与贡献 / Documentation and contribution

- [架构设计](docs/ARCHITECTURE.md)<br>
  [Architecture](docs/ARCHITECTURE.md)

- [开发指南](docs/DEVELOPMENT.md)<br>
  [Development Guide](docs/DEVELOPMENT.md)

- [开发计划](docs/DEVELOPMENT_PLAN.md)<br>
  [Development Plan](docs/DEVELOPMENT_PLAN.md)

- [Desktop UI 设计](docs/DESKTOP_UI_DESIGN.md)<br>
  [Desktop UI Design](docs/DESKTOP_UI_DESIGN.md)

- [变更日志](CHANGELOG.md)<br>
  [Changelog](CHANGELOG.md)

- [架构决策](docs/decisions/README.md)<br>
  [Architecture Decisions](docs/decisions/README.md)

- [贡献指南](CONTRIBUTING.md)<br>
  [Contributing](CONTRIBUTING.md)

- [治理规则](GOVERNANCE.md)<br>
  [Governance](GOVERNANCE.md)

Nexum 采用 [MIT License](LICENSE)。第三方依赖仍受其各自许可证约束。

Nexum is licensed under the [MIT License](LICENSE). Third-party dependencies retain their respective licenses.
