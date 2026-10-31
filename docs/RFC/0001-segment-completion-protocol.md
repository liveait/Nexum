# RFC 0001：分段下载完成协议 / Segment completion protocol

- 状态：已实现；验证记录见开发计划，最终交付以对应 PR 的独立审查及合并前后 CI 为准。日期：2026-10-31；基线：`f518b43`。
  Status: implemented; see the Development Plan for verification, and the corresponding PR for independent review and pre/post-merge CI before final delivery. Date: 2026-10-31; baseline: `f518b43`.

## 摘要与动机 / Summary and motivation

把 [ADR 0007](../decisions/0007-http-completion-reconciliation.md) 的完成凭证及离线对账接到未启用的分段原型，补齐取消与发布之间的控制状态协调。现有 `download_to_segments` 保留；新增带 `CompletionContext` 的兼容入口，不改 Engine Adapter、RPC、任务 Schema 或 v1 凭证格式。此 RFC 记录新增公共 API 的合约，Server 仍不派发分段。

Apply ADR 0007 receipts and offline reconciliation to the disabled segment prototype, coordinating cancellation and publication state. Keep `download_to_segments` and add a compatible entry accepting `CompletionContext`, without changing the Engine Adapter, RPC, task schema or v1 receipt format. This RFC records the additional public API contract; Server segmentation remains disabled.

## 设计 / Design

1. 新增 `download_to_segments_with_completion`；与原入口共用校验及 Worker 路径。按任务、来源、目标及暂存路径绑定先尝试对账；已证明完成的文件不重新发 HTTP，也不创建或截断暂存文件；新入口无需重建分段计划／ETag 就能恢复。没有凭证时才校验新传输的段集合及强 ETag，且必须早于请求和创建文件；旧入口校验顺序不变。
   Add `download_to_segments_with_completion`, sharing validation/workers with the existing entry. Reconcile task/source/destination/staging bindings first; receipt-proven complete bytes require neither new HTTP requests nor staging creation/truncation, and recovery does not require reconstructing a segment plan/ETag. With no receipt, validate fresh segment coverage and strong ETag before requests or staging creation. Preserve the original entry’s validation order.
2. 所有 Worker join 后才进入完成阶段，复核持有文件的身份、长度并同步，读取完整摘要后原子写凭证。完成阶段仍须配合暂停／取消；可在全部 Worker 退出后明确移交给单协调者，也可让凭证辅助方法调用 coordinator checkpoint。不能直接用单 Worker 的暂停标志绕过仍活跃的分段参与者。
   Enter completion only after all workers join. Verify the held file identity/length and sync it, then hash the complete bytes and atomically write the receipt. Completion still cooperates with pause/cancel. Either explicitly hand control to one coordinator after worker exit or invoke coordinator checkpoints from receipt helpers; never bypass live participants with a single-worker pause flag.
3. 分段路径从未创建或拥有续传 sidecar，准备凭证必须显式忽略它；凭证 `resume_metadata` 为 `None`。既有单连接的 sidecar 策略不变。
   Segment attempts do not create or own resumable sidecars; receipt preparation must explicitly exclude them, with `resume_metadata=None`. Preserve single-connection sidecar behavior.
4. 发布控制锁决定取消或提交哪一个先成功。取消赢得锁时不发布；提交成功后不再误报取消。若在发布前暂停，恢复后的身份检查必须在最新控制决定之后、紧邻实际提交，不能复用暂停前的路径检查结果。保留不覆盖目标操作，成功的目标永不作为清理对象。原入口也使用相同的取消／提交协调。
   The publication control lock decides whether cancellation or commit wins. Cancellation winning the lock prevents publication; a successful commit cannot later become cancelled. If publication pauses, recheck identity after the latest control decision immediately before commit; do not reuse pre-pause path checks. Preserve no-clobber operations and never clean up the published destination. The original entry uses the same cancellation/commit coordination.
5. 不能把准备凭证返回错误等同于凭证不存在：写入后目录同步仍可能失败。凭证已落盘后的普通错误保留完整字节和凭证；取消尚未发布的尝试须清理本次凭证及自有暂存文件，清理成功才返回 `Cancelled`；清理失败返回包含具体原因的 `Failed`，明确取消清理尚未完成，并保留可诊断证据及 `control.is_cancelled()` 状态。调用者不得自动重试已请求取消的 control；删除／复用任务前须显式处理残留证据。未确认归属时保留文件。
   A preparation error does not prove receipt absence: directory sync may fail after receipt publication. Ordinary errors after a receipt lands preserve complete bytes and receipt. Cancelling an unpublished attempt must remove its own receipt and staging; return `Cancelled` only after successful cleanup. Cleanup failure returns a concrete `Failed` stating that cancellation cleanup is incomplete, retaining diagnostic evidence and `control.is_cancelled()`. Callers must not automatically retry a cancelled control, and must explicitly handle residual evidence before task deletion/reuse. Preserve files when ownership is uncertain.
6. 成功后凭证由调用者保留，最终进度和 `Completed` 原子持久化成功后再按既有清理 API 清除；本轮只验证 Engine 合约，不宣称 Server 多连接的数据库接线已完成。
   After success, retain the receipt until the caller atomically persists final progress and `Completed`, then use existing cleanup APIs. This slice accepts the Engine contract, not Server multi-connection persistence wiring.

## 兼容性、安全与边界 / Compatibility, safety and limits

保留旧 API、单连接路径、Range／validator 校验、持有句柄、全 Worker 屏障及无关 sidecar。控制锁原子性不等于文件系统身份绑定的原子性：路径检查到 rename／hard-link／unlink 的 TOCTOU 保持后续阻断项。同步文件操作和不返回的调用方回调无法强制终止。Server 启用、安全回退及段级恢复均不在本轮。

Preserve the old API, single-connection path, range/validator checks, held handles, all-worker barriers and unowned sidecars. Atomic control state is not atomic filesystem identity binding: check-to-rename/hard-link/unlink TOCTOU remains a later enablement blocker. Synchronous filesystem calls and non-returning caller callbacks cannot be forcibly terminated. Server enablement, safe fallback and segment recovery are outside this slice.

## 替代方案与验收 / Alternatives and acceptance

不直接把现有 prepare helper 无条件接入分段，因为它可能认领无关 sidecar；不使用一次性路径长度判断代替身份／摘要。直接 Engine 集成测试覆盖正常拼接、凭证、离线恢复、同内容不同身份、目标冲突、错误凭证、sidecar 保留和摘要阶段控制；真实 Server 进程另行消费新入口实际生成的凭证，在 HTTP fixture 结束后恢复精确完成状态及最终进度，并保留无关 sidecar；这不启用 Server 分段派发。私有可控故障覆盖发布锁竞争及凭证已写但目录同步失败。完整 Rust 门槛、固定 SHA 审查、最新 main/head 组合 CI 及实际合并 SHA 的 main push CI 必须通过。

Do not attach the existing prepare helper unconditionally, because it may claim an unowned sidecar. Length alone does not replace identity/digest proofs. Direct Engine integration tests cover assembly, receipts, offline recovery, same bytes with different identity, destination conflicts, invalid receipts, sidecar preservation and hash-stage controls. A real Server process also consumes receipts generated through the new entry after its HTTP fixture has stopped, verifying exact completion/progress and unowned-sidecar preservation without enabling segment dispatch. Private controlled faults cover publication-lock races and receipt publication followed by directory-sync failure. Require complete Rust gates, fixed-SHA review, latest-main/head combination CI and actual-merge-SHA main push CI.
