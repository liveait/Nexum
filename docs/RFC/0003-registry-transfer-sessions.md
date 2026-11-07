# RFC 0003 — Registry 传输会话路由 / Registry transfer-session routing

- F0 方案已通过固定提交 `44113fd` 的独立审查，实现与本地完整 Rust 验证通过。真实消费者、最终源码审查及合并前后 CI 由对应 PR 记录。
  The F0 design passed independent review at fixed commit `44113fd`. Implementation and complete local Rust checks pass. The corresponding PR records real consumers, final-source review and pre/post-merge CI.
- 日期 / Date `2026-11-06`
- 实现基线 / Implementation baseline `b9bb2d4bc9aebfe9d3c8813d33f9bfbc03ed56a9`

## 摘要与动机 / Summary and motivation

首发已要求 FTP、torrent 和 Magnet。现有 Registry 只能按名称调用同步 Adapter，真实 Server 直接创建 HTTP 会话。[ADR 0008](../decisions/0008-long-running-transfer-contract.md) 已接受长期传输设计，F0 补充可选工厂和协议路由，使 Server 经 Registry 使用现有单连接 HTTP 路径，为后续协议提供接入点。

The first release requires FTP, torrent and Magnet. Registry currently starts synchronous adapters by name, while the real Server constructs HTTP sessions directly. [ADR 0008](../decisions/0008-long-running-transfer-contract.md) accepts the long-running transfer design. F0 adds optional factories and protocol routing so the Server uses the existing single-connection HTTP path through Registry, providing the entry point for subsequent protocols.

## API 与选择 / API and selection

`EngineAdapter` 增加默认返回 `None` 的 `session_factory()`，返回类型为 `Option<Arc<dyn TransferSessionFactory>>`。既有 Adapter、`register()`、按名称启动和控制继续采用原签名；旧 Adapter 本身只需既有的 `Send` 边界，工厂单独要求 `Send + Sync`。获取工厂只允许有界的内存操作。

Add defaulted `EngineAdapter::session_factory()` returning `None`, with type `Option<Arc<dyn TransferSessionFactory>>`. Existing adapters, `register()`, named starts and controls retain their signatures. Existing adapters keep their current `Send` bound, while factories separately require `Send + Sync`. Retrieving a factory permits only bounded in-memory work.

```rust
pub trait TransferSessionFactory: Send + Sync {
    fn start(
        &self,
        request: TransferSessionRequest,
    ) -> Result<Box<dyn TransferSession>, EngineError>;
}
```

`TransferSessionRequest` 拥有来源、目标、稳定 partial 路径、既有 `HttpTransferControl`及可选 `CompletionContext`。F0 保留 HTTP 控制和完成格式；FTP 的类型化恢复及凭据输入、BT 多文件存储和完成证明由后续协议 RFC 定义。

`TransferSessionRequest` owns the source, destination, stable partial path, existing `HttpTransferControl` and optional `CompletionContext`. F0 retains HTTP control and completion formats. Later protocol RFCs define typed FTP recovery/authentication inputs and BT multi-file storage/completion proofs.

Registry 的 `session_factory_for_protocol()` 只选择 `supported_protocols` 匹配且提供工厂的 Adapter，协议比较忽略 ASCII 大小写。没有可用工厂或多个 Adapter 同时匹配时返回明确错误。歧义在选择时拒绝，既有注册接口继续接受同步 Adapter。能力中的分段参数不决定 Server 执行策略。

Registry's `session_factory_for_protocol()` selects adapters whose `supported_protocols` match and that provide a factory, using ASCII-case-insensitive comparison. Missing and ambiguous factories return explicit errors. Selection rejects ambiguity while the existing registration API continues accepting synchronous adapters. Segment capabilities do not determine Server execution policy.

## 启动、控制与错误 / Startup, controls and errors

Server 在领取任务前、持有 Core 锁时选择并保存工厂。选择失败时任务保持排队，槽位与目标预约未取得。Worker 在锁外调用 `start_transfer_session()`；HTTP 工厂检查 partial，按现有行为为每次尝试创建 HTTP 引擎，并委托 `start_download_with_partial()`。返回的 trait object 沿用既有进度轮询和完成保存路径。

The Server selects and retains the factory under the Core lock before claiming a task. Failed selection leaves it queued without acquiring a slot or destination reservation. The worker calls `start_transfer_session()` outside the lock. The HTTP factory validates partial data, preserves per-attempt engine construction and delegates to `start_download_with_partial()`. The returned trait object follows existing progress polling and completion persistence.

统一启动入口在调用工厂前检查取消，并在返回错误时通过私有 `mark_finished()` 唤醒等待暂停的调用者。工厂只有在自行启动的 Worker 已停止写入并退出后才可返回 `Err`。Server 把启动错误与已请求取消同时发生的情况归一为取消，避免自动重试。工厂 panic 的隔离属于后续插件／运行时边界。

The common start entry checks cancellation before invoking a factory and uses private `mark_finished()` on errors to wake pause waiters. A factory may return `Err` only after workers it started have stopped writing and exited. The Server normalizes startup errors concurrent with requested cancellation to cancellation, preventing automatic retry. Factory-panic isolation belongs to subsequent plugin/runtime boundaries.

待对账完成凭证的任务不选择或启动网络工厂，继续由 Ready 后的受控 Worker 离线对账，保留凭证消失后的失败语义。HTTP 成功会话沿用现有停止写入后的终态等待语义；本切片不定义其他协议的多 Worker join 或通用跨重启恢复格式。

Tasks with pending completion receipts bypass network-factory selection/start and retain controlled post-Ready offline reconciliation, including failure when a pending receipt disappears. Successful HTTP sessions retain their existing terminal wait after writing stops. This slice leaves multi-worker joins and general restart-recovery formats to the respective protocols.

## 兼容性与安全 / Compatibility and safety

沿用 [ADR 0003](../decisions/0003-http-transfer-controls.md)、[ADR 0004](../decisions/0004-cross-restart-http-resume.md) 和 [ADR 0007](../decisions/0007-http-completion-reconciliation.md) 的控制、来源／Range 校验、文件与 sidecar 身份、发布前取消、v1 凭证及原子 final progress／Completed 保存。HTTP/HTTPS 是本切片唯一 Server 传输路径。FTP／BT 尚须完成 Resolver、凭据、存储、恢复、客户端和平台安全工作，才可启用。

Retain controls, source/Range validation, file/sidecar identity, pre-publication cancellation, v1 receipts and atomic final progress/Completed persistence from [ADR 0003](../decisions/0003-http-transfer-controls.md), [ADR 0004](../decisions/0004-cross-restart-http-resume.md) and [ADR 0007](../decisions/0007-http-completion-reconciliation.md). HTTP/HTTPS are the only Server transfer paths in this slice. FTP/BT enablement requires their resolver, credentials, storage, recovery, clients and platform-safety work.

## 验证与交付 / Verification and delivery

- 直接覆盖同步 Adapter 默认兼容、命名启动、协议大小写、未知协议、声明协议但无工厂及多工厂歧义。实际 HTTP 工厂验证文件、进度与完成凭证。
  Directly cover default synchronous-adapter compatibility, named starts, protocol case, unknown protocols, declared protocols without factories and ambiguity. Exercise the real HTTP factory's files, progress and receipts.
- 统一启动入口覆盖开始前取消和启动错误唤醒暂停。Server 消费者用受控工厂委托真实 HTTP，证明锁外启动及 Registry 选择；启动失败与取消保持队列、目标、槽位和重试一致性。离线凭证恢复绕过工厂。
  Cover cancellation before start and pause wakeup on start failure. A controlled factory delegates real HTTP in Server consumer tests, proving Registry selection and startup outside the lock. Startup failures/cancellation preserve queue, destination, slot and retry consistency. Offline receipt recovery bypasses the factory.
- 完整 Server unit／managed／persistence 和 Engine 回归覆盖控制、停滞、Range／来源变化、续传进度、SQLite 完成保存失败及离线恢复。真实 Server／CLI 走正常任务入口。运行完整 Rust 门槛、固定 SHA 独立审查及合并前后 CI。
  Full Server unit/managed/persistence and Engine regressions cover controls, stalls, Range/source changes, whole-file resume progress, rejected SQLite completion saves and offline recovery. Real Server/CLI checks use normal task entry points. Run complete Rust gates, fixed-SHA independent review and pre/post-merge CI.

## 后续协议决策 / Subsequent protocol decisions

FTP RFC 决定依赖、FTPS 模式、凭据作用域、版本化恢复格式和可信前缀复用。BT RFC 决定 v1／hybrid／v2-only、torrent 内容上传、发现／上传／做种策略、文件树预约、分片恢复、持有文件存储、真正的写入退出屏障和多文件完成证明。实际协议采用的文件发布／清理机制须通过平台验证，首发门槛见[开发计划](../DEVELOPMENT_PLAN.md#2-delivery-milestones)。

The FTP RFC decides dependencies, FTPS modes, credential scope, versioned recovery and trustworthy prefix reuse. The BT RFC decides v1/hybrid/v2-only, torrent-content upload, discovery/upload/seeding, tree reservations, piece recovery, held-file storage, actual write-drain barriers and multi-file completion proofs. Each protocol's publication/cleanup mechanisms require platform verification. See the [Development Plan](../DEVELOPMENT_PLAN.md#2-delivery-milestones) for first-release gates.

<a id="local-consumers-and-baseline-finding"></a>

## 本地消费者与基线问题 / Local consumers and baseline finding

固定产品源码 `311aa4a` 的真实 Server／CLI 检查完成 8 MiB 文件下载、完整哈希和 SQLite Completed、暂停后同 GET 恢复、取消清理、三槽占满后释放给第四任务，以及 Server 强退后从 524,288 字节精确 Range／If-Range 续传。关闭源站后离线重开没有额外 GET，18 份 SQLite 副本完整，所属进程均已退出。本轮采用私有数据与下载目录，首发协议的最终原生候选验收继续按开发计划执行。

Real Server/CLI checks at product source `311aa4a` cover 8 MiB downloads, full hashes and SQLite Completed, same-GET pause/resume, cancellation cleanup, release of one of three occupied slots to a fourth task, and exact Range/If-Range resume from 524,288 bytes after a Server crash. Offline reopen with the origin stopped adds no GET. All 18 SQLite backups pass integrity checks and owned processes exit. Checks use private data/download directories, with final native-candidate acceptance for first-release protocols tracked in the Development Plan.

消费者首轮“暂停返回后的首个 SQLite 计数已是最终暂停值”断言失败，原记录保留。F0 的三次精确观测中，文件长度和 SHA256 已停止变化，数据库落后一个 64 KiB 块并在约 0.100–0.114 秒后补齐。独立构建的基线 `b9bb2d4` 使用同一脚本也在三次观测中复现，约 0.101–0.127 秒补齐；状态始终为 Paused，速率为零，ETA 为空。既有控制、完成和进度轮询函数逐字节相同。

The initial consumer assertion that the first SQLite count after pause already equals the final paused count fails, with the original result retained. In three F0 observations, file length and SHA256 stop changing while the database trails by one 64 KiB chunk and catches up after approximately 0.100–0.114 seconds. An independently built `b9bb2d4` baseline reproduces this with the same script in three observations, catching up after approximately 0.101–0.127 seconds. State remains Paused with zero speed and no ETA. Existing control, completion and progress-polling functions are byte-identical.

该继承的进度快照延迟作为已知 P2 接受本次工厂路由独立交付。文件已停止写入，最终完整性、取消清理和重启恢复通过，F0 没有扩大该窗口。首发前另行修复暂停时进度采样／持久化的同步，并增加对应的确定性回归；本次不把原始计数断言记为通过，也不据此完成 M1／M2／M5。

This inherited progress-snapshot delay is accepted as a known P2 for independent delivery of factory routing. Files have stopped writing, final integrity/cancellation/restart recovery pass, and F0 does not widen the observed window. Before the first release, fix pause-time progress sampling/persistence synchronization separately and add a deterministic regression. The original counter assertion remains failed, and this slice completes neither M1, M2 nor M5.
