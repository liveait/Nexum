# RFC 0004：FTP 凭据、传输与恢复 / FTP credentials, transfer and recovery

- 状态：提案；固定依赖与协议实现仍须分别验证。日期：2026-11-07；基线：`503a3e79d89b8937b1373fc668209ec9fca43bef`（PR #84）。
  Status: proposed; pinned dependencies and protocol implementation require separate verification. Date: 2026-11-07; baseline: `503a3e79d89b8937b1373fc668209ec9fca43bef` (PR #84).

## 目标与实际基线 / Goal and actual baseline

首发必须支持匿名及用户名／密码 FTP 单文件下载，并通过 CLI 和原生 Desktop 的创建、排队、进度、暂停／恢复、取消、重启恢复与完整性检查。F0 已由 Registry 工厂进入既有 HTTP session；`ResolveKind` 仍没有 FTP，Desktop 两层来源校验与 Server 传输派发仍限 HTTP(S)，但 Server 的旧 `task.create → Core::create_task` 已接受 Magnet／Local 并保存原始 source，没有对应传输引擎。`ResumableEngine` 没有可用实现。本 RFC 是后续实施契约，不是 FTP 已支持的声明。

The first release must support anonymous and username/password FTP file downloads through CLI and native Desktop creation, queueing, progress, pause/resume, cancellation, restart recovery and integrity checks. F0 routes existing HTTP sessions through Registry factories. `ResolveKind` still lacks FTP. Both Desktop validation layers and Server transfer dispatch remain HTTP(S)-only, but legacy Server `task.create → Core::create_task` already accepts Magnet/Local and stores raw source without a transfer engine. `ResumableEngine` has no usable implementation. This RFC specifies future implementation rather than declaring FTP support.

保留 [RFC 0003](0003-registry-transfer-sessions.md) 的同步 Adapter 兼容、锁外启动、启动失败收尾与取消不重试，以及 [ADR 0003](../decisions/0003-http-transfer-controls.md)、[0004](../decisions/0004-cross-restart-http-resume.md)、[0007](../decisions/0007-http-completion-reconciliation.md) 的槽位、文件保护、控制、完成凭证和原子最终进度／Completed。不得只把 FTP 字符串送入 HTTP helper。

Preserve RFC 0003's synchronous-adapter compatibility, startup outside the lock, failed-start cleanup and cancellation without retry, plus ADRs 0003/0004/0007 for slots, file protection, controls, receipts and atomic final progress/Completed. FTP strings must not simply enter HTTP helpers.

## 首发范围与依赖 / First-release scope and dependency

采用 TYPE I、单控制连接及最多一条被动数据连接。首发要求普通 FTP；显式 TLS 作为独立能力实施／验收，输入设置为 `plain` 或 `required_explicit_tls`，后者必须在认证前执行 AUTH TLS 并保护数据通道，失败不得降级。未实现的模式立即拒绝。`ftps://` 的隐式 TLS 不自动映射为显式 TLS。目录浏览、递归、上传、主动模式、SFTP 和 FTP 多连接不属于本 RFC。

Use TYPE I, one control connection and at most one passive data connection. Plain FTP is required for the first release. Implement and validate explicit TLS as a separate capability selected by `plain` or `required_explicit_tls`; the latter requires AUTH TLS before authentication and a protected data channel, without downgrade on failure. Reject unavailable modes immediately. Do not silently interpret implicit `ftps://` as explicit TLS. Directory browsing, recursion, uploads, active mode, SFTP and FTP multi-connection are outside this RFC.

显式 TLS 必须按原始来源主机名校验控制与数据两通道的证书链／主机名，不把被动连接的 peer IP 当作新的 TLS 身份。AUTH TLS 后成功完成 `PBSZ 0` 和 `PROT P` 再接收文件数据；拒绝 `PROT C`、`CCC`、跳过验证和任何明文降级。fixture 验证控制证书不受信／错主机名时不发送密码，数据 TLS 失败时不接收明文数据或发布文件；不能声称后者会撤回此前已向通过验证的控制端发送的认证。[RFC 4217](https://www.rfc-editor.org/rfc/rfc4217.html)

For explicit TLS, validate certificate chains/hostnames on both channels against the original source hostname, not the passive peer IP as a new TLS identity. After AUTH TLS, require successful `PBSZ 0` and `PROT P` before file data. Reject `PROT C`, `CCC`, verification bypass and downgrade. Fixtures verify no password is sent on control certificate/hostname failure and no plaintext data is accepted/published on data-TLS failure; the latter cannot retract authentication already sent to a verified control endpoint. [RFC 4217](https://www.rfc-editor.org/rfc/rfc4217.html)

候选固定为 `suppaftp = 12.2.1`（上游提交 `18e0a88b0047dcffbf778c59ed5900b7e19251db`），关闭默认特性，先验证 `tokio-rustls-ring`。上游 workspace 声明 Rust 1.88；项目当前说明为 1.85+，依赖实施包必须明确调整／验证最低工具链，不能用新 stable 编译证明 1.85 兼容。依赖须进入锁文件并检查传递许可证、平台构建和 TLS provider。libcurl 是备选，新增 C/TLS 打包成本应单独比较。[固定清单](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/Cargo.toml) · [特性](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/crates/suppaftp/Cargo.toml)

Pin candidate `suppaftp = 12.2.1` (upstream commit `18e0a88b0047dcffbf778c59ed5900b7e19251db`), disable defaults and first verify `tokio-rustls-ring`. Its workspace declares Rust 1.88, while this project documents 1.85+. The dependency slice must explicitly update/test the minimum toolchain; compilation on a newer stable does not prove 1.85 compatibility. Lock dependencies and check transitive licenses, platform builds and TLS providers. Compare libcurl separately if needed, including C/TLS packaging costs. [Pinned manifest](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/Cargo.toml)

该版本 Tokio 控制实现会 trace 完整命令；认证库的日志不得泄漏 USER／PASS。`no-log` 映射到全局 `log/max_level_off`，不能未经评估静默关闭全仓日志。依赖实施须选择可审查的脱敏修复或不可被运行配置重开的日志隔离，并在最高日志级别以哨兵密码检查 stdout、stderr、Debug 和错误。未通过前不接收产品密码。[控制源码](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/crates/suppaftp/src/async_ftp/tokio_ftp/control.rs)

The candidate's Tokio control implementation traces rendered commands. Library logging must not expose USER/PASS. `no-log` enables global `log/max_level_off`, so assess application-wide effects before choosing it. The dependency slice must adopt reviewed redaction or logging isolation that runtime configuration cannot reopen, with sentinel-password checks at maximum verbosity across stdout, stderr, Debug and errors. Product passwords remain unavailable until this passes. [Control source](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/crates/suppaftp/src/async_ftp/tokio_ftp/control.rs)

## 来源与凭据契约 / Source and credential contract

新增能力协商及版本化 `task.create_transfer`，与 BT 共用显式类型化入口；旧 `task.create`、TaskView 和 HTTP 调用保留。旧 Server 对新方法返回 method-not-found，客户端不得退回会忽略新参数的旧创建方法。FTP 输入包含 `source_url`、安全模式、`path_mode` 和匿名／凭据输入；URL userinfo 在客户端入口拆出，Server 再次校验，持久来源只保留无 userinfo 的规范 URL。仅接受有效 UTF-8 路径，按 URL 组件解码一次并拒绝 CR、LF、NUL 及编码的分隔符。剥去 URL 语法的首个 `/` 后，默认 `login_relative`，`absolute` 则加回根 `/`；二者分别把 `ftp://host/pub/a.bin` 解释为 `pub/a.bin` 和 `/pub/a.bin`。将 path_mode 绑定恢复记录，拒绝旧 `%2F` 绝对路径缩写并提示使用显式模式，不猜测编码或本地路径。

Add capability negotiation and versioned `task.create_transfer`, sharing an explicit typed entry with BT while retaining legacy `task.create`, TaskView and HTTP calls. Old Servers return method-not-found; clients must not fall back to an old method that ignores new arguments. FTP input includes `source_url`, security, `path_mode` and anonymous/credential input. Split userinfo at client ingestion and validate again on the Server; persist a canonical URL without userinfo. Require valid UTF-8, decode URL components once and reject CR/LF/NUL and encoded separators. Strip the URL's first syntax `/`: default `login_relative` maps `ftp://host/pub/a.bin` to `pub/a.bin`, while `absolute` prepends `/` to produce `/pub/a.bin`. Bind path_mode in recovery and reject legacy `%2F` absolute-path shorthand with guidance to use explicit mode, without guessing encodings or local paths.

来源密码属于拥有任务的 Server，与 Desktop 的 RPC 凭据服务完全隔离。macOS 首发使用 Server 可调用的 Keychain provider，服务名独立，account 绑定随机 Server 数据实例 ID、origin、随机 credential ID 和 revision。SQLite 只保存不含密码的引用；密码字段只写，不进入任务、事件、TaskView、恢复／完成凭证、日志或 Debug。CLI 使用隐藏输入或 stdin，不提供密码命令行参数。提交密码只允许验证了 Server 身份且加密的 TLS；托管模式通过拥有的启动管道绑定临时 TLS 证书 pin、端点及进程 generation，在同一 TLS 连接验证通过后才发送密码，不在旧明文入口传密码。

The task-owning Server owns origin passwords separately from Desktop RPC credentials. On macOS, use a separate Keychain service scoped by random Server-data-instance ID, origin, random credential ID and revision. SQLite stores secret-free references. Passwords are write-only and excluded from tasks, events, TaskView, recovery/completion records, logs and Debug. CLI uses hidden input or stdin rather than password arguments. Submit passwords only over encrypted TLS with verified Server identity. Managed mode binds an ephemeral TLS certificate pin, endpoint and process generation through the owned bootstrap pipe, verifies that TLS connection, then sends the password; never use the legacy plaintext entry for passwords.

仅固定回环端口／PID／generation 并发送 Bearer 只能认证客户端，不能排除子进程退出后的端口替代 listener。F1 须先定义兼容的托管 bootstrap／TLS pin 协商，再允许来源密码；旧 HTTP 客户端和旧托管协议可保留，但不声明新认证 FTP 能力。外部 Server 使用已验证证书链／主机名的 TLS；测试用替代 listener、证书换代／错误 pin、断连重连和模式切换证明密码未发往替代端。

Pinning a loopback port/PID/generation and sending a bearer authenticates only the client, without excluding a replacement listener after child exit. F1 must define compatible managed-bootstrap/TLS-pin negotiation before accepting origin passwords. Retain old HTTP clients/managed protocol without advertising new authenticated FTP capability. External Servers use TLS with validated chains/hostnames. Test replacement listeners, certificate rotation/wrong pins, reconnects and mode switches to prove passwords are not sent to substitutes.

任务引用 `credential_id + revision`；更新密码使旧恢复绑定失效并重新验证来源。缺少、锁定、拒绝访问或删除的凭据报告可操作错误，不匿名重试、不使用其他 Server 的密码。其他平台 provider 未交付前仍可运行现有 HTTP 功能，但不声明支持持久化的认证 FTP；测试的内存 provider 不能充当产品持久化实现。

Tasks bind `credential_id + revision`. Updating a secret invalidates the old recovery binding and requires source revalidation. Missing, locked, denied or removed credentials yield actionable errors without anonymous retries or credentials from another Server. Existing HTTP remains available on other platforms; persistent authenticated FTP requires a delivered provider. A test-only memory provider is not product persistence.

新 Server 保留旧 `task.create` 的 HTTP(S) 行为；在任何持久化／事件前拒绝该旧方法新增的非 HTTP 任务，并返回需使用类型化入口的错误，这是对原来不可传输占位创建的明确兼容限制。旧 Magnet／Local 记录保持不自动派发，只允许显式迁移／重新导入；旧记录的公开展示也须脱敏。新 FTP 在领取前必须具备合法 origin／安全模式绑定，新 BT 必须具备 metainfo／inspection 与文件选择绑定；不能仅因 Resolver 或 Registry 开始识别新协议就领取旧任务，不能从旧 source 自动导入密码或客户端文件路径。

The new Server retains legacy `task.create` behavior for HTTP(S). Before any persistence/events, reject new non-HTTP tasks through that legacy method with a typed-entry-required error: an explicit compatibility restriction on previously non-transferable placeholders. Existing Magnet/Local records remain inert until explicit migration/reimport, with redacted public display. Before claiming, FTP requires valid origin/security bindings and BT requires metainfo/inspection/selection bindings. New Resolver/Registry support must not auto-claim old records or import passwords/client paths from legacy source strings.

新增按任务关联的版本化 origin 记录，创建分三步：先持久化 pending 引用，再写入该引用对应的 Keychain 项，最后用一次数据库事务创建任务并激活关联。任务 ID 是幂等键；同 ID 不重复创建任务或凭据，输入不符返回冲突，不隐式覆盖密码。崩溃后 pending 项可恢复或按明确的孤儿规则清理，不能把其他任务项当作垃圾。最终数据库保存失败应补偿已创建的凭据，补偿失败保留 pending 证据。更新密码也先写新 revision 的 pending 项和 provider 项，再事务激活新关联并记录旧 revision 清理意图，不覆盖仍被旧尝试使用的项。

Add versioned task-associated origin records. Creation first persists a pending reference, then writes its Keychain item, then creates the task and activates the association in one database transaction. Task ID is the idempotency key: do not duplicate tasks/credentials; conflicting input fails without implicitly replacing passwords. Recover pending items after crashes or clean them under explicit orphan rules, never treating other task items as garbage. Compensate created credentials after final database failure and retain pending evidence if compensation fails. Updates similarly create pending/provider items for a new revision, then transactionally activate it and record cleanup intent for the old revision without overwriting an item still used by an old attempt.

删除任务先停止传输；数据库删除任务与保存 provider cleanup tombstone 必须是同一事务。tombstone 不随 task 外键级联删除，保留 Server 实例、origin、credential ID、revision 和操作 ID，清理特定 provider 项成功／已不存在后才删除。崩溃和 Keychain 清理失败在重启后继续，不能复活任务；旧 revision 也只有全部读者退出后按同一机制清理。测试覆盖每个跨 Keychain／SQLite 窗口、更新中重启、删除提交后尚未清理及重复补偿。

Stop transfers before removal, and commit task deletion plus a provider cleanup tombstone in one database transaction. The tombstone survives task foreign-key cascades and binds Server instance, origin, credential ID, revision and operation ID. Remove it only after that specific provider item is deleted/already absent. Resume cleanup after crashes/Keychain failure without resurrecting tasks; retire old revisions through the same mechanism only after readers exit. Test every Keychain/SQLite boundary, update/restart, deletion-before-cleanup and duplicate compensation.

## 会话、控制与错误 / Sessions, controls and errors

### 托管 TLS 身份切片 / Managed TLS identity slice

基线为 `4d59d02079f333d8fbda775cdc373842cb3bf1fc`。保留控制版本 1 的 `start`、五字段就绪帧及明文端点。新增显式 `start_tls` 和控制版本 2，携带原有版本／Bearer 字段及父进程每次启动随机生成的 32 字节十六进制 `generation`。Server 每次启动生成内存专用自签名证书与私钥，只通过拥有句柄的 stdout 管道返回版本、PID、端点、原样 generation 和有界 DER 证书。版本 2 的 shutdown 必须使用版本 2。新 Desktop 只请求版本 2，拒绝旧就绪帧、错误 generation 及无效证书。

The baseline is `4d59d02079f333d8fbda775cdc373842cb3bf1fc`. Retain control-version-1 `start`, its five-field readiness frame and plaintext endpoint. Add explicit `start_tls` with control version 2, the existing version/bearer fields and a fresh parent-generated 32-byte hexadecimal `generation`. Each Server launch generates an in-memory self-signed certificate/private key. Its owned stdout pipe returns versions, PID, endpoint, the unchanged generation and bounded DER certificate. Version-2 shutdown requires version 2. New Desktop requests only version 2 and rejects old readiness, mismatched generations and invalid certificates.

后端路由绑定该端点、代次与完整证书。每次 RPC／事件连接仅允许 TLS 1.3，在同一连接上完成完整证书固定、WebPKI 名称／有效期及握手签名校验后才加载并发送 Bearer。禁用 TLS 恢复和 early data，不信任系统根来扩展此托管身份，不回退明文。证书／私钥／端点不会进入设置或前端事件。外部模式沿用独立的系统证书验证和按地址凭据。

The backend route binds this endpoint, generation and complete certificate. Every RPC/event connection requires TLS 1.3 and completes exact-certificate pinning, WebPKI name/time validation and handshake-signature verification on that connection before loading/sending the bearer. Disable TLS resumption and early data. Managed identity uses its pinned certificate as the sole trust root and has no plaintext fallback. Certificates/private keys/endpoints stay outside settings and frontend events. External mode retains independent system-root verification and address-scoped credentials.

2026-11-12 的并行测试实证了 Darwin `pipe → CLOEXEC` 创建窗口中的跨子进程继承，测试 fixture 已协调自身所有进程创建。该修复的范围限测试。产品正常切换／退出会先发送显式 Shutdown；About 通过 `/usr/bin/open` 打开固定项目链接，可能与托管创建并发，目前未复现产品中的持久持有或泄漏。开放来源密码能力前，F1 还须封闭或验证这些原生 helper 并发创建时的私有 bootstrap FD 隔离及父进程异常退出后的 EOF 边界，不能从测试锁推导产品已具备原子管道保证。

Parallel tests on 2026-11-12 demonstrate cross-child inheritance in Darwin's `pipe → CLOEXEC` creation window. The fixture coordinates all its process creation, within test scope. Normal product mode changes/quit send explicit Shutdown first. About opens fixed project links through `/usr/bin/open`, which can overlap managed startup, with no observed product persistent retention or leakage. Before enabling origin-password capability, F1 must close or verify private bootstrap FD isolation across these concurrent native helpers and EOF after abnormal parent exit. The test lock does not establish product-wide atomic pipe creation.

#### 原生进程创建协调 / Native process launch coordination

本切片基线为 `853432e747b0152f0f67930d8577483355fc8718`。Desktop 库持有唯一的进程创建锁，当前托管 Server 和 macOS About 的三个固定项目 URL 均通过该库创建子进程。锁覆盖 stdio 创建、fork／spawn 和 exec 返回。托管启动在取得锁前后检查已有 enabled 取消标记及共享的 6 秒期限，等待超时或取消时直接结束启动。Start 帧、Ready、TLS 探活、子进程等待及重启退避在锁外执行。

This slice starts from `853432e747b0152f0f67930d8577483355fc8718`. The Desktop library owns one process-launch lock, shared by managed Server creation and the three fixed macOS About project URLs. The lock covers stdio creation, fork/spawn and exec completion. Managed startup checks its existing enabled cancellation flag and shared six-second deadline before and after acquiring the lock, ending startup on timeout/cancellation. Start frames, Ready, TLS probing, child waits and restart backoff run outside the lock.

macOS 的 About 入口使用与当前锁定 opener 相同的 `/usr/bin/open -- URL` 及 null stdio，创建后在锁外等待退出状态。其他平台沿用现有 opener 入口。测试辅助进程也通过同一创建函数，长时间的编译／helper 等待放在锁外。验证包含持锁时取消／超时且未创建子进程、长期 helper 不阻塞托管 Ready、helper 在 exec 后报告额外 FD 的类型、异常终止父进程后 Server 收到 EOF 退出，以及 helper 继续存活时同数据目录可重新启动。

The macOS About entry uses the locked opener's `/usr/bin/open -- URL` behavior and null stdio, waiting for exit status outside the lock. Other platforms retain their existing opener entry. Test helpers use the same spawn function, with compilation/helper waits outside the lock. Verification covers cancellation/deadline expiry under contention without creating a child, managed Ready while a helper remains alive, post-exec helper reports of extra FD types, Server EOF exit after abnormal parent termination, and same-data-directory restart while that helper stays alive.

验收包括旧版本 1 的原始帧兼容、新版本 RPC／HTTP／事件鉴权、错误证书及同端口替代 listener、重启换证／旧路由拒绝、事件重连和模式隔离，并从实际 `.app` 验证托管启动及 HTTP 下载。来源密码的输入、Keychain／SQLite 协调和认证 FTP 能力仍由后续 F1 切片交付。

Acceptance covers raw version-1 compatibility, new-version RPC/HTTP/event authentication, wrong certificates and replacement listeners on the same port, certificate rotation/stale-route rejection, event reconnects and mode isolation, plus managed startup and HTTP download through the actual `.app`. Subsequent F1 slices deliver origin-password input, Keychain/SQLite coordination and authenticated FTP capability.

新 FTP 工厂经 Registry 显式选择并在锁外返回受控 session。暂不改变旧 `TransferSessionRequest` 的公共 struct literal；新增类型化请求／扩展工厂方法时保留默认不支持实现，HTTP 仍使用旧入口。来源、认证、连接、读取和最终回复不持有 Core 锁。工厂早期失败也须终结控制；任何已创建 worker 必须在返回错误前停止并 join。

Select the new FTP factory explicitly through Registry and return a controlled session outside the Core lock. Preserve existing public `TransferSessionRequest` struct literals: typed requests/extended factory methods need default unsupported implementations, while HTTP retains its entry. Resolve/authenticate/connect/read/finalize outside the Core lock. Early factory failure finishes controls, stopping and joining any created workers before returning an error.

EPSV 优先；仅在明确不支持时回退一次 PASV。数据连接固定到控制连接实际 peer IP，忽略 PASV 提供的主机地址，不连接第三方地址。初始连接／欢迎／单命令／最终回复期限各 5 秒；数据读取每 1 秒检查控制，连续 8 次无数据判为停滞，所有等待可取消。沿用删除的 10 秒 worker 等待预算，超时保留占用并报告，不能假定阻塞文件 I/O 已退出。暂停等待 worker 停写并取得同一边界的进度快照，持久化后才返回。取消唤醒暂停和限速等待，join 后才释放目标与槽位；不能以 sleep 证明退出。

Prefer EPSV, falling back once to PASV only when explicitly unsupported. Pin data connections to the actual control peer IP, ignoring the PASV host and preventing third-party connections. Initially bound connection/banner/individual commands/final replies to five seconds each. Data reads check controls every second and fail after eight consecutive empty windows; all waits are cancellable. Retain the ten-second removal wait, keeping reservations and reporting timeout rather than assuming blocked file I/O exited. Pause waits for quiescence, snapshots progress at that boundary and persists it before replying. Cancellation wakes pause/throttling waits and joins before releasing destinations/slots; sleep is not proof of exit.

重试由 Scheduler 统一计数；FTP 库不另开无限重试。临时网络／适用 4xx 可在预算内重试；认证／权限、取消、本地安全、凭证和最终保存错误不自动重试。用结构化内部错误分类，再生成脱敏可读消息，不按错误字符串猜测类别。暂停进度的继承 P2 须在新协议控制消费者启用前修复并回归。

The Scheduler owns retry accounting, without an independent unbounded library loop. Transient network/applicable 4xx errors may retry within budget. Authentication/permission, cancellation, local safety, receipt and final-persistence errors do not. Classify internal errors structurally before rendering redacted messages, rather than parsing strings. Fix and regress the inherited pause-progress P2 before enabling new protocol control consumers.

## 版本化恢复 / Versioned recovery

FTP 的 SIZE、MDTM／MLST unique 和 REST 不等于 HTTP 强 validator／If-Range；同长度、时间戳或对象 ID 不能证明内容未变。首版固定策略 `verify_prefix_same_retr`：先验证持有的本地文件身份和持久前缀摘要，再从零发一次 RETR，逐块比较已有前缀，匹配后在同一响应继续追加。REST 快速追加本轮不启用；恢复会重传前缀，界面说明这一流量成本。[RFC 3659](https://www.rfc-editor.org/rfc/rfc3659.html)

FTP SIZE, MDTM/MLST unique and REST are not HTTP strong validators/If-Range. Matching length, timestamps or object IDs do not prove immutable content. Initial strategy `verify_prefix_same_retr` verifies held local identity and a durable prefix digest, starts one RETR from zero, compares the saved prefix chunk by chunk, then appends from that same response. Fast REST append stays disabled. Recovery retransfers the prefix; explain this traffic cost. [RFC 3659](https://www.rfc-editor.org/rfc/rfc3659.html)

独立 `ftp-resume/v1` 最少绑定以下字段，不复用 `HttpResumeValidator` 或重解释旧 `ResumeMetadata`：

An independent `ftp-resume/v1` binds at least these fields, without reusing `HttpResumeValidator` or reinterpreting legacy `ResumeMetadata`:

| 字段 / Field | 约束 / Constraint |
| --- | --- |
| `task_id`, `attempt_generation` | 拒绝旧尝试写回；重启后用单独 generation 领取。 / Fence stale attempts with a new generation on recovery. |
| `source`, `security`, `credential_ref` | 无 userinfo 的规范来源、端点、路径、模式、凭据 revision。 / Canonical secret-free endpoint/path, mode and credential revision. |
| `destination`, `parent_identity`, `partial_identity` | 绑定目标与持有对象，拒绝不同身份的替换文件。 / Bind destination/held objects and reject replacement identities. |
| `trusted_prefix_bytes`, `prefix_sha256` | 只信已 sync 并原子提交的前缀，不能用预分配长度代替。 / Trust only synced, atomically checkpointed bytes, never preallocation length. |
| `remote_facts`, `binary_mode`, `strategy` | 可用 SIZE／modify／unique 仅为一致性辅助；固定 TYPE I 和策略。 / Available facts are consistency hints; bind TYPE I and the strategy. |

检查点顺序为数据 sync → 原子持久化前缀证明；恢复验证在 Ready 后的 worker 内执行，不阻塞六秒就绪预算。实际长度小于可信前缀或摘要不符即保留文件并报错。实际长度大于检查点时，只有在持有身份及安全修改机制证明后才能丢弃未提交尾部。未知版本保留证据并拒绝恢复。进度只计已经验证的前缀与新增字节，不重复累计网络重读。

Checkpoint ordering is data sync followed by atomic prefix-proof persistence. Validate in post-Ready workers without consuming the six-second readiness budget. A short file or digest mismatch fails with files retained. Trim an uncommitted tail only with held identity and a proven safe modification mechanism. Unknown versions fail with evidence retained. Progress counts verified prefix plus new bytes, without double-counting retransferred data.

前缀不匹配时关闭整个连接，在同一次尝试内最多重启一次零偏移下载。放弃旧 partial 必须证明拥有的对象／名称绑定；不能证明则保留并报告安全冲突，不以回退绕过。每次 RETR 都需数据 EOF、显式 `TransferStream.finish()` 正向终结回复、已知长度与最终实际长度一致，以及可用的传输前后元数据一致。EOF、REST setter 成功或 stream Drop 都不能证明完成。[传输流源码](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/crates/suppaftp/src/async_ftp/tokio_ftp/transfer_stream.rs)

On prefix mismatch, close the whole connection and permit at most one zero-offset restart in that attempt. Retiring the old partial requires proof of its object/name ownership; otherwise retain it and report a safety conflict. Every RETR requires data EOF, positive explicit `TransferStream.finish()` completion, matching known/actual length and available before/after metadata. EOF, a successful REST setter or stream Drop is insufficient. [Transfer stream source](https://github.com/veeso/suppaftp/blob/18e0a88b0047dcffbf778c59ed5900b7e19251db/crates/suppaftp/src/async_ftp/tokio_ftp/transfer_stream.rs)

普通 FTP 不能证明恶意／持续变动来源提供一致快照；本策略阻止盲目拼接未经验证的旧前缀，不声明远端不可变性。有外部预期摘要时完成必须校验，否则 SHA256 仅用于本地完成／恢复证明。

Plain FTP cannot prove a consistent snapshot from a malicious or continuously changing origin. This strategy prevents blind reuse of an unchecked prefix without asserting remote immutability. Verify an external expected digest when provided; otherwise SHA256 establishes local completion/recovery proof only.

## 完成、文件安全与兼容 / Completion, file safety and compatibility

停写、长度验证和 sync 后先保存完成证明，再在同一取消决定锁内安全地不覆盖发布，最后原子保存最终进度和 Completed。保存失败保留证明，重启优先离线对账，不发额外 RETR。沿用旧 HTTP v1 路径／读取，FTP 新格式与目录独立版本化，不能静默修改 `http-completions` 含义。

After quiescence, length validation and sync, persist completion proof before safe no-clobber publication under the cancellation decision lock, then atomically persist final progress and Completed. Retain proof on failure and reconcile offline before another RETR. Keep legacy HTTP v1 paths/readers; separately version FTP formats/directories without silently changing `http-completions` semantics.

[RFC 0002](0002-segment-file-ownership.md) 的同 UID 目录项替换威胁模型同样约束新路径。复用 `check → rename/link/unlink`、随机目录、0700 或多做 stat 都不能证明安全发布／清理。当前 macOS 机制尚未通过；先完成持有对象的写入与控制试验，最终 FTP 产品派发等待平台发布／恢复／条件清理证明，或经明确接受的范围修订。无法证明所有权时保留对象并返回具体错误。此 RFC 不修改威胁模型或把既有 HTTP 局限标成已修复。

RFC 0002's same-UID directory-entry replacement model applies to the new path. Reusing `check → rename/link/unlink`, random directories, 0700 or extra stat calls does not prove safe publication/cleanup. The macOS mechanism remains unverified. Begin with held-object I/O/control probes; final FTP product dispatch waits for proven platform publication/recovery/conditional cleanup, or an explicitly accepted scope revision. Preserve objects and report specific errors when ownership cannot be proven. This RFC neither changes the threat model nor marks existing HTTP limitations fixed.

<a id="implementation-and-acceptance"></a>

## 实施与验收 / Implementation and acceptance

2026-11-07 的隔离编译试验通过：在 macOS arm64、Rust 1.98.1 上对上述固定 FTP／BT 特性组合生成独立锁文件，再执行 `cargo check --locked` 成功；七个关键发布包源码与固定上游源码逐字节一致。产品 Cargo 文件未改变，未连接 FTP 或 peer；这不验证 MSRV、传递许可证、密码脱敏、运行行为或原生打包。

The 2026-11-07 isolated compile probe passes on macOS arm64/Rust 1.98.1: generate a separate lockfile for the pinned FTP/BT feature combination, then run `cargo check --locked`. Seven critical published source files exactly match pinned upstream sources. Product Cargo files are unchanged and no FTP/peer connection occurred. MSRV, transitive licenses, redaction, runtime behavior and native packaging remain unverified.

| 切片 / Slice | 交付与证据 / Deliverable and evidence |
| --- | --- |
| F1 依赖与凭据 / Dependency and credentials | 固定构建、MSRV／许可证、密码日志哨兵、provider 故障与 pending 创建／重启／删除补偿；HTTP 旧调用兼容。 / Pinned builds, MSRV/licenses, secret sentinels, provider failures and pending creation/restart/removal compensation, retaining legacy HTTP. |
| F2 来源与单流 / Sources and one stream | 严格解析、匿名／认证 fixture、EPSV／PASV peer 绑定、拒绝终结回复、期限与取消；直接调用及真实 Server／CLI。 / Strict parsing, auth fixtures, passive peer binding, negative completion, deadlines and cancellation through direct and real consumers. |
| F3 恢复与完成 / Recovery and completion | 同长度来源替换、前缀重验、最多一次重启、完整进度、SIGKILL、SQLite 失败、离线对账，以及最后检查后的文件替换保护。 / Same-length changes, prefix recheck, one bounded restart, full progress, crashes, database failure, offline reconciliation and precise replacement races. |
| F4 Desktop 与首发 / Desktop and first release | 同一候选匿名／认证来源、密码不回显、两种 Server 模式、切换隔离、暂停取消／槽位释放、退出重开与最终摘要。 / One candidate covers auth, write-only secrets, both Server modes, switching, controls/slots, relaunch and hashes. |

每个实现包固定 SHA 独立审查、运行完整 Rust 门槛及最新 base/head、实际 main push CI；Desktop 另执行前端与原生验收。FTP 启用前须关闭相关文件安全及控制阻断项。方案、实现和验收分别记载，M5 不因 RFC 或编译试验完成而关闭。

Each implementation slice needs fixed-SHA independent review, full Rust gates and CI for latest base/head and the actual merged main. Desktop adds frontend/native checks. Resolve file-safety/control blockers before FTP enablement. Record design, implementation and acceptance separately; an RFC or compile probe does not complete M5.
