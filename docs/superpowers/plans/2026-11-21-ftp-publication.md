# FTP 发布与离线完成收敛实施计划 / FTP publication and offline completion implementation plan

> 执行代理：逐任务使用 subagent-driven-development；复选项记录实际完成情况，不把方案当作验收。 / Agentic workers: execute task by task with subagent-driven-development; checkboxes record actual completion, not design acceptance.

**目标 / Goal:** 已保存的 FTP 完成证明经不覆盖发布及跨进程离线恢复，最终由 Core 原子保存整文件进度和 Completed。

Persisted FTP completion evidence converges through no-clobber publication and offline process-restart recovery to atomic Core whole-file progress and Completed.

**架构 / Architecture:** 单一 directory-relative `linkat(..., 0)`，保留来源别名，不自动 unlink、rename、copy 或 clone。SQLite 独立持久 publication intent 分离原 producer 与当前 owner；最终命名空间操作重新持有授权预约，最终完成另做 CAS。

Use one directory-relative linkat algorithm retaining the staging alias. A separate durable SQLite intent distinguishes original producer/current owner; fresh namespace authorization and final completion CAS are separate boundaries.

**技术 / Tech stack:** Rust、既有 libc、serde、SHA256、rusqlite；不新增依赖。 / Rust with existing dependencies, without new dependencies.

**方案 / Spec:** [RFC 0004](../../RFC/0004-ftp-credentials-and-recovery.md#held-publication-convergence)；[已接受范围](../../RFC/0002-segment-file-ownership.md#ordinary-desktop-ftp-bt)。基线 / Base: `dbc479b6b52012812ef55f461a9d27c888aee9fc`。

## 全局约束 / Global Constraints

- 仅首发 macOS FTP 普通桌面范围；M4／旧 HTTP 的保证不改变，不增加管理员服务。残余检查到路径调用竞态明确保留。 / Only the accepted macOS FTP profile; preserve M4/HTTP boundaries and disclose residual check/use races.
- 不接 Server／CLI／Desktop、FTPS 或 BT 数据；不自动清理 staging alias、目标或 journal。 / No product dispatch, FTPS, BT payloads or automatic cleanup.
- completion 的原 producing generation 永不改写；当前 owner、随机 authorization、revision 与完整 Downloading StoredTask 必须在 namespace guard 和最终 Completed CAS 中同时匹配。 / Preserve the producer; bind current owner/authorization/revision/full Downloading task at both fences.
- proof 与 plan 各最多 64 KiB；plan 独立格式 `ftp-publication/v1`，拒绝未知／额外／缺失字段、超限、错误来源／身份／叶名称。opaque 类型不公开 FileIdentity，不提供公开成功证据反序列化，Debug 不泄露私有路径／证明。 / Bound opaque versioned records; validate strictly and redact Debug.
- 来源与最终名称在同一持有父目录，仅允许单叶名称；binding.destination 必须是实际绝对、规范化目标，父目录对象须匹配。现有项包括断链、hard link、目录均不得覆盖。 / Bind the real absolute destination, held parent and strict leaf names; preserve every existing destination kind.
- hash／网络不在最终 Core／SQLite 锁内。锁序为 control → SQLite Immediate reservation → 有限 namespace syscall；显式 release 错误不可报成功，SQLite 250ms busy timeout 不代表文件系统延迟上限。 / Keep hashing/network outside final locks; preserve lock order and report release errors.
- syscall 成功立即保留 committed 事实；后续取消、sync、release 或数据库失败不反向删除／改名。所有出口终结 control 并唤醒等待者；取消先赢则不发布。 / Preserve committed facts and finish controls on every path.
- schema 6 新增窄表及 observed-intent marker，原数据完整保留；未来 schema 拒绝，迁移及写入故障完整回滚。只检测 marker／已观察快照丢失，不声称任意数据库历史防篡改。 / Add a narrow table/marker atomically, without generic tamper-detection claims.
- Completed／orphan journal 是独立只读诊断视图，不通过 active-attempt API 读取，不可授权 I/O；任务移除后保留旧 incarnation，重复 ID 不可采用。 / Separate non-executable diagnostics from active authorization.
- 保留所有基线测试；完整 Rust 门槛、真实消费者、固定 SHA 独立审查及前后 CI 必须通过。 / Retain all baseline tests and complete all gates.

## 审查焦点 / Review Focus

1. 非恶意冲突／相同字节替换必须在可检测点失败保留；不冒称消除残余竞态。Task 1 覆盖。 / Detect/preserve conflicts and identity substitutions without overstating race guarantees.
2. generation 未变但任务变为 Queued／Paused／Failed 或其他字段变化，旧授权不得发布。Task 2/3 覆盖。 / Fence full task changes even with unchanged generation.
3. 重新领取旧 proof 不自动重新授权；marker 或 intent 丢失／格式未知须可诊断。Task 2 覆盖。 / Reclaim does not rearm; reject missing/unknown observed evidence.
4. 数据库 commit 后不得再经会失败的 active-attempt decode 阻止内存交换。Task 3 覆盖。 / Prepare fallible results before commit and publish memory only after commit.
5. Failed＋pending intent 重启仍可离线完成，不经通用 claim 清零，不产生额外 RETR。Task 3 覆盖。 / Preserve progress and pending recovery across restart without another RETR.

## Task 1: Engine 同身份发布与离线核验 / Engine same-identity publication and offline verification

**文件 / Files:** 新建 `crates/engine/src/ftp/publication.rs`、`publication_tests.rs`、`crates/engine/tests/ftp_publication.rs`；修改 `ftp/completion.rs`、`ftp.rs`。publication 是 completion 的私有子模块，窄 re-export 避免公开证明内部字段。 / Private child module with narrow exports.

**接口 / Interfaces:** 产出 / Produce `FtpPublicationPlan`、`FtpPublicationRequest<'a>`、`FtpPublishedCompletion`、`FtpPublicationError`；`publish_completion_with_guard`、`reconcile_publication_with_guard` 复用 `FtpCompletionGuard` 的 fallible release。plan 提供 `prepare`、bounded `encode/decode` 及 binding／producer／owner／authorization／progress 只读访问器；published 提供 plan／progress 只读访问器，禁止外部构造。精确 Rust 参数可在实现时按借用规则收敛，语义不可省略。 / Exact signatures may be consolidated for borrowing while preserving these boundaries.

- [ ] 写直接测试，先记录缺少新行为的失败；不存在的 API 编译错误不计作基线行为缺陷。 / Write failing behavior tests; missing-API compilation is not a baseline behavior regression.
- [ ] 实现 bounded plan：消费独立 `FtpCompletionRequest`、原 proof、held file/parent、source leaf、current owner/authorization；从 binding 校验真实 final leaf/parent。`same_inode_no_clobber_hardlink_retains_staging` 核对两个名称同 inode、完整哈希和 no unlink。 / Bind/validate independently supplied proof and destination.
- [ ] 覆盖 `existing_regular_symlink_hardlink_and_directory_are_preserved`、`partial_or_parent_identity_mismatch_is_rejected`、`equal_bytes_replacement_and_corruption_are_retained`、`unknown_or_malformed_publication_plan_is_rejected`、`source_and_final_leaf_names_are_strictly_bound`。 / Test real filesystem results.
- [ ] 增加新 control 的 committed 观测和最终决定边界；不复用已经结束的 receive/verify control。hash 分块可暂停／取消；最后 control 锁内取得新 guard，重查身份后 linkat，立即记录 committed。release 后做必要目录 sync；控制运行保持至全部 I/O 返回，sync 失败保留 committed 事实。reconcile 已存在 final 经验证后同样表示已提交事实。 / Finish only after all I/O; committed errors never undo publication.
- [ ] 用私有 barrier/fault injection 覆盖 `cancel_winning_before_syscall_prevents_publication`、`publication_winning_before_cancel_remains_committed`、`pause_and_cancel_during_hash_preserve_every_entry`、`release_validation_and_sync_failures_preserve_committed_intent`、`finished_controls_are_not_reused`；生产无环境／fixture URL 开关。 / Deterministic tests without production test backdoors.
- [ ] 运行新 integration、completion／FTP 既有测试及 engine all-targets check/Clippy；保存实际 red/green 日志，提交并独立任务审查。 / Run targeted gates, retain logs, commit and review.

## Task 2: Storage schema 6 与发布授权 / Storage schema 6 and publication authority

**文件 / Files:** 新建 `crates/storage/src/ftp_publication.rs`、`crates/storage/tests/ftp_publication.rs`；修改 `ftp_resume.rs`、`origin.rs`、`lib.rs`。 / Add narrow publication persistence.

**接口 / Interfaces:** 消费 Task 1 opaque plan bytes；产出 `FtpPublicationIntent`、只读 completed/orphan record、prepare/rearm、专用 claim、`guard_ftp_publication`、`complete_ftp_publication`。Storage 不依赖 Engine；Core 负责 typed 解码匹配。 / Store bounded opaque bytes without an Engine dependency.

- [ ] schema 6 新表 `ftp_publications` 以 instance/operation 为键，保存 version、task、authorization、owner、attempt revision、prepared/completed phase、opaque plan；`ftp_attempts` 增加 publication_seen/current-intent 绑定用于 observed loss。迁移拒绝未来版本，中途 ALTER／CREATE 及最终 version 更新故障均完整回滚。 / Persist incarnation-scoped authority and observed-loss markers atomically.
- [ ] FtpAttempt 完整快照包含 intent；claim 新 generation 保留旧 owner/authorization，仅显式 prepare/rearm 才写新授权和 revision。所有旧 claim/save/guard 比较完整快照，不允许前缀或第二完成证明越过已存在 intent。 / Preserve complete snapshot CAS.
- [ ] 在一个 Immediate 事务实现保进度的恢复 claim；prepared guard 与最终完成同时核对完整预期 StoredTask、Downloading、来源、completion、attempt、intent 和 current owner。namespace guard 保持至 Engine 决定后显式 release；不在 guard 做 hash／网络。 / Bind the full task and all authority fields.
- [ ] 最终事务一次保存 whole progress、Completed 和 completed intent；返回值在 commit 前完成可失败验证，completed/orphan 只读接口不经 active-origin 检查。移除保留旧 journal，禁止同 ID 新 operation 采用。 / Atomically complete and retain non-executable diagnostics.
- [ ] 覆盖 migration rollback、prepare/rearm/CAS、旧 owner、任务状态／字段改变而 generation 不变、writer busy／release 失败、missing marker/intent、Completed／orphan、同 ID 重建和最终写入失败重试；不弱化旧 schema 初始化／迁移断言。 / Direct real SQLite failure/race tests.
- [ ] 运行 storage 全套测试和 all-targets check/Clippy，提交并独立任务审查。 / Verify, commit and review.

## Task 3: Core 恢复领取、完成 CAS 与真实跨进程消费者 / Core convergence and real process consumers

**文件 / Files:** 新建 `crates/core/src/ftp_publication.rs`、`crates/core/tests/ftp_publication.rs`；修改 `core/lib.rs`、`ftp_resume.rs`、相关 RFC／开发计划。仅窄 `pub(super)` 桥接 stored/typed，禁止公开可伪造的字段。 / Keep module bridges narrow.

**接口 / Interfaces:** `claim_queued_ftp_completion`、`prepare_ftp_publication`、`publish_ftp_completion`／`reconcile_ftp_publication`、`complete_ftp_publication`。接口分离持久 prepare、实际 namespace guard、最终 Completed CAS；不得拿旧 verify 返回 Progress 当未来许可。 / Keep three distinct authority boundaries.

- [ ] 专用 claim 克隆候选 tasks/scheduler，保留已证明整文件进度，Storage 原子领取成功后才交换任务／槽位／事件。失败不改变内存、事件、SQLite 或文件。 / Publish candidate state only after durable success.
- [ ] 为 Failed＋pending intent 使用窄恢复 selector／既有 recover-with-pending hook，保留 HTTP 行为，不扩大普通 claim。Completed 结果从独立诊断视图读取；commit 后不调用 active typed decode。 / Preserve failed pending recovery and completed diagnostics.
- [ ] prepare/rearm 校验 Engine plan 与独立 origin/producer/current owner；实际发布阶段给新 Storage guard，保留真实 StorageError；最终 CAS 只接受对应 opaque published evidence。 / Refence each boundary.
- [ ] `recovery_claim_preserves_whole_progress_scheduler_and_events`、`intent_and_completion_write_faults_preserve_memory_and_events`、`stale_owner_cannot_publish_or_save_completed`、`atomic_completed_releases_slot_only_after_sqlite_commit`、`recreated_task_id_cannot_adopt_prior_intent` 直接调用新 Core 入口。 / Exercise actual consumers.
- [ ] `real_ftp_publication_converges_offline_after_process_crash`：实际绝对目标／held parent，公开来源和调度入口→真实 FTP→proof save→intent→publish→Completed；私有 pipe 屏障在 intent 后／link 前后／SQLite 拒写后 SIGKILL，独立进程在源站关闭后 recover/claim/rearm/reconcile/complete。核对哈希、原身份、全进度、终态、槽位／事件、无额外 RETR、同 inode 双名称及现有目标／无关哨兵。 / Test actual process boundaries and offline convergence.
- [ ] 全量 fmt、workspace all-targets check、workspace serial test、独立默认并行 managed、all-targets Clippy `-D warnings`；TRACE 检查凭据／URI/passkey 哨兵。新 API 还由独立仓库外消费者验证。 / Run complete gates and independent consumers.
- [ ] 固定最终 SHA 独立全分支审查，核对最新 base/head CI；按已有授权合并并验证实际 main push CI。只在全通过后更新交付状态和同步 checkout，保留未验原生／其他平台／分发缺口。 / Review and deliver with actual merged-main evidence.
