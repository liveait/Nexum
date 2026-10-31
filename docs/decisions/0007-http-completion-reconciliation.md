<a id="adr-0007-reconcile-http-file-publication-and-task-completion"></a>
<a id="adr-0007http-文件发布与任务完成对账"></a>

# ADR 0007：HTTP 文件发布与任务完成对账 / Reconcile HTTP File Publication and Task Completion

- 状态：设计已接受；下文记录 2026-10-11 的实现验证，不代表当前提交已验收<br>
  Status: Accepted design; implementation verification from 2026-10-11 recorded below, not current acceptance
- 日期 / Date: 2026-10-11

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

[ADR 0004](0004-cross-restart-http-resume.md) 能在不覆盖已有目标的前提下发布完整 HTTP 文件。文件发布和 SQLite 任务保存无法组成一个事务：进程可能在发布后、保存 `Completed` 前退出。原先恢复后的 worker 会把自己已下载的完整文件当作目标冲突。没有 validator 或已知总长度的响应同样存在此问题。[ADR 0006](0006-managed-macos-server.md) 要求解决此窗口后才能通过 M1。

[ADR 0004](0004-cross-restart-http-resume.md) safely publishes a complete HTTP file without replacing an existing destination. File publication and SQLite task persistence cannot be one transaction: a process can exit after publication but before saving `Completed`. Previously the recovered worker treated its own complete destination as a conflict. This also affects responses without validators or a known total length. [ADR 0006](0006-managed-macos-server.md) requires recovery across this window before M1 can pass.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

1. 响应通过长度校验、读取至 EOF 且暂存文件同步后，engine 在发布前原子写入有大小上限的 JSON 完成凭证。凭证保存在 Server 私有的 `data_dir/http-completions/`，以任务 ID 的 SHA-256 命名；绑定任务 ID、来源、规范化目标及暂存路径、实际字节数、原始可选总长度、文件身份和 SHA-256。既有 HTTP validator sidecar 保持独立；通用任务存储不新增 HTTP 字段。<br>
   After a validated response reaches EOF and the staging file is synced, the engine writes an atomic, bounded JSON completion receipt before publication. The receipt lives in the Server's private `data_dir/http-completions/`, named by SHA-256 of the task ID. It binds the task ID, source, normalized destination and staging paths, actual bytes, original optional total, file identity and SHA-256. The existing HTTP validator sidecar remains separate; generic task storage gains no HTTP fields.
2. 保留既有不覆盖目标的原子 rename 和 hard-link 回退。最终进度与 `Completed` 通过 Core 的一次 repository update 同时持久化成功后，才清理完成凭证。保存失败保留凭证供下次对账；绝不覆盖已有目标。<br>
   Keep the existing atomic no-clobber rename and hard-link fallback. Retain the completion receipt until final progress and `Completed` are persisted together by one Core repository update. A persistence failure retains the receipt for another attempt; an existing destination is never overwritten.
3. worker 在发 HTTP 请求前尝试凭证对账。目标是普通文件且身份、长度、摘要均匹配时，无需网络即可完成任务。目标不存在、完整暂存文件匹配时，可无需网络重新发布。仅摘要相同不能认领用户替换的文件。拒绝符号链接、特殊文件、损坏或超大的凭证以及绑定不符；保留冲突的用户文件并报告错误。<br>
   Before making an HTTP request, the worker tries receipt reconciliation. A regular destination with matching identity, length and digest can finish without network access. If the destination is absent, matching complete staging bytes can be published without network access. A matching digest alone cannot identify an unrelated replacement file. Reject symbolic links, special files, malformed or oversized receipts and binding mismatches. Preserve conflicting user files and report an error.
4. 启动只读取有上限的小型凭证元数据。具有正确绑定凭证的失败任务可重新排队对账；其他失败任务保持原恢复规则。摘要校验放在 Server 就绪后的 HTTP worker，配合暂停和取消。已经完成的任务只清理残留凭证，不重新读取文件摘要，避免超出托管 Server 六秒就绪预算。<br>
   Startup reads only bounded receipt metadata. A failed task with a correctly bound pending receipt may be requeued for reconciliation; other failed tasks retain their existing recovery behavior. Hashing runs in the HTTP worker after Server readiness and cooperates with pause/cancel. Already completed tasks need only residual-receipt cleanup, without rehashing their files. This keeps the managed Server within its six-second readiness budget.
5. 删除残留暂存路径前校验文件身份，包括 hard-link 回退后两个路径同时存在的情况。删除任务先停止 worker、删除数据库任务，再清理凭证。清理失败时报告错误，并将剩余私有凭证保留为孤儿证据；不会重新创建已删除的数据库任务。复用不存在的任务 ID 前必须成功清理孤儿凭证，避免旧任务证据误认新任务。清理不删除最终目标。<br>
   Cleanup of a leftover staging path checks file identity before unlinking it, including the hard-link case where both paths remain. Task removal stops its worker and deletes the database task before clearing the receipt. If cleanup fails, report the error and retain any remaining private receipt as orphan evidence; the removed database task is not resurrected. Reuse of an absent task ID requires successful orphan-receipt cleanup before creation, so a previous task's evidence cannot complete a new task. Cleanup never deletes the final destination.

<a id="implementation-status"></a>
<a id="实施状态"></a>

## 实施状态 / Implementation Status

历史基线 `f1817f4` 至 `cf88455` 的 `open_core` 在监听前同步校验待恢复文件的 SHA-256，偏离第 4 条的设计，可能超出托管 Server 六秒就绪预算。2026-10-31 的恢复就绪切片把完整校验移到就绪后的 HTTP Worker：启动只检查有上限的凭证绑定；恢复任务领取 Scheduler 槽位并登记目标，摘要读取与暂停／取消等待不持有 Server 全局锁。对账失败保留具体错误和凭证，不进入普通网络重试；启动已观察到的凭证随后消失，也不能转为普通 GET。已经完成的任务仍只按身份清理残留，不重读下载摘要。

In historical baselines `f1817f4` through `cf88455`, `open_core` hashes pending downloads before listening, deviating from decision 4 and potentially exceeding the managed Server's six-second readiness budget. The 2026-10-31 readiness slice moves full verification to HTTP workers after readiness: startup checks only bounded receipt bindings, while recovery claims scheduler slots and reserves destinations. Digest reads and pause/cancel waits do not hold the Server mutex. Reconciliation failures retain concrete errors and receipts without ordinary network retries; a receipt observed at startup cannot fall back to a GET if it subsequently disappears. Already completed tasks still perform identity-checked residual cleanup without rehashing downloads.

Engine 新增兼容入口 `reconcile_pending_completion`：沿用既有凭证格式与校验，要求凭证存在并返回精确完成进度；无论成功、失败或取消，都终结本次 control 并唤醒暂停等待者。原 `reconcile_completion` 保留“无凭证时返回 `None`，允许调用者继续下载”的语义。RPC、任务 Schema 与 v1 凭证均不改变。直接 Engine 测试、可控阻塞验证器的测试专用 managed 子进程，以及真实生产 Server 的 managed／persistence 消费测试分别覆盖控制边界、就绪／RPC 响应与离线恢复；生产二进制没有故障注入环境开关。

The Engine adds the compatible `reconcile_pending_completion` entry: it reuses receipt format and validation, requires an existing receipt and returns exact final progress, and finishes its control on success, failure or cancellation so pause waiters wake. Existing `reconcile_completion` keeps its optional `None` result, allowing callers without receipts to continue a download. RPC, task schema and v1 receipts remain unchanged. Direct Engine tests, a test-only managed subprocess with a controllably blocked verifier, and production Server managed/persistence consumer tests separately cover controls, readiness/RPC responsiveness and offline recovery. The production binary has no fault-injection environment switch.

此切片的最终交付仍要求当前 SHA 的完整门槛、独立审查及合并前后 CI；大文件原生 App 恢复和 Windows 行为仍需分别验收。后端进程测试不代表 M0/M1 原生验收完成，验证记录见[开发指南](../DEVELOPMENT.md#development-workflow-entry)。

Final delivery of this slice still requires complete gates, independent review and pre/post-merge CI at the current SHA. Native-App large-file recovery and Windows behavior need their own acceptance. Backend process tests do not complete native M0/M1 acceptance; see the [Development Guide](../DEVELOPMENT.md#development-workflow-entry) for records.

### 分段入口应用（实施方案） / Applying the protocol to segments (implementation design)

[RFC 0001](../RFC/0001-segment-completion-protocol.md) 记录将本 ADR 接到未启用分段原型的兼容入口。它保留 v1 凭证格式：所有 Worker 退出后准备整文件证明，先对账后发请求，发布与取消共享控制锁。分段没有创建续传 sidecar，故凭证不得包含其所有权；最终进度与 `Completed` 的数据库提交仍由未来调用者负责。本轮实际实现及验收结果见开发计划和对应 PR，方案不代表 Server 多连接已启用。

[RFC 0001](../RFC/0001-segment-completion-protocol.md) records a compatible entry applying this ADR to the disabled segment prototype. Preserve v1 receipts: prepare a whole-file proof after worker exit, reconcile before requests, and serialize publication/cancellation through the control lock. Segments do not create resume sidecars, so their receipts must not claim sidecar ownership. Future callers remain responsible for atomically persisting final progress and `Completed`. The development plan and corresponding PR record implementation/acceptance; the design does not enable Server segmentation.

<a id="verification-plan"></a>
<a id="验证计划"></a>

## 验证计划 / Verification plan

使用隔离临时存储和本地 HTTP fixture，覆盖发布前已写凭证、发布后尚未保存完成、保存完成后尚未清理、无 validator 且未知长度、持久化失败后重启、任务 ID 复用、hard-link 回退、取消、内容或身份变化，以及损坏／符号链接／特殊文件凭证。核对准确字节和持久化状态，确认恢复不发第二次 HTTP 请求。不新增无条件暴露的生产崩溃注入入口，不操作运行中 Desktop 的任务数据库。

Use isolated temporary storage and local HTTP fixtures. Cover receipt written before publication, publication before SQLite completion, completion persisted before cleanup, unknown lengths without validators, persistence failures and restart, task ID reuse, hard-link fallback, cancellation, changed content or identity, and invalid/symlink/special-file receipts. Assert exact file bytes and persisted state, and that recovery needs no second HTTP request. Do not add an unconditional production crash-injection entry point or touch the running Desktop's task database.

<a id="consequences"></a>
<a id="影响"></a>

## 影响 / Consequences

- 进程崩溃后可让完整文件与任务状态收敛，无需重复下载，也不认领无关文件。<br>
  Completed bytes and task state can converge after a process crash without redownloading or claiming an unrelated file.
- 准备凭证和待对账恢复需要完整读取一次本地文件摘要；不得拖慢 Server 就绪。<br>
  Full digest validation costs one local read during preparation and pending recovery. It must not delay Server readiness.
- 本决策解决进程崩溃窗口。硬件掉电保障仍取决于平台和文件系统的同步语义；凭证不是下载字节的备份。<br>
  This closes a process-crash window. Power-loss durability still depends on platform and filesystem synchronization semantics; the receipt is not a backup of downloaded bytes.
- M0 实际 VoiceOver 朗读以及 M1 全新用户／升级最终验收仍是独立门槛。本次后端变更不代表任一里程碑完成。<br>
  M0 VoiceOver speech and M1 final clean-user/upgrade acceptance remain separate gates. This backend change does not complete either milestone.

<a id="implemented-verification"></a>
<a id="已实现验证"></a>

## 已实现验证 / Implemented verification

10 月 11 日的 engine 测试覆盖凭证准备与发布、强制 hard-link 回退、恢复修改时间后的摘要变化、相同内容但不同文件身份、空响应、绑定错误、超大／损坏记录、符号链接／FIFO 保护，以及暂停／取消。Core 测试证明最终进度、状态和事件同时提交，保存被拒绝时全部保持不变。

On October 11, engine tests cover receipt preparation and publication, forced hard-link fallback, digest changes with restored modification time, different file identities with identical content, empty responses, invalid bindings, oversized/corrupt records, symlink/FIFO protection, and pause/cancel. Core tests prove final progress/state and events either commit together or remain unchanged after a rejected update.

Server 测试使用真实子进程和独立临时数据库。本地 HTTP fixture 提供下载字节，SQLite trigger 拒绝完成保存，另有测试同时拒绝两个终态；随后终止子进程、用同一数据目录重启，核对完整文件和持久化 `Completed`。其他 fixture 重建发布前和持久化后边界，在 HTTP listener 已关闭后恢复未知长度响应，保留替换文件并拒绝不安全的任务 ID 复用。后端证据见[发布流程](../RELEASE.md#http-完成对账后端证据)。本切片未重启原生 App；Windows 文件身份已实现，但未在本机 macOS 验证中实际执行。

Server tests use real child processes and independent temporary databases. A local HTTP fixture supplies bytes while SQLite triggers reject completion (and, separately, both terminal states); the test terminates and restarts the child with the same data directory, then verifies the complete file and durable `Completed` state. Other fixtures reconstruct pre-publication and post-persistence boundaries, recover unknown-length responses after the HTTP listener has closed, preserve replacement files, and reject unsafe task ID reuse. See the backend evidence in [Release Process](../RELEASE.md#http-completion-reconciliation-backend-evidence). No native App restart was part of this slice. Windows file identity is implemented but has not been executed in this macOS verification.

<a id="alternatives-considered"></a>
<a id="考虑过的替代方案"></a>

## 考虑过的替代方案 / Alternatives considered

- **按大小或摘要接受任何已有文件**：可能误认用户创建的替换文件，无法证明由本任务发布。<br>
  **Accept any existing file by size or digest:** can claim a user-created replacement and provides no proof of publication by this task.
- **先保存 `Completed` 再发布文件**：可能在最终文件不存在时报告成功。<br>
  **Save `Completed` before publication:** can report success while the final file is absent.
- **给每个持久化任务增加 HTTP 字段**：改变通用存储契约，也无法让文件系统和 SQLite 提交原子化。<br>
  **Put HTTP fields in every stored task:** changes the generic storage contract without making filesystem and SQLite commits atomic.
- **监听前校验所有凭证的文件摘要**：大文件拖慢托管启动，可能反复触发 supervisor 超时。<br>
  **Hash every receipt before listening:** delays managed startup for large downloads and can trigger repeated supervisor timeouts.
