<a id="changelog"></a>
<a id="变更日志"></a>

# 变更日志 / Changelog

此处记录 Nexum 的重要变更，条目不代表所有里程碑已验收。PR #58 已合并为 `f1817f4`，恢复单连接 HTTP 与启动完成对账，验证记录见[开发指南](docs/DEVELOPMENT.md#development-workflow-entry)。PR #61 已将大文件下载统一到会话路径，并通过合并后 Rust 与 Desktop macOS CI；M4/M5 仍属部分实现，M0/M1/M3 的剩余验收见[开发计划](docs/DEVELOPMENT_PLAN.md)。

This changelog records notable changes, not acceptance of every milestone. PR #58 merged as `f1817f4`, restoring single-connection HTTP and startup completion reconciliation; see the [Development Guide](docs/DEVELOPMENT.md#development-workflow-entry) for verification records. PR #61 routes large downloads through the session path and passed post-merge Rust and Desktop macOS CI. M4/M5 remain partial; see the [Development Plan](docs/DEVELOPMENT_PLAN.md) for remaining M0/M1/M3 acceptance.

<a id="unreleased"></a>
<a id="未发布"></a>

## 未发布 / Unreleased

<a id="added"></a>
<a id="新增"></a>

### 新增 / Added

- **单文件多连接 HTTP 基础——未接入产品，待补齐与验收**：引擎保留 `probe_range()`、`split_segments()` 和 `download_to_segments()`；分段实现以 `Range: bytes=0-0` 探测能力，可为文件构造最多 8 个并发段，每段使用独立的 current-thread Tokio runtime 和 std 线程。这些 API 的存在及辅助测试不代表多连接下载已支持。

  **Multi-connection HTTP foundations — outside the product path, incomplete and unaccepted**: the engine retains `probe_range()`, `split_segments()`, and `download_to_segments()`. The segment implementation probes with `Range: bytes=0-0` and can construct up to 8 concurrent segments, using a separate current-thread Tokio runtime and std thread per segment. API presence and helper tests do not establish multi-connection download support.

  各段共享 `HttpTransferControl` 实现全局带宽上限，通过持有的文件句柄按显式偏移写入预分配的稀疏部分文件。既有合约要求失败／取消时中止全部 Worker 并清理部分文件、保护已有目标，并在服务端不支持、校验失败或段长度异常时安全回退单连接。

  Segments share `HttpTransferControl` for a global bandwidth cap and write to explicit offsets in a pre-allocated sparse partial file through a held file handle. The intended contract aborts all workers and removes the partial on failure/cancel, preserves existing destinations, and falls back safely to a single connection for unsupported servers, failed validation, or abnormal segment lengths.

  PR #61 已移除 Server 自动分段／探测入口及按 fixture URL 绕过探测的逻辑，全部 HTTP/HTTPS 下载统一进入带完成凭证的单连接 session。此项收敛不等于 M4 完成；段级持久化、跨重启段恢复、安全单连接回退及真实分段路径的保护仍待实现和验收。

  PR #61 removed automatic segmentation/probing and fixture-URL probing bypasses from the Server; every HTTP/HTTPS download uses the single-connection session with completion receipts. This does not complete M4: durable segment state, cross-restart segment recovery, safe single-connection fallback, and protection through the real segment path remain to be implemented and accepted.

  对 `3768c3a` 的直接审查曾复现错误 Range／校验器被接受、不安全的部分文件处理及取消后仍发布。PR #58 修复单连接 session 与启动恢复，但没有验收分段执行；未完成这些保护前不重新启用产品分段入口。

  Direct review at `3768c3a` reproduced incorrect Range/validator acceptance, unsafe partial-file handling, and publication after cancellation. PR #58 repairs the single-connection session and startup recovery, without accepting segment execution. Product segment dispatch remains disabled until these protections are complete.

- **分段原型的输入与响应校验**：要求按序、无缺口／重叠的完整段集合及有效强 ETag，在请求／创建暂存文件前拒绝零长、越界、整数溢出以及缺失、弱或语法无效的 ETag。Range 探测校验精确的一字节范围与实际响应，响应头至 EOF 的总等待上限为 3 秒；拒绝重复的关键响应头。各段校验 `206`、起止／总长、相同 ETag、identity 内容编码及实际长度；传输编码仅允许缺省或单一 `chunked`，保留具体失败原因。零字节文件不分段，拆段不再使用浮点计算。

  **Input and response checks for the segment prototype**: require ordered, complete segments without gaps/overlap and a valid strong ETag. Before requests/staging creation, reject zero length, out-of-bounds ranges, integer overflow, and missing, weak or syntactically invalid ETags. Range probes validate the exact one-byte range and body within a three-second total deadline through headers and EOF. Reject duplicate critical response headers; each segment checks `206`, bounds/total, a matching ETag, identity content encoding and actual length. Transfer-Encoding is limited to absent or a single `chunked` value, preserving specific failure causes. Empty files are not segmented, and splitting no longer uses floating-point arithmetic.

  PR #62 已通过完整门槛、独立审查及合并前后 CI，合并为 `3fcd259`。此项不改变单连接的 ETag/Last-Modified 策略，多连接仍未接回 Server。

  PR #62 passed the full gates, independent review and CI before/after merge, landing as `3fcd259`. This preserves the single-connection ETag/Last-Modified policy; Server multi-connection dispatch remains disabled.

- **分段原型的暂存文件持有与身份保护**：`create_new` 拒绝并保留已有普通文件、符号链接、FIFO 或目录。所有段共享持有的普通文件句柄，以显式偏移写入；在请求、写入及发布／清理边界核对文件身份，最终长度从持有句柄读取。已观察到路径被替换时拒绝后续写入、发布或清理替换文件；分段原型不删除其未创建的 JSON sidecar。

  **Held staging files and identity checks for the segment prototype**: `create_new` rejects and preserves pre-existing regular files, symlinks, FIFOs and directories. Every segment writes at explicit offsets through one held regular-file handle. Requests, writes, publication and cleanup check file identity, and the final length comes from the held handle. An observed path replacement rejects further writes/publication or cleanup of the replacement. The segment prototype does not delete JSON sidecars it did not create.

  身份检查与基于路径的 rename／hard-link／unlink 之间仍存在 TOCTOU，尚不能抵抗同权限进程持续并发改名；文件系统原子身份绑定、安全回退及段级恢复仍待实现和验收，Server 多连接保持禁用。

  A TOCTOU window remains between identity checks and path-based rename/hard-link/unlink; continuous concurrent renames by a same-privilege process are not covered. Atomic filesystem identity binding, safe fallback and segment recovery still need implementation and acceptance; Server multi-connection routing remains disabled.

- **分段 Worker 的暂停／退出屏障**：分段 Worker 和进度协调者共同参与暂停确认；全部停稳后才报告暂停，恢复时继续原请求。响应头与响应体等待保留同一个 Future 分段检查控制，不因控制轮询重发 GET；限速与暂停等待也观察组内失败或取消。任一 Worker、进度回调或线程启动失败时唤醒其他参与者，收拢全部已启动 Worker 后才清理和标记完成，保留具体错误及取消类型。

  **Segment-worker pause/termination barriers**: segment workers and the progress coordinator jointly acknowledge pause only when all are parked, then resume their existing requests. Header/body waits retain one future while polling controls without repeating GETs; bandwidth and pause waits also observe group failure/cancellation. Worker, progress-callback and thread-start failures wake other participants. Join every started worker before cleanup/completion, preserving concrete failures and cancellation types.

  这是未启用分段原型的协作控制；同步文件系统调用、底层系统 DNS 解析与不返回的用户回调无法被强制终止。Server 多连接仍禁用，原子文件所有权、安全回退及段级恢复仍为后续门槛。

  This is cooperative control for the disabled segment prototype; synchronous filesystem calls, underlying OS DNS resolution and caller callbacks that never return cannot be forcibly terminated. Server multi-connection routing remains disabled; atomic filesystem ownership, safe fallback and segment recovery remain subsequent gates.

- **分段原型的完成凭证与发布协调**：新增兼容的 `download_to_segments_with_completion` 入口，按任务／来源／目标绑定优先离线对账；无凭证才验证新传输计划。所有 Worker 退出后准备身份／摘要凭证，发布和取消在同一控制锁内协调，提交成功后不再误报迟到取消；恢复暂停后在锁内重新校验文件身份。

  **Segment-prototype receipts and publication coordination**: add compatible `download_to_segments_with_completion`, prioritizing offline reconciliation bound to task/source/destination; validate a fresh transfer plan only without a receipt. After worker exit, prepare identity/digest proofs and coordinate publication/cancellation under one control lock. Successful commits cannot become late cancellations; recheck identity under the lock after a publication pause.

  凭证已落盘后的普通错误保留完整字节和证明，即使错误发生在目录同步。取消先核对凭证归属、清理自有暂存文件，再清理匹配凭证；清理失败明确返回 `Failed`，保留取消请求状态和诊断证据，调用者不能自动重试已请求取消的 control。分段凭证不认领无关续传 sidecar；成功后的凭证由调用者在最终进度／`Completed` 原子保存后确认清理。

  Ordinary errors after receipt publication retain complete bytes and proof, including directory-sync failure. Cancellation validates proof ownership, removes owned staging, then removes matching proof. Cleanup failure explicitly returns `Failed`, preserving cancellation-request state and diagnostic evidence; callers must not automatically retry a cancelled control. Segment receipts never claim unowned resume sidecars. Successful receipts await caller acknowledgement after atomic final-progress/`Completed` persistence.

  真实 Server 测试验证现有恢复消费者可离线处理新凭证；Server 分段派发、安全回退和段级恢复仍未启用或验收，路径检查到改名／删除的竞态仍待处理。后续顺序已记录在开发计划，本轮公共 API 合约见 RFC 0001。

  Real Server tests verify existing recovery consumers can handle new receipts offline. Server segment dispatch, safe fallback and segment restart reuse remain disabled or unaccepted; check-to-rename/unlink races remain open. The development plan records the sequence, and RFC 0001 records this slice’s public API contract.

- **恢复测试的终态同步**：第二个 Server 的数据锁保护测试在重启后等待异步恢复／重试进入 `Failed`，再同时核对 RPC 与 SQLite 终态和持久化错误。监听就绪不代表任务已完成重试；保留原有拒锁及活动任务状态保护。

  **Terminal-state synchronization in recovery tests**: after restart, the second-Server lock test waits for asynchronous recovery/retries to reach `Failed`, then verifies RPC/SQLite terminal state and the persisted error together. Listening readiness does not imply completed retries. Existing lock-refusal and live-task state protections remain covered.

- **HTTP 完成对账**：单连接 session 在发布前以私有凭证绑定完整字节和任务／文件身份。PR #58 恢复 session 的凭证接线、最终进度与 `Completed` 一次持久化，并在 Server 监听和网络派发前真正执行启动对账；可无网络校验已发布文件或重新发布完整暂存文件，包括无 validator 且未知长度的完整响应。清理和 ID 复用保留冲突文件，具体失败原因进入任务视图。<br>
  **HTTP completion reconciliation**: before publication, the single-connection session binds complete bytes to task/file identity using private receipts. PR #58 restores session receipt wiring, atomic final progress/`Completed` persistence, and startup reconciliation before the Server listens or dispatches network work. Published files can verify or complete staging bytes can republish without network access, including complete unknown-length responses without validators. Cleanup and ID reuse preserve conflicting files, and task views retain specific failure causes.

- **Desktop 托管 Server**：随包可执行文件校验、私有鉴权启动、稳定 App 数据、有限子进程管理、托管／外部设置、后端生成任务 ID 和不冲突目标。关闭窗口保留下载，退出停止拥有的子进程。M1 最终发布验收仍待完成。<br>
  **Desktop managed Server**: bundled executable verification, private authenticated startup, stable App storage, bounded child supervision, Managed/External settings, backend-generated IDs and collision-safe destinations. Closing the window keeps downloads running; Quit stops the owned child. M1 final release acceptance remains open.

- **Core**：任务生命周期、Scheduler、Resolver 集成与事件收集。SQLite 存储和重启恢复现已接入运行中的 Server；恢复时会将原先下载中、暂停或重试中的任务归一为 `Queued` 并持久化。<br>
  **Core**: task lifecycle, scheduler, resolver integration, and event collection. SQLite storage and restart recovery now support the running server; recovery persists the normalized `Queued` state of previously downloading, paused, or retrying tasks.

- **Engine**：内存与 HTTP 适配器。HTTP 适配器跟随重定向，将响应暂存到 `.part` 文件，在完整下载后重命名到目标路径，在每个响应块写入后报告进度，并为 Server Worker 提供协作式暂停/取消控制。Server 自有的传输路径现在会保留稳定的隐藏部分文件和原子 JSON sidecar，并在重启后仅续传校验器匹配的 `206 Partial Content` 响应。通用同步 Adapter 仍不提供暂停/恢复能力。<br>
  **Engine**: in-memory and HTTP adapters. The HTTP adapter follows redirects, stages the response in a `.part` file, renames a complete download into place, reports progress after each written response chunk, and exposes cooperative pause/cancel control for server workers. The server-owned path now keeps a stable hidden partial file and atomic JSON sidecar, and resumes only a validator-matched `206 Partial Content` response after restart. The generic synchronous adapter remains without pause/resume capability.

- **Protocol**：JSON-RPC 2.0 任务与服务器信息方法、版本标识、凭据字段、事件封装和缓冲类型，以及面向订阅客户端的 `events.event` 通知。`task.get` 和 `task.list` 返回的 `TaskView` 包含持久化进度，以及表示最近一次传输错误的 `error` 字段。Protocol 定义了认证失败错误 `-32001`；Server 会在 RPC 分发和事件订阅前校验配置的 Bearer 或 ApiKey 凭据。<br>
  **Protocol**: JSON-RPC 2.0 task and server-info methods, a version identifier, credential fields, event envelope/buffer types, and `events.event` notifications for subscribed clients. `TaskView`, returned by `task.get` and `task.list`, includes persisted progress and an `error` field for the most recent transfer error. The protocol defines `-32001` for authentication failures; the server validates configured Bearer or ApiKey credentials before RPC dispatch and event subscription.

- **Server**：仅监听本机的逐行 TCP JSON-RPC 服务，每个连接使用一个线程。Server 持有数据目录锁，将任务保存在 `data_dir/nexum.sqlite`，并在监听前执行恢复；目录、锁、数据库或恢复失败会阻止启动。

  **Server**: localhost, line-delimited TCP JSON-RPC with one thread per connection. It holds a data-directory lock, stores tasks in `data_dir/nexum.sqlite`, recovers them before listening, and fails startup on directory, lock, database, or recovery errors.

  排队 HTTP/HTTPS 任务、启动时恢复排队任务，以及活动 HTTP 传输完成或失败，都会触发 Server 自动派发器。派发器遵守 Scheduler 并发上限，只领取符合条件的任务，并跳过不支持的来源和重叠目标。

  Queueing an HTTP/HTTPS task, recovering queued work at startup, and completing or failing an active HTTP transfer trigger the server's automatic dispatcher. It claims eligible tasks up to the scheduler concurrency limit while skipping unsupported sources and overlapping destinations.

  `task.start` 仍是手动 kick，会启动一个 HTTP/HTTPS Worker 后返回，然后使用同一派发器填充其他槽位。响应新增至少 1 MiB 或经过 250 ms 时 Server 持久化中间进度，并同时持久化最终进度和 `Completed`。

  `task.start` remains a manual kick that launches one HTTP/HTTPS worker and returns before transfer completion, then uses the same dispatcher to fill other slots. The server persists progress when a response has added 1 MiB or 250 ms have elapsed, then persists final progress and `Completed` together.

  失败时会在 `TaskView` 中保留错误，并在默认三次重试预算未耗尽时重新排队；有可用槽位时派发器会启动重试，预算耗尽后任务保持 `Failed`。新一轮领取会清除旧错误。

  Failures preserve the error in `TaskView` and requeue while the default three-retry budget remains; the dispatcher starts the retry when a slot is available, and after the budget is exhausted the task stays `Failed`. A new claim clears the previous error.

  活跃 HTTP Worker 现在支持响应块边界 `task.pause`、同进程 `task.resume` 和破坏性 `task.remove` 取消；响应体读取按 1 秒无数据窗口检查暂停／取消，连续 8 次无数据判为停滞；删除最多等待 Worker 10 秒，超时返回错误。重启恢复会对校验器匹配的 HTTP 部分响应使用 `Range`/`If-Range`；校验器缺失或变化、收到 `200`、`Content-Range` 无效、长度不一致，或响应没有校验器时，都会从零开始。

  Active HTTP workers now support chunk-boundary `task.pause`, same-process `task.resume`, and destructive `task.remove` cancellation; response-body reads check pause/cancel at one-second no-data windows and fail as stalled after eight consecutive empty windows. Removal waits up to 10 seconds for the worker and returns an error on timeout. Restart recovery resumes a validator-matched HTTP partial with `Range`/`If-Range`; missing or changed validators, a `200` response, invalid `Content-Range`, inconsistent length, or a response without a validator fall back to byte zero.

  客户端可以通过独立 `events.subscribe` TCP 连接接收带序号的 Task/Scheduler 通知；事件流没有回放缓冲，连接或重连后客户端会刷新完整快照。配置的 `max_connections` 会在 TCP 接入时执行，超出上限的连接会立即关闭。

  Clients can use a dedicated `events.subscribe` TCP connection for sequenced task and scheduler notifications; the stream has no replay buffer, so clients refresh a full snapshot after connecting or reconnecting. The configured `max_connections` limit is enforced at TCP admission; excess connections are closed immediately.

  启用 `require_auth` 后，Server 会在每个 RPC 和 `events.subscribe` 前要求匹配的 `auth_scheme` 与非空 `auth_token`；缺少或无效凭据会返回 `-32001`（`authentication required`）。`server.auth` 只返回当前 scheme，不会暴露 token；认证配置不完整时会阻止启动。

  When `require_auth` is enabled, the server requires a matching `auth_scheme` and non-empty `auth_token` before every RPC and `events.subscribe`; missing or invalid credentials return `-32001` (`authentication required`). `server.auth` reports the active scheme without exposing the token, and incomplete authentication configuration stops startup.

- **CLI**：任务命令、可复用的 TCP JSON-RPC Client、保存服务器地址与明文凭据的配置、`auth set`/`auth clear` 校验、服务器信息查询、RPC 错误格式化，以及使用系统根证书和主机名校验的 `tls://host:port` 传输。明文凭据只会发给实际回环 peer；TLS 凭据只会在握手成功后发送。<br>
  **CLI**: task commands, a reusable TCP JSON-RPC client, saved server address and plain-text credential fields, `auth set`/`auth clear` validation, server inspection, RPC error formatting, and a verified `tls://host:port` transport using system roots and hostname validation. Plaintext credentials are sent only to an actual loopback peer; TLS credentials are sent only after a successful handshake.

- **Desktop**：通过 TCP 连接 Server 的 Tauri 2 + React 任务界面，提供服务器地址输入、原生目标选择器、独立事件订阅、按连接检测 sequence 缺口、去抖任务刷新，以及重连后完整快照同步的轮询回退。在 macOS 上，通用设置可按 Server 地址将 Bearer/ApiKey 凭据保存到 Keychain 或清除。

  **Desktop**: Tauri 2 + React task UI connected to the TCP server, with a server-address field, native destination picker, dedicated event subscription, per-connection sequence-gap detection, debounced task refresh, and polling fallback with full-snapshot resync after reconnects. On macOS, General settings save and clear per-Server Bearer/ApiKey credentials in Keychain.

  secret 不会回读到 React；Tauri 为普通 RPC 和 `events.subscribe` 附加凭据。保存、清除或切换当前 Server 会重启事件流并刷新 Task 快照。

  The secret is not returned to React; Tauri attaches it to ordinary RPC and `events.subscribe`. Saving, clearing, or changing the active Server restarts the event stream and refreshes the task snapshot.

  明文凭据仍仅用于回环地址；已校验的 TLS 地址使用按传输身份隔离的 Keychain 账户。其他平台的 Desktop 仍可无认证连接，但不支持保存或清除凭据。

  Plaintext credentials remain restricted to loopback addresses; verified TLS addresses use transport-scoped Keychain accounts. Other platforms retain unauthenticated Desktop connections but do not support credential save or clear.

  Tauri 的 JavaScript 与 Rust 包已固定为兼容的次版本，macOS bundle ID 也不再以 `.app` 结尾。

  Tauri JavaScript and Rust versions are pinned to compatible minor releases, and the macOS bundle identifier no longer ends in `.app`.

- **Desktop 外观与语言**：明暗配色跟随 macOS 系统外观，并保留现有 Nexum 强调色。设置提供跟随系统、英文和简体中文选择；语言即时与 Server 地址、刷新策略一起保存，旧设置默认跟随系统。类型化文案目录覆盖 Desktop 自有的可见及无障碍文字；Server/操作系统的原始诊断信息保持原文。手动主题和强调色控制、原生视觉检查与截图回归覆盖仍待完成。<br>
  **Desktop appearance and language**: light and dark palettes follow the macOS system appearance while retaining the existing Nexum accent. Settings offer System, English, and Simplified Chinese; the choice saves immediately with the Server address and refresh policy, and older settings default to System. A typed message catalog covers Desktop-owned visible and accessibility-facing strings; raw Server/OS diagnostics remain verbatim. Manual theme and accent-color controls, native visual checks, and screenshot regression coverage remain planned.

- **Desktop 可访问性**：`⌘N` 打开 Add Download，`⌘F` 聚焦 Downloads 搜索；Escape 可关闭两者。任务选择支持 Enter/Space、上/下方向键及 Home/End。Add Sheet 限制 Tab 焦点并在关闭时恢复焦点；任务控件与 Inspector 进度提供可访问名称和状态。桌面过渡及不确定进度动画遵循减少动效设置。任务原生上下文菜单仍待实现。<br>
  **Desktop accessibility**: `⌘N` opens Add Download and `⌘F` focuses Downloads search; Escape closes either surface. Task selection supports Enter/Space, Up/Down, and Home/End. The Add sheet traps Tab focus and restores focus on close, while task controls and Inspector progress expose accessible names and status. Desktop transitions and indeterminate progress animation honor reduced-motion preferences. Native task context menus remain planned.

- **Browser**：Manifest V3 右键菜单、链接检测和弹窗配置。扩展根据保存的地址通过 HTTP 或 HTTPS `POST /jsonrpc` 桥接发送 `task.create` 和 `task.queue`；不带 scheme 的 `host:port` 保持 HTTP，`tls://host:port` 映射到浏览器 HTTPS，由浏览器正常校验证书且不降级到 HTTP。通过校验的扩展 Origin 可收到 JSON 请求的 CORS 头，网页 Origin 会被拒绝，事件订阅仍只走 TCP，凭据设置尚未提供。<br>
  **Browser**: Manifest V3 context menu, link detection, and popup configuration. The extension sends `task.create` and `task.queue` through the server's HTTP or HTTPS `POST /jsonrpc` bridge based on the saved address; bare `host:port` remains HTTP, while `tls://host:port` maps to browser HTTPS with normal certificate validation and no HTTP downgrade. JSON requests from validated extension origins receive CORS headers while web-page origins are rejected, event subscriptions remain TCP-only, and credential settings are not yet available.

- **Security**：凭据、TLS 和限流配置类型。Server 现在会按配置执行 Bearer/ApiKey 鉴权，以及由 TCP 与 HTTP `/jsonrpc` 共用一个令牌桶的可选进程级 RPC 限流。

  **Security**: credential, TLS, and rate-limit types. The server enforces configured Bearer/ApiKey authentication and optional process-wide RPC rate limiting with one token bucket shared by TCP and HTTP `/jsonrpc`.

  限流默认关闭，启用时须提供完整、有效的速率和突发容量参数。已解析且通过认证的请求（包括 `events.subscribe`）消耗令牌；认证失败、事件 heartbeat 和 HTTP `OPTIONS` 预检不消耗。

  Limiting is disabled by default; complete, valid rate and burst settings are required to enable it. Parsed, authenticated requests, including `events.subscribe`, consume tokens; authentication failures, event heartbeats, and HTTP `OPTIONS` preflight do not.

  带 `id` 的超额调用返回 JSON-RPC `-32002`（HTTP 状态码 `200`）；无 `id` 的通知会被丢弃，不返回 JSON-RPC 响应。限流配置无效或不完整，以及显式传入的 `--config` 文件无法读取，会阻止启动。

  Excess calls with an `id` receive JSON-RPC `-32002` (HTTP status `200`); notifications without an `id` are dropped without a JSON-RPC response. Invalid or incomplete rate-limit settings and unreadable files explicitly passed with `--config` stop startup.

  Server 现在支持成对配置 PEM 证书和私钥，让 TCP JSON-RPC、事件订阅和 HTTP `/jsonrpc` 使用 TLS；CLI 和 Desktop 会校验 `tls://` Server 名称和系统根证书，Browser 会将 `tls://` 映射到浏览器 HTTPS 信任路径且不降级。

  The server now accepts paired PEM certificate and private-key paths for TLS across TCP JSON-RPC, event subscriptions, and HTTP `/jsonrpc`; the CLI and Desktop verify `tls://` server names with system roots, and Browser maps `tls://` to the browser's HTTPS trust path without downgrade.

- **Tests**：运行时生成的证书覆盖 TLS TCP/HTTPS 成功请求、明文拒绝、Browser 扩展 CORS 允许/拒绝、TCP/HTTPS 上的配置鉴权、带鉴权的 `events.subscribe`、Server 启动时拒绝不完整或无效的证书/私钥材料，以及握手时拒绝不受信任或主机名不匹配的证书。CLI 测试验证只有可信握手后才发送凭据，信任或主机名校验失败后不发送凭据，且不重试明文；Desktop 测试验证普通 RPC 和事件订阅使用相同的凭据门控。<br>
  **Tests**: runtime-generated certificates cover successful TLS TCP/HTTPS requests, plaintext rejection, browser-extension CORS allow/reject behavior, configured TCP/HTTPS authentication, authenticated `events.subscribe`, Server startup rejection for incomplete or invalid certificate/key material, and handshake rejection for untrusted or wrong-hostname certificates. CLI tests verify that credentials are sent after a trusted handshake, never sent after trust or hostname failure, and never retried over plaintext; Desktop tests exercise the same credential gate for RPC and event subscriptions.

- **Plugin**：Manifest、Permission、Capability、生命周期状态及 Provider Trait 基础；Core 可记录插件 Manifest，但尚未加载 Provider 提供的 Engine 或 Resolver。<br>
  **Plugin**: manifest, permission, capability, lifecycle-state, and provider-trait foundations. Core tracks plugin manifests but does not yet load provider engines or resolvers.

- **Media**：媒体与工作流模型，以及模拟的内存探测和任务执行；尚未接入实际媒体处理。<br>
  **Media**: media and workflow models with simulated, in-memory probing and job execution; no real media processing is wired in.

<a id="documentation"></a>
<a id="文档"></a>

### 文档 / Documentation

- 同步 README、架构、开发指南和合并后的开发计划，使其与当前仓库实现状态一致，并记录认证配置合约与 CLI 凭据流程。<br>
  Synchronized README, architecture, development guides, and the consolidated development plan with the implemented repository state, including the configured authentication contract and CLI credential workflow.

- 保持英文与简体中文项目文档同步；对应根文档现合并为单个双语文件。<br>
  Kept English and Simplified Chinese project documentation aligned; corresponding root documents now share one bilingual file.

- 记录 Dependabot 的 `glib 0.18.5` 告警，以及 GTK 3/Tauri 约束如何阻止不安全地强制替换为 `glib 0.20`；Tauri 2.12/wry 0.57 在 Linux 上仍使用受影响的 GTK 3 版本线。<br>
  Recorded the Dependabot `glib 0.18.5` advisory and the GTK 3/Tauri constraint that prevents an unsafe forced `glib 0.20` override; Tauri 2.12/wry 0.57 still use the affected GTK 3 line on Linux.

<a id="fixed"></a>
<a id="修复"></a>

### 修复 / Fixed

- **CI**：移除重复 Workflow 并修复 Workspace 兼容性问题。当前基线执行格式化、检查、Clippy 和部分测试，但排除 Server unit/persistence；绿色 CI 不代表完整 Rust 验收，须恢复完整门槛并复验 PR #58 最终 SHA。<br>
  **CI**: removed redundant workflow configuration and repaired workspace compatibility issues. The current baseline has formatting/check/Clippy gates and selected tests, but excludes Server unit/persistence tests. Green CI does not prove full Rust acceptance; restore the complete gate and verify PR #58 at its final SHA.

- **Branding**：刷新 Desktop、Browser Extension 与 macOS 图标资源，并修复无效的 PNG / ICNS 数据。<br>
  **Branding**: refreshed Desktop, Browser Extension, and macOS icon assets and corrected invalid PNG/ICNS payloads.

## [0.1.0] — 待定 / TBD

- 初始项目结构与公开开发基础。<br>
  Initial project structure and public development foundation.
