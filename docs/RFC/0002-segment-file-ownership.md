# RFC 0002：分段文件发布与清理的身份边界 / Identity boundaries for segment publication and cleanup

- 状态：提案；平台调查与实验不代表发布机制已通过。初稿：2026-10-31，基线 `cf884557140a252f20b87dea71794b23bf07b6f4`；本轮核对：2026-11-21，基线 `5a9bc7de2166a2117f55e2f10784d4e4cacbb03f`。安全实现、行为验收及下述架构／范围选择仍待完成。
  Status: proposed; platform research and experiments do not accept a publication mechanism. Initial draft: 2026-10-31 at `cf884557140a252f20b87dea71794b23bf07b6f4`; current audit: 2026-11-21 at `5a9bc7de2166a2117f55e2f10784d4e4cacbb03f`. Safe implementation, behavioral acceptance and the architecture/scope choice below remain open.

## 摘要与范围 / Summary and scope

[RFC 0001](0001-segment-completion-protocol.md) 已协调分段取消与发布的控制状态，但未关闭文件身份检查到路径操作之间的 TOCTOU。本 RFC 明确该阻断项的威胁模型、平台候选机制和实施门槛。**本提案不以更多路径检查、随机名称或 `0700` 目录宣称原子所有权；未找到同时满足 macOS、现有 v1 凭证及安全清理要求的可直接替换方案，多连接继续禁用。**

[RFC 0001](0001-segment-completion-protocol.md) coordinates segment cancellation and publication controls without closing identity-check-to-path-operation TOCTOU. This RFC defines the threat model, platform candidates and implementation gates for that blocker. **Additional path checks, random names or `0700` directories do not establish atomic ownership. No drop-in mechanism has yet been established that satisfies macOS, existing v1 receipts and safe cleanup together; segmentation stays disabled.**

本轮只新增 RFC，不改变下载、RPC、任务 Schema、凭证格式或 Server 路由。后续实施必须保留 [ADR 0003](../decisions/0003-http-transfer-controls.md)、[ADR 0004](../decisions/0004-cross-restart-http-resume.md) 和 [ADR 0007](../decisions/0007-http-completion-reconciliation.md) 的控制、续传、完整性及完成恢复约定。单连接同样调用部分共享完成 helper；本 RFC 的调查不等于单连接具备对抗目录替换的保证，也不在本轮扩大修改该产品路径。

This slice adds an RFC only, changing no downloads, RPC, task schema, receipt format or Server routing. Future implementation must preserve the control, resume, integrity and completion-recovery contracts in ADRs 0003, 0004 and 0007. Single-connection transfers also call some shared completion helpers; this investigation does not establish resistance to directory replacement for that path, nor expand this slice into modifying it.

## 威胁模型与所需保证 / Threat model and required guarantees

参与者可在任意两次系统调用之间移动、删除或替换下载目录项，包括暂存名称、最终名称和祖先目录；可使用普通文件、符号链接及 hard link，不要求与 Nexum 协作。参与者可能与 App 同 UID，因此随机名称、应用私有目录权限和进程内 Mutex 都不能被当作该参与者无法写入的证据。凭证目录若也在此参与者可写的范围内，同样需要所有权协议，不能用“私有”一词排除它。

An actor may move, remove or replace download directory entries between any two system calls, including staging names, final names and ancestors, using regular files, symlinks or hard links without cooperating with Nexum. The actor may share the App's UID: unpredictable names, application-directory permissions and process-local mutexes do not prove exclusion. If the actor can also mutate the receipt directory, its entries need an ownership protocol too; calling it private does not exclude it.

本 RFC 限定为目录项替换。任意修改已打开 inode 的内容、修改进程内存或控制内核的对手不在本次保证内；这不是声称同 UID 进程天然不能做这些事。字节内容仍须按现有长度、校验器和 SHA-256 规则检查。

This RFC bounds the guarantee to directory-entry replacement. Arbitrary mutation of an open inode's contents, process-memory modification and kernel control are outside this guarantee; this does not claim that same-UID processes inherently lack those powers. Existing length, validator and SHA-256 checks still apply to bytes.

未来机制须满足下列条件；任何一项不成立时，不启用该平台／文件系统上的安全分段路径：

A future mechanism must satisfy these conditions; otherwise safe segmentation remains unavailable on that platform/filesystem:

1. **来源绑定与目标保护：** 提交必须发布已持有、已证明完整的文件对象，不能重新按可替换的来源名称选择对象；目标已存在时失败，不覆盖任何类型的目录项。发布后的再次检查只能发现错误，不能撤销已经发布的无关文件。
   **Source binding and destination protection:** publication must use the held, proven-complete object rather than select it again through a replaceable source name. An existing destination entry of any type causes failure. Post-publication checks detect errors without undoing publication of an unrelated object.
2. **安全清理：** 清理只能作用于拥有的对象／名称绑定。不能以 `stat/open → unlink(name)` 的再次检查代替原子条件删除，也不能把“移入隔离目录后验证”当作无副作用检查；移动本身已作用于可能无关的目录项。没有安全清理原语时保留证据并返回具体失败，不猜测删除、不误报取消完成。
   **Safe cleanup:** cleanup may affect only an owned object/name binding. Repeated `stat/open → unlink(name)` is not atomic conditional deletion; moving into quarantine before verification already mutates a potentially unrelated entry. Without a safe primitive, retain evidence and return a concrete failure instead of guessing or reporting completed cancellation.
3. **目录绑定：** 使用经验证并持有的目标父目录对象及单个叶名称，避免祖先路径被替换后操作无关目录。目录 fd 本身不会锁住其中的叶项，也不保证该目录一直位于原路径；若产品要求原始目标字符串持续指向该目录，需要额外机制，不能由路径复查推导原子保证。
   **Directory binding:** use a verified, held destination-parent object and one leaf name, avoiding operations in unrelated directories after ancestor replacement. A directory fd does not lock its leaf entries or keep the directory at its original path. A requirement that the original destination string continually identify that directory needs an additional mechanism, not an atomicity claim inferred from rechecks.
4. **恢复与控制：** 发布前持久化足够的完成证明，崩溃后拒绝同内容但不同身份的替换文件；发布和取消仍由同一控制决定序列化。最终进度和 `Completed` 原子保存后才清理凭证；失败、取消和任务 ID 复用不得绕过残留证据处理。
   **Recovery and controls:** persist sufficient proof before publication and reject identical-content replacement files with different identities after a crash. Keep publication/cancellation serialized by the same control decision. Clean receipts only after atomic final-progress/`Completed` persistence; failures, cancellation and task-ID reuse must handle residual evidence explicitly.

这些保证以发布系统调用的线性化时刻为边界。对手在成功发布之后再删除或替换最终名称，不可能由一次发布调用永久阻止；恢复必须拒绝被替换文件，而不是据此清理它。若无法证明目标名称绑定，保留文件与凭证并报告冲突。

The publication syscall's linearization point bounds these guarantees. One publication call cannot permanently prevent later removal or replacement by the actor. Recovery must reject replacements rather than remove them. When the destination binding cannot be established, retain files and proof and report a conflict.

## 当前调用链与 v1 凭证约束 / Current call paths and v1 receipt constraints

- `SegmentStaging::publish` 在持有句柄上检查／同步，但仍传路径给 `commit_download`；`cleanup` 先检查身份再 `remove_file`。`completion::publish_completion` 在控制锁内重新打开／核对来源，再调用同一路径提交；`cleanup_completion_partial` 与分段取消的凭证清理也存在检查后按路径删除。控制锁不约束外部目录修改者。
  `SegmentStaging::publish` validates/syncs the held handle but passes paths to `commit_download`; `cleanup` checks identity before `remove_file`. `completion::publish_completion` reopens/checks the source under the control lock before the same path-based commit. `cleanup_completion_partial` and segment-cancellation receipt cleanup also check before path-based removal. The control lock does not constrain external directory writers.
- `CompletionReceipt` v1 只有一份 `file` 证明，绑定来源／目标／暂存路径、身份、长度、修改时间和 SHA-256。Unix 身份包含设备／inode／可选创建时间；Windows 包含卷序列号／文件索引／创建时间。rename 或 hard link 可保留对象身份；clone/copy 产生的新对象不能自动沿用原证明。现有 `snapshot` 相等要求和拒绝同内容不同身份的测试不得削弱。
  v1 `CompletionReceipt` contains one `file` proof binding source/destination/staging paths, identity, length, modification time and SHA-256. Unix identities include device/inode/optional creation time; Windows uses volume serial/file index/creation time. Rename/hard links can retain object identity; a clone/copy cannot automatically reuse that proof. Preserve snapshot equality and tests rejecting equal bytes with a different identity.
- 当前凭证只记录路径，没有目标父目录对象身份或“来源对象→新发布对象”的证明转换。新增此类语义须另行定义版本兼容与崩溃协议，不能在 v1 内静默解释。
  Receipts currently contain paths without destination-parent identity or a proof transition from a source object to a distinct published object. Such semantics require an explicit versioning/crash protocol rather than silent reinterpretation of v1.

## 平台机制调查 / Platform mechanism research

以下是官方 API 文档支持的能力及本项目推导的约束，不是所有内核／文件系统均已执行的结论。来源于 2026-10-31 核对。

The following separates documented capabilities from project-specific deductions. It is not execution evidence for every kernel/filesystem. Sources were checked on 2026-10-31.

| 平台 / Platform | 候选与事实 / Candidate and fact | 项目约束与结论 / Project constraints and conclusion |
| --- | --- | --- |
| macOS | `renamex_np(RENAME_EXCL)` 和 `renameatx_np` 仍按来源名称解析；后者只锚定目录。`fclonefileat(srcfd, dstdirfd, leaf, flags)` 按来源 fd 克隆，拒绝已有目标，文档描述为全有或全无；克隆后的写入相互独立。[Apple rename](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/rename.2)、[Apple clonefile](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/clonefile.2)。<br>Rename APIs still resolve a source name; directory-relative rename only anchors directories. `fclonefileat` takes a source fd and exclusively creates an all-or-nothing clone with independent later writes. | fd clone 是来源绑定候选，但本机实验确认新 inode，与 v1 证明不相等；发布后才记录新身份会重新打开发布／保存崩溃窗口。所调查的 `unlink/unlinkat` 没有“仅当叶项身份等于持有文件才删除”的参数。[Apple unlink](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/unlink.2)。因此不能直接替换现有实现。<br>Local cloning produces a new inode, incompatible with v1 proof equality. Journaling that new identity after publication reopens the crash window. Surveyed unlink APIs offer no expected-file-identity condition. This is not a drop-in replacement. |
| Linux | `linkat(fd, "", dirfd, leaf, AT_EMPTY_PATH)` 可按 fd 创建同 inode 的链接，但要求 `CAP_DAC_READ_SEARCH`；文档还给出 procfs `/proc/self/fd/N` 配合 `AT_SYMLINK_FOLLOW` 的替代方式。`O_TMPFILE` 可创建无名文件，需文件系统支持，不能带阻止后续链接的 `O_EXCL`。[Linux linkat](https://man7.org/linux/man-pages/man2/link.2.html)、[Linux open](https://man7.org/linux/man-pages/man2/open.2.html)。<br>fd linking can retain identity; `AT_EMPTY_PATH` needs the documented capability, with a procfs alternative. Supported filesystems can provide anonymous `O_TMPFILE` staging without `O_EXCL`. | 同 inode 发布有候选，不能假设普通用户具备 capability 或 procfs。无名暂存关闭后消失，v1 的稳定路径无法恢复；保留有名别名则仍需安全清理协议。`unlinkat` 没有按文件 fd 删除特定名称绑定的条件。[Linux unlinkat](https://man7.org/linux/man-pages/man2/unlink.2.html)。本轮未在 Linux 执行，不能启用或回退到路径 rename/link。<br>Do not assume capability/procfs availability. Anonymous staging disappears after close and lacks v1's recovery path; named aliases retain the cleanup problem. `unlinkat` has no held-file conditional removal. Linux execution remains unverified; do not enable it or fall back to path-based publication. |
| Windows | `SetFileInformationByHandle` 接受 `FileRenameInfo` 和 `FileDispositionInfo`；重命名可设置 `ReplaceIfExists=false`，删除需 `DELETE` 访问。`CreateFileW` 的共享删除标志影响后续重命名／删除打开。[Microsoft handle API](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)、[rename info](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)、[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)。<br>Handle rename/disposition are candidates; rename can reject existing targets, deletion requires `DELETE`, and sharing flags govern later delete/rename opens. | 应调查自创建起持有正确访问／共享模式的句柄，不再使用 `MoveFileExW(source_path, ...)`。还须原生验证目标目录句柄、reparse point、hard-link 名称语义及关闭时删除；不能把按句柄处置当作按路径删除的完全等价替代。当前 Windows 分段与偏移写入均无本轮执行证据。<br>Investigate correct access/sharing from initial creation instead of path-based `MoveFileExW`. Native validation must cover destination-directory handles, reparse points, hard-link name semantics and delete-on-close. Handle disposition is not automatically equivalent to deleting a specified path. No Windows segment/offset-write execution is claimed here. |

## 决策、兼容性与替代方案 / Decision, compatibility and alternatives

2026-11-19 补充核对 Apple DTS 的 2026 年 10 月回复：当前缺少从已打开 fd 创建同对象 hard link、并阻止同用户来源名称替换的受支持 API；后续回复也确认 rename 具有该来源选择问题。该证据继续支持当前发布依赖状态。持有 fd 的文件内容修改可独立验证，FTP 尾部修复按 RFC 0004 推进。 [Apple DTS](https://developer.apple.com/forums/thread/850377)

On 2026-11-19, review of Apple DTS's October 2026 replies confirms the lack of a supported API for creating a same-object hard link from an open fd while preventing same-user source-name substitution. A follow-up confirms the source-selection issue for rename too. This evidence supports the current publication dependency status. Held-fd content changes can be verified independently, with FTP tail repair proceeding under RFC 0004. [Apple DTS](https://developer.apple.com/forums/thread/850377)

本 RFC 选择**先证明平台机制，再引入内部所有权抽象**。后续抽象须将持有的暂存对象、目标父目录对象、叶名称和完成证明作为一个尝试的输入，明确区分“已发布”“因不支持／冲突而未发布”“清理未完成”；保留控制锁的提交结果。缺少任一保证时失败并保留证据，不降级到 `check → rename/link/unlink`。本轮不新增公共 API 或对外能力声明。

This RFC chooses **platform proof before an internal ownership abstraction**. A future abstraction must take the held staging object, destination-parent object, leaf name and proof as one attempt, distinguishing published, unsupported/conflicted-before-publication and incomplete-cleanup outcomes while retaining the control lock's commit result. Missing guarantees fail with evidence retained; never downgrade to `check → rename/link/unlink`. This slice adds no public API or capability claim.

macOS 仍是阻断路径。下一实施切片先做 fd clone 的崩溃／身份协议原型，或找到可保持同对象身份的受支持机制；若需要受保护的来源命名空间，须证明内核强制的安全边界（例如不同安全主体的受限 broker），不能只凭同 UID 的目录权限。v2 凭证不是现成解法：必须解释系统调用已创建新对象但进程尚未获得／持久化其身份时，如何拒绝替换文件并恢复。没有该证明前，不迁移 v1 或启用分段。

macOS remains the blocking path. The next implementation slice must first prototype fd-clone crash/identity semantics or establish a supported mechanism retaining the same object identity. A protected source namespace requires an enforced security boundary, such as a restricted broker under a distinct security principal, rather than same-UID directory permissions alone. v2 receipts are not a ready-made solution: they must explain recovery and replacement rejection if the syscall created a new object before the process obtained/persisted its identity. Do not migrate v1 or enable segmentation without that proof.

以下替代方案不满足此威胁模型：增加身份检查次数；把来源改成随机名称；只加 `0700`；依赖 advisory lock；先交换／移走条目再校验；发布后发现错误再删除目标；用大小／摘要匹配替代文件身份。`*at` 与禁跟随符号链接可保护路径解析，但不能单独把最后叶项的比较与使用变成一个条件操作。保留这些防御的现有价值，不扩大其保证。

Rejected substitutes are extra identity checks, random source names, `0700` alone, advisory locks, swapping/moving entries before validation, deleting a destination after a failed post-check, and treating matching size/digest as ownership. Directory-relative/no-symlink APIs help constrain resolution but alone do not make the final leaf's comparison and use one conditional operation. Retain existing defenses without overstating them.

## 实施与验收门槛 / Implementation and acceptance gates

1. **平台证明切片：** 在私有临时目录用原生 API 和显式 channel/barrier，在最后一次检查后、系统调用前替换普通文件／符号链接／hard link，并覆盖祖先替换。证明来源 fd 与已选择父目录绑定、不覆盖目标及错误时无无关项删除；不使用固定 sleep 猜测竞态。对不支持的卷、权限和 API，验证具体失败且无危险回退。
   **Platform proof:** use native APIs, private temporary directories and explicit channels/barriers to replace regular/symlink/hard-link entries after the last check but before the syscall, including ancestor replacement. Prove fd source/selected-parent binding, no-clobber behavior and no unrelated deletion on errors. Do not infer races from fixed sleeps. Unsupported volumes, permissions and APIs must fail concretely without unsafe fallback.
2. **完成证明切片：** 如机制改变身份，先独立定义并审查新凭证版本、旧记录读取、任务 ID 复用和回滚兼容。用真实进程终止覆盖发布调用前、调用已生效但未记录返回身份、数据库保存前后、取消及清理失败。离线恢复不得认领相同字节的新身份；凭证和无关 sidecar 保持可诊断。
   **Completion proof:** if identity changes, separately define/review receipt versioning, legacy reads, task-ID reuse and rollback compatibility first. Terminate real processes before publication, after the syscall takes effect but before returned identity is journaled, around database persistence, and during cancellation/cleanup failures. Offline recovery must reject equal-byte replacements and retain diagnostic receipts/unowned sidecars.
3. **消费者切片：** 机制与证明都通过后再接 Engine 分段、Server 和原生 App。回归全部 Range／validator、Worker 屏障、暂停／恢复、取消／发布、完成保存、冲突和跨重启场景；各声明平台分别原生执行。每个实现切片运行开发指南完整 Rust 门槛（包括 Server unit、managed、persistence），固定 SHA 独立审查和合并前后 CI；原生验收不能由 helper 测试替代。
   **Consumers:** integrate Engine segments, Server and the native App only after mechanism/proof acceptance. Regress range/validator checks, worker barriers, pause/resume, cancellation/publication, final persistence, conflicts and restart behavior; execute natively on every claimed platform. Each implementation slice needs the Development Guide's complete Rust gates, including Server unit/managed/persistence, fixed-SHA review and CI before/after merge. Helper tests cannot replace native acceptance.

## 本轮实验与验证边界 / Local experiments and verification limits

2026-10-31，在 macOS 27.2（26B123）、arm64 上，用 `python3` 的 `tempfile.TemporaryDirectory(prefix="nexum-rfc0002-")` 隔离数据，`ctypes.CDLL(None, use_errno=True)` 直接调用系统 API，依次执行以下确定性操作；每步均断言精确字节和文件身份，不启动 Server、不接触实际下载目录。实验由顺序排列精确构造两次调用之间的对手操作，**不是产品完整执行路径的回归测试**。

On 2026-10-31, macOS 27.2 (26B123), arm64, isolated `python3` experiments used `tempfile.TemporaryDirectory(prefix="nexum-rfc0002-")` and `ctypes.CDLL(None, use_errno=True)` to call system APIs. Each step asserted exact bytes and identities without starting a Server or touching real downloads. Sequential steps precisely constructed actor operations between calls; **these are not full product-path regression tests**.

| 实验步骤 / Experiment | 实际结果 / Observed result |
| --- | --- |
| 持有原文件 fd → 核对路径 inode → 将原文件移到另一名称 → 原路径写入替换文件 → `renamex_np(..., RENAME_EXCL)`。<br>Hold original fd → check named inode → move original aside → create replacement at source → exclusive rename. | 成功发布替换文件；原 fd 仍指向保存的原文件。证实目标不覆盖不等于来源身份绑定。<br>The replacement was published while the held original remained intact. No-clobber does not bind source identity. |
| 持有原文件 fd → 核对路径 inode → 将原文件移走 → 原路径写入替换文件 → `unlink`。<br>Hold/check original → move it aside → create replacement → unlink checked name. | 替换文件被删除，原文件仍在移走位置。证实检查后删除的窗口。<br>The replacement was removed; the original remained at its displaced name, demonstrating check/unlink TOCTOU. |
| 持有原文件 fd → 替换来源路径 → `fclonefileat(fd, held_parent_fd, leaf, 0)` → 对同一目标再调用。<br>Hold original fd → replace source name → fd clone into held parent → repeat at the same leaf. | 第一次克隆为原 fd 的完整字节，inode 不同；第二次返回 `EEXIST` 且保留目标。证实来源绑定／不覆盖候选及 v1 身份不兼容。<br>The first clone contained original bytes with a distinct inode; the second returned `EEXIST` and preserved it. This supports source binding/no-clobber while exposing v1 incompatibility. |

本轮仅文档变更，执行 diff／本地链接检查；不因上述实验宣称产品修复。未执行 Linux／Windows 机制、完整 Rust 门槛、原生 App、崩溃恢复或合并后 CI；这些仍是对应实现的验收条件。M4 未完成。

This documentation-only slice checks the diff and local links; the experiments do not establish a product fix. Linux/Windows primitives, full Rust gates, native App behavior, crash recovery and post-merge CI were not executed here and remain implementation acceptance requirements. M4 is incomplete.

## 2026-11-21：fd clone 进程终止实验 / fd-clone process-termination probe

基线 `5a9bc7de2166a2117f55e2f10784d4e4cacbb03f`；macOS 27.2.1（26B131）、arm64、APFS。在独立临时目录直接调用 `fclonefileat`，以 pipe 屏障控制十个子进程场景，并在 clone 返回、尚未打开／读取／记录目标身份时发送 `SIGKILL`；另启动两个恢复进程读取已持久化的模型记录。脚本执行退出码 0，十个场景断言通过，stderr 为空，所有自建子进程均已回收。它是仓库外的平台实验，不调用 Nexum Engine／Core／Server，不是真实 FTP 完成证明或产品恢复链验收。

Base: `5a9bc7de2166a2117f55e2f10784d4e4cacbb03f`; macOS 27.2.1 (26B131), arm64, APFS. Call `fclonefileat` directly in isolated temporary directories, using pipe barriers for ten child-process scenarios. Send `SIGKILL` after clone returns but before opening/reading/recording destination identity; two separate recovery processes read the persisted model record. The script exits 0 with ten passing scenario assertions, empty stderr and all owned children reaped. This external platform probe does not call Nexum Engine/Core/Server and does not accept actual FTP completion proofs or product recovery.

| 场景 / Scenario | 实际结果 / Observed result |
| --- | --- |
| 来源名称分别被普通文件、符号链接和 hard link 替换，三个场景。<br>Replace source names with regular files, symlinks and hard links, three cases. | clone 均取原持有 fd 的字节，生成不同 inode；替换项与无关哨兵保留。<br>Each clone uses the held original's bytes and creates a different inode, preserving replacement entries and unrelated sentinels. |
| 目标已为普通文件、符号链接、hard link 和目录，四个场景。<br>Existing regular-file, symlink, hard-link and directory destinations, four cases. | 均返回 `EEXIST`（17），保留目标和无关项。<br>Each returns `EEXIST` (17), preserving the destination and unrelated entries. |
| 目标父目录改名后原路径放入替换项，一个场景。<br>Rename the destination parent and replace its old pathname, one case. | 仅向持有的父目录对象克隆，不操作替换项；不证明原目标字符串仍指向该目录。<br>Clone only into the held parent without touching the replacement; this does not preserve the original pathname's binding. |
| probe 在系统调用前收到取消，一个场景。<br>Cancel the probe before the syscall, one case. | 不调用 clone、不创建目标、不删除文件；不代表 Nexum 控制锁验收。<br>No clone, destination creation or deletion; this is not Nexum control-lock acceptance. |
| clone 返回后、记录新身份前终止进程，再分别恢复原 clone 和同字节替换项，一个场景及两个恢复进程。<br>Kill after clone returns and before new identity is recorded, then recover the original clone and an equal-byte replacement, one case and two recovery processes. | 两者长度／SHA256 相同，inode 均不同于预存来源。旧来源身份检查均拒绝，纯摘要检查均接受；因此这两种规则分别不能完成原 clone 恢复或不能拒绝替换项。<br>Both match length/SHA256 and both differ from the persisted source inode. Source-identity checks reject both; digest-only checks accept both. The former cannot recover the original clone, while the latter cannot reject its replacement. |

进程终止场景中观察者记录的 source／原 clone／替换 inode 分别为 `61076322`／`61076326`／`61076329`，设备号 `16777233`，长度 86016，SHA256 `dc800a1ea846a6a6375a2b06b2ae23f58200261c77dabb3541424d56982bfe79`。原 clone 身份只供实验观察，未写入可供恢复进程信任的发布记录。模型只使用 v1 身份匹配的必要 device/inode 子条件，不声称复刻完整凭证。原始 `results.json` 的 SHA256 为 `74ec769a51c6927fc2e99b97085033a1ee27ef76f2a260a1a42cb79c4e5e61af`；脚本、执行命令、平台信息和原始产物由本次交付记录保留。

For the termination case, observer-only source/original-clone/replacement inodes are `61076322`/`61076326`/`61076329` on device `16777233`, length 86016, with SHA256 `dc800a1ea846a6a6375a2b06b2ae23f58200261c77dabb3541424d56982bfe79`. The observer's original-clone identity is not written into trusted recovery records. The model uses only the necessary device/inode subset of v1 identity matching, rather than reproducing the entire receipt. Raw `results.json` has SHA256 `74ec769a51c6927fc2e99b97085033a1ee27ef76f2a260a1a42cb79c4e5e61af`; this delivery retains scripts, commands, platform details and raw artifacts.

**结论限定：** 本实验确认 fd 来源绑定和不覆盖候选，也确认直接把 clone 换入现有 v1 协议不能闭合已描述的崩溃窗口；它不是所有可能 v2 协议均不可能的证明。没有测试硬件掉电、跨卷／其他文件系统、系统容器隔离、独立安全主体 broker、Linux／Windows 或原生 App。不新增危险清理或回退，FTP 派发和 M4 继续受原门槛约束。

**Bounded conclusion:** the probe supports fd-source binding/no-clobber and shows that directly substituting clone into the existing v1 protocol leaves this crash window open. It does not prove every possible v2 protocol impossible. Power loss, cross-volume/other filesystems, system-container isolation, independent-principal brokers, Linux/Windows and the native App were not tested. Add no unsafe cleanup/fallback; FTP dispatch and M4 retain the original gates.

## 2026-11-21：受保护来源命名空间候选 / Protected-source namespace candidates

FTP 的 SQLite 完成证明已在 PR #100 保存原生产 generation、持有对象身份及整文件进度，但数据库授权不改变目录项权限。本节只补充平台证据和待决定的架构，不接受新威胁范围、不迁移 v1、不启用 FTP 派发或 M4。旧 HTTP 路径的局限也不因此消失。

PR #100 persists the original FTP producer generation, held-object identity and whole-file progress, but SQLite authority does not change namespace access. This section records platform evidence and an architecture decision still to be made. It accepts no revised threat model, v1 migration, FTP dispatch or M4 enablement, and does not repair the legacy HTTP limitations.

Apple 文档指出，macOS 14 起 App Sandbox 容器与代码签名关联；macOS 15 起 App Group 容器也对非沙盒 App 提供系统数据保护。它是值得验证的操作系统边界，不能与普通 `0700` 目录等同。但其他 App 可经用户授权访问，辅助程序也有签名要求；App Group 的无提示访问还依赖分发与成员身份条件。文档本身不证明当前 Nexum、开发签名候选或任意 `--data-dir` 已受到该保护。[容器保护](https://developer.apple.com/documentation/xcode/protecting-local-app-data-using-containers) · [文件访问与签名条件](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox) · [App Group 配置](https://developer.apple.com/documentation/xcode/configuring-app-groups)

Apple documents code-signature association for App Sandbox containers from macOS 14 and system data protection for nonsandboxed App Group containers from macOS 15. This is an OS boundary worth testing rather than ordinary `0700` permissions. Other apps can receive user authorization, helpers have signing requirements, and prompt-free App Group access has distribution/membership conditions. Documentation alone does not establish protection for current Nexum, development-signed candidates or arbitrary `--data-dir` locations. [Container protection](https://developer.apple.com/documentation/xcode/protecting-local-app-data-using-containers) · [File access and signing conditions](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox) · [App Group configuration](https://developer.apple.com/documentation/xcode/configuring-app-groups)

若来源和 journal 的排他访问确经系统强制，预存来源身份后，从持有的受保护来源目录发布到持有的目标父目录，有望在发布时保留同一对象；之后目标被替换仍按原身份拒绝恢复，且不删除普通下载目录中的替换项。这只是协议候选：须实际证明容器访问拒绝、被授予额外访问权的进程边界、辅助程序身份、取消、进程终止、升级及失败恢复。容器所在卷之外的目标还可能跨卷，不能以 copy／clone 或不安全路径回退掩盖 `EXDEV`。独立 Server／CLI、自选数据目录和外接盘必须分别定义能力，不能沿用托管 App 的保护声明。

If exclusive source/journal access is enforced by the OS, persisting source identity before publishing from a held protected source directory into the held destination parent may preserve the same object at publication. Later destination replacement must still fail recovery against that identity, without deleting replacement entries in ordinary download folders. This remains a protocol candidate requiring actual container-access denial, boundaries for additionally authorized processes, helper identity, cancellation, termination, upgrade and failure recovery. Targets outside the container's volume can cross filesystems: copy/clone or unsafe pathname fallbacks must not conceal `EXDEV`. Standalone Server/CLI, selected data directories and external volumes need their own capability definitions rather than inheriting managed-App claims.

### 实施前需要确定的范围 / Scope decision before implementation

1. **保持当前同 UID 对手保证，设计独立权限的最小发布 broker。** 该路线需要安装授权、请求方认证、最小权限和文件所有者管理、升级／卸载及各目标卷的验收，不能把普通同用户 helper 当作隔离。完整设计与平台验证前不启用产品；本次未安装系统服务。
   **Retain the current same-UID adversary guarantee through an independent-principal publication broker.** This route adds installation authorization, caller authentication, least privilege and ownership management, upgrade/removal and per-volume acceptance; an ordinary same-user helper is not isolation. Complete design/platform verification precedes enablement; this work installs no system service.
2. **签名容器的受限模式。** 优先验证 macOS 15 及以上的正式签名 App Group 来源／journal，仅承诺通过测试的同卷目标和托管 App／随包 Server 组合。必须明确拥有用户额外授权／Full Disk Access 的程序是否处于信任边界之外；这不是自动满足现有任意同 UID 写者模型。最低系统版本、外接盘／独立 Server 限制和正式签名身份都需随方案接受，再做真实隔离和崩溃验证。
   **Use a signed-container restricted profile.** First verify a formally signed App Group source/journal on macOS 15 or later, covering only tested same-volume destinations and managed App/bundled Server combinations. Explicitly define whether programs with additional user grants/Full Disk Access are trusted; this does not automatically satisfy the current arbitrary same-UID writer model. Accept the minimum OS, external-volume/standalone-Server limits and distribution identity requirements before actual isolation/crash testing.
3. **显式修订为普通桌面安全范围。** 若维护者选择首发不对抗具有同账户写权限的恶意程序持续替换目录项，必须另行接受这一范围修订，记录残留的检查到路径操作竞态。已有目标不覆盖、符号链接保护、完整性、取消、恢复及失败保留仍必须实现和验收；不得把修订表述为 TOCTOU 已修复，也不得自动扩大到所有平台或旧 HTTP 路径。
   **Explicitly revise to an ordinary desktop security scope.** If the maintainer chooses a first release without resistance to persistent namespace substitution by malicious same-account writers, separately accept that scope change and record the residual check-to-path-operation races. No-clobber destinations, symlink protection, integrity, cancellation, recovery and failure preservation remain implementation/acceptance requirements. Never describe the revision as a TOCTOU fix or silently extend it to every platform or legacy HTTP.

以上三项均未因本次文档更新而选定。产品接线继续受原门槛约束；BT 元数据和网络控制等不依赖文件发布的工作可按各自 RFC 并行推进。

None of these options is selected by this documentation update. Product integration retains the original gates; BT metadata/network-control work that does not depend on file publication can proceed under its own RFC.
