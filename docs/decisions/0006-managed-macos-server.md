<a id="adr-0006-app-managed-local-server-for-macos"></a>
<a id="adr-0006由-macos-app-托管本地-server"></a>

# ADR 0006：由 macOS App 托管本地 Server / App-Managed Local Server for macOS

- 状态：M1 设计已采纳；Server 启动协议和 Desktop 管理者已接入，最终验收待完成<br>
  Status: Accepted for M1; Server bootstrap and Desktop supervision wired; final M1 acceptance pending
- 日期 / Date: 2026-10-10

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

M1 托管切片之前，Desktop 是 TCP JSON-RPC 客户端，默认连接 `127.0.0.1:39100`，在 `settings.json` 保存地址，必须先单独启动 Server。Server 默认将 `nexum.sqlite` 放在相对于启动目录的 `./data`，并锁定该数据目录。同一个监听器还承载 HTTP 桥接与事件流。仅绑定回环地址不能证明 Server 属于 Desktop：其他本地进程可先占用固定端口或调用 RPC。HTTP 临时文件及其校验元数据位于目标文件旁，遵循 [ADR 0004](0004-cross-restart-http-resume.md)。

Before the M1 managed slice, the Desktop was a TCP JSON-RPC client. It defaults to `127.0.0.1:39100`, stores that address in `settings.json`, and needs a separately started Server. The Server defaults to `./data/nexum.sqlite` relative to its launch directory and locks that data directory. Its listener also carries the HTTP bridge and event stream. A loopback listener alone does not authenticate the Desktop: another local process can bind a fixed port before the Server or call its RPC methods. HTTP partial files and their validator sidecars live beside each destination under [ADR 0004](0004-cross-restart-http-resume.md).

M1 要让 macOS App 在打开后粘贴 HTTP(S) URL 就能下载。Nexum 尚未发布，因此 M1 不需要迁移早期开发构建或单独启动的 CLI Server 数据。已有开发数据不能被删除，也不能被静默接管。

M1 makes a macOS installation usable by opening the App and pasting an HTTP(S) URL. Nexum has not been released, so M1 does not need migration from earlier development builds or separately launched CLI Servers. Existing development data must not be deleted or silently attached.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

<a id="ownership-identity-and-packaging"></a>
<a id="进程归属身份与打包"></a>

### 进程归属、身份与打包 / Ownership, identity, and packaging

1. Tauri Rust 后端是 **托管模式**中唯一的进程管理者，只启动一个随 App 打包的 `nexum-server` 子进程。前端通过后端命令请求任务操作；不选择托管 RPC 地址，也不接收托管凭据。App 不使用系统服务、登录项、Shell 或 `PATH` 中的可执行文件。App 包含与目标架构匹配的 Server，并直接启动解析出的包内绝对路径。构建时将 Server 二进制文件的 SHA-256 摘要嵌入 App，后端启动前连同构建版本和协议版本一起校验；缺失、被修改或不兼容时显示启动错误，不能回退到已安装的 Server。M3 再完成 App 与嵌套二进制文件的分发签名。<br>
   The Tauri Rust backend is the sole supervisor of one bundled `nexum-server` child for the App's **Managed** mode. The frontend requests task operations from backend commands; it never selects the managed RPC address or receives the managed credential. The App does not use a system service, a login item, a shell, or an executable found on `PATH`. The App bundles the matching architecture's Server executable and launches its resolved absolute bundle path directly. The App build embeds the Server binary's SHA-256 digest, which the backend checks before launch alongside its build and protocol versions; a missing, changed, or incompatible binary is a visible startup error, with no fallback to an installed Server. M3 adds distribution signing of the App and nested binary.
2. 默认进入托管模式。**外部模式**仍是设置中的独立选项，可指定地址、TLS 行为及按地址存储的 Keychain 凭据。切换模式绝不向外部地址发送托管令牌，也不停止外部进程。M1 不包含对发布前旧版 Desktop 设置的兼容或数据迁移路径。<br>
   Managed mode is the default. **External** mode remains a separate Settings choice for a user-specified address, TLS behavior, and per-address Keychain credential. Switching modes never sends the managed token to an external address and never stops an external process. No compatibility or data migration path for pre-release Desktop settings is part of M1.
3. App 保留子进程的操作系统句柄及私有 stdin/stdout 管道，以此确认归属。托管启动使用专用 Server 参数，stdin 承载简短的分帧控制协议，stdout 只发送一条就绪响应；诊断信息写入 stderr。父进程通过 stdin 请求关闭；父进程死亡会关闭管道，子进程随之退出。仅凭监听地址、协议版本响应或数据目录锁，不会接管一个进程。第二个 App 进程必须激活已有实例，或者显示明确的“已运行”错误；不得对同一数据目录再启动一个托管 Server。崩溃后重新打开 App，须先确认旧进程已退出且数据目录锁已释放，再启动新子进程。<br>
   The App owns the child by retaining its OS process handle and private stdin/stdout pipes. Managed startup uses a dedicated Server flag, with a small framed control protocol on stdin and exactly one readiness response on stdout; diagnostics go to stderr. The parent sends shutdown over stdin, and parent death closes it so the child exits. The App does not adopt a listener merely because it occupies the expected address, reports a protocol version, or holds the data-directory lock. A second App process must activate the first instance or show a clear already-running error; it must not start a second managed Server against the same data directory. After a crash, a new App instance starts a new child only after the old process has exited and the data-directory lock is free.

<a id="endpoint-and-authentication"></a>
<a id="端点与鉴权"></a>

### 端点与鉴权 / Endpoint and authentication

4. 托管子进程只在 `127.0.0.1` 上绑定由操作系统分配的端口（`port 0`）；手动启动的外部 Server 仍可使用固定的 `39100` 默认端口。子进程通过私有 stdout 管道报告实际绑定地址、进程／构建身份和协议版本，不回传密钥。后端只接受自己启动的进程通过该管道发出的就绪消息，校验版本并执行带鉴权的 `server.version` 探测；超出有限启动时间时显示错误。这样端口冲突不能将 App 引向无关 Server；不得盲目回退 `39100` 或自行扫描空闲端口。<br>
   The managed child binds only `127.0.0.1` on an OS-assigned port (`port 0`); the fixed `39100` default remains for manually started external Servers. The child reports its actual bound address, process/build identity, and protocol version through its private stdout pipe, without the secret. The backend accepts readiness only from the process handle it spawned, verifies the version and an authenticated `server.version` probe, and gives up with a visible error after a bounded startup timeout. A port collision therefore cannot redirect the App to an unrelated Server; no blind fallback to `39100` or to a later free-port scan is allowed.
5. 后端用密码学安全随机数生成每次启动专用的 Bearer 密钥，在子进程监听前通过私有 stdin 管道传入，不放入命令行参数、环境变量、设置文件、日志或 WebView 事件。子进程对每个 JSON-RPC 请求、`events.subscribe` 和 HTTP `/jsonrpc` 请求强制校验该密钥。托管启动路径增加从管道接收密钥的能力后，可复用现有 Server 鉴权。后端只在内存中保存密钥，并在确认对端是回环地址后附加到托管 RPC 与事件连接。Server 仍只绑定回环地址；CORS 不是鉴权边界。浏览器配对、远程连接，以及向 CLI 或扩展共享该密钥，均不属于 M1。<br>
   The backend generates a cryptographically random, per-launch Bearer secret and passes it over the child's private stdin pipe before the child binds, never as a command-line argument, environment variable, settings file, log, or WebView event. The child requires it for every JSON-RPC call, `events.subscribe`, and HTTP `/jsonrpc` request. Existing Server authentication can enforce this once the managed startup path accepts a pipe-delivered secret. The backend retains the secret in memory only and attaches it to managed RPC and event connections after confirming the loopback peer. The Server still binds to loopback; CORS is not an authentication boundary. Browser pairing, remote access, and sharing this secret with CLI or extensions are outside M1.
6. 托管模式的后端命令使用进程管理者持有的端点与凭据，不接受前端传入的 `server` 字符串；现有 RPC、事件和凭据命令必须移除或由后端按模式限制，使托管界面无法指定任意 Server。只开放具体任务操作，不提供任意 RPC 转发。托管模式的新增下载入口接受 HTTP(S) URL、可编辑的建议文件名，以及通过原生选择器改选的目录；后端／Server 校验名称、生成任务 ID，并在已有任务和部分文件路径之间分配不冲突的目标，分配时不创建最终文件。若使用响应元数据命名，必须先校验再分配目标。外部进程在分配后创建同名文件时，Engine 的不覆盖提交仍会拒绝它。外部模式继续使用其显式地址及现有 RPC 契约。<br>
   Managed backend commands use the supervisor's endpoint and credential rather than accepting a frontend-supplied `server` string. Existing RPC, event, and credential commands must be removed or gated by backend mode so the managed UI cannot address an arbitrary Server. They expose specific task actions instead of arbitrary RPC forwarding. The managed add-download path accepts an HTTP(S) URL, an editable suggested filename, and an optional directory selected through a native chooser; the backend/server validate the name, generate the task ID, and allocate a unique destination against existing tasks and partial-file paths without creating the final file. Any response metadata used for naming must be validated before allocation. The no-clobber engine commit still rejects a file created externally after allocation. External mode continues to use its explicit address and existing RPC contract.

<a id="storage-and-lifecycle"></a>
<a id="存储与生命周期"></a>

### 存储与生命周期 / Storage and lifecycle

7. 托管数据库和锁文件放在稳定的 Tauri App 数据目录下的专用 `server/` 子目录，并依赖保持不变的 App 标识符 `com.nexum.desktop`；绝不放在 App 包、当前工作目录、临时目录或下载目录。默认保存位置是通过 macOS 系统 API 解析的用户“下载”目录本身；App 不额外创建 `Nexum` 子目录。目录不存在或不可写时要求用户作出可操作的选择，不会静默退回 App 数据目录。用户自选的默认下载目录保存在 App 设置中，也可为单个任务另选目标；每个任务选定的目标以绝对路径持久化。同名文件冲突绝不静默覆盖已有文件。后端拒绝路径穿越和不安全名称；Server 继续拒绝数据目录内目标及符号链接目标。<br>
   The managed database and lock live under the stable Tauri application data directory, in a dedicated `server/` subdirectory keyed to the unchanged App identifier `com.nexum.desktop`. They never live in the App bundle, current working directory, temporary directory, or download directory. The default save location is the user's macOS Downloads directory itself, resolved with the platform API; the App does not create a `Nexum` subdirectory. A missing or unwritable Downloads directory requires an actionable choice and does not silently fall back to application data. A user-selected default download directory is saved in App settings, and the user may select a different destination for an individual task; each task's destination is persisted as an absolute path. Name collisions never silently overwrite an existing file. The backend rejects path traversal and unsafe names; the Server continues to reject destinations in its data directory and symlink destinations.
8. 新托管数据目录与手动启动的 CLI／开发 Server 所用的 `./data` 相互独立。App 不扫描、导入、连接或删除那些独立数据。托管模式创建任务后，重开 App 与升级 App 都继续使用同一个托管目录。未来如需迁移托管数据库 schema，必须在事务中完成并保留可恢复备份；不支持的降级须明确报错，不得改写数据。App 更新必须保持标识符和数据路径不变。任务目标路径保持绝对路径，目标旁的临时文件和 sidecar 留在原处，供校验后续传。<br>
   The new managed data directory starts independently of `./data` used by manually launched CLI/development Servers. The App neither scans, imports, attaches to, nor deletes that separate data. Once managed mode creates tasks, relaunches and App upgrades keep using the same managed directory. Any future managed-database schema migration must be transactional and preserve a recoverable backup; an unsupported downgrade must fail visibly rather than rewrite data. App updates must preserve the identifier and data path. Task destinations remain absolute paths, and destination-side partial files and sidecars remain in place for validated resume.
9. M1 中托管下载只在 App 进程运行时进行。关闭最后一个窗口仍让 App 进程和子进程运行，可通过常规 macOS Dock／菜单重新打开窗口；明确“退出”才停止两者。正常退出或切到外部模式时，进程管理者要求自己的子进程关闭，等待有限时间，必要时终止它；绝不只凭端口或 PID 文件杀进程。子进程意外退出时，采用有限次数的退避重启，之后显示可操作的错误和手动重试入口，不无限循环。恢复后的 Server 使用现有任务恢复及带校验的 HTTP 续传逻辑。每次子进程重启或事件流缺口后，App 都重新拉取完整任务快照。强制停止可能留下目标旁的临时文件和 sidecar，但不得把不完整目标标为完成，也不能破坏已有文件。`Range`／`If-Range` 校验与不覆盖已有目标的原子完成规则仍以 [ADR 0004](0004-cross-restart-http-resume.md) 为准。<br>
   Managed downloads run only while the App process is running in M1. Closing the last window keeps the App process and child running, with the usual macOS Dock/menu path to reopen the window; explicit Quit stops them. On normal quit or a switch to External mode, the supervisor asks its own child to shut down, waits a bounded interval, then terminates it if necessary; it never kills a process discovered by port or PID file alone. An unexpected child exit triggers bounded restart attempts with backoff, followed by an actionable error and manual retry rather than an endless crash loop. The recovered Server applies its existing task recovery and validator-checked HTTP resume. The App refreshes a full task snapshot after every child restart or event-stream gap. A forced stop may leave a destination-side partial and sidecar, but must not expose an incomplete destination as completed or damage a preexisting file. [ADR 0004](0004-cross-restart-http-resume.md) remains authoritative for `Range`/`If-Range` validation and atomic completion without replacing an existing destination.

<a id="consequences-and-implementation-order"></a>
<a id="影响与实施顺序"></a>

## 影响与实施顺序 / Consequences and implementation order

- 为 Server 增加托管启动模式及私有控制／就绪协议，为 Tauri 增加进程管理者；现有 CLI 参数和手动启动 Server 的行为保持兼容。<br>
  Add a managed startup mode and private control/readiness protocol to the Server, plus a Tauri supervisor. Keep the existing CLI flags and manually started Server behavior compatible.
- 增加明确的托管／外部设置字段。重构 Desktop 命令和事件订阅，在 Rust 后端解析当前端点。<br>
  Introduce an explicit Managed/External settings schema. Refactor Desktop commands and event subscriptions to resolve the active endpoint in Rust.
- 增加托管模式的默认目标和任务创建命令。完成发布版 App 的实际操作前，不能声称通过 M1 验收。<br>
  Add a managed default-destination and task-creation command. Do not claim the M1 acceptance gate until these are usable from the release App.
- `port 0` 要求报告实际绑定端口；当前 Server 日志打印的是配置端口，不能充当就绪协议。当前 `--auth-token` 会在进程参数中暴露密钥，托管子进程不得使用。<br>
  `port 0` requires reporting the actual bound port; the current Server log line prints the configured port and is insufficient. The current `--auth-token` flag exposes a secret in process arguments and must not be used for the managed child.

<a id="first-implementation-slice-managed-server-bootstrap"></a>
<a id="首个实现切片server-托管启动协议"></a>

### 首个实现切片：Server 托管启动协议 / First implementation slice: managed Server bootstrap

2026 年 10 月 11 日，Server 增加专用 `--managed` 入口，仅接受显式的绝对 `--data-dir` 和可选的正整数 `--max-connections`。托管入口拒绝外部配置、固定端口、TLS 及命令行鉴权参数；手动启动 Server 的原有路径仍保留。Desktop 管理者已在后续切片中接入此入口。

On October 11, 2026, the Server gained a dedicated `--managed` entry point that accepts only an explicit absolute `--data-dir` and an optional positive `--max-connections`. It rejects external configuration, fixed-port, TLS, and command-line authentication flags. The existing manually started Server path remains available. The following Desktop slice now calls this entry point.

- `nexum_protocol::managed` 定义私有管道帧：4 字节大端长度加 JSON，单帧上限 4096 字节。stdin 首帧为 `type=start`，携带控制版本 `1`、Server 包版本、RPC 版本和用 64 个十六进制字符表示的 32 字节 Bearer 密钥。Server 校验后才创建数据目录、打开数据库和监听；5 秒未收到完整启动帧即失败。Desktop 管理者现已在每次启动时随机生成密钥。<br>
  `nexum_protocol::managed` defines private pipe frames: a four-byte big-endian length followed by JSON, capped at 4096 bytes per frame. The first stdin frame has `type=start`, control version `1`, the Server package version, RPC version, and a 32-byte Bearer secret encoded as 64 hexadecimal characters. The Server validates it before creating storage, opening the database, or listening; an incomplete startup frame fails after five seconds. The Desktop supervisor now generates the random secret for each launch.
- stdout 只写一条就绪帧，包含控制／Server／RPC 版本、子进程 PID 和 `listener.local_addr()` 得到的实际回环端点。密钥不在其中；日志走 stderr，帧解析错误不回显输入。下述 Desktop 切片已校验包内二进制 SHA-256、就绪消息并执行带鉴权的探活。<br>
  stdout carries exactly one readiness frame with the control/Server/RPC versions, child PID, and actual loopback endpoint from `listener.local_addr()`. It contains no secret. Diagnostics use stderr, and frame-parser errors do not echo input. The following Desktop slice verifies the bundled binary SHA-256, readiness, and authenticated health probe.
- 启动后 `type=shutdown`（控制版本 `1`）、stdin EOF 或非法控制帧会结束子进程。监听循环每 25 ms 检查控制通道；不等待阻塞中的 RPC、事件或 HTTP Worker，也不调用删除／取消任务来清除部分文件。进程退出依赖现有事务持久化和跨重启恢复；这不是传输完成前的全量刷新保证。数据目录锁由共享 Server 状态持有，直到保有数据库的 Worker 随进程退出。<br>
  After startup, `type=shutdown` with control version `1`, stdin EOF, or an invalid control frame ends the child. The listener checks its control channel every 25 ms. It does not wait for blocked RPC, event, or HTTP workers and does not remove/cancel tasks to clear partial files. Exit relies on existing transactional persistence and cross-restart recovery; it does not guarantee flushing every in-flight byte before exit. Shared Server state retains the data-directory lock until workers holding the database exit with the process.
- 隔离进程测试覆盖随机端点、TCP/HTTP/事件鉴权、错误和超时启动、重复数据目录、就绪输出脱敏、关闭／EOF 后的锁释放及停顿传输的续传与目标保护。测试用独立临时目录与 HTTP fixture，不接管开发 Server 数据。<br>
  Isolated process tests cover ephemeral endpoints, TCP/HTTP/events authentication, invalid/timed-out startup, duplicate data-directory ownership, private readiness output, lock release after Shutdown/EOF, and stalled-transfer resume and destination protection. They use independent temporary directories and HTTP fixtures without adopting development Server data.

首个切片仅实现 Server 协议；下述 Desktop 切片已接入它。M1 仍需完成最终验收。

The first slice implements only Server bootstrap; the following Desktop slice now connects it. M1 still requires final acceptance.

<a id="second-implementation-slice-desktop-supervision"></a>
<a id="第二个实现切片desktop-托管接线"></a>

### 第二个实现切片：Desktop 托管接线 / Second implementation slice: Desktop supervision

2026 年 10 月 11 日，`managed_server.rs` 接入有限启动探活、拥有句柄的关闭、三次退避重启及手动重试。设置默认托管，外部模式显示地址与 Keychain。具体 RPC／事件命令从设置和管理者解析当前路由，不接受前端地址；事件重连重新解析端点，并要求完整快照。托管状态只包含状态、代次和诊断，不包含端点或令牌。

On October 11, 2026, `managed_server.rs` wires bounded readiness/probing, handle-owned shutdown, three crash restarts with backoff, and manual retry. Settings default to Managed; External exposes the address and Keychain controls. Specific RPC/event commands resolve the current backend route without accepting a frontend endpoint. Event reconnect resolves the fresh route and requests a full snapshot. Public status contains only state, generation, and diagnostics.

构建预先生成目标架构 Server，将本机临时签名后的文件作为资源放到 `Contents/Resources/nexum-server`，并把 SHA-256 嵌入 Desktop。release 构建缺少匹配资源即失败；运行时缺失、改变、不兼容或启动失败显示重建／重试提示，不回退到 PATH 或固定端口。托管数据目录使用稳定的 `app_data_dir()/server/`。关闭窗口隐藏 App，明确退出停止子进程；Dock 重新打开界面。

The build stages the target-architecture Server, applies its local ad-hoc signature before hashing, bundles it as `Contents/Resources/nexum-server`, and embeds SHA-256 in Desktop. A release build without matching resources fails; missing, changed, incompatible, or failed executables produce rebuild/retry guidance without PATH or fixed-port fallback. Managed storage uses stable `app_data_dir()/server/`. Closing the window hides the app, Quit stops its child, and Dock reopen restores the window.

新增下载在 Rust 中校验 HTTP(S) URL、安全名称和目录，生成随机 ID，绕开已有任务、完整文件、稳定部分文件及 sidecar 分配名称；不创建最终文件。默认使用系统 Downloads，可为单项或设置中的默认位置改选目录。此切片保留了既有 Server 恢复规则，后续 [ADR 0007](0007-http-completion-reconciliation.md) 切片增加完整文件／Completed 对账；全新用户、升级和实际旁白朗读验收仍待完成。实际构建与原生检查见[发布流程](../RELEASE.md)。

Rust validates the HTTP(S) URL, safe filename and folder, generates random IDs, and allocates around task destinations, complete files, stable partials, and sidecars without creating a final file. System Downloads is the default, with per-task and saved default-folder selection. This slice retained existing Server recovery. The subsequent [ADR 0007](0007-http-completion-reconciliation.md) slice adds complete-file/Completed reconciliation; final clean-user, upgrade, and actual VoiceOver speech checks remain open. See [Release Process](../RELEASE.md) for artifact and native evidence.

<a id="acceptance-and-verification-boundary"></a>
<a id="验收与验证边界"></a>

## 验收与验证边界 / Acceptance and verification boundary

- 在全新 macOS 用户环境中，实际构建的 `.app` 启动随包 Server，将粘贴的 HTTP(S) URL 下载到不会覆盖同名文件的默认目标，不需要终端、Server 地址、手动任务 ID 或手动文件路径。退出／重开和 App 升级后，任务状态与已完成文件仍在。<br>
  In a fresh macOS account, the actual built `.app` starts its bundled Server and downloads a pasted HTTP(S) URL to a collision-safe default destination, without a terminal, Server address, manual task ID, or manual file path. Task state and completed bytes survive quit/relaunch and an App upgrade.
- 测试覆盖随包 Server 可执行文件缺失或不兼容、启动超时、子进程退出及重启上限、重复启动 App、外部固定端口被占用，以及伪装成 `39100` Server 的进程；任何情形下 App 都不能连接或停止无关进程。托管 RPC、HTTP 桥接和事件流拒绝缺失或错误凭据。令牌不得出现在参数、环境变量、日志、设置或前端载荷中。<br>
  Tests cover a missing/incompatible bundled Server executable, startup timeout, child exit/restart budget, duplicate App startup, an occupied fixed external port, and a process impersonating `39100`; none causes the App to attach to or stop the unrelated process. Managed RPC, HTTP bridge, and events reject missing/wrong credentials. The token does not appear in args, environment, logs, settings, or frontend payloads.
- 显式的外部模式可使用配置的凭据连接单独启动的 Server。切换模式保留托管任务及外部地址，退出托管模式不影响外部 Server。已有 CLI／开发目录 `./data` 保持原样。<br>
  Explicit External mode can connect to a separately launched Server with its configured credential. Switching modes preserves managed tasks and the external address; quitting Managed mode leaves an external Server running. Existing CLI/development `./data` remains untouched.
- 测试验证 App 重开与升级后打开同一个托管数据库。崩溃／强制停止测试验证已有目标不会被破坏，部分数据经校验后续传或安全地从零开始。还须覆盖完整目标文件发布后、`Completed` 持久化前的窗口，将任务与完好的文件对账，而不是重新下载后报失败。<br>
  Tests verify that App relaunch and upgrade reopen the same managed database. Crash/forced-stop tests verify that a preexisting destination is preserved and validated partial data resumes or safely restarts. They must also cover the gap between publishing a complete destination and persisting `Completed`, reconciling the task with the intact file instead of reporting a failed re-download.
- 发布版 App 的原生验收包括全新启动、粘贴 URL 下载、退出／重开、子进程异常退出、升级、权限错误及可操作的错误展示。自动化进程／协议测试不能代替原生验收。<br>
  Native release-App checks cover clean launch, paste-to-download, quit/relaunch, abnormal child exit, upgrade, permissions failure, and actionable error presentation. Automated process/protocol tests do not substitute for this native check.

<a id="alternatives-considered"></a>
<a id="考虑过的替代方案"></a>

## 考虑过的替代方案 / Alternatives considered

- **端口 `127.0.0.1:39100` 被占用时直接连接：** 不能证明进程归属，也无法阻止本地进程伪装 Server。<br>
  **Connect to `127.0.0.1:39100` if occupied:** cannot establish ownership or prevent a local process from impersonating the Server.
- **使用常驻 launch agent：** 在首个可用 App 之前就引入独立安装、升级、安全管理，并改变退出与后台下载语义。<br>
  **Run a persistent launch agent:** changes quit/background semantics and requires a separate installation/update/security lifecycle before the first usable App.
- **自动复用 `./data`：** 路径取决于启动目录，可能选择无关或正在使用的开发数据。<br>
  **Reuse `./data` automatically:** its location depends on the launch directory and could select unrelated or active development data.
- **通过命令行传递密钥，或只依靠回环地址／CORS：** 前者暴露密钥，后者无法对本地 RPC 鉴权。<br>
  **Pass the token by command-line flag or rely on loopback/CORS:** exposes the token or leaves local RPC unauthenticated.
