<a id="nexum-development-plan"></a>
<a id="nexum-开发计划"></a>

# Nexum 开发计划 / Nexum Development Plan

导航 / Contents: [1. 当前起点与 Motrix 差距 / 1. Current baseline and Motrix gaps](#1-current-baseline-and-motrix-gaps) · [2. 按交付物推进的里程碑 / 2. Delivery milestones](#2-delivery-milestones) · [3. 范围与下一步 / 3. Scope and immediate next step](#3-scope-and-immediate-next-step)

本计划以 2026 年 10 月 9 日的代码为起点，按“先交付可独立使用的 macOS 下载器，再补齐下载能力和生态集成”排序。对标范围参考 [Motrix 官方功能清单](https://github.com/agalwood/Motrix/blob/main/README.md)及其 [v2.0.0-beta.46 发布页](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46)；这是功能差距清单，不代表已逐项实测 Motrix，也不承诺 Nexum 会逐项复制其实现或发布时间。

This plan starts from the code as of October 9, 2026, and orders work around a standalone macOS downloader first, followed by download capabilities and ecosystem integration. The comparison uses the [official Motrix feature list](https://github.com/agalwood/Motrix/blob/main/README.md) and its [v2.0.0-beta.46 release](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46). It is a feature-gap reference, not a claim that we tested every Motrix feature or a commitment to copy every implementation or release on a fixed date.

“已有”表示代码路径已接通，“待验收”表示实现存在但尚缺指定产物的实际检查，“计划”表示还没有可用的端到端实现。只有通过相应里程碑的验收条件，才把能力写成已交付。当前调用路径见[架构设计](ARCHITECTURE.md)，构建、截图与发布操作见[发布流程](RELEASE.md)，TLS 的已完成决策见 [ADR 0005](decisions/0005-tls-transport.md)。

“Present” means a code path is wired, “needs validation” means an implementation lacks a check of the stated artifact, and “planned” means there is no usable end-to-end implementation. A capability is delivered only after its milestone acceptance criteria pass. See [Architecture](ARCHITECTURE.md) for current call paths, [Release Process](RELEASE.md) for build, screenshot, and release steps, and [ADR 0005](decisions/0005-tls-transport.md) for the completed TLS decision.

状态核对 / Status reconciliation（2026-10-15，代码基线 `f9ce5b6`）：PR #58 修复单连接会话、续传与完成恢复；[PR #61](https://github.com/liveait/Nexum/pull/61) 将所有大小的 HTTP 下载统一到此路径，合并提交的 [main push CI](https://github.com/liveait/Nexum/actions/runs/38722240545) 中 Rust 与 Desktop macOS 均通过。M4 多连接及原生发布验收仍未完成。

Status reconciliation (2026-10-15, code baseline `f9ce5b6`): PR #58 repaired single-connection sessions, resume, and completion recovery. [PR #61](https://github.com/liveait/Nexum/pull/61) routes HTTP downloads of every size through this path, with Rust and Desktop macOS passing in the merged commit's [main push CI](https://github.com/liveait/Nexum/actions/runs/38722240545). M4 multi-connection and native release acceptance remain incomplete.

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
- **已记录进展**：10 月 11 日实现 Server 的专用托管启动入口、私有分帧控制／就绪协议、随机回环端口、管道传递的强制 Bearer 鉴权和关闭／EOF 退出；隔离测试检查鉴权、数据锁及停顿下载恢复。细节见 [ADR 0006](decisions/0006-managed-macos-server.md#首个实现切片server-托管启动协议)。后续切片已接入 Tauri 管理者、包内哈希校验、仅后端持有的路由／鉴权、托管／外部设置、后端任务 ID 和不冲突目标，以及关闭窗口隐藏和退出停止。原生 codeload 下载、冲突保留、退出／重开、一次子进程崩溃恢复、外部模式隔离及保存目录快捷键检查均通过；产物哈希和截图记录在[发布流程](RELEASE.md#托管-server-原生验收)。后续[完成对账切片](decisions/0007-http-completion-reconciliation.md)通过隔离 engine 和 Server 进程测试覆盖完整文件／持久化状态窗口。欢迎流程、`check_fresh_install`、`get_upgrade_info` 和 `last_seen_version` 已接入源码，但源码存在不等于全新用户／升级验收通过；部分文件元数据清理也必须在最终下载路径复验。当前 `open_core` 在监听前对待完成凭证执行整文件摘要校验，偏离 ADR 0007 的就绪后对账设计；大文件可能超过托管 Server 六秒就绪预算，需要后续修复并验收。此前中文版本的“M1 完成”与英文版本不一致，现统一为 **部分实现、最终验收待完成**。

  **Recorded progress:** On October 11, the Server gained a dedicated managed entry point, private framed control/readiness protocol, ephemeral loopback port, mandatory pipe-delivered Bearer authentication, and Shutdown/EOF exit. Isolated tests check authentication, data-directory locking, and stalled-download recovery. See [ADR 0006](decisions/0006-managed-macos-server.md#first-implementation-slice-managed-server-bootstrap). The following slice has wired the Tauri supervisor, bundle hash verification, backend-only route/authentication, Managed/External settings, backend task IDs and collision-safe destinations, close-to-hide and Quit shutdown. Native codeload download, collision preservation, Quit/relaunch, one child crash recovery, External isolation and saved-folder shortcut checks passed; artifact hashes and screenshots are recorded in [Release Process](RELEASE.md#managed-server-native-qa). The subsequent [completion-reconciliation slice](decisions/0007-http-completion-reconciliation.md) covers the complete-file/persisted-state window in isolated engine and Server process tests. The source includes a welcome flow, `check_fresh_install`, `get_upgrade_info`, and `last_seen_version`, but implementation presence does not verify clean-user/upgrade acceptance; partial-metadata cleanup also needs verification through the final download path. Current `open_core` hashes pending completion files before listening, diverging from ADR 0007's post-readiness design; large files may exceed the managed Server's six-second readiness budget, requiring a follow-up fix and acceptance. The former Chinese completion claim conflicted with this version and is corrected to **partially implemented, final acceptance pending**.

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

- **范围与依赖**：使用 Cloudflare Pages 托管独立的中英文静态官网，首版包含产品功能与截图、下载、文档入口、更新日志，以及源码和反馈入口。官网准备可与 M3 签名工作并行；首发 App 的原生验收仍优先，正式下载上线须等待实际 App 通过 M3。

  **Scope and dependency:** Host an independent English/Chinese static website on Cloudflare Pages. The first version includes product features and screenshots, downloads, documentation links, a changelog, and source/feedback links. Website preparation may overlap with M3 signing work; native acceptance of the first App remains the priority, and official downloads go live only after the actual App passes M3.
- **首版下载**：官方下载链接指向 GitHub Releases，自动同步公开发布元数据。下载页的版本、支持的 OS/CPU 架构、包大小、SHA256、签名与公证状态以实际发布产物为准；未完成验收的平台不得显示可下载安装。同步所需的私有 token 不得交给浏览器。

  **Initial downloads:** Link official downloads to GitHub Releases and automatically synchronize public release metadata. The download page's version, supported OS/CPU architectures, package size, SHA256, signing, and notarization status must reflect actual release artifacts. An unvalidated platform must not offer an installable download. Private tokens used for synchronization must not reach the browser.
- **部署工作**：分别配置 PR 预览和 `main` 生产站，确定官网域名并配置 HTTPS；预览构建与生产发布使用各自的地址和发布流程。

  **Deployment work:** Configure PR previews and the `main` production site separately, decide the website domain, and configure HTTPS. Preview builds and production releases use their own addresses and publication flows.
- **验收**：中英文路径、语言切换及页面链接均可用；从官网链接下载真实安装包，检查 SHA256、签名和公证状态与发布记录一致，并完成安装启动检查。PR 预览不会更新生产站或生产下载元数据。

  **Acceptance:** English/Chinese paths, language switching, and page links work. Download a real installer through the website link, verify its SHA256, signing, and notarization status against release records, and complete install/launch checks. PR previews do not update the production site or its download metadata.
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
- **记录进度（未启用、待验收）**：Engine 中已有 `probe_range()`、`split_segments()` 和 `download_to_segments()` 原型。`f9ce5b6` 基线审查发现分段路径缺少响应范围及身份校验、可信 staging、完整暂停/退出屏障及完成凭证；只测试拆段和探测辅助方法不能验收分段下载。PR #61 已撤去 Server 自动分段入口及基于 `127.0.0.1` 的测试地址绕行，所有来源统一使用已验证的会话下载路径。PR #62 已补齐输入与响应校验，通过完整 Rust 门槛、独立审查及合并前后 CI，合并为 `3fcd259`。当前切片补齐持有的暂存句柄与同步边界身份检查；路径身份检查到 rename／unlink 的竞态仍未封闭，其余保护仍待实现。PR #58 的完成恢复修复已交付，但不会自动修复分段原型。**缺口**：以下安全执行切片、段级持久状态、跨重启段恢复及有界单连接回退，全部完成直接行为验收前不重新启用多连接。

  **Recorded progress (disabled, awaiting acceptance):** The Engine contains `probe_range()`, `split_segments()`, and `download_to_segments()` prototypes. Review of baseline `f9ce5b6` found missing response range/identity validation, trusted staging, complete pause/termination barriers, and completion receipts on the segment path. Tests of splitting and probing alone do not accept segment downloads. PR #61 removed automatic Server segment selection and the `127.0.0.1` fixture bypass so every source uses the validated session path. PR #62 added input and response validation, passing the full Rust gates, independent review and CI before/after merge as `3fcd259`. The current slice adds held staging handles and identity checks at synchronization boundaries; the race between path identity checks and rename/unlink is still open. The remaining protections are unfinished. PR #58 delivered completion recovery without repairing the segment prototype. **Gaps:** the safety slice below, durable segment state, cross-restart segment reuse, and bounded single-connection fallback need direct behavioral acceptance before multi-connection routing is re-enabled.

#### M4 实施切片 / M4 implementation slices

1. **已交付：统一安全执行路径。** 基于 `f1817f4`，先移除 Server 自动选择未验收分段下载及按 fixture URL 绕过探测的逻辑，所有来源使用已有单连接会话、完成凭证与持锁发布。用超过 1 MiB 的真实子进程 fixture 验证不发额外探测请求、文件内容、最终进度、完成保存失败后重启对账及既有目标保护。PR #61 已通过完整 Rust 门槛、独立审查及合并前后 CI；此切片已交付，M4 仍未完成。

   **Delivered: one safe execution path.** From `f1817f4`, remove automatic selection of the unvalidated segment path and fixture-URL probe bypasses. All sources use the existing single-connection session, completion receipts, and locked publication. A real subprocess fixture above 1 MiB must verify no extra probe request, exact bytes, final progress, recovery after failed completion persistence, and existing-destination protection. PR #61 passed the full Rust gates, independent review, and CI before and after merge. This slice is delivered; M4 remains incomplete.
2. **已交付：输入与响应校验（PR #62，基线 `f9ce5b6`）。** 在网络请求和创建／截断暂存文件前，要求非空、按序、完整且无重叠的段集合，拒绝越界与整数溢出；零字节文件不分段，拆段使用整数计算。分段原型仅接受语法有效的强 ETag，不修改单连接的 ETag/Last-Modified 兼容策略。探测必须校验精确 `bytes 0-0/total`、实际一字节及 EOF；每段校验 `206`、精确起止／总长、相同 ETag、identity 表示及实际响应长度；传输编码仅允许缺省或单一 `chunked`，拒绝未知、多值或重复编码。直接调用 Engine 分段 API 覆盖正常拼接和异常拒绝，保留具体错误。该切片不接回 Server；完整文件／控制／凭证门槛见下一项。

   **Delivered: input and response validation (PR #62, baseline `f9ce5b6`).** Before network requests or staging creation/truncation, require nonempty, ordered, complete and nonoverlapping segments; reject out-of-bounds ranges and integer overflow. Do not segment empty files; use integer splitting. The segment prototype accepts only syntactically valid strong ETags, preserving the single-connection ETag/Last-Modified policy. Probes must validate exact `bytes 0-0/total`, one actual byte and EOF; each segment validates `206`, exact bounds/total, a matching ETag, an identity representation and actual response length. Transfer-Encoding is limited to absent or a single `chunked` value, rejecting unknown, listed or duplicate codings. Exercise the segment Engine API directly for successful assembly and rejection cases, retaining specific errors. This slice does not restore Server routing; the next item retains the complete file/control/receipt gate.
3. **当前切片：持有暂存句柄与身份检查（基线 `3fcd259`）。** 以 `create_new` 创建普通文件，拒绝并保留已有普通文件、符号链接、FIFO 及目录；持有同一个文件句柄，所有 Worker 使用显式偏移写入。请求／各次写入前、所有 Worker join 后及发布／清理前核对文件身份，最终长度从持有句柄读取。已观察到替换时保留替换项；清理只尝试删除自己创建且身份匹配的部分文件，不触碰未拥有的 JSON sidecar。直接测试并发精确拼接、响应延迟／写入中／最终进度回调中的路径替换，以及正常失败／取消清理。边界：检查与基于路径的 rename／hard-link／unlink 之间仍有 TOCTOU，不能宣称原子文件所有权；该缺口保留为启用多连接的阻断门槛。

   **Current slice: held staging handles and identity checks (baseline `3fcd259`).** Create a regular file with `create_new`, rejecting and preserving pre-existing regular files, symlinks, FIFOs and directories. Retain one file handle and use explicit offsets in every worker. Check identity before requests/writes, after all workers join and before publication/cleanup; read the final length from the held handle. Preserve observed replacements; cleanup only attempts to remove the identity-matching partial created by this attempt, without touching unowned JSON sidecars. Directly test exact concurrent assembly, path replacement during delayed responses, mid-write and final progress callbacks, and ordinary failure/cancellation cleanup. Boundary: TOCTOU remains between checks and path-based rename/hard-link/unlink, so this is not atomic file ownership. Keep that gap as a blocker for enabling multiple connections.
4. **下一切片：所有 Worker 暂停／退出屏障。** 保证暂停确认覆盖全部 Worker，任一失败、取消或进度回调失败后所有 Worker 有界退出，清理和完成通知在退出后发生。之后补齐完成凭证、原子文件所有权与取消／发布，以及安全且有界的单连接回退。

   **Next slice: all-worker pause/termination barriers.** Acknowledge pause only after every worker is paused; bound termination after any worker failure, cancellation or progress callback failure, and clean up/notify completion only after termination. Then add completion receipts, atomic file ownership and cancellation/publication, and bounded safe single-connection fallback.
5. **恢复多连接前的门槛。** 校验完整无重叠的段集合及响应范围/validator，使用持有的可信 staging 文件，保证所有 worker 的暂停与失败退出、实际字节计数、完成凭证和原子取消/发布；直接测试分段 API 和 Server 消费路径，再接通有界、安全的单连接回退。

   **Gate before re-enabling multiple connections.** Validate complete, nonoverlapping segment coverage and response ranges/validators; retain a trusted staging handle. Verify all-worker pause and failure termination, actual byte counts, completion receipts, and atomic cancellation/publication. Test both the segment API and Server consumer before wiring bounded, safe single-connection fallback.
6. **段级恢复。** 在前项稳定后设计并实现持久段状态、身份/摘要校验及跨重启段复用；验收前中断的多连接任务不得宣称可按段续传。

   **Segment recovery.** Once the previous slice is stable, design and implement durable segment state, identity/digest checks, and cross-restart segment reuse. Do not claim per-segment restart recovery before acceptance.

**安全入口切片本地验证（2026-10-15，macOS）**：新增 3 项大文件 Server 回归在旧产品代码 `f1817f4` 上全部失败，修复后全部通过；验证单次普通 GET、精确文件内容及最终进度、完成保存失败后的离线对账和已有目标保护。`cargo fmt --all -- --check`、`cargo check --locked --workspace --all-targets`、`cargo test --locked --workspace -- --test-threads=1`、`cargo clippy --locked --workspace --all-targets -- -D warnings` 全部通过；workspace 共 326 项，含 Server unit 29、managed 10、persistence 54。此记录不包含原生 App 验收；独立审查和 CI 以对应 PR 的最终 SHA 为准。

**Local checks for the safe-routing slice (2026-10-15, macOS)**: all three new large-file Server regressions failed with the old `f1817f4` product code and passed after the fix. They verify one ordinary GET, exact bytes and final progress, offline reconciliation after failed completion persistence, and existing-destination protection. `cargo fmt --all -- --check`, `cargo check --locked --workspace --all-targets`, `cargo test --locked --workspace -- --test-threads=1`, and `cargo clippy --locked --workspace --all-targets -- -D warnings` passed: 326 workspace tests, including Server unit 29, managed 10, and persistence 54. This is not native App acceptance; the corresponding PR records independent review and CI for its final SHA.

**输入与响应校验本地验证（2026-10-15，macOS）**：首批 8 项直接回归在 `f9ce5b6` 产品代码上为 1 项通过、7 项失败；修复并补充边界后，12 项直接集成测试全部通过，包括合法 chunked 探测／拼接、请求 identity 协商及错误范围／校验器／长度／重复头拒绝。完整格式、workspace check、workspace test（`--test-threads=1`）及 Clippy `-D warnings` 门槛全部通过；共 338 项，0 失败、0 忽略，含 Engine unit 52、分段集成 12、Server unit 29、managed 10、persistence 54。独立审查新增的 2 项传输编码回归在 `f13bcec` 产品代码上均失败，修复后通过；仅接受缺省或单一 `chunked`。首轮 Clippy 的测试字符串格式提示已修正。旧 probe fixture 补回缺失的 CRLF，保留原成功／长度断言，新增真实 HTTP 用例覆盖等效行为。此项不包含原生 App 验收；最终独立审查 SHA 与合并前后 CI 记录在对应 PR，M4 仍未完成。

**Local checks for input and response validation (2026-10-15, macOS)**: the first eight direct regressions produced one pass and seven failures with `f9ce5b6` product code. After the fix and boundary additions, all twelve direct integration tests passed, including valid chunked probes/assembly, identity request negotiation, and rejection of invalid ranges, validators, lengths and duplicate headers. Formatting, workspace check, workspace test (`--test-threads=1`), and Clippy `-D warnings` all passed: 338 tests, zero failures and zero ignored, including Engine unit 52, segment integration 12, Server unit 29, managed 10 and persistence 54. Both additional transfer-coding regressions from independent review failed on `f13bcec` product code and passed after the fix; only absent or a single `chunked` value is accepted. The initial Clippy warning about test string formatting was corrected. Missing CRLFs in old probe fixtures were restored while retaining success/length assertions; new real HTTP cases cover the same behavior. This is not native App acceptance. The corresponding PR records the final independently reviewed SHA and CI before/after merge; M4 remains incomplete.

**暂存句柄切片聚焦验证（2026-10-15，macOS）**：13 项直接 Engine 暂存回归在 `3fcd259` 产品代码上为 3 项通过、10 项失败，修复后全部通过；通过 channel／Condvar 同步路径替换与 16 段并发响应，不依赖固定等待猜测写入时机。`cargo test --locked -p nexum-engine -- --test-threads=1` 通过 52 项 unit、13 项 staging 集成及 12 项校验集成；Engine all-targets Clippy `-D warnings` 与格式检查通过。完整格式、workspace check、workspace test（`--test-threads=1`）及 all-targets Clippy `-D warnings` 全部通过：351 项，0 失败、0 忽略，含 Server unit 29、managed 10、persistence 54。最终并发 fixture 修改后，13 项 staging 集成测试再次通过。固定 SHA 独立审查及合并前后 CI 记录在对应 PR。未执行 Windows 偏移写入或原生 App 验收；M4 未完成，多连接未启用。

**Focused checks for held staging (2026-10-15, macOS)**: the thirteen direct Engine staging regressions produced three passes and ten failures with `3fcd259` product code and all passed after the fix. Channels/Condvars synchronize path replacement and sixteen concurrent segment responses, without fixed waits to guess write timing. `cargo test --locked -p nexum-engine -- --test-threads=1` passed 52 unit, 13 staging integration and 12 validation integration tests. Engine all-targets Clippy `-D warnings` and formatting passed. Formatting, workspace check, workspace test (`--test-threads=1`) and all-targets Clippy `-D warnings` passed: 351 tests, zero failures and zero ignored, including Server unit 29, managed 10 and persistence 54. After the final concurrent-fixture update, all 13 staging integration tests passed again. The corresponding PR records fixed-SHA independent review and CI before/after merge. Windows offset writes and native App acceptance were not executed; M4 remains incomplete and multi-connection routing disabled.

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

官网部署按上述 M3 工作包推进：先确定域名和发布元数据契约，再建设中英文官网与预览流程；正式下载随 M3 验收通过后上线，R2 镜像按实际流量需求后续安排。

Website deployment follows the M3 work package above: decide the domain and release metadata contract, then build the English/Chinese website and preview flow. Official downloads go live after M3 acceptance passes; schedule an R2 mirror later according to actual traffic needs.

Motrix 清单中的自动 Tracker 列表、UPnP/NAT-PMP、上传限速、登录时启动、统计 Dashboard 和远程/headless 控制仍需单独决定范围；本计划不把它们算作 M0–M7 的已承诺交付。Media/Automation、AI/MCP、远程设备、手动主题与强调色也不在首个 macOS 下载器发布门槛中。本地文件来源是否需要导入/传输路径另作产品决策。可靠事件回放、显式 CA/证书固定和更严格的协议兼容性检查按多客户端或远程连接的实际需求立项；当前事件断线后通过完整任务快照恢复，[ADR 0005](decisions/0005-tls-transport.md)记录已实现的 TLS 边界。以上项目不会借由已有数据模型或页面占位被标成可用功能。

Motrix features such as automatic Tracker lists, UPnP/NAT-PMP, upload limits, launch at login, a statistics Dashboard, and remote/headless control need separate scope decisions; this plan does not count them as committed M0–M7 deliverables. Media/Automation, AI/MCP, remote devices, and manual themes/accent colors are also outside the first macOS downloader release gate. Whether local-file sources need an import/transfer path is a separate product decision. Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks should be scheduled for concrete multi-client or remote-connection needs. Clients currently recover from a disconnected event stream by refreshing a full task snapshot; [ADR 0005](decisions/0005-tls-transport.md) records the implemented TLS boundary. Existing data models or page placeholders alone do not make these features usable.

**契约工作包（部分实现、待验收）**：[ADR 0008](decisions/0008-long-running-transfer-contract.md) 已记录 `EngineCapabilities`、`TransferSession`、`ResumableEngine` 与 `ResumeMetadata`，但能力路由与完整恢复接线仍未完成，FTP/FTPS 未实现。状态以 M5 的缺口和验收为准，不能把文档和 trait 定义当成已交付协议。

**Contract work package (partial, awaiting acceptance):** [ADR 0008](decisions/0008-long-running-transfer-contract.md) records `EngineCapabilities`, `TransferSession`, `ResumableEngine`, and `ResumeMetadata`; capability routing and complete recovery wiring remain unfinished, and FTP/FTPS is unimplemented. M5 gaps and acceptance criteria determine delivery; documentation and trait definitions do not establish a shipped protocol.

**M5 HTTP 会话实现状态**：`HttpTransferSession` 在后台线程运行，回调更新进度，`HttpTransferControl` 提供控制，`wait(Box<Self>)` 等待终态。`f1817f4` 已修复会话传入完成上下文、整文件续传进度、具体失败原因及启动完成对账，并通过独立复核、完整门槛与合并后 CI。PR #61 已统一大文件和小文件的 Server 会话入口；能力路由、通用恢复实现及 FTP/FTPS 仍待完成。基线 `3768c3a` 的 40 项通过、11 项失败记录保留为历史，不再视为当前结果。

**M5 HTTP session implementation status:** `HttpTransferSession` runs in a background thread, callbacks update progress, `HttpTransferControl` provides controls, and `wait(Box<Self>)` waits for termination. `f1817f4` repaired completion-context wiring, whole-file resume progress, concrete failures, and startup completion reconciliation, passing independent review, full gates, and post-merge CI. PR #61 unified Server session routing for both large and small files. Capability routing, generic recovery implementations, and FTP/FTPS remain unfinished. The `3768c3a` record of 40 passes and 11 failures remains historical evidence, not the current result.

**下一工作包**：PR #61 已交付安全入口，PR #62 已交付分段输入／响应校验。当前完成暂存句柄及身份检查的小切片后，下一项是所有 Worker 暂停／失败退出屏障；随后补齐完成凭证、原子文件所有权与取消／发布、安全回退，再实现段级持久化与跨重启恢复。身份检查到基于路径的 rename／unlink 的 TOCTOU 仍是多连接重新启用的阻断项。M5 能力路由及 FTP/FTPS 后续推进。M0 VoiceOver 和 M1 全新用户／中断／升级最终验收仍待完成；M3 签名、公证和分发保持计划状态。官网按 M3 准备，正式下载等待实际发布产物验收。

**Next work package:** PR #61 delivered safe routing and PR #62 delivered segment input/response validation. After the current held-staging/identity-check slice, add all-worker pause/failure termination barriers, then completion receipts, atomic file ownership and cancellation/publication, safe fallback, durable segment state and restart recovery. TOCTOU between identity checks and path-based rename/unlink remains a blocker for re-enabling multi-connection routing. M5 capability routing and FTP/FTPS follow. M0 VoiceOver and M1 final clean-user/interruption/upgrade acceptance remain open; M3 signing, notarization and distribution remain planned. Prepare the website under M3, with official downloads waiting for acceptance of the actual artifact.
