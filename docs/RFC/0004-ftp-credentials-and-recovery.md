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

### 旧来源入口保护切片 / Legacy source boundary slice

本切片基线为 `c1bd721d828349e31b395056085ed41656db69d9`。旧 `task.create` 共用参数预检，检查 id、source、destination 的字符串参数，并按 Resolver 的两端空白与 ASCII 大小写规则只接受 HTTP／HTTPS scheme。Server 在清理完成凭证之前预检，通用 dispatcher 使用同一校验。非 HTTP 请求在任务持久化及事件产生前返回固定错误，非 HTTP 通知在相同边界被拒绝并保持无响应。

This slice starts from `c1bd721d828349e31b395056085ed41656db69d9`. Legacy `task.create` shares parameter validation for string id/source/destination values and permits HTTP/HTTPS schemes using the Resolver's trim and ASCII-case rules. The Server validates before completion-receipt cleanup, and the generic dispatcher uses the same check. Non-HTTP requests return fixed errors before task persistence or event creation. Non-HTTP notifications are rejected at the same boundary and produce no response.

历史非 HTTP 任务的 get／list 通过 TaskView 输出固定来源占位文字，已有错误转换为显式重新导入提示。SQLite 保存原记录，公开状态、进度和目标字段继续提供实际任务信息。真实 Server 回归覆盖拒绝时保留完成 sidecar、通知、TLS RPC／HTTPS、历史记录重启及并行 HTTP 下载。此包先落实凭据接入所需的旧来源边界，后续 F1 接入版本化类型入口及 Keychain／SQLite pending 协调。

TaskView get/list responses use a fixed source placeholder for historical non-HTTP tasks and convert existing errors into explicit reimport guidance. SQLite retains the original records, while public state/progress/destination fields describe the actual tasks. Real Server regressions cover preserved completion sidecars on rejection, notifications, TLS RPC/HTTPS, historical-record restarts and concurrent HTTP downloads. This package establishes the legacy-source boundary for credential ingestion. Subsequent F1 work adds the versioned typed entry and Keychain/SQLite pending coordination.

完整门槛中两项既有 managed 用例通过 Magnet 占位任务验证鉴权／事件和 EOF 保存。按新的入口约定，改用未排队的 HTTP 任务，保留全部鉴权、事件、退出预算、锁释放及重开比对，并增加来源和 Created 状态断言。新增回归单独覆盖旧非 HTTP 请求的拒绝与历史记录保留。

Two existing managed tests used Magnet placeholders to verify authentication/events and EOF persistence. Under the new ingestion contract they use unqueued HTTP tasks, retaining authentication, event, exit-budget, lock-release and relaunch comparisons while adding source/Created-state assertions. New regressions separately cover rejection of legacy non-HTTP requests and retention of historical records.

Persistence 用例按同一约定迁移。历史状态恢复和不支持来源保持排队的用例从私有 SQLite 预置旧 Magnet 记录，经过真实 Server 启动与恢复。鉴权和清理用例使用有效 HTTP 来源，HTTP 桥接的排队检查改为受控响应的 Downloading → Completed 并核对文件字节，CORS 与认证断言继续执行。完整 72 项 persistence 测试通过，原先 6 项旧占位输入失败的日志保留。

Persistence fixtures follow the same contract. Historical-state recovery and inert unsupported-source tests seed legacy Magnet records in private SQLite and exercise real Server startup/recovery. Authentication/cleanup fixtures use valid HTTP sources. The HTTP bridge queue check follows a controlled response through Downloading → Completed and verifies file bytes, retaining CORS/authentication assertions. All 72 persistence tests pass, with logs retained for the six former placeholder-input failures.

新增能力协商及版本化 `task.create_transfer`，与 BT 共用显式类型化入口；旧 `task.create`、TaskView 和 HTTP 调用保留。旧 Server 对新方法返回 method-not-found，客户端不得退回会忽略新参数的旧创建方法。FTP 输入包含 `source_url`、安全模式、`path_mode` 和匿名／凭据输入；URL userinfo 在客户端入口拆出，Server 再次校验，持久来源只保留无 userinfo 的规范 URL。仅接受有效 UTF-8 路径，按 URL 组件解码一次并拒绝 CR、LF、NUL 及编码的分隔符。剥去 URL 语法的首个 `/` 后，默认 `login_relative`，`absolute` 则加回根 `/`；二者分别把 `ftp://host/pub/a.bin` 解释为 `pub/a.bin` 和 `/pub/a.bin`。将 path_mode 绑定恢复记录，拒绝旧 `%2F` 绝对路径缩写并提示使用显式模式，不猜测编码或本地路径。

Add capability negotiation and versioned `task.create_transfer`, sharing an explicit typed entry with BT while retaining legacy `task.create`, TaskView and HTTP calls. Old Servers return method-not-found; clients must not fall back to an old method that ignores new arguments. FTP input includes `source_url`, security, `path_mode` and anonymous/credential input. Split userinfo at client ingestion and validate again on the Server; persist a canonical URL without userinfo. Require valid UTF-8, decode URL components once and reject CR/LF/NUL and encoded separators. Strip the URL's first syntax `/`: default `login_relative` maps `ftp://host/pub/a.bin` to `pub/a.bin`, while `absolute` prepends `/` to produce `/pub/a.bin`. Bind path_mode in recovery and reject legacy `%2F` absolute-path shorthand with guidance to use explicit mode, without guessing encodings or local paths.

来源密码属于拥有任务的 Server，与 Desktop 的 RPC 凭据服务完全隔离。macOS 首发使用 Server 可调用的 Keychain provider，服务名独立，account 绑定随机 Server 数据实例 ID、origin、随机 credential ID 和 revision。SQLite 只保存不含密码的引用；密码字段只写，不进入任务、事件、TaskView、恢复／完成凭证、日志或 Debug。CLI 使用隐藏输入或 stdin，不提供密码命令行参数。提交密码只允许验证了 Server 身份且加密的 TLS；托管模式通过拥有的启动管道绑定临时 TLS 证书 pin、端点及进程 generation，在同一 TLS 连接验证通过后才发送密码，不在旧明文入口传密码。

The task-owning Server owns origin passwords separately from Desktop RPC credentials. On macOS, use a separate Keychain service scoped by random Server-data-instance ID, origin, random credential ID and revision. SQLite stores secret-free references. Passwords are write-only and excluded from tasks, events, TaskView, recovery/completion records, logs and Debug. CLI uses hidden input or stdin rather than password arguments. Submit passwords only over encrypted TLS with verified Server identity. Managed mode binds an ephemeral TLS certificate pin, endpoint and process generation through the owned bootstrap pipe, verifies that TLS connection, then sends the password; never use the legacy plaintext entry for passwords.

仅固定回环端口／PID／generation 并发送 Bearer 只能认证客户端，不能排除子进程退出后的端口替代 listener。F1 须先定义兼容的托管 bootstrap／TLS pin 协商，再允许来源密码；旧 HTTP 客户端和旧托管协议可保留，但不声明新认证 FTP 能力。外部 Server 使用已验证证书链／主机名的 TLS；测试用替代 listener、证书换代／错误 pin、断连重连和模式切换证明密码未发往替代端。

Pinning a loopback port/PID/generation and sending a bearer authenticates only the client, without excluding a replacement listener after child exit. F1 must define compatible managed-bootstrap/TLS-pin negotiation before accepting origin passwords. Retain old HTTP clients/managed protocol without advertising new authenticated FTP capability. External Servers use TLS with validated chains/hostnames. Test replacement listeners, certificate rotation/wrong pins, reconnects and mode switches to prove passwords are not sent to substitutes.

`server.capabilities.ftp.durable_credentials` 仅在当前 RPC 连接已完成 TLS 握手且 Server 提供 macOS Keychain 持久化时返回 true。旧托管 v1 及独立 Server 的明文 TCP／HTTP 连接仍声明匿名 FTP 元数据创建和种子检查能力。

`server.capabilities.ftp.durable_credentials` is true only after the current RPC connection completes a TLS handshake and the Server provides macOS Keychain persistence. Legacy managed v1 and standalone plaintext TCP/HTTP still advertise anonymous FTP metadata creation and torrent inspection.

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

2026-11-12 的并行测试实证了 Darwin `pipe → CLOEXEC` 创建窗口中的跨子进程继承。基线 `853432e7` 的测试 fixture 已协调自身所有进程创建，产品 About 与托管创建仍可能重叠。以下切片将协调扩展至当前产品入口，并验证私有 bootstrap FD 隔离及父进程异常退出后的 EOF 边界。

Parallel tests on 2026-11-12 demonstrate cross-child inheritance in Darwin's `pipe → CLOEXEC` creation window. At baseline `853432e7`, the fixture coordinates its process creation, while production About can overlap managed startup. The following slice extends coordination to the current product entry points and verifies private bootstrap FD isolation and EOF after abnormal parent exit.

#### 原生进程创建协调 / Native process launch coordination

本切片基线为 `853432e747b0152f0f67930d8577483355fc8718`。Desktop 库持有唯一的进程创建锁，当前托管 Server 和 macOS About 的三个固定项目 URL 均通过该库创建子进程。锁覆盖 stdio 创建、fork／spawn 和 exec 返回。托管启动在取得锁前后检查已有 enabled 取消标记及共享的 6 秒期限，等待超时或取消时直接结束启动。Start 帧、Ready、TLS 探活、子进程等待及重启退避在锁外执行。

This slice starts from `853432e747b0152f0f67930d8577483355fc8718`. The Desktop library owns one process-launch lock, shared by managed Server creation and the three fixed macOS About project URLs. The lock covers stdio creation, fork/spawn and exec completion. Managed startup checks its existing enabled cancellation flag and shared six-second deadline before and after acquiring the lock, ending startup on timeout/cancellation. Start frames, Ready, TLS probing, child waits and restart backoff run outside the lock.

macOS 的 About 入口使用与当前锁定 opener 相同的 `/usr/bin/open -- URL` 及 null stdio，创建后在锁外等待退出状态。其他平台沿用现有 opener 入口。测试辅助进程也通过同一创建函数，长时间的编译／helper 等待放在锁外。验证包含持锁时取消／超时且未创建子进程、长期 helper 不阻塞托管 Ready、helper 在 exec 后报告额外 FD 的类型、异常终止父进程后 Server 收到 EOF 退出，以及 helper 继续存活时同数据目录可重新启动。

The macOS About entry uses the locked opener's `/usr/bin/open -- URL` behavior and null stdio, waiting for exit status outside the lock. Other platforms retain their existing opener entry. Test helpers use the same spawn function, with compilation/helper waits outside the lock. Verification covers cancellation/deadline expiry under contention without creating a child, managed Ready while a helper remains alive, post-exec helper reports of extra FD types, Server EOF exit after abnormal parent termination, and same-data-directory restart while that helper stays alive.

受控 helper 枚举 `/dev/fd` 全部条目并拒绝任何额外 pipe，覆盖父进程已关闭的子端读句柄。真实随包 Server 已验证 helper 持续存活时的 Ready、父进程 SIGKILL 后 EOF 退出及同数据目录重启。原生 `.app` 验证三个固定链接、HTTP 完整文件、App 异常退出及正常退出／重开，见[发布记录](../RELEASE.md#native-process-isolation-native-qa)。

The controlled helper enumerates every `/dev/fd` entry and rejects any extra pipe, covering child read ends already closed by the parent. The real bundled Server verifies Ready, EOF exit after parent SIGKILL and same-directory restart while the helper remains alive. The native `.app` verifies all three fixed links, complete HTTP bytes, abnormal App exit and normal quit/relaunch. See the [release record](../RELEASE.md#native-process-isolation-native-qa).

测试宿主可以向测试程序继承已有 FD。隔离测试父进程先枚举已有非 stdio 句柄，记录类型与原始标志，并为这些句柄设置 close-on-exec，然后才创建 bootstrap 管道、helper 和并发屏障。helper 继续对全部额外 pipe 执行严格断言。句柄枚举期间完成标志操作，读取或设置失败会使 fixture 失败。该边界通过受控注入一条已有管道的两个 FD 验证。

The test host can pass existing FDs into the test executable. Before creating bootstrap pipes, helpers or the concurrency barrier, the isolated test parent enumerates its existing non-stdio handles, records types/original flags and marks them close-on-exec. The helper retains its strict assertion against every extra pipe. Flag operations occur while enumeration remains open, and inspection/update failures fail the fixture. Controlled injection of a preexisting pipe's two descriptors verifies this boundary.

验收包括旧版本 1 的原始帧兼容、新版本 RPC／HTTP／事件鉴权、错误证书及同端口替代 listener、重启换证／旧路由拒绝、事件重连和模式隔离，并从实际 `.app` 验证托管启动及 HTTP 下载。来源密码的输入、Keychain／SQLite 协调和认证 FTP 能力仍由后续 F1 切片交付。

Acceptance covers raw version-1 compatibility, new-version RPC/HTTP/event authentication, wrong certificates and replacement listeners on the same port, certificate rotation/stale-route rejection, event reconnects and mode isolation, plus managed startup and HTTP download through the actual `.app`. Subsequent F1 slices deliver origin-password input, Keychain/SQLite coordination and authenticated FTP capability.

新 FTP 工厂经 Registry 显式选择并在锁外返回受控 session。暂不改变旧 `TransferSessionRequest` 的公共 struct literal；新增类型化请求／扩展工厂方法时保留默认不支持实现，HTTP 仍使用旧入口。来源、认证、连接、读取和最终回复不持有 Core 锁。工厂早期失败也须终结控制；任何已创建 worker 必须在返回错误前停止并 join。

Select the new FTP factory explicitly through Registry and return a controlled session outside the Core lock. Preserve existing public `TransferSessionRequest` struct literals: typed requests/extended factory methods need default unsupported implementations, while HTTP retains its entry. Resolve/authenticate/connect/read/finalize outside the Core lock. Early factory failure finishes controls, stopping and joining any created workers before returning an error.

EPSV 优先；仅在明确不支持时回退一次 PASV。数据连接固定到控制连接实际 peer IP，忽略 PASV 提供的主机地址，不连接第三方地址。初始连接／欢迎／单命令／最终回复期限各 5 秒；数据读取每 1 秒检查控制，连续 8 次无数据判为停滞，所有等待可取消。沿用删除的 10 秒 worker 等待预算，超时保留占用并报告，不能假定阻塞文件 I/O 已退出。暂停等待 worker 停写并取得同一边界的进度快照，持久化后才返回。取消唤醒暂停和限速等待，join 后才释放目标与槽位；不能以 sleep 证明退出。

Prefer EPSV, falling back once to PASV only when explicitly unsupported. Pin data connections to the actual control peer IP, ignoring the PASV host and preventing third-party connections. Initially bound connection/banner/individual commands/final replies to five seconds each. Data reads check controls every second and fail after eight consecutive empty windows; all waits are cancellable. Retain the ten-second removal wait, keeping reservations and reporting timeout rather than assuming blocked file I/O exited. Pause waits for quiescence, snapshots progress at that boundary and persists it before replying. Cancellation wakes pause/throttling waits and joins before releasing destinations/slots; sleep is not proof of exit.

重试由 Scheduler 统一计数；FTP 库不另开无限重试。临时网络／适用 4xx 可在预算内重试；认证／权限、取消、本地安全、凭证和最终保存错误不自动重试。用结构化内部错误分类，再生成脱敏可读消息，不按错误字符串猜测类别。HTTP 暂停进度的继承问题已按 [RFC 0003](0003-registry-transfer-sessions.md#http-pause-progress-boundary) 实施修复和确定性回归。FTP 消费者仍须验证各自的停写边界、进度持久化与取消清理。

The Scheduler owns retry accounting, without an independent unbounded library loop. Transient network/applicable 4xx errors may retry within budget. Authentication/permission, cancellation, local safety, receipt and final-persistence errors do not. Classify internal errors structurally before rendering redacted messages, rather than parsing strings. [RFC 0003](0003-registry-transfer-sessions.md#http-pause-progress-boundary) records the HTTP pause-progress fix and deterministic regressions for the inherited issue. The FTP consumer still requires its own write boundary, progress persistence and cancellation cleanup verification.

<a id="held-file-receive"></a>

### 持有文件接收切片 / Held-file receive slice

基线为 `c1a03660db9a80be14a07531a563278bebfa470c`。本切片在 Engine 中接收已解析的端点、已解码的命令路径、匿名或用户名／密码输入和调用方独占的空文件句柄。复用现有 Tokio 和单 Worker 暂停控制。调用方控制线程生命周期，在回收 Worker 后才释放文件；Engine 只写入并同步持有的对象。Server 派发、恢复记录和文件发布依赖后续边界。

This slice starts at `c1a03660db9a80be14a07531a563278bebfa470c`. Engine takes a resolved endpoint, decoded command path, anonymous or username/password input, and a caller-exclusive empty file handle. It uses the existing Tokio runtime and single-worker pause controls. The caller owns the thread lifecycle and joins before releasing the file. Engine writes and syncs the held object. Server dispatch, recovery records and publication retain their follow-up dependencies.

固定候选 suppaftp 的实际试验发现 trace 泄漏认证命令及回复，且接受缺少 CRLF 或匹配多行结束码的最终回复。本切片用有界 FTP 回复解析和固定错误类型，拒绝不完整的回复。IPv4（包括 IPv4 映射地址）仅在 EPSV 返回 500 或 502 时尝试一次 PASV，数据地址始终来自控制连接。SIZE 可用时在数据 EOF、226／250 及最终 SIZE 后检查长度一致。暂停时间从主动网络等待预算中扣除。恢复撤销尚未确认的暂停时，暂停调用结束并报告未停写。

The pinned suppaftp probe exposed login commands and replies in trace logs and accepted final replies without CRLF or a matching multiline terminator. This slice uses bounded FTP reply parsing and structured errors that reject incomplete replies. On IPv4, including mapped addresses, only EPSV 500 or 502 permits one PASV attempt. The data address always comes from the control connection. When SIZE is available, lengths must agree after data EOF, 226/250 and the final SIZE response. Active network deadlines exclude time parked for pause. A resume that supersedes an unacknowledged pause releases its caller with an unpaused result.

实际回环消费者验证同一 RETR 暂停／恢复、停写字节快照、数据和最终回复等待中取消、目录项替换后只写原对象、IPv4 PASV 地址固定、IPv6 EPSV、回复和数据停滞上限，以及长度和认证错误。认证异常检查确认错误的 Debug 和 Display 不包含合成认证字段及源站回复。FTP 明文来源、已解析端点和持有文件是本切片的执行范围。

Actual loopback consumers verify pause/resume on one RETR, the quiescent byte snapshot, cancellation during data and final-reply waits, held-object writes across directory-entry replacement, IPv4 PASV address pinning, IPv6 EPSV, reply and data-stall bounds, and length and authentication failures. Authentication failures verify that Debug and Display omit synthetic credentials and origin replies. This slice operates on plain FTP, a resolved endpoint and a held file.

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

<a id="held-prefix-recovery"></a>

### 持有文件前缀恢复切片 / Held-file prefix recovery slice

基线为 `b6c9a8936382323d9a6fefa67e1d83a874ebc672`。新增独立的 `ftp-held-prefix/v1` 证明，绑定 Server 实例、任务、无秘密来源 origin、命令路径及路径模式、目标、凭据 ID／revision、尝试 generation、父目录及文件身份、已同步前缀长度／SHA-256 和可用 SIZE。仅从持有的文件及目录句柄取得身份。编码记录最大 64 KiB，未知版本／字段和绑定不符均拒绝。

The baseline is `b6c9a8936382323d9a6fefa67e1d83a874ebc672`. An independent `ftp-held-prefix/v1` proof binds the Server instance, task, secret-free origin, command path and path mode, destination, credential ID/revision, attempt generation, parent and file identities, synced prefix length/SHA-256 and available SIZE. Identities come from held file and directory handles. Encoded records are limited to 64 KiB. Unknown versions/fields and binding mismatches fail.

创建证明先确认调用方声明的可信长度与实际长度一致，分块计算摘要并同步文件，再返回有界编码。调用方负责在其持久层原子保存、按任务领取新 generation 及序列化尝试。恢复先校验绑定、身份及本地摘要，再发一次零偏移 RETR，比较网络前缀，匹配后在同一响应追加。恢复重读不增加整文件进度，追加前再次检查暂停／取消。前缀不匹配、短文件或未确认尾部保留原对象并报告具体错误。

Proof creation requires the caller's trusted length to match the actual length, hashes chunks and syncs the file before returning bounded encoded data. The caller atomically persists it, claims a new task generation and serializes attempts. Recovery validates bindings, identities and the local digest before one zero-offset RETR, compares its prefix and appends from that same response. Prefix retransmission leaves whole-file progress unchanged. Pause/cancel is checked before appending. A mismatched prefix, short file or uncommitted tail preserves the held object and returns a specific error.

此层证明作为后续 `ftp-resume/v1` 持久化消费者的输入。持久层的原子领取、目录项所有权、安全发布及最终状态保存仍由对应消费者验收。验收使用实际进程退出、重新打开句柄及真实 FTP 回环连接，检查恢复后的字节、摘要和进度。

This layer supplies proof to the later `ftp-resume/v1` persistence consumer. Atomic claiming, directory-entry ownership, safe publication and final-state persistence retain their consumer acceptance gates. Tests use actual process exit, reopened handles and real loopback FTP connections to check recovered bytes, digest and progress.

<a id="sqlite-checkpoint-claims"></a>

### SQLite 检查点与尝试领取 / SQLite checkpoints and attempt claiming

本切片基线为 `8b90113bce4d170b67c24ef18825c2bbdf4589cc`。schema 4 新增每任务的 FTP 尝试行，在同一 SQLite 事务中保存原始有界前缀证明、可信进度及递增的检查点 revision。领取使用预期的旧 generation 比较，生成随机新 generation，保留旧证明。保存同时比较当前 generation、revision、Server 实例和完整来源 incarnation；旧尝试、重复提交及删除重建后的旧引用均拒绝。

This slice starts at `8b90113bce4d170b67c24ef18825c2bbdf4589cc`. Schema 4 adds a per-task FTP attempt row. One SQLite transaction saves the original bounded prefix proof, trusted progress and increasing checkpoint revision. Claims compare the expected old generation, assign a fresh random generation and retain the old proof. Saves compare the current generation, revision, Server instance and full origin incarnation. Stale attempts, duplicate submissions and references surviving task recreation are rejected.

Core 从独立保存的来源重建引擎 binding，使用证明的只读字段检查绑定、generation 及可信进度。数据库拒绝提交时，内存进度与事件保持原状。删除任务在同一事务中清理尝试记录，保留原有凭据 tombstone。schema 3 数据按原样迁移；旧版本程序拒绝 schema 4。

Core reconstructs Engine bindings from the independently stored origin and validates proof bindings, generation and trusted progress through read-only proof metadata. A rejected database commit preserves in-memory progress and events. Task removal clears attempt records in the same transaction while retaining existing credential tombstones. Schema 3 data migrates intact. Older programs reject schema 4.

领取标识只隔离数据库提交，该 API 不领取调度槽位。调用方仍须先停止并回收旧 Worker，持有 Server 数据目录的独占权及目标文件的独占写入权，再领取新尝试。摘要与网络操作在 Core 锁外执行，提交时重新检查领取。新 API 由跨进程消费者验证；Server 下载派发、安全发布、完成对账和 App 接线继续按后续门槛推进。

Claim tokens fence database commits. This API does not claim a scheduler slot. Before a new claim the caller still stops and joins the old worker and holds exclusive Server data-directory ownership and target-file writer access. Hashing and networking run outside the Core lock, with the claim checked again at commit. Cross-process consumers verify the new APIs. Server download dispatch, safe publication, completion reconciliation and App integration retain their later gates.

数据库拒绝新的检查点时，文件中已同步的新增字节仍保留。旧证明面对这些未提交尾部会明确失败。安全截尾和文件所有权机制完成后才能接入自动恢复。

When SQLite rejects a new checkpoint, newly synced file bytes remain. The old proof rejects that uncommitted tail explicitly. Automatic recovery requires the safe trimming and file-ownership mechanisms.

<a id="held-tail-repair"></a>

### 持有对象尾部修复 / Held-object tail repair

基线为 `7653eeba615518b47f477d4d3ef33a41f3632937`。新增显式 `repair_uncommitted_tail` 入口，复用当前来源 binding、新领取的 generation 和原前缀证明。先验证父目录／文件句柄身份及可信前缀摘要，再以原文件句柄调用 `File::set_len`，截到可信长度并同步。空前缀和已经等长的文件同样校验／同步，可用于中断后的幂等重试。普通 `resume` 继续要求精确长度，修复由拥有文件的消费者明确调用。 [Rust File](https://doc.rust-lang.org/std/fs/struct.File.html#method.set_len)

The baseline is `7653eeba615518b47f477d4d3ef33a41f3632937`. A separate `repair_uncommitted_tail` entry uses the current origin binding, newly claimed generation and original prefix proof. It validates held parent/file identities and the trusted prefix digest, then calls `File::set_len` through that file handle and syncs it. Empty prefixes and already exact-length files are validated and synced too, permitting idempotent retry after interruption. Ordinary `resume` keeps its exact-length requirement. The file-owning consumer explicitly requests repair. [Rust File](https://doc.rust-lang.org/std/fs/struct.File.html#method.set_len)

截断、同步、可信进度及修复完成在暂停／取消的同一决定锁内执行。先取得该锁的取消阻止修复，暂停使修复停驻；先取得锁的修复完成后，后到的取消观察已完成状态。本地文件 I/O 的阻塞仍受调用方退出／join 边界约束。同步失败保留具体错误和原证明，文件可能已经截断，重试会重新验证前缀并同步。

Truncation, sync, trusted progress and successful repair completion share the pause/cancel decision lock. Cancellation winning that lock prevents repair, while pause parks it. When repair wins, later cancellation observes its finished state. Blocking local file I/O still requires the caller's exit/join boundary. Sync failure preserves its specific error and the original proof. The file may already be truncated; retry revalidates and syncs the prefix.

所有修改只作用于证明绑定的对象。目录项被换成另一普通文件、符号链接或无关 hard link 时，替换对象保留。指向同一 inode 的其他名称会观察到该对象的长度变化。调用方负责回收旧 Worker、独占写入及按 SQLite claim 隔离提交；证明和数据库进度在修复期间保持原样。精确替换、控制交错、失败注入和实际子进程终止后重试均需验证。发布、名称清理和最终 Completed 继续按各自门槛推进。

All mutations affect the proof-bound object. Replacing its former entry with another regular file, symlink or unrelated hard link preserves the replacement. Other names for the same inode observe its length change. The caller joins the old worker, owns file writes and fences commits through the SQLite claim. Proof and database progress stay intact during repair. Exact replacement races, control interleavings, injected failure and retry after actual child termination require validation. Publication, name cleanup and final Completed retain their own gates.

<a id="held-completion-proof"></a>

### 持有文件完成证明切片 / Held-file completion-proof slice

基线 `48ae2a4950a6b09beee0da969a9dfb3c9c1a45c6`。本切片落实已规划的发布前证明生成及离线重核，不操作目录项。新增 `receive_complete`／`resume_complete`，只有真实 RETR 的 EOF、226／250、可用的 SIZE 一致性和同步成功后，才从持有文件生成独立 `ftp-held-completion/v1`。绑定来源／任务 incarnation／凭据 revision、独立当前 generation、父目录和文件身份、完整长度及 SHA256；严格有界编码沿用 64 KiB 上限，不把等长前缀证明当作终结回复证明。

At baseline `48ae2a4950a6b09beee0da969a9dfb3c9c1a45c6`, this slice implements planned prepublication proof generation and offline verification without directory-entry operations. New `receive_complete`/`resume_complete` generate a separate `ftp-held-completion/v1` from the held file only after actual RETR EOF, 226/250, available SIZE consistency and successful sync. Bind origin/task incarnation/credential revision, independently supplied current generation, parent/file identities, complete length and SHA256. Strict bounded encoding retains the 64 KiB cap. An exact-length prefix proof is not evidence of the terminal FTP reply.

`FtpCompletionRequest` 提供独立 binding／generation；`verify_completion` 必须精确匹配两者，并重新检查持有对象身份、长度及摘要。未知格式、前缀证明、同内容不同身份、错误来源／generation、损坏及多余尾部均拒绝。证明生成／重核期间沿用分块取消／暂停，不把摘要扫描量报告为下载量。成功结果只在最后控制决定后返回；原 receive／resume 和 HTTP v1 行为保持兼容。

`FtpCompletionRequest` supplies independent binding/generation; `verify_completion` matches both exactly and rechecks held identities, length and digest. Reject unknown formats, prefix proofs, equal-content replacement identities, wrong origin/generation, corruption and extra tails. Proof generation/verification retain chunked pause/cancellation without counting digest scanning as downloads. Return success only after the final control decision. Preserve existing receive/resume and HTTP v1 behavior.

外部结构化导入只通过有界 `decode`，公开证明类型不提供直接反序列化。离线重核的可信进度与成功完成在同一决定锁内发布；哈希后取消获胜时，不发布该核验进度。网络实写进度仍保留原有含义。

External structured imports use bounded `decode`; the public proof type provides no direct deserialization. Offline verified progress and successful completion are published under one decision lock. Cancellation winning after hashing prevents that verified progress publication. Network-written progress retains its existing meaning.

消费者用公开 Core 入口取得来源和 generation，经真实 FTP 生成证明；测试拥有并预先打开证明文件，写入／同步编码后退出进程。另一进程在源站离线时从未重新领取的 Core 尝试独立取得 binding／generation，重开持有文件并核验；错误 generation 不自动接受。此测试文件不是产品 journal 发布协议。产品 SQLite 完成证明保存、重新领取时保留旧证明授权、Ready 后恢复、最终 Completed、发布／清理和 Server 路由留给后续；不新增 schema 或产品能力声明。

A consumer obtains origin/generation through public Core APIs and generates proof over actual FTP. Its test owns an already-open proof file, writes/syncs the encoding and exits. With the origin offline, another process obtains binding/generation independently from the unreclaimed Core attempt, reopens held files and verifies; mismatched generations are not implicitly authorized. The test file is not a product journal-publication protocol. Durable product SQLite completion proofs, retained-proof authorization across reclaim, recovery after Ready, final Completed, publication/cleanup and Server routing remain later work. Add no schema or product capability claim.

实施顺序：先记录真实 FTP 成功而新入口尚未实现的失败测试；分离单流接收与控制结束，增加完整证明及重核；覆盖终结失败／SIZE 变化／未知长度／零文件、对象／来源／generation／格式异常、精确替换及摘要中暂停／取消；加入 Core 跨进程离线消费者。最后固定 SHA，完整 Rust 门槛及独立真实源站验证、审查和合并前后 CI 全通过后交付。

Execution order: record a failing real-FTP success case before implementing the new entry; separate reception from final control completion and add full proofs/verification; cover terminal failure/SIZE changes/unknown length/empty files, object/origin/generation/format failures, exact replacement and pause/cancel during hashing; add the cross-process offline Core consumer. Freeze the SHA and deliver only after complete Rust gates, independent real-origin validation/review and CI before/after merge pass.

<a id="sqlite-completion-proof"></a>
### SQLite 完成证明与离线重核 / SQLite completion proof and offline verification

最终快照检查在取消决定锁内以 SQLite `BEGIN IMMEDIATE` 持有写预约，连接 busy timeout 为 250 ms；持续持有至完成决定，再显式回滚只读事务并报告释放失败，该事务不修改数据库。禁止 guard 执行网络、重入 Core 或长时间工作。此受保护路径不向 control 发布核验进度，只有成功释放后才返回当时有效的核验证据；已保存 SQLite 进度不变，后续发布必须重新取得授权。锁冲突、读取或释放失败不报告成功，取消和验证由先获得决定锁者决定。原 Engine 无 guard 重核保留原有进度行为。

The final snapshot check runs under the cancellation decision lock with a SQLite `BEGIN IMMEDIATE` writer reservation; existing connections use a 250 ms busy timeout. It holds the reservation until the finished decision, then explicitly rolls back the read-only transaction and reports any release failure. No database changes occur in that reservation. Never perform network, reentrant Core calls or long-running work in this guard. This guarded path does not publish control progress; it returns a point-in-time verification result only after successful release. The previously saved SQLite progress remains unchanged, and a later publication must obtain fresh authorization. A lock conflict/read/release failure cannot report success; whichever action takes the decision lock first determines cancellation or verification. Existing unguarded Engine verification retains its original progress behavior.

保留的前缀证明仍可由调用方读取；本切片只拒绝完成证明存在后的数据库前缀覆盖，不能阻止调用方绕过 Core 直接调用 Engine 发起 RETR。未来 Server 派发必须先识别并离线对账已观察到的完成证明，禁止将其静默降级为普通网络重试。

The retained prefix remains readable by callers. This slice rejects database prefix replacement after completion, but cannot prevent a caller from invoking Engine RETR directly. Future Server dispatch must identify and reconcile an observed completion proof offline first, without silently falling back to ordinary network retries.

基线 `815b57bb0cbf4d7cd4c4cb1febffd3ae6c71893e`。schema 5 在 `ftp_attempts` 同一行增加独立版本化完成证明、原生产 generation 和 `completion_seen` 标记，保留 schema 4 的前缀证明。已观察标记仍在而证明字段丢失、版本未知或任务进度与证明不一致时拒绝领取并保留记录。完成证明、可信整文件进度与 revision 在同一 SQLite 事务提交；任务状态不因此变为 Completed。证明须匹配独立加载的来源 binding、当前生产 generation 和完整长度。成功前不发布 Core 内存进度或事件。

At base `815b57bb0cbf4d7cd4c4cb1febffd3ae6c71893e`, schema 5 adds separately versioned completion proof, original producing generation and a `completion_seen` marker to the same `ftp_attempts` row, preserving schema 4 prefix proofs. If the marker remains but proof fields disappear, the version is unknown, or persisted task progress differs from the proof, reject claims and retain the record. Commit completion proof, trusted whole-file progress and revision in one SQLite transaction without setting Completed. Match proof against independently loaded origin binding, current producing generation and complete length; publish no Core progress/event before commit.

领取以 instance、完整来源 incarnation、generation、revision、前缀及完成证明组成的完整快照 CAS；重新领取产生新 generation，但保留原完成证明与生产 generation，不重写或将当前 generation 冒认为原生产者。已有完成证明时拒绝后续前缀或第二份完成证明覆盖。离线重核前后均对照当前完整数据库快照，以保存的原生产 generation、来源绑定及调用方持有的文件／目录句柄调用 Engine 重核；调用方仍负责停止旧 writer、独占对象及保存时与取消协调。SQLite 授权不证明目录项所有权。

Claim via the complete snapshot of instance, origin incarnation, generation, revision, prefix and completion proof. Reclaim changes generation but preserves the completion proof and original producer; never rewrite or substitute the current generation. Reject subsequent prefix or second completion replacement. Before and after offline verification compare the current complete database snapshot, then use its retained producer, origin binding and caller-held file/directory handles for Engine verification. Callers still stop old writers, own the objects and coordinate cancellation when saving. SQLite authority does not prove pathname ownership.

迁移 schema 4 时保留旧记录和来源；未知版本、畸形／缺失字段、陈旧 revision、旧 task incarnation 和来源不一致均失败保留证据。数据库故障回滚整个证明／进度更新；删除与同 ID 重建使旧快照失效。重复领取保留证明而不授权旧快照提交。直接测试迁移、限制／冲突／故障及并发 CAS；真实 FTP 生成证明，持久化后由独立进程重开、源站离线验证，并覆盖取消与对象替换。完整检查、独立审查和前后 CI 在对应 PR 留证。此切片不发布文件、不保存 Completed、不接入 Server FTP 路由。

Migrate schema 4 without altering old records or origins. Reject unknown versions, malformed/missing fields, stale revisions, old task incarnations and origin mismatch while retaining evidence. Database failure rolls back proof/progress together. Deletion and same-ID recreation invalidate old snapshots; repeated claims retain proof without authorizing stale commits. Direct tests cover migration, limits/conflicts/failures and concurrent CAS. A real FTP producer persists proof before an independent process reopens and verifies offline, including cancellation and object replacement. Record full gates, independent review and pre/post-merge CI in the PR. No publication, Completed persistence or Server FTP routing is introduced.

## 完成、文件安全与兼容 / Completion, file safety and compatibility

<a id="held-publication-convergence"></a>
### 持有对象发布与离线完成收敛方案 / Held-object publication and offline completion convergence design

基线 `dbc479b6b52012812ef55f461a9d27c888aee9fc`。本包在已接受的普通桌面范围内使用单一 directory-relative `linkat(..., 0)` 不覆盖发布，保留来源别名，不执行自动清理或 copy／clone／rename 回退。不接 Server／CLI／App、FTPS、BT 或 M4；平台／文件系统不支持时具体失败，不宣称未验平台能力。

At the stated base, use one directory-relative linkat no-clobber publication algorithm under the accepted ordinary-desktop scope. Retain the source alias without automatic cleanup or copy/clone/rename fallback. No Server/CLI/App, FTPS, BT or M4 integration; unsupported platforms/filesystems fail explicitly without an unverified capability claim.

独立 `ftp-publication/v1` 有界 plan 绑定原完成证明、来源 incarnation、真实绝对目标、held parent、两个单叶名称、原 producer 与当前 owner/authorization。schema 6 的独立 intent 及 observed marker 先于发布保存，claim 保留旧授权但不自动 rearm。实际发布在 control 决定锁内重新核对完整 Downloading task／attempt／intent，持有 SQLite 预约跨 linkat；hash 在外。发布后错误保留 committed 事实和文件，最终进度／Completed／intent completed 另经完整 CAS 同事务保存。

A bounded versioned plan binds the original completion, origin incarnation, real absolute destination, held parent, strict leaves, original producer and current owner/authorization. Persist a separate schema-6 intent/observed marker before publication. Claims retain old authorization without automatically rearming. Under the control decision lock, revalidate the entire Downloading task/attempt/intent and hold a SQLite reservation across linkat; hash outside final locks. Preserve committed facts/files on postpublication errors, then atomically CAS final progress, Completed and completed intent separately.

专用恢复领取保留整文件进度与调度／事件原子性，Failed＋pending intent 不得被普通恢复漏掉。Completed／orphan 采用独立只读诊断接口，任务移除及同 ID 重建不能重新授权旧 journal。检测已观察记录丢失，不声称任意数据库历史防篡改。[实施计划](../superpowers/plans/2026-11-21-ftp-publication.md) 记录任务、直接测试与真实跨进程消费者；本段为实现方案，不代表代码或行为验收。

Dedicated recovery claims preserve whole-file progress and scheduler/event atomicity, including Failed tasks with pending intents. Separate read-only Completed/orphan diagnostics cannot reauthorize removed/recreated tasks. Detect observed-record loss without generic historical-tamper claims. The [implementation plan](../superpowers/plans/2026-11-21-ftp-publication.md) defines tasks/direct/process consumers; this is a design rather than implementation/behavioral acceptance.

停写、长度验证和 sync 后先保存完成证明，再在同一取消决定锁内安全地不覆盖发布，最后原子保存最终进度和 Completed。保存失败保留证明，重启优先离线对账，不发额外 RETR。沿用旧 HTTP v1 路径／读取，FTP 新格式与目录独立版本化，不能静默修改 `http-completions` 含义。

After quiescence, length validation and sync, persist completion proof before safe no-clobber publication under the cancellation decision lock, then atomically persist final progress and Completed. Retain proof on failure and reconcile offline before another RETR. Keep legacy HTTP v1 paths/readers; separately version FTP formats/directories without silently changing `http-completions` semantics.

[RFC 0002 的首发普通桌面范围](0002-segment-file-ownership.md#ordinary-desktop-ftp-bt) 于 2026-11-21 由维护者明确接受，适用于首发 macOS FTP。保留不覆盖、符号链接、完整性、取消、恢复和身份不匹配时的失败保留，不承诺抵御同账户恶意程序持续替换目录项，不增加管理员服务。`check → rename/link/unlink` 的残余竞态并未因此修复；不能按修改者意图区分竞态，也不能把随机目录、0700 或额外 stat 称为原子所有权。此范围不改变 M4 或旧 HTTP 的保证。产品派发仍等待发布／清理实现、SQLite 完成保存、取消及离线恢复的真实消费者验收，不能因范围接受而启用。

On 2026-11-21 the maintainer explicitly accepted [RFC 0002's first-release ordinary-desktop scope](0002-segment-file-ownership.md#ordinary-desktop-ftp-bt) for macOS FTP. Retain no-clobber, symlink, integrity, cancellation, recovery and mismatch-failure preservation without promising resistance to persistent same-account namespace tampering or adding an administrator service. This does not repair residual `check → rename/link/unlink` races, distinguish the writer's intent, or turn random names, 0700 or extra stat calls into atomic ownership. It changes neither M4 nor legacy HTTP guarantees. Product dispatch still requires implemented publication/cleanup, atomic SQLite completion, cancellation and actual offline-recovery consumers; scope acceptance cannot enable it.

<a id="implementation-and-acceptance"></a>

## 实施与验收 / Implementation and acceptance

### F1 来源凭据实现状态 / F1 origin-credential implementation status

Keychain 原子新增同时把 SQLite 已持久化的 `operation_id` 保存为 `kSecAttrGeneric` 二进制所有权标记。读取和删除在同一个 provider 查询中匹配精确引用及该标记。相同 service／account 的不同 owner 新增仍返回冲突。失败新增后的 pending 丢弃即使失败或中断，恢复也只会清理该操作实际拥有的项目。

Keychain adds atomically store the already-durable SQLite `operation_id` as a binary `kSecAttrGeneric` ownership marker. Reads and deletion match both the exact reference and marker in one provider query. A different owner adding the same service/account still receives a duplicate conflict. If discarding pending after a failed add fails or is interrupted, recovery cleans only an item owned by that operation.

2026-11-14 基于 `f8b89c6` 实现 `task.create_transfer` v1、严格 FTP 路径、真实 CLI stdin 和 TCP／HTTP 的实际 TLS 校验。macOS provider 使用独立、非同步的 Keychain 项、原子 add-only 写入及非交互查询／删除。schema 3 保留随机数据实例、版本化来源、pending 与精确 cleanup tombstone，覆盖任务激活、失败补偿、删除和重启清理。provider 在 Core 锁外运行，恢复清理在 Ready 后启动。同 ID 重试比较输入及 provider 中的秘密，冲突拒绝改密。

On 2026-11-14, implement `task.create_transfer` v1, strict FTP paths, actual CLI stdin and actual TCP/HTTP TLS assurance from `f8b89c6`. The macOS provider uses separate nonsynchronized Keychain items, atomic add-only writes and noninteractive queries/deletion. Schema 3 retains random data-instance identity, versioned origins, pending and exact cleanup tombstones for activation, compensation, removal and restart cleanup. Providers run outside Core and recovery starts after Ready. Same-ID retries compare input and provider secrets, rejecting changed passwords.

本包声明元数据创建、macOS 持久凭据和 `transfer:false`。`required_explicit_tls` 在副作用前拒绝，FTP 排队／恢复也显式返回不可用。当前只创建 revision 1，秘密更新及在途 revision 退休留给传输实现。确定性测试覆盖真实 SQLite、真实匿名 Server／CLI 入口、并发重试、锁定／缺失／拒绝、激活补偿、删除回滚及 provider 等待时的 HTTP 控制。固定 SHA 的独立审查、实际 Keychain／随包 Server／CLI、原生候选和最新 CI 由本包交付记录补充。M5 及 F2／F3／F4 保持开放。

This package advertises metadata creation, macOS durable credentials and `transfer:false`. Reject `required_explicit_tls` before side effects and explicitly return unavailable for FTP queue/resume. Create revision 1, with secret updates and in-flight retirement accompanying transfers. Deterministic tests cover real SQLite, real anonymous Server/CLI ingress, concurrent retries, locked/missing/denied providers, activation compensation, removal rollback and HTTP controls during provider waits. Delivery records add fixed-SHA independent review, actual Keychain/bundled Server/CLI, native candidates and latest CI. M5 and F2/F3/F4 remain open.

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
