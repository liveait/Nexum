<a id="nexum-development-plan"></a>
<a id="nexum-开发计划"></a>

# Nexum 开发计划 / Nexum Development Plan

导航 / Contents: [1. 当前起点与 Motrix 差距 / 1. Current baseline and Motrix gaps](#1-current-baseline-and-motrix-gaps) · [2. 按交付物推进的里程碑 / 2. Delivery milestones](#2-delivery-milestones) · [3. 范围与下一步 / 3. Scope and immediate next step](#3-scope-and-immediate-next-step)

本计划以 2026 年 10 月 9 日的代码为起点，按“先交付可独立使用的 macOS 下载器，再补齐下载能力和生态集成”排序。对标范围参考 [Motrix 官方功能清单](https://github.com/agalwood/Motrix/blob/main/README.md)及其 [v2.0.0-beta.46 发布页](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46)；这是功能差距清单，不代表已逐项实测 Motrix，也不承诺 Nexum 会逐项复制其实现或发布时间。

This plan starts from the code as of October 9, 2026, and orders work around a standalone macOS downloader first, followed by download capabilities and ecosystem integration. The comparison uses the [official Motrix feature list](https://github.com/agalwood/Motrix/blob/main/README.md) and its [v2.0.0-beta.46 release](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46). It is a feature-gap reference, not a claim that we tested every Motrix feature or a commitment to copy every implementation or release on a fixed date.

“已有”表示代码路径已接通，“待验收”表示实现存在但尚缺指定产物的实际检查，“计划”表示还没有可用的端到端实现。只有通过相应里程碑的验收条件，才把能力写成已交付。当前调用路径见[架构设计](ARCHITECTURE.md)，构建、截图与发布操作见[发布流程](RELEASE.md)，TLS 的已完成决策见 [ADR 0005](decisions/0005-tls-transport.md)。

“Present” means a code path is wired, “needs validation” means an implementation lacks a check of the stated artifact, and “planned” means there is no usable end-to-end implementation. A capability is delivered only after its milestone acceptance criteria pass. See [Architecture](ARCHITECTURE.md) for current call paths, [Release Process](RELEASE.md) for build, screenshot, and release steps, and [ADR 0005](decisions/0005-tls-transport.md) for the completed TLS decision.

历史交付核对 / Historical delivery reconciliation（2026-10-31，#66 合并后基线 `cf88455`）：PR #58 修复单连接会话、续传与完成恢复；PR #61 将所有大小的 HTTP 下载统一到此路径；PR #62 与 PR #63 分别补齐未启用分段原型的输入／响应校验和暂存文件句柄／身份检查。PR #64 已交付分段 Worker 暂停／退出屏障，PR #65 修复既有恢复测试的终态同步。PR #66 已合并为 `cf884557140a252f20b87dea71794b23bf07b6f4`，交付分段完成凭证、离线对账及取消／发布协调；[实际合并 SHA 的 main push CI](https://github.com/liveait/Nexum/actions/runs/41191978580) 中 Rust、Desktop macOS 及其全部检查步骤均通过。该交付仍未封闭文件发布／清理竞态；M4 多连接及原生发布验收仍未完成。

Historical delivery reconciliation (2026-10-31, #66 post-merge baseline `cf88455`): PR #58 repaired single-connection sessions, resume and completion recovery; PR #61 routes HTTP downloads of every size through that path. PR #62 and PR #63 added input/response validation and held staging/identity checks to the disabled segment prototype. PR #64 delivered segment-worker pause/termination barriers, and PR #65 repaired terminal-state synchronization in an existing recovery test. PR #66 merged as `cf884557140a252f20b87dea71794b23bf07b6f4`, delivering segment receipts, offline reconciliation and cancellation/publication coordination. Rust, Desktop macOS and every check step passed in [main push CI for the actual merge SHA](https://github.com/liveait/Nexum/actions/runs/41191978580). That delivery leaves publication/cleanup races open; M4 multi-connection and native release acceptance remain incomplete.

最新交付对账（2026-10-31）：[PR #71](https://github.com/liveait/Nexum/pull/71) 已合为 `a3b48cc`，[main push CI](https://github.com/liveait/Nexum/actions/runs/41211980057) 全部门槛通过，交付 Ready 后受控完成对账及完成保存失败不重试；[PR #72](https://github.com/liveait/Nexum/pull/72) 已合为 `9976a53`，[main push CI](https://github.com/liveait/Nexum/actions/runs/41212847290) 通过，补齐图标容器和实际 App 包校验。PR #73 已合为 `38ee159`，[main push CI](https://github.com/liveait/Nexum/actions/runs/41213957671) 全部门槛通过，交付已选石墨灰 App 图标与透明 N 派生资源；PR #69 的重试／快照及入口改进已合为 `6ad16f7`，[main push CI](https://github.com/liveait/Nexum/actions/runs/41214993927) 全部门槛通过；PR #74 的品牌与精简 README 已合为 `6d4ecc3`，[main push CI](https://github.com/liveait/Nexum/actions/runs/41216122169) 全部门槛通过；PR #68 已合为 `cf107ba`，RFC 0002 仍为文件所有权提案，不是已接受的平台安全证明或 TOCTOU 实现。官网与精确 main-CI 发布屏障由 PR #70 提供；实际生产部署结果以 Website workflow 和公开 URL 的独立验证记录为准，正式安装包仍等待 M3。实现、源候选 QA、合并后 CI 与里程碑验收分别记录。

Latest delivery reconciliation (2026-10-31): [PR #71](https://github.com/liveait/Nexum/pull/71) merged as `a3b48cc`, with every gate passing in [main push CI](https://github.com/liveait/Nexum/actions/runs/41211980057), delivering controlled post-Ready reconciliation and nonretryable completion-save failure. [PR #72](https://github.com/liveait/Nexum/pull/72) merged as `9976a53`, with passing [main push CI](https://github.com/liveait/Nexum/actions/runs/41212847290), adding icon-container and real-App bundle checks. PR #73 merged as `38ee159`, with every gate passing in [main push CI](https://github.com/liveait/Nexum/actions/runs/41213957671), delivering the selected graphite App icon and transparent N derivatives. PR #69 retry/snapshot and entry improvements merged as `6ad16f7`, with every gate passing in [main push CI](https://github.com/liveait/Nexum/actions/runs/41214993927). PR #74's branded, concise README merged as `6d4ecc3`, with every gate passing in [main push CI](https://github.com/liveait/Nexum/actions/runs/41216122169). PR #68 merged as `cf107ba`; RFC 0002 remains a file-ownership proposal, not accepted platform-safety proof or a TOCTOU implementation. PR #70 provides the website and exact-main-CI publication barrier. Actual production deployment depends on separately recorded Website workflow and public-URL verification; official installers still wait for M3. Implementation, source-candidate QA, post-merge CI and milestone acceptance are separate records.

<a id="1-current-baseline-and-motrix-gaps"></a>
<a id="1-当前起点与-motrix-差距"></a>

## 1. 当前起点与 Motrix 差距 / 1. Current baseline and Motrix gaps

| 领域<br>Area | Nexum 当前状态<br>Nexum today | 尚需补齐<br>Remaining gap |
| --- | --- | --- |
| 任务与 HTTP/HTTPS<br>Tasks and HTTP/HTTPS | Server 使用 SQLite 保存并恢复任务；排队、并发槽位、重试、进度、暂停和删除已接通。单连接 HTTP 下载有暂存文件及经 ETag/Last-Modified 校验的跨重启续传。<br>The server stores and recovers tasks in SQLite. Queueing, concurrency slots, retries, progress, pause, and removal are wired. Single-connection HTTP downloads use staged files and cross-restart resume validated by ETag/Last-Modified. | 安全的单文件多连接下载及其段级持久化／跨重启恢复。速率、ETA、限速与单连接停滞读取控制已验证。没有校验器的未完成响应从零重下；凭证证明完整的字节可在本地对账。<br>Safe multi-connection downloads of one file, durable segment state, and cross-restart recovery. Speed, ETA, limits, and stalled-read controls on one connection are verified. Incomplete responses without validators restart from byte zero; receipt-proven complete bytes can reconcile locally. |
| macOS Desktop | 原生 Tauri App 有任务列表、添加下载、设置、Keychain 凭据、明暗外观和中英文界面。固定版本 CLI 构建的 release 候选产物已有原生明暗及窄窗口截图，已知和未知大小已完成任务均显示满格进度，完成键盘导航/选择检查及暂停/减弱动态效果下的进度轨道采样比较。<br>The native Tauri app has a task list, Add Download, Settings, Keychain credentials, light/dark appearance, and English/Chinese UI. The pinned-CLI release candidate has native Light/Dark and narrower-window captures, completed known-size and unknown-size tasks with full progress, keyboard navigation/selection checks, and sampled progress-track comparisons for Paused and Reduce Motion states. | 随包 Server 和托管／外部模式已接线；全新用户、中断和升级的最终验收仍待完成；后端完成对账 fixture 已覆盖发布／保存窗口。VoiceOver 实际朗读仍待确认。动态效果证据为采样帧及源码检查，不是连续录像。<br>The bundled Server and managed/external mode are now wired; final clean-user, interruption, and upgrade acceptance remain open. Backend completion-reconciliation fixtures now cover the publication/persistence window. Actual VoiceOver speech remains open. Motion evidence consists of sampled frames and source inspection, not continuous recording. |
| 安全与协议<br>Security and protocol | TCP/HTTP JSON-RPC、事件订阅、可选认证、RPC 限流和经校验的 TLS 已接通。<br>TCP/HTTP JSON-RPC, event subscriptions, optional authentication, RPC rate limiting, and verified TLS are wired. | 可靠事件回放、显式 CA/证书固定及更严格的协议兼容性检查按实际需求补充；已完成的 TLS 接线不再列为近期主线。<br>Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks can follow concrete needs; completed TLS wiring is no longer a near-term workstream. |
| FTP 与 BT/磁力<br>FTP and BT/magnet | Magnet Resolver 只校验 `xt=urn:btih:` 参数存在；本地文件来源只完成校验；Server 目前只派发 HTTP/HTTPS Worker；FTP 未形成传输路径。<br>The magnet resolver only checks for the presence of an `xt=urn:btih:` parameter; local-file sources are only validated; the server dispatches only HTTP/HTTPS workers. FTP has no transfer path. | FTP、BT 种子和磁力链接的真实下载、控制、恢复与完整性验证。通用 Engine Adapter 的同步接口还不能直接承载现有 Server Worker 的长期运行控制。<br>Real FTP, torrent, and magnet downloads with control, recovery, and integrity checks. The synchronous general Engine Adapter cannot directly provide the long-running control already implemented in the server HTTP worker. |
| Browser 与系统集成<br>Browser and system integration | 扩展可通过 HTTP/HTTPS `/jsonrpc` 投递链接。<br>The extension can submit links through HTTP/HTTPS `/jsonrpc`. | 扩展没有凭据设置，目标路径仍写到 `/tmp/nexum-*`；缺可发布的依赖锁定、浏览器验收、菜单栏/通知/链接接管等桌面集成。<br>The extension has no credential setting and writes destinations to `/tmp/nexum-*`. It lacks locked release dependencies, browser validation, and desktop integration such as menu bar, notifications, and URL handling. |
| 分发与扩展性<br>Distribution and extensibility | CI 检查 Rust 并构建本机临时签名、含已校验 Server 的 macOS App；Plugin、Media、Automation 有基础模型。<br>CI checks Rust and builds a locally ad-hoc-signed macOS app with a verified bundled Server. Plugin, Media, and Automation have foundation models. | 没有正式签名、公证、安装升级验收或自动发布；Windows/Linux 安装包未验收。Plugin Provider、真实媒体处理和自动化尚未接入产品路径。<br>No distribution signing, notarization, install/upgrade validation, or automated release. Windows/Linux installers are unvalidated. Plugin providers, real media processing, and automation are not wired into the product path. |

<a id="2-delivery-milestones"></a>
<a id="2-按交付物推进的里程碑"></a>

## 2. 按交付物推进的里程碑 / 2. Delivery milestones

### 后续推进顺序（2026-10-31） / Remaining sequence (2026-10-31)

| 顺序 / Order | 工作与进入下一阶段的条件 / Work and exit criteria |
| --- | --- |
| 已交付 / Delivered | PR #66 按 [RFC 0001](RFC/0001-segment-completion-protocol.md) 接通分段完成凭证、离线对账及控制锁下取消／发布；不重复开发，不代表 M4 完成。 / PR #66 delivered segment receipts, offline reconciliation and cancellation/publication under the control lock; do not repeat it or treat it as M4 acceptance. |
| 1 | 依据已形成的 [RFC 0002 提案](https://github.com/liveait/Nexum/pull/68)验证平台机制与身份边界，再封闭身份检查到 rename／hard-link／unlink 的竞态；提案本身不等于实现；在最后一次检查后精确插入替换，验收原目标、替换文件和无关 sidecar 保护。 / Validate platform mechanisms and identity boundaries under the drafted [RFC 0002 proposal](https://github.com/liveait/Nexum/pull/68) before closing check-to-rename/hard-link/unlink races; the proposal is not an implementation; inject replacement after the last check and verify protection of existing destinations, replacements and unowned sidecars. |
| 2 | 加入安全且有界的单连接回退：先停止并 join 全部 Worker，每次任务尝试最多降级一次；取消及本地文件安全错误不自动回退，沿用 Scheduler 重试预算。 / Add bounded safe single-connection fallback: stop and join all workers first, allow at most one downgrade per task attempt, never automatically fall back after cancellation or local file-safety errors, and retain Scheduler retry limits. |
| 3 | 先固定独立版本化段恢复格式，绑定来源、目标、validator、文件身份及可信字节范围；校验后跨重启复用，不把预分配长度当作进度。 / Fix a separately versioned segment-recovery format binding source, destination, validator, identity and proven byte ranges; validate before reuse and never treat preallocated length as progress. |
| 4 | 通过显式策略接通真实 Server 分段派发及 CLI／原生 App 验收；暂停、取消、回退、槽位释放和恢复全部通过后才默认启用并完成 M4，未验收平台继续禁用分段。 / Wire real Server segment dispatch through an explicit policy and validate CLI/native App consumers; enable by default and complete M4 only after pause, cancellation, fallback, slot release and recovery pass, leaving unvalidated platforms disabled. |
| 5 | 完善 M5 Registry 能力路由与通用恢复，再依次推进 FTP/FTPS、BT/磁力及浏览器／系统集成。 / Complete M5 Registry capability routing and generic recovery, then FTP/FTPS, BT/magnet and browser/system integration. |

macOS 首发准备可并行推进：PR #71 的大文件恢复就绪修复已交付，不重复实现；接下来补 VoiceOver、全新 OS 用户／异常中断／升级的最终验收，随后签名、公证、安装包和官网生产部署。首个 HTTP 版本不依赖 FTP/BT 全部完成；正式下载仍须以实际产物验收为准。以上是依赖顺序，不是日期承诺。

macOS launch preparation can proceed in parallel: PR #71 delivered the large-file recovery-readiness fix; do not repeat it. Next complete VoiceOver and final clean-OS-user/abnormal-interruption/upgrade acceptance, then signing, notarization, installers and website production deployment. The first HTTP release does not require complete FTP/BT; official downloads still depend on acceptance of the actual artifact. This is a dependency sequence, not a date commitment.

近期兼容边界：沿用既有 RPC、TaskView 和 v1 完成凭证。启动只检查有界凭证元数据，摘要对账放到 Ready 后、不持有 Server 全局锁的可控 Worker；恢复任务对账前不得重新派发。PR #69 已实现 Add Sheet 原位重试排队（保留已创建 ID）、旧快照过期提示、唯一顶部添加入口、本地／远程说明及欢迎／升级提示的中英文，已合为 `6ad16f7`，其[合并后完整 CI](https://github.com/liveait/Nexum/actions/runs/41214993927) 通过；后续验收保留这些行为，不重新列为未实现功能。URL 校验已有实现；任务上下文菜单和移除确认留到后续体验工作，不新增为本轮首发门槛。每项工作独立 worktree／单问题 PR，按当前 SHA 记录验证与未验收项。

Near-term compatibility: retain existing RPC, TaskView and v1 completion receipts. Startup checks only bounded receipt metadata; digest reconciliation runs after Ready in a controllable worker without holding the global Server lock, and recovering tasks cannot dispatch again before reconciliation. PR #69 implements in-sheet queue retry retaining the created ID, explicit stale-snapshot indicators, one toolbar Add entry, local/remote descriptions and bilingual welcome/upgrade messages, merged as `6ad16f7` with passing [complete post-merge CI](https://github.com/liveait/Nexum/actions/runs/41214993927). Preserve these behaviors in later acceptance rather than listing them as unimplemented work. URL validation already exists; task context menus and removal confirmation remain later polish rather than new launch gates. Use a separate worktree and focused PR per package, recording checks and acceptance gaps against the current SHA.

里程碑 / Milestones: [M0](#m0) · [M1](#m1) · [M2](#m2) · [M3](#m3) · [M4](#m4) · [M5](#m5) · [M6](#m6) · [M7](#m7) · [M8](#m8) · [M9](#m9)

顺序表示依赖和优先级，不是时间表。M0–M3 是第一条可发布的 macOS HTTP 下载器路径；M4–M7 缩小与 Motrix 的核心功能差距；M8–M9 在前述能力稳定后推进。HTTP 引擎工作与分发准备可以并行，但 M3 的发布验收必须使用最终实际产物。

The order expresses dependencies and priority, not dates. M0–M3 form the first releasable macOS HTTP downloader path; M4–M7 close the core Motrix capability gaps; M8–M9 follow once those paths are stable. HTTP engine work and distribution preparation may overlap, but M3 release validation must use the final actual artifact.

<a id="m0--validate-the-existing-release-app-natively"></a>
<a id="m0--完成现有-release-app-的原生验收"></a>

<a id="m0"></a>

### M0 — 完成现有 release App 的原生验收 / M0 — Validate the existing release app natively

- **工作**：单独启动使用固定 Tauri CLI 构建的 release App；按[发布流程](RELEASE.md#macos-原生界面截图留证)保存明暗和窄窗口证据；在真实 App 中检查中英文、键盘焦点、VoiceOver 实际朗读，以及“减弱动态效果”下的不确定进度。

  **Work:** Launch the release app built with the pinned Tauri CLI on its own. Capture light, dark, and narrow-window evidence using the [Release Process](RELEASE.md#native-macos-visual-evidence). Check English/Chinese, keyboard focus, actual VoiceOver speech, and indeterminate progress under Reduce Motion in the real app.
- **验收**：同一候选 App 能连接本地 Server、完成已知 HTTP 文件并显示正确状态；上述原生检查有记录。debug App 截图不能替代 release 产物证据。发现的问题先修复并复验。

  **Acceptance:** The same candidate app connects to a local server, completes a known HTTP file, and shows the right state; the native checks have recorded results. Debug-app screenshots do not stand in for release-artifact evidence. Fix and repeat any failing check.
- **已记录进展**：10 月 10 日源码提交 `1df5eab8411081e0177037029025f6e2e81fdfcc` 的 release 候选产物已有明暗及窄窗口截图，完成已知和未知大小任务检查、Sheet/搜索焦点及任务上/下/Home/End/Enter/Space 检查，并保存暂停/减弱动态效果下的进度采样比较。产物身份、采样限制、偏好恢复和已完成的隔离 fixture 清理见[发布流程证据](RELEASE.md#10-月-10-日-release-候选产物原生验收)。M0 尚未完成：仍需确认 VoiceOver 实际朗读。

  **Recorded progress:** The October 10 release candidate at source commit `1df5eab8411081e0177037029025f6e2e81fdfcc` has Light/Dark and narrower-window captures, completed known-size and unknown-size task checks, sheet/search focus and task Up/Down/Home/End/Enter/Space checks, and sampled Paused/Reduce Motion progress comparisons. See the [Release Process evidence](RELEASE.md#october-10-release-candidate-native-qa) for artifact identity, sampling limits, restored preferences, and completed isolated-fixture cleanup. M0 is not complete: actual VoiceOver speech still needs confirmation.

- **本轮来源限定的界面 QA**：[PR #69](https://github.com/liveait/Nexum/pull/69) 已实现重试／过期快照、唯一添加入口和本地／远程设置，19 项前端测试通过，已合为 `6ad16f7`，其[合并后完整 CI](https://github.com/liveait/Nexum/actions/runs/41214993927) 通过；隔离原生候选已复验添加入口与本地／远程说明。集成源码 `9965cd25c94fde23d3dfb775b67c4be6f63130a0` 的 Nexum Brand QA（`com.nexum.qa.brand-mark`）通过本轮侧栏透明 N 标记原生检查；已选石墨灰 App／Dock 图标保留。PR #73 已合且[合并后完整 CI](https://github.com/liveait/Nexum/actions/runs/41213957671) 通过；PR #74 的品牌 README 已合为 `6d4ecc3`，其[合并后完整 CI](https://github.com/liveait/Nexum/actions/runs/41216122169) 通过。此前 10 月 10 日的证据仍只属于其记录源码，不自动视为新候选全部复验。上述检查均不替代 VoiceOver、全新 OS 用户、升级或最终签名安装包验收。

  **Source-scoped UI QA this round:** [PR #69](https://github.com/liveait/Nexum/pull/69) implements retry/stale snapshots, one Add entry and local/remote settings, with 19 passing frontend tests, merged as `6ad16f7` with passing [complete post-merge CI](https://github.com/liveait/Nexum/actions/runs/41214993927). Isolated native candidates rechecked the Add entry and local/remote descriptions. Nexum Brand QA (`com.nexum.qa.brand-mark`) at integration source `9965cd25c94fde23d3dfb775b67c4be6f63130a0` passed this round's native transparent-N sidebar checks. The selected graphite App/Dock icon remains. PR #73 is merged with passing [complete post-merge CI](https://github.com/liveait/Nexum/actions/runs/41213957671); PR #74's branded README merged as `6d4ecc3` with passing [complete post-merge CI](https://github.com/liveait/Nexum/actions/runs/41216122169). October 10 evidence remains tied to its recorded source rather than accepting every check on a newer candidate. None of these checks accepts VoiceOver, a clean OS user, upgrades or a final signed installer.

<a id="m1--a-double-click-ready-macos-http-app"></a>
<a id="m1--双击即用的-macos-http-app"></a>

<a id="m1"></a>

### M1 — 双击即用的 macOS HTTP App / M1 — A double-click-ready macOS HTTP app

- **先决设计**：记录 Desktop 如何内置、启动、探活、复用和停止本机 Server，如何区分 App 托管实例与用户配置的外部 Server，以及稳定数据目录、默认下载目录、端口冲突和本地鉴权策略。产品尚未发布，不做旧任务自动迁移；保留“设置”里的外部 Server 连接能力。

  **Design first:** Document how the Desktop bundles, starts, probes, reuses, and stops a local server; how it distinguishes an app-managed instance from a user-configured external server; and the stable data directory, default download directory, port-conflict, and local-authentication policies. The product is unreleased, so there is no automatic migration of old tasks. Preserve external server connections under Settings.
- **实现切片**：将 Server 随 App 打包并管理其生命周期；把任务数据库放入稳定的应用数据目录，保留目标文件旁的部分文件及原子完成语义；从 URL/可信响应信息安全生成文件名、自动生成任务 ID 并处理重名，允许用户改选目标；提供受控默认下载目录、受保护的本地通信和明确的故障/恢复提示。

  **Implementation slices:** Bundle the server with the app and manage its lifecycle. Put the task database in a stable application data directory while preserving destination-side partial files and atomic completion. Derive a safe filename from the URL/trusted response metadata, generate task IDs, handle name collisions, and let users override the destination. Provide a controlled default download directory, protect the managed local connection, and show actionable failure/recovery states.
- **验收**：在干净 macOS 用户环境中，双击 App、粘贴 URL 即可下载到默认目录，无需另开终端或手动填写任务 ID/目标路径；退出、重开、异常中断和升级后任务及文件仍可恢复。端口被占用或 Server 启动失败时可诊断，不会误连或误停未知进程；外部 Server 模式仍可用。

  **Acceptance:** In a clean macOS user environment, double-clicking the app and pasting a URL downloads to the default directory without a separate terminal or manually entering a task ID/destination path. Tasks and files survive quit, relaunch, abnormal interruption, and upgrade. A busy port or failed server startup is diagnosable; the app neither connects to nor stops an unknown process. External server mode still works.
- **已记录进展**：10 月 11 日实现 Server 的专用托管启动入口、私有分帧控制／就绪协议、随机回环端口、管道传递的强制 Bearer 鉴权和关闭／EOF 退出；隔离测试检查鉴权、数据锁及停顿下载恢复。细节见 [ADR 0006](decisions/0006-managed-macos-server.md#首个实现切片server-托管启动协议)。后续切片已接入 Tauri 管理者、包内哈希校验、仅后端持有的路由／鉴权、托管／外部设置、后端任务 ID 和不冲突目标，以及关闭窗口隐藏和退出停止。原生 codeload 下载、冲突保留、退出／重开、一次子进程崩溃恢复、外部模式隔离及保存目录快捷键检查均通过；产物哈希和截图记录在[发布流程](RELEASE.md#托管-server-原生验收)。后续[完成对账切片](decisions/0007-http-completion-reconciliation.md)通过隔离 engine 和 Server 进程测试覆盖完整文件／持久化状态窗口。欢迎流程、`check_fresh_install`、`get_upgrade_info` 和 `last_seen_version` 已接入源码，但源码存在不等于全新用户／升级验收通过；部分文件元数据清理也必须在最终下载路径复验。该历史启动局限已由 [PR #71](https://github.com/liveait/Nexum/pull/71) 修复：启动仅检查有界凭证元数据，Ready 后由不持 Server 全局锁、可暂停／取消的 Worker 对账；完整字节的 Completed 保存失败不进入普通网络重试。`a3b48cc` 的合并后完整 CI 已通过；Ready 表示控制与监听可用，不表示恢复已到终态。此前中文版本的“M1 完成”与英文版本不一致，现统一为 **部分实现、最终验收待完成**。

  **Recorded progress:** On October 11, the Server gained a dedicated managed entry point, private framed control/readiness protocol, ephemeral loopback port, mandatory pipe-delivered Bearer authentication, and Shutdown/EOF exit. Isolated tests check authentication, data-directory locking, and stalled-download recovery. See [ADR 0006](decisions/0006-managed-macos-server.md#first-implementation-slice-managed-server-bootstrap). The following slice has wired the Tauri supervisor, bundle hash verification, backend-only route/authentication, Managed/External settings, backend task IDs and collision-safe destinations, close-to-hide and Quit shutdown. Native codeload download, collision preservation, Quit/relaunch, one child crash recovery, External isolation and saved-folder shortcut checks passed; artifact hashes and screenshots are recorded in [Release Process](RELEASE.md#managed-server-native-qa). The subsequent [completion-reconciliation slice](decisions/0007-http-completion-reconciliation.md) covers the complete-file/persisted-state window in isolated engine and Server process tests. The source includes a welcome flow, `check_fresh_install`, `get_upgrade_info`, and `last_seen_version`, but implementation presence does not verify clean-user/upgrade acceptance; partial-metadata cleanup also needs verification through the final download path. That historical startup limitation is repaired by [PR #71](https://github.com/liveait/Nexum/pull/71): startup checks bounded receipt metadata, with reconciliation after Ready in pause/cancel-aware workers outside the global Server lock. Failed Completed persistence for complete bytes never enters ordinary network retry. Complete post-merge CI passed at `a3b48cc`; Ready means control/listening availability, not terminal recovery. The former Chinese completion claim conflicted with this version and is corrected to **partially implemented, final acceptance pending**.

- **隔离原生大文件证据**：源码 `85715c3c641dc50630b995f82c9fe81321270750` 的独立 release 候选（`com.nexum.qa.recovery`）在源站关闭后，从真实 Engine 生成的 64GiB 完整 staging／v1 凭证恢复；原生暂停／恢复、精确最终进度、完整 SHA256 全量读取比对及退出重开检查通过。详细时序、文件／数据库结果及“清理前二进制 QA 数据库副本未保存”的证据缺口见 [PR #71](https://github.com/liveait/Nexum/pull/71)。不重建替代证据，也不据此宣称完整 M0/M1、全新 OS 用户／升级、Windows 或 M3 分发验收通过。

  **Isolated native large-file evidence:** the separate release candidate at source `85715c3c641dc50630b995f82c9fe81321270750` (`com.nexum.qa.recovery`) recovers a real Engine-generated 64GiB complete staging file/v1 receipt with the origin stopped. Native pause/resume, exact final progress, an independent full-file SHA256 read and quit/relaunch checks passed. [PR #71](https://github.com/liveait/Nexum/pull/71) records timing, file/database outcomes and the missing binary pre-cleanup QA database-copy gap. Do not reconstruct substitute evidence or treat this as complete M0/M1, clean-OS-user/upgrade, Windows or M3 distribution acceptance.

<a id="m2--http-quality-needed-for-the-first-release"></a>
<a id="m2--首发所需的-http-下载质量"></a>

<a id="m2"></a>

### M2 — 首发所需的 HTTP 下载质量 / M2 — HTTP quality needed for the first release

- **工作**：从真实传输字节计算速率；只在长度可信时显示预计时间；定义全局/单任务限速设置并将 Scheduler 算出的上限传到 Worker；让阻塞读取中的 `task.pause` 和 `task.remove`（内部取消 Worker）及时响应，同时保持原目标文件和可恢复的部分文件安全。

  **Work:** Compute speed from actual transferred bytes; show ETA only when the length is trustworthy. Define global/per-task limit settings and pass Scheduler-calculated limits to workers. Make `task.pause` and `task.remove` (which cancels the worker internally) respond promptly during blocked reads while preserving the existing destination and recoverable partial files.
- **验收**：受控本地服务覆盖慢响应、断线、校验器变化、错误 `Content-Range`、多任务竞争、重试及目标冲突。长时间窗口内实测速率符合配置上限；本地停顿响应 fixture 下，暂停和删除各自在 5 秒内完成。成功文件哈希正确；中断、失败和删除不覆盖已有的正确目标文件。

  **Acceptance:** Controlled local servers cover slow responses, disconnects, changed validators, invalid `Content-Range`, competing tasks, retries, and destination conflicts. Measured throughput over a sustained window respects configured limits; pause and removal each finish within 5 seconds against a local stalled-response fixture. Successful files have the correct hashes; interruption, failure, and removal do not overwrite an existing good destination.
- **已记录进展**：速率按实际传输字节估算；只在总长可信时显示 ETA。`bandwidth` 支持 `unlimited`、`fixed` 与 `shared`，Scheduler 下发上限并在活跃任务数变化时刷新。PR #58 将响应体读取检查缩短为 1 秒，连续 8 次无数据检查失败后终止停滞响应，并修复会话续传、最终进度和完成恢复。`f1817f4` 的 main CI 与本地 Server 全套检查已通过（见本页状态核对）；先前 `3768c3a` 的失败属于已修复基线，不能继续记为当前阻断。M4 分段执行仍单独待验收，M0/M1 原生产物验收没有因此完成。

  **Recorded progress:** Speed is estimated from actual transferred bytes, with ETA only for a trustworthy total. `bandwidth` supports `unlimited`, `fixed`, and `shared`; the Scheduler supplies caps and refreshes them as the active task count changes. PR #58 shortened body-read control checks to one second, fails a response after eight consecutive no-data checks, and repaired session resume, final progress, and completion recovery. The `f1817f4` main CI and local complete Server checks passed (see the status reconciliation above). Earlier `3768c3a` failures describe the repaired baseline, not current blockers. M4 segment execution remains separately unvalidated; native M0/M1 artifact acceptance is still open.

<a id="m3--installable-and-trusted-macos-distribution"></a>
<a id="m3--可安装可信任的-macos-发布"></a>

<a id="m3"></a>

### M3 — 可安装、可信任的 macOS 发布 / M3 — Installable and trusted macOS distribution

- **工作**：固定构建输入和版本，产出适合声明架构的安装包；完成 Developer ID 签名、公证、Staple、Gatekeeper、安装/升级/卸载与干净机器冒烟。发布工作流和更新策略须与实际支持范围一致。

  **Work:** Pin build inputs and versions, produce an installer for each claimed architecture, and complete Developer ID signing, notarization, stapling, Gatekeeper, install/upgrade/uninstall, and clean-machine smoke checks. Release workflow and update policy must match the actual support scope.
- **验收**：用户从发布产物安装并双击启动，无需开发工具或终端，即可完成 M1–M2 的 HTTP 下载；系统验证签名和公证通过，升级保留任务数据。将版本、哈希、支持的 macOS/CPU 架构和未完成的 FTP/BT/扩展能力写入发布说明。操作步骤与证据留在[发布流程](RELEASE.md)。

  **Acceptance:** A user installs the release artifact and launches it by double-clicking, without development tools or a terminal, then completes M1–M2 HTTP downloads. System signing and notarization checks pass; upgrade preserves task data. Release notes state version, hashes, supported macOS/CPU architectures, and unfinished FTP/BT/extension capabilities. Keep procedures and evidence in the [Release Process](RELEASE.md).

<a id="website-deployment--supporting-m3-work-package-planned"></a>
<a id="官网部署--m3-配套工作包计划"></a>

#### 官网部署 — M3 配套工作包（计划） / Website deployment — Supporting M3 work package (planned)

- **范围与依赖**：使用本仓库 GitHub Pages 项目站点托管中英文静态官网，首版包含产品功能与截图、下载、文档入口、更新日志，以及源码和反馈入口。官网准备可与 M3 签名工作并行；首发 App 的原生验收仍优先，正式下载上线须等待实际 App 通过 M3。

  **Scope and dependency:** Host the English/Chinese static website as this repository's GitHub Pages project site. The first version includes product features and screenshots, downloads, documentation links, a changelog, and source/feedback links. Website preparation may overlap with M3 signing work; native acceptance of the first App remains the priority, and official downloads go live only after the actual App passes M3.
- **首版下载**：官方下载链接指向 GitHub Releases，自动同步公开发布元数据。下载页的版本、支持的 OS/CPU 架构、包大小、SHA256、签名与公证状态以实际发布产物为准；未完成验收的平台不得显示可下载安装。同步所需的私有 token 不得交给浏览器。

  **Initial downloads:** Link official downloads to GitHub Releases and automatically synchronize public release metadata. The download page's version, supported OS/CPU architectures, package size, SHA256, signing, and notarization status must reflect actual release artifacts. An unvalidated platform must not offer an installable download. Private tokens used for synchronization must not reach the browser.
- **部署工作**：PR 构建只提供可下载的预览产物，`main` push／手动 dispatch 的构建成功后，还须由精确 main-CI 屏障核对同仓库、同 SHA 的 `ci.yml` main push 中 Rust／Desktop macOS 及全部必需步骤成功，并确认 main ref 未前进，才通过 GitHub Pages environment 发布；适配 `/Nexum/` 中英文、资源和语言切换路径。默认使用 `https://liveait.github.io/Nexum/` 与 HTTPS，独立域名后续配置。稳定 release 通过单独工作流请求 main 重建，不让 tag ref 绕过 environment 约束。

  **Deployment work:** PR builds provide downloadable preview artifacts; only successful main push/manual-dispatch builds can deploy through the GitHub Pages environment, after the exact-main-CI gate verifies the same repository/SHA's ci.yml main push, successful Rust/Desktop macOS jobs and every required step, and an unchanged main ref. Support `/Nexum/` for languages, assets and navigation. Use the default HTTPS project URL, with a custom domain later. Stable releases dispatch a main rebuild separately rather than bypassing environment constraints with a tag ref.
- **验收**：中英文路径、语言切换及页面链接均可用；从官网链接下载真实安装包，检查 SHA256、签名和公证状态与发布记录一致，并完成安装启动检查。PR 预览产物不会更新生产站或生产下载元数据；构建、同 SHA main CI 及实际 Pages 部署分别记录。官网与屏障实现由 PR #70（含屏障源码 `cf12d36`）承接；实际生产部署结果以 Website workflow 与公开 URL 验证分别留证，实现或 PR 构建通过不等于已上线。

  **Acceptance:** English/Chinese paths, language switching, and page links work. Download a real installer through the website link, verify its SHA256, signing, and notarization status against release records, and complete install/launch checks. PR preview artifacts do not update production or download metadata; record the build, same-SHA main CI and actual Pages deployment separately. PR #70 (including gate source `cf12d36`) provides the website and barrier; record actual production deployment through the Website workflow and public-URL checks. Implementation or a passing PR build does not establish that the site is live.
- **后续选项**：按实际流量和费用限制决定是否增加 R2 版本化镜像、独立下载域名与 CDN，并保留 GitHub Releases 回退。启用前验证两处元数据和哈希一致、`Range` 支持、下载文件名及文件完整性。

  **Later option:** Decide whether to add versioned R2 mirrors, a separate download domain, and a CDN based on actual traffic and cost limits, while retaining GitHub Releases as a fallback. Before enabling the mirror, verify matching metadata and hashes, `Range` support, download filenames, and file integrity.

<a id="m4--multi-connection-http-downloads-of-one-file"></a>
<a id="m4--单文件多连接-http-下载"></a>

<a id="m4"></a>

### M4 — 单文件多连接 HTTP 下载 / M4 — Multi-connection HTTP downloads of one file

- **工作**：在单连接路径稳定后增加可回退的 `Range` 分段下载、段级持久状态与原子完成；继续覆盖并发、暂停、重试和跨重启续传。

  **Work:** Once the single-connection path is stable, add fallback-capable `Range` segments, persistent segment-level state, and atomic completion. Keep concurrency, pause, retry, and cross-restart resume coverage.
- **验收**：受控服务分别提供正常 `Range`、拒绝 `Range`、校验器变化和段长度异常。单连接与多连接结果的文件哈希一致；服务端不支持分段或校验失败时安全回退，不覆盖已有的正确目标文件。

  **Acceptance:** Controlled servers serve valid `Range`, reject `Range`, change validators, and return abnormal segment lengths. Single- and multi-connection results have identical hashes. Unsupported segments or failed validation fall back safely without overwriting an existing good destination.
- **记录进度（未启用、待验收）**：Engine 中已有 `probe_range()`、`split_segments()` 和 `download_to_segments()` 原型。`f9ce5b6` 基线审查发现分段路径缺少响应范围及身份校验、可信 staging、完整暂停/退出屏障及完成凭证；只测试拆段和探测辅助方法不能验收分段下载。PR #61 已撤去 Server 自动分段入口及基于 `127.0.0.1` 的测试地址绕行，所有来源统一使用已验证的会话下载路径。PR #62 已补齐输入与响应校验，通过完整 Rust 门槛、独立审查及合并前后 CI，合并为 `3fcd259`。PR #63 已交付持有的暂存句柄与同步边界身份检查，PR #64 已交付所有 Worker 暂停／退出屏障；PR #65 已恢复主分支完整门槛。PR #66 已交付完成凭证与取消／发布协调；[PR #68](https://github.com/liveait/Nexum/pull/68) 已合入 RFC 0002 文件所有权提案（`cf107ba`），不是已接受的平台证明或 TOCTOU 修复。路径身份检查到 rename／unlink 的竞态、安全回退和段级恢复仍待实现。PR #58 的完成恢复修复已交付，但不会自动修复分段原型。**缺口**：以下安全执行切片、段级持久状态、跨重启段恢复及有界单连接回退，全部完成直接行为验收前不重新启用多连接。

  **Recorded progress (disabled, awaiting acceptance):** The Engine contains `probe_range()`, `split_segments()`, and `download_to_segments()` prototypes. Review of baseline `f9ce5b6` found missing response range/identity validation, trusted staging, complete pause/termination barriers, and completion receipts on the segment path. Tests of splitting and probing alone do not accept segment downloads. PR #61 removed automatic Server segment selection and the `127.0.0.1` fixture bypass so every source uses the validated session path. PR #62 added input and response validation, passing the full Rust gates, independent review and CI before/after merge as `3fcd259`. PR #63 delivered held staging handles and identity checks at synchronization boundaries, and PR #64 delivered all-worker pause/termination barriers. PR #65 restored complete main gates. PR #66 delivered receipts and cancellation/publication coordination. [PR #68](https://github.com/liveait/Nexum/pull/68) merged the RFC 0002 file-ownership proposal (`cf107ba`); it is neither accepted platform proof nor a TOCTOU fix. Identity-check to rename/unlink races, safe fallback and segment recovery remain open. PR #58 delivered completion recovery without repairing the segment prototype. **Gaps:** the safety slice below, durable segment state, cross-restart segment reuse, and bounded single-connection fallback need direct behavioral acceptance before multi-connection routing is re-enabled.

#### M4 实施切片 / M4 implementation slices

1. **已交付：统一安全执行路径。** 基于 `f1817f4`，先移除 Server 自动选择未验收分段下载及按 fixture URL 绕过探测的逻辑，所有来源使用已有单连接会话、完成凭证与持锁发布。用超过 1 MiB 的真实子进程 fixture 验证不发额外探测请求、文件内容、最终进度、完成保存失败后重启对账及既有目标保护。PR #61 已通过完整 Rust 门槛、独立审查及合并前后 CI；此切片已交付，M4 仍未完成。

   **Delivered: one safe execution path.** From `f1817f4`, remove automatic selection of the unvalidated segment path and fixture-URL probe bypasses. All sources use the existing single-connection session, completion receipts, and locked publication. A real subprocess fixture above 1 MiB must verify no extra probe request, exact bytes, final progress, recovery after failed completion persistence, and existing-destination protection. PR #61 passed the full Rust gates, independent review, and CI before and after merge. This slice is delivered; M4 remains incomplete.
2. **已交付：输入与响应校验（PR #62，基线 `f9ce5b6`）。** 在网络请求和创建／截断暂存文件前，要求非空、按序、完整且无重叠的段集合，拒绝越界与整数溢出；零字节文件不分段，拆段使用整数计算。分段原型仅接受语法有效的强 ETag，不修改单连接的 ETag/Last-Modified 兼容策略。探测必须校验精确 `bytes 0-0/total`、实际一字节及 EOF；每段校验 `206`、精确起止／总长、相同 ETag、identity 表示及实际响应长度；传输编码仅允许缺省或单一 `chunked`，拒绝未知、多值或重复编码。直接调用 Engine 分段 API 覆盖正常拼接和异常拒绝，保留具体错误。该切片不接回 Server；完整文件／控制／凭证门槛见下一项。

   **Delivered: input and response validation (PR #62, baseline `f9ce5b6`).** Before network requests or staging creation/truncation, require nonempty, ordered, complete and nonoverlapping segments; reject out-of-bounds ranges and integer overflow. Do not segment empty files; use integer splitting. The segment prototype accepts only syntactically valid strong ETags, preserving the single-connection ETag/Last-Modified policy. Probes must validate exact `bytes 0-0/total`, one actual byte and EOF; each segment validates `206`, exact bounds/total, a matching ETag, an identity representation and actual response length. Transfer-Encoding is limited to absent or a single `chunked` value, rejecting unknown, listed or duplicate codings. Exercise the segment Engine API directly for successful assembly and rejection cases, retaining specific errors. This slice does not restore Server routing; the next item retains the complete file/control/receipt gate.
3. **已交付：持有暂存句柄与身份检查（PR #63，基线 `3fcd259`）。** 以 `create_new` 创建普通文件，拒绝并保留已有普通文件、符号链接、FIFO 及目录；持有同一个文件句柄，所有 Worker 使用显式偏移写入。请求／各次写入前、所有 Worker join 后及发布／清理前核对文件身份，最终长度从持有句柄读取。已观察到替换时保留替换项；清理只尝试删除自己创建且身份匹配的部分文件，不触碰未拥有的 JSON sidecar。直接测试并发精确拼接、响应延迟／写入中／最终进度回调中的路径替换，以及正常失败／取消清理。边界：检查与基于路径的 rename／hard-link／unlink 之间仍有 TOCTOU，不能宣称原子文件所有权；该缺口保留为启用多连接的阻断门槛。

   **Delivered: held staging handles and identity checks (PR #63, baseline `3fcd259`).** Create a regular file with `create_new`, rejecting and preserving pre-existing regular files, symlinks, FIFOs and directories. Retain one file handle and use explicit offsets in every worker. Check identity before requests/writes, after all workers join and before publication/cleanup; read the final length from the held handle. Preserve observed replacements; cleanup only attempts to remove the identity-matching partial created by this attempt, without touching unowned JSON sidecars. Directly test exact concurrent assembly, path replacement during delayed responses, mid-write and final progress callbacks, and ordinary failure/cancellation cleanup. Boundary: TOCTOU remains between checks and path-based rename/hard-link/unlink, so this is not atomic file ownership. Keep that gap as a blocker for enabling multiple connections.
4. **已交付：所有 Worker 暂停／退出屏障（PR #64；PR #65 恢复主分支门槛）。** 暂停确认须覆盖全部活跃 Worker 与进度／发布协调者；确认后暂存字节和进度保持稳定。正在等待响应头、响应体、限速或暂停的 Worker，遇取消、任一 Worker 失败或进度回调失败时均须有界退出；全部 join 后才清理并通知完成，保留具体失败原因与取消类型。直接调用分段 API，以受控 HTTP fixture 验收暂停／恢复、失败／取消退出及清理顺序，并完整回归单连接消费者。该切片不接回 Server，也不解决完成凭证、基于路径操作的 TOCTOU、原子取消／发布、安全回退或段级恢复。

   **Delivered: all-worker pause/termination barriers (PR #64; PR #65 restored main gates).** Pause acknowledgement must cover all active workers and the progress/publication coordinator; staging bytes and progress must remain stable afterwards. Workers waiting for response headers, body data, bandwidth or resume must terminate within a bounded interval after cancellation, any worker failure or a progress-callback error. Join every worker before cleanup/completion notification, preserving concrete failures and cancellation types. Exercise the segment API through controlled HTTP fixtures for pause/resume, failure/cancellation and cleanup order, and run complete single-connection consumer regressions. This slice does not restore Server routing or solve completion receipts, path-operation TOCTOU, atomic cancellation/publication, safe fallback or segment recovery.
   控制采用协作式边界：网络等待、限速和暂停须可中断；同步文件系统调用、底层系统 DNS 解析以及调用方不返回的进度回调不具备线程强制终止保证；运行时的有限关闭等待不代表可强杀系统解析调用。验收使用有上限的受控场景，不把该保证扩大到任意系统阻塞。

   Controls are cooperative: network, bandwidth and pause waits must be interruptible. Synchronous filesystem calls, underlying operating-system DNS resolution and caller progress callbacks that never return cannot be forcibly terminated; a bounded runtime shutdown does not forcibly kill a resolver call. Acceptance uses bounded controlled scenarios and does not promise termination during arbitrary operating-system stalls.
5. **已交付：完成凭证与取消／发布协调（PR #66，基线 `f518b43`）。** 按 [RFC 0001](RFC/0001-segment-completion-protocol.md) 添加兼容的带凭证入口，保留旧入口；所有 Worker 退出后准备身份／摘要凭证，在同一控制锁中协调取消与不覆盖发布。凭证后的普通失败保留恢复证据；取消清理成功才返回 `Cancelled`，清理失败明确返回 `Failed` 并保留取消状态，由调用者显式处理残留凭证；分段始终不认领无关 sidecar。控制原子性不代表文件系统路径竞态已解决，Server 多连接仍禁用。

   **Delivered: receipts and cancellation/publication coordination (PR #66, baseline `f518b43`).** Follow RFC 0001 for a compatible receipt-aware entry while retaining the old API. Prepare identity/digest proofs after worker exit; coordinate cancellation and no-clobber publication under one control lock. Ordinary errors after receipt publication retain recovery evidence. Return `Cancelled` only after successful cancellation cleanup; on cleanup failure, return `Failed` while retaining cancelled state, requiring the caller to handle residual proof explicitly. Never claim unowned sidecars. Atomic control does not solve filesystem path races; Server segmentation stays disabled.
6. **后续安全执行与回退。** 先按 RFC 0002 提案验证平台文件身份机制，再封闭发布／清理竞态，随后加入有界、安全的单连接回退；保留已经交付的段集合／响应校验、持有句柄、Worker 屏障、字节计数及完成协议。两项均须直接验证，再进入段恢复实现。

   **Remaining safe execution and fallback.** Validate platform file-identity mechanisms under the RFC 0002 proposal, close publication/cleanup races, then add bounded, safe single-connection fallback. Preserve delivered segment/response validation, held handles, worker barriers, byte counts and completion protocol. Directly validate both packages before implementing segment recovery.
7. **段级恢复与最终启用。** 在前项稳定后设计并实现持久段状态、身份／摘要校验及跨重启段复用；验收前中断的多连接任务不得宣称可按段续传。随后通过显式策略接入真实 Server 消费路径，完成 CLI／原生 App 控制、回退与恢复验收，再默认启用多连接。

   **Segment recovery and final enablement.** Once the previous slice is stable, design and implement durable segment state, identity/digest checks, and cross-restart segment reuse. Do not claim per-segment restart recovery before acceptance. Then wire the real Server consumer through an explicit policy and validate CLI/native App controls, fallback and recovery before enabling multiple connections by default.

**安全入口切片本地验证（2026-10-15，macOS）**：新增 3 项大文件 Server 回归在旧产品代码 `f1817f4` 上全部失败，修复后全部通过；验证单次普通 GET、精确文件内容及最终进度、完成保存失败后的离线对账和已有目标保护。`cargo fmt --all -- --check`、`cargo check --locked --workspace --all-targets`、`cargo test --locked --workspace -- --test-threads=1`、`cargo clippy --locked --workspace --all-targets -- -D warnings` 全部通过；workspace 共 326 项，含 Server unit 29、managed 10、persistence 54。此记录不包含原生 App 验收；独立审查和 CI 以对应 PR 的最终 SHA 为准。

**Local checks for the safe-routing slice (2026-10-15, macOS)**: all three new large-file Server regressions failed with the old `f1817f4` product code and passed after the fix. They verify one ordinary GET, exact bytes and final progress, offline reconciliation after failed completion persistence, and existing-destination protection. `cargo fmt --all -- --check`, `cargo check --locked --workspace --all-targets`, `cargo test --locked --workspace -- --test-threads=1`, and `cargo clippy --locked --workspace --all-targets -- -D warnings` passed: 326 workspace tests, including Server unit 29, managed 10, and persistence 54. This is not native App acceptance; the corresponding PR records independent review and CI for its final SHA.

**输入与响应校验本地验证（2026-10-15，macOS）**：首批 8 项直接回归在 `f9ce5b6` 产品代码上为 1 项通过、7 项失败；修复并补充边界后，12 项直接集成测试全部通过，包括合法 chunked 探测／拼接、请求 identity 协商及错误范围／校验器／长度／重复头拒绝。完整格式、workspace check、workspace test（`--test-threads=1`）及 Clippy `-D warnings` 门槛全部通过；共 338 项，0 失败、0 忽略，含 Engine unit 52、分段集成 12、Server unit 29、managed 10、persistence 54。独立审查新增的 2 项传输编码回归在 `f13bcec` 产品代码上均失败，修复后通过；仅接受缺省或单一 `chunked`。首轮 Clippy 的测试字符串格式提示已修正。旧 probe fixture 补回缺失的 CRLF，保留原成功／长度断言，新增真实 HTTP 用例覆盖等效行为。此项不包含原生 App 验收；最终独立审查 SHA 与合并前后 CI 记录在对应 PR，M4 仍未完成。

**Local checks for input and response validation (2026-10-15, macOS)**: the first eight direct regressions produced one pass and seven failures with `f9ce5b6` product code. After the fix and boundary additions, all twelve direct integration tests passed, including valid chunked probes/assembly, identity request negotiation, and rejection of invalid ranges, validators, lengths and duplicate headers. Formatting, workspace check, workspace test (`--test-threads=1`), and Clippy `-D warnings` all passed: 338 tests, zero failures and zero ignored, including Engine unit 52, segment integration 12, Server unit 29, managed 10 and persistence 54. Both additional transfer-coding regressions from independent review failed on `f13bcec` product code and passed after the fix; only absent or a single `chunked` value is accepted. The initial Clippy warning about test string formatting was corrected. Missing CRLFs in old probe fixtures were restored while retaining success/length assertions; new real HTTP cases cover the same behavior. This is not native App acceptance. The corresponding PR records the final independently reviewed SHA and CI before/after merge; M4 remains incomplete.

**暂存句柄切片聚焦验证（2026-10-15，macOS）**：13 项直接 Engine 暂存回归在 `3fcd259` 产品代码上为 3 项通过、10 项失败，修复后全部通过；通过 channel／Condvar 同步路径替换与 16 段并发响应，不依赖固定等待猜测写入时机。`cargo test --locked -p nexum-engine -- --test-threads=1` 通过 52 项 unit、13 项 staging 集成及 12 项校验集成；Engine all-targets Clippy `-D warnings` 与格式检查通过。完整格式、workspace check、workspace test（`--test-threads=1`）及 all-targets Clippy `-D warnings` 全部通过：351 项，0 失败、0 忽略，含 Server unit 29、managed 10、persistence 54。最终并发 fixture 修改后，13 项 staging 集成测试再次通过。固定 SHA 独立审查及合并前后 CI 记录在对应 PR。未执行 Windows 偏移写入或原生 App 验收；M4 未完成，多连接未启用。

**Focused checks for held staging (2026-10-15, macOS)**: the thirteen direct Engine staging regressions produced three passes and ten failures with `3fcd259` product code and all passed after the fix. Channels/Condvars synchronize path replacement and sixteen concurrent segment responses, without fixed waits to guess write timing. `cargo test --locked -p nexum-engine -- --test-threads=1` passed 52 unit, 13 staging integration and 12 validation integration tests. Engine all-targets Clippy `-D warnings` and formatting passed. Formatting, workspace check, workspace test (`--test-threads=1`) and all-targets Clippy `-D warnings` passed: 351 tests, zero failures and zero ignored, including Server unit 29, managed 10 and persistence 54. After the final concurrent-fixture update, all 13 staging integration tests passed again. The corresponding PR records fixed-SHA independent review and CI before/after merge. Windows offset writes and native App acceptance were not executed; M4 remains incomplete and multi-connection routing disabled.

**Worker 屏障切片本地验证（2026-10-31，macOS）**：首批 5 项直接分段回归在 `d922bda` 产品代码上全部失败；最终 12 项集成测试为 2 项通过、10 项失败，修复后 12 项全部通过。真实 HTTP fixture 覆盖双 Worker 的响应头／响应体混合暂停、恢复时每段仅一次 GET、暂停后的字节和进度稳定、响应头／响应体／暂停期间取消、发布前协调者暂停、低带宽下的失败／取消及回调错误／panic。6 项私有单元测试补充已预留限速预算后的暂停／首个错误中断、线程创建失败、Worker panic、连续两轮暂停及 runtime 有限关闭；生产 Worker 直接使用同一组控制与 runtime helper。首次包级 Clippy 的两处嵌套条件格式提示已修正，未增加抑制。完整 `cargo fmt --all -- --check`、`cargo check --locked --workspace --all-targets`、`cargo test --locked --workspace -- --test-threads=1` 及 `cargo clippy --locked --workspace --all-targets -- -D warnings` 全部通过：369 项，0 失败、0 忽略，含 Engine unit 58、controls 12、staging 13、validation 12、Server unit 29、managed 10、persistence 54。固定 SHA 审查与合并前后 CI 以对应 PR 记录为准。本轮没有原生 App 或 Windows 验收；Server 多连接未启用，M4 尚未完成。

**Local checks for worker barriers (2026-10-31, macOS)**: all five initial direct segment regressions failed with `d922bda` product code. The final twelve integration tests produced two passes and ten failures before the fix, and all twelve passed afterwards. Real HTTP fixtures cover two workers pausing across header/body waits, exactly one GET per segment after resume, stable paused bytes/progress, cancellation during header/body/pause waits, a paused publication coordinator, low-bandwidth failure/cancellation, and callback errors/panics. Six private unit tests cover pause/first-error interruption after budget reservation, thread-start failure, worker panic, two successive pause generations, and bounded runtime shutdown; production workers use those same control/runtime helpers. The first package Clippy run reported two nested-condition style issues; these were corrected without suppression. Complete formatting, workspace check, workspace test (`--test-threads=1`) and all-targets Clippy `-D warnings` passed: 369 tests, zero failures and zero ignored, including Engine unit 58, controls 12, staging 13, validation 12, Server unit 29, managed 10 and persistence 54. The corresponding PR records fixed-SHA review and CI before/after merge. This slice includes no native App or Windows acceptance; Server segmentation stays disabled and M4 remains incomplete.

**完成协议切片本地验证（2026-10-31，macOS）**：新增带凭证的分段入口及 13 项直接集成测试，另以 11 项新增私有单元场景覆盖控制移交、摘要暂停／取消、提交竞争及凭证写入后的故障。真实 Server 进程在 HTTP fixture 结束后消费新入口生成的已发布／暂存凭证，验证离线恢复、精确最终进度和 SQLite 完成状态，并保留无关 sidecar；这不启用 Server 分段派发。完整格式、workspace all-targets check、串行 workspace test 及 all-targets Clippy `-D warnings` 全通过：394 项，0 失败、0 忽略，含 Engine unit 69、completion 13、controls 12、staging 13、validation 12，Server unit 29、managed 10、persistence 55。另补充取消清理失败后的控制状态断言；新增回归在 `d9c3c22` 上确认两入口的六种早期错误都未标记控制结束，修复后全部通过并重跑完整门槛。首次消费者测试误将 TaskView 的顶层进度字段当作嵌套字段，已按真实 RPC 合约更正并保留字节及数据库相等断言；首次 Clippy 的大错误类型提示通过装箱私有凭证解决，未增加抑制。基线没有新 API，因此不把编译失败记作行为复现。固定 SHA 审查及合并前后 CI 以对应 PR 为准；无 Windows 或原生 App 验收，M4 未完成。

**Local checks for the completion-protocol slice (2026-10-31, macOS)**: the receipt-aware entry has thirteen direct integration tests, plus eleven new private unit scenarios covering handoff, hash pause/cancellation, commit races and faults after receipt publication. A real Server process consumes published/staged receipts generated by the new entry after its HTTP fixture stops, checking offline recovery, exact final progress and SQLite completion while preserving unowned sidecars; segment dispatch remains disabled. Complete formatting, workspace all-targets check, serial workspace tests and all-targets Clippy `-D warnings` passed: 394 tests, zero failures/ignored, including Engine unit 69, completion 13, controls 12, staging 13, validation 12, Server unit 29, managed 10 and persistence 55. Cancellation-cleanup-failure tests also gained cancelled/uncommitted assertions. A new regression at `d9c3c22` found all six early-error cases across both entries left controls unfinished; the fix passes these cases and the complete gates were rerun. The initial consumer test incorrectly treated flat TaskView progress fields as nested; it was corrected to the actual RPC contract while preserving byte/database equality assertions. The initial Clippy large-error finding was resolved by boxing the private receipt, without suppression. The baseline has no new API, so compilation failure is not counted as a behavioral reproduction. The corresponding PR records fixed-SHA review and pre/post-merge CI; no Windows or native App acceptance is claimed, and M4 remains incomplete.

<a id="m5--general-transfer-runtime-contract-and-ftp"></a>
<a id="m5--通用传输运行契约与-ftp"></a>

<a id="m5"></a>

### M5 — 通用传输运行契约与 FTP / M5 — General transfer runtime contract and FTP

- **先决设计**：为长期运行的 Engine 定义能力声明、异步启动/进度、暂停/取消、重试、恢复元数据、错误和 Server 派发契约。现有 `EngineAdapter::start` 同步返回，而实际 HTTP 控制位于 Server Worker；先让 HTTP 通过新契约并保持 M2–M4 行为，再增加新协议。明确 FTP/FTPS 与凭据的支持范围。

  **Design first:** Define capability declarations, asynchronous start/progress, pause/cancel, retry, recovery metadata, errors, and server dispatch for long-running engines. The current `EngineAdapter::start` returns synchronously while real HTTP control lives in a separate server worker. Move HTTP through the new contract without regressing M2–M4 before adding another protocol. Decide the FTP/FTPS and credential support scope explicitly.
- **验收**：真实 FTP 来源可通过 CLI 和 Desktop 创建、排队、下载、暂停、恢复、取消及重启恢复；服务端不支持续传时安全地从零开始。认证失败、网络断开和目标冲突有可见错误，完成文件可校验。HTTP 回归测试保持通过。

  **Acceptance:** A real FTP source can be created, queued, downloaded, paused, resumed, canceled, and recovered after restart from CLI and Desktop; restart safely from byte zero when the server does not support resume. Authentication failures, disconnects, and destination conflicts have visible errors; completed files can be verified. HTTP regression tests remain green.
- **记录进度（契约与 HTTP 初步实现，待验收）**：[ADR 0008](decisions/0008-long-running-transfer-contract.md) 已记录长期传输契约：`EngineCapabilities` 新增 `supports_range`、`supports_retry`、`supported_protocols`、`max_concurrent_segments`，`TransferSession` 提供 `progress()`、`state()`、`error()`、`pause()`、`resume()`、`cancel()`、`wait()`，`ResumableEngine` 与 `ResumeMetadata` 已定义；同步 `EngineAdapter::start()` 弃用但保留给简单引擎。HTTP 会话和 Server 初步调用已存在。**缺口**：`ResumableEngine` 尚无可用实现；Server 仍直接选择 HTTP/HTTPS，没有按能力声明路由到 Engine Registry；PR #58 已修复并验证单连接会话控制、进度和完成恢复，多连接仍需单独验证；FTP/FTPS 及其凭据、续传、控制与重启恢复尚未实现。设计记录和类型声明不等于 M5 完成。

  **Recorded progress (contract and initial HTTP implementation, awaiting acceptance):** [ADR 0008](decisions/0008-long-running-transfer-contract.md) records the long-running contract: `EngineCapabilities` adds `supports_range`, `supports_retry`, `supported_protocols`, and `max_concurrent_segments`; `TransferSession` provides `progress()`, `state()`, `error()`, `pause()`, `resume()`, `cancel()`, and `wait()`; `ResumableEngine` and `ResumeMetadata` are defined. Synchronous `EngineAdapter::start()` is deprecated but retained for simple engines. Initial HTTP session and Server calls exist. **Gaps:** `ResumableEngine` has no usable implementation; the Server still directly selects HTTP/HTTPS without routing through Engine Registry capability declarations; PR #58 repaired and verified single-connection session controls, progress, and completion recovery; multi-connection execution still needs separate validation; FTP/FTPS credentials, resume, controls, and restart recovery are unimplemented. Recorded design and type declarations do not complete M5.

<a id="m6--torrent-and-magnet-downloads"></a>
<a id="m6--bt-种子与磁力下载"></a>

<a id="m6"></a>

### M6 — BT 种子与磁力下载 / M6 — Torrent and magnet downloads

- **工作**：在 M5 契约上实现种子元数据、Magnet 元数据获取、文件选择、Tracker/Peer 管理、分片校验、任务持久化和恢复；明确做种、分享率和网络权限策略。

  **Work:** Build on M5 with torrent metadata, magnet metadata retrieval, file selection, Tracker/Peer management, piece verification, task persistence, and recovery. Decide seeding, ratio, and network-permission policies.
- **验收**：受控种子与 Magnet fixture 能从 CLI 和 Desktop 下载选定文件；暂停、重启和失败重试后分片校验及最终文件哈希正确。Tracker 状态和错误可见；未实现的做种能力不得标成已支持。

  **Acceptance:** Controlled torrent and magnet fixtures download selected files through CLI and Desktop. Piece verification and final hashes remain correct after pause, restart, and failure retry. Tracker state and errors are visible; unimplemented seeding must not be advertised as supported.

<a id="m7--browser-and-macos-system-integration"></a>
<a id="m7--browser-与-macos-系统集成"></a>

<a id="m7"></a>

### M7 — Browser 与 macOS 系统集成 / M7 — Browser and macOS system integration

- **依赖**：基于 M1 的受控默认下载目录和受保护的本地连接，在本阶段设计浏览器配对；扩展不能自行拼接任意 Server 文件路径。

  **Dependency:** Build on M1's controlled default download directory and protected local connection, then design browser pairing in this milestone. The extension must not construct arbitrary server-side file paths.
- **工作**：为扩展增加可信配对、凭据存储、目标目录选择或 Server 管理的默认目录；提交锁文件并纳入 CI，按实际支持的浏览器分别打包验收。补齐 macOS 菜单栏、通知、打开文件/文件夹、下载链接接管和退出行为；在 M6 完成后处理 `magnet:` 与 `.torrent` 系统关联。

  **Work:** Add trusted pairing, credential storage, destination selection or a server-managed default directory to the extension. Commit a lockfile, include the extension in CI, and package/test each browser that is claimed as supported. Add macOS menu-bar controls, notifications, open file/folder actions, download URL handling, and defined quit behavior. Handle `magnet:` and `.torrent` system associations after M6 is complete.
- **验收**：在启用鉴权的已安装 App 中，浏览器可投递链接到预期目录并看到任务结果；无权限的来源不能写入任务。宣称支持的浏览器及系统入口分别有端到端记录。

  **Acceptance:** From an installed app with authentication enabled, a browser submits a link to the expected directory and shows its task result; unauthorized origins cannot create tasks. Each claimed browser and system entry point has its own end-to-end evidence.

<a id="m8--windowslinux-delivery"></a>
<a id="m8--windowslinux-交付"></a>

<a id="m8"></a>

### M8 — Windows/Linux 交付 / M8 — Windows/Linux delivery

- **依赖**：移植 M1 的本机服务生命周期和 M3 的发布流程，为 M7 的系统集成提供平台对应实现；不得沿用 macOS Keychain 假设。

  **Dependency:** Port M1's local server lifecycle and M3's release flow, and provide platform equivalents for M7's system integration; macOS Keychain assumptions cannot carry over unchanged.
- **验收**：每个宣称支持的平台分别通过凭据存储、路径、安装包、升级、系统集成和 HTTP/FTP/BT 下载的原生检查。未完成验收的平台只列为开发目标。

  **Acceptance:** Each claimed platform separately passes native checks for credential storage, paths, installers, upgrades, system integration, and HTTP/FTP/BT downloads. An unvalidated platform remains a development target.

<a id="m9--executable-plugin-ecosystem-later-product-decision"></a>
<a id="m9--可执行插件生态后续产品决策"></a>

<a id="m9"></a>

### M9 — 可执行插件生态（后续产品决策） / M9 — Executable plugin ecosystem (later product decision)

- **工作**：内建传输路径稳定后，确定插件 SDK、装载/卸载、版本兼容、权限隔离和故障边界；接通现有 Provider，而非仅改变 PluginManager 状态。是否提供插件市场另作产品决策。

  **Work:** Once built-in transfer paths are stable, decide the plugin SDK, loading/unloading, version compatibility, permission isolation, and failure boundaries. Wire the existing providers instead of only changing PluginManager state. A plugin marketplace is a separate product decision.
- **验收**：至少一个真实 Engine 或 Resolver 插件完成安装、调用、权限拒绝、故障隔离和卸载的端到端验证，才称为插件支持。

  **Acceptance:** At least one real Engine or Resolver plugin passes end-to-end installation, invocation, permission denial, fault isolation, and removal before plugin support is claimed.

<a id="3-scope-and-immediate-next-step"></a>
<a id="3-范围与下一步"></a>

## 3. 范围与下一步 / 3. Scope and immediate next step

官网部署按上述 M3 工作包推进：沿用 GitHub Pages 默认项目地址，固定发布元数据契约并建设中英文官网与预览产物流程；正式下载随 M3 验收通过后上线，R2 镜像按实际流量需求后续安排。

Website deployment follows the M3 work package above: use the default GitHub Pages project address, fix the release metadata contract and build the bilingual website/preview-artifact flow. Official downloads go live after M3 acceptance passes; schedule an R2 mirror later according to actual traffic needs.

Motrix 清单中的自动 Tracker 列表、UPnP/NAT-PMP、上传限速、登录时启动、统计 Dashboard 和远程/headless 控制仍需单独决定范围；本计划不把它们算作 M0–M7 的已承诺交付。Media/Automation、AI/MCP、远程设备、手动主题与强调色也不在首个 macOS 下载器发布门槛中。本地文件来源是否需要导入/传输路径另作产品决策。可靠事件回放、显式 CA/证书固定和更严格的协议兼容性检查按多客户端或远程连接的实际需求立项；当前事件断线后通过完整任务快照恢复，[ADR 0005](decisions/0005-tls-transport.md)记录已实现的 TLS 边界。以上项目不会借由已有数据模型或页面占位被标成可用功能。

Motrix features such as automatic Tracker lists, UPnP/NAT-PMP, upload limits, launch at login, a statistics Dashboard, and remote/headless control need separate scope decisions; this plan does not count them as committed M0–M7 deliverables. Media/Automation, AI/MCP, remote devices, and manual themes/accent colors are also outside the first macOS downloader release gate. Whether local-file sources need an import/transfer path is a separate product decision. Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks should be scheduled for concrete multi-client or remote-connection needs. Clients currently recover from a disconnected event stream by refreshing a full task snapshot; [ADR 0005](decisions/0005-tls-transport.md) records the implemented TLS boundary. Existing data models or page placeholders alone do not make these features usable.

**契约工作包（部分实现、待验收）**：[ADR 0008](decisions/0008-long-running-transfer-contract.md) 已记录 `EngineCapabilities`、`TransferSession`、`ResumableEngine` 与 `ResumeMetadata`，但能力路由与完整恢复接线仍未完成，FTP/FTPS 未实现。状态以 M5 的缺口和验收为准，不能把文档和 trait 定义当成已交付协议。

**Contract work package (partial, awaiting acceptance):** [ADR 0008](decisions/0008-long-running-transfer-contract.md) records `EngineCapabilities`, `TransferSession`, `ResumableEngine`, and `ResumeMetadata`; capability routing and complete recovery wiring remain unfinished, and FTP/FTPS is unimplemented. M5 gaps and acceptance criteria determine delivery; documentation and trait definitions do not establish a shipped protocol.

**M5 HTTP 会话实现状态**：`HttpTransferSession` 在后台线程运行，回调更新进度，`HttpTransferControl` 提供控制，`wait(Box<Self>)` 等待终态。`f1817f4` 已修复会话传入完成上下文、整文件续传进度、具体失败原因及启动完成对账，并通过独立复核、完整门槛与合并后 CI。PR #61 已统一大文件和小文件的 Server 会话入口；能力路由、通用恢复实现及 FTP/FTPS 仍待完成。基线 `3768c3a` 的 40 项通过、11 项失败记录保留为历史，不再视为当前结果。

**M5 HTTP session implementation status:** `HttpTransferSession` runs in a background thread, callbacks update progress, `HttpTransferControl` provides controls, and `wait(Box<Self>)` waits for termination. `f1817f4` repaired completion-context wiring, whole-file resume progress, concrete failures, and startup completion reconciliation, passing independent review, full gates, and post-merge CI. PR #61 unified Server session routing for both large and small files. Capability routing, generic recovery implementations, and FTP/FTPS remain unfinished. The `3768c3a` record of 40 passes and 11 failures remains historical evidence, not the current result.

**下一工作包**：不重复实现 #66 完成协议、#71 恢复就绪或已实现的 #69 UI。近期完成 M0 VoiceOver 与最终候选的必要原生复验、M1 全新 OS 用户／异常中断／升级，以及 M3 Developer ID 签名、公证、安装与升级验收；官网生产发布遵循 GitHub Pages 的精确 main-CI 屏障，真实部署证据独立记录，正式下载继续等待 M3。M4 依据 RFC 0002 提案先取得可验证的平台身份安全机制，再实现竞态保护、有界回退、持久段恢复及真实 Server／CLI／App routing 验收，多连接在此前保持禁用。M5–M9 继续按上述依赖推进，不能因类型、模型、提案或页面占位而标为完成。

**Next work package:** do not repeat #66 completion protocols, #71 recovery readiness or implemented #69 UI. Complete M0 VoiceOver and necessary native rechecks on the final candidate, M1 clean-OS-user/abnormal-interruption/upgrade checks, and M3 Developer ID signing, notarization, installation and upgrade acceptance. Website production publication follows the GitHub Pages exact-main-CI barrier with deployment evidence recorded separately; official downloads still wait for M3. For M4, obtain verifiable platform identity safety under the RFC 0002 proposal before race protection, bounded fallback, durable segment recovery and real Server/CLI/App routing acceptance; keep multi-connection disabled until then. M5–M9 follow the dependencies above; types, models, proposals and page placeholders do not complete them.
