<a id="nexum-development-plan"></a>
<a id="nexum-开发计划"></a>

# Nexum 开发计划 / Nexum Development Plan

导航 / Contents: [1. 当前起点与 Motrix 差距 / 1. Current baseline and Motrix gaps](#1-current-baseline-and-motrix-gaps) · [2. 按交付物推进的里程碑 / 2. Delivery milestones](#2-delivery-milestones) · [3. 范围与下一步 / 3. Scope and immediate next step](#3-scope-and-immediate-next-step)

本计划以 2026 年 10 月 9 日的代码为起点，按“先交付可独立使用的 macOS 下载器，再补齐下载能力和生态集成”排序。对标范围参考 [Motrix 官方功能清单](https://github.com/agalwood/Motrix/blob/main/README.zh-CN.md)及其 [v2.0.0-beta.46 发布页](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46)；这是功能差距清单，不代表已逐项实测 Motrix，也不承诺 Nexum 会逐项复制其实现或发布时间。

This plan starts from the code as of October 9, 2026, and orders work around a standalone macOS downloader first, followed by download capabilities and ecosystem integration. The comparison uses the [official Motrix feature list](https://github.com/agalwood/Motrix/blob/main/README.zh-CN.md) and its [v2.0.0-beta.46 release](https://github.com/agalwood/Motrix/releases/tag/v2.0.0-beta.46). It is a feature-gap reference, not a claim that we tested every Motrix feature or a commitment to copy every implementation or release on a fixed date.

“已有”表示代码路径已接通，“待验收”表示实现存在但尚缺指定产物的实际检查，“计划”表示还没有可用的端到端实现。只有通过相应里程碑的验收条件，才把能力写成已交付。当前调用路径见[架构设计](ARCHITECTURE.zh-CN.md)，构建、截图与发布操作见[发布流程](RELEASE.zh-CN.md)，TLS 的已完成决策见 [ADR 0005](decisions/0005-tls-transport.zh-CN.md)。

“Present” means a code path is wired, “needs validation” means an implementation lacks a check of the stated artifact, and “planned” means there is no usable end-to-end implementation. A capability is delivered only after its milestone acceptance criteria pass. See [Architecture](ARCHITECTURE.md) for current call paths, [Release Process](RELEASE.md) for build, screenshot, and release steps, and [ADR 0005](decisions/0005-tls-transport.md) for the completed TLS decision.

状态核对 / Status reconciliation（2026-10-14，文档基线 `33bd16d`）：计划的初始能力对照来自 10 月 9 日；之后的实现和验收记录按下文区分。当前 HTTP 回归仍待最终 SHA 的修复与验收；PR #58 的存在或 CI 状态不能代替独立复验及合并后 `main` CI。

Status reconciliation (2026-10-14, documentation baseline `33bd16d`): the original capability comparison dates from October 9; later implementation and acceptance records are distinguished below. Current HTTP regressions await fixes and verification at the final SHA; the existence or CI status of PR #58 does not replace independent revalidation and post-merge main CI.

<a id="1-current-baseline-and-motrix-gaps"></a>
<a id="1-当前起点与-motrix-差距"></a>

## 1. 当前起点与 Motrix 差距 / 1. Current baseline and Motrix gaps

| 领域<br>Area | Nexum 当前状态<br>Nexum today | 尚需补齐<br>Remaining gap |
| --- | --- | --- |
| 任务与 HTTP/HTTPS<br>Tasks and HTTP/HTTPS | Server 使用 SQLite 保存并恢复任务；排队、并发槽位、重试、进度、暂停和删除已接通。单连接 HTTP 下载有暂存文件及经 ETag/Last-Modified 校验的跨重启续传。<br>The server stores and recovers tasks in SQLite. Queueing, concurrency slots, retries, progress, pause, and removal are wired. Single-connection HTTP downloads use staged files and cross-restart resume validated by ETag/Last-Modified. | 真实速率与预计时间、实际传输限速、阻塞读取期间及时暂停及删除时及时中断 Worker，以及安全的单文件多连接下载。没有校验器的未完成响应从零重下；凭证证明完整的字节可在本地对账。<br>Real speed and ETA, limits applied to transfers, prompt pause and worker cancellation on removal during a blocked read, and safe multi-connection downloads of one file. Incomplete responses without validators restart from byte zero; receipt-proven complete bytes can reconcile locally. |
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

- **工作**：单独启动使用固定 Tauri CLI 构建的 release App；按[发布流程](RELEASE.zh-CN.md#macos-原生界面截图留证)保存明暗和窄窗口证据；在真实 App 中检查中英文、键盘焦点、VoiceOver 实际朗读，以及“减弱动态效果”下的不确定进度。

  **Work:** Launch the release app built with the pinned Tauri CLI on its own. Capture light, dark, and narrow-window evidence using the [Release Process](RELEASE.md#native-macos-visual-evidence). Check English/Chinese, keyboard focus, actual VoiceOver speech, and indeterminate progress under Reduce Motion in the real app.
- **验收**：同一候选 App 能连接本地 Server、完成已知 HTTP 文件并显示正确状态；上述原生检查有记录。debug App 截图不能替代 release 产物证据。发现的问题先修复并复验。

  **Acceptance:** The same candidate app connects to a local server, completes a known HTTP file, and shows the right state; the native checks have recorded results. Debug-app screenshots do not stand in for release-artifact evidence. Fix and repeat any failing check.
- **已记录进展**：10 月 10 日源码提交 `1df5eab8411081e0177037029025f6e2e81fdfcc` 的 release 候选产物已有明暗及窄窗口截图，完成已知和未知大小任务检查、Sheet/搜索焦点及任务上/下/Home/End/Enter/Space 检查，并保存暂停/减弱动态效果下的进度采样比较。产物身份、采样限制、偏好恢复和已完成的隔离 fixture 清理见[发布流程证据](RELEASE.zh-CN.md#10-月-10-日-release-候选产物原生验收)。M0 尚未完成：仍需确认 VoiceOver 实际朗读。

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
- **已记录进展**：10 月 11 日实现 Server 的专用托管启动入口、私有分帧控制／就绪协议、随机回环端口、管道传递的强制 Bearer 鉴权和关闭／EOF 退出；隔离测试检查鉴权、数据锁及停顿下载恢复。细节见 [ADR 0006](decisions/0006-managed-macos-server.zh-CN.md#首个实现切片server-托管启动协议)。后续切片已接入 Tauri 管理者、包内哈希校验、仅后端持有的路由／鉴权、托管／外部设置、后端任务 ID 和不冲突目标，以及关闭窗口隐藏和退出停止。原生 codeload 下载、冲突保留、退出／重开、一次子进程崩溃恢复、外部模式隔离及保存目录快捷键检查均通过；产物哈希和截图记录在[发布流程](RELEASE.zh-CN.md#托管-server-原生验收)。后续[完成对账切片](decisions/0007-http-completion-reconciliation.zh-CN.md)通过隔离 engine 和 Server 进程测试覆盖完整文件／持久化状态窗口。欢迎流程、`check_fresh_install`、`get_upgrade_info` 和 `last_seen_version` 已接入源码，但源码存在不等于全新用户／升级验收通过；部分文件元数据清理也必须在最终下载路径复验。此前中文版本的“M1 完成”与英文版本不一致，现统一为 **部分实现、最终验收待完成**。

  **Recorded progress:** On October 11, the Server gained a dedicated managed entry point, private framed control/readiness protocol, ephemeral loopback port, mandatory pipe-delivered Bearer authentication, and Shutdown/EOF exit. Isolated tests check authentication, data-directory locking, and stalled-download recovery. See [ADR 0006](decisions/0006-managed-macos-server.md#first-implementation-slice-managed-server-bootstrap). The following slice has wired the Tauri supervisor, bundle hash verification, backend-only route/authentication, Managed/External settings, backend task IDs and collision-safe destinations, close-to-hide and Quit shutdown. Native codeload download, collision preservation, Quit/relaunch, one child crash recovery, External isolation and saved-folder shortcut checks passed; artifact hashes and screenshots are recorded in [Release Process](RELEASE.md#managed-server-native-qa). The subsequent [completion-reconciliation slice](decisions/0007-http-completion-reconciliation.md) covers the complete-file/persisted-state window in isolated engine and Server process tests. The source includes a welcome flow, `check_fresh_install`, `get_upgrade_info`, and `last_seen_version`, but implementation presence does not verify clean-user/upgrade acceptance; partial-metadata cleanup also needs verification through the final download path. The former Chinese completion claim conflicted with this version and is corrected to **partially implemented, final acceptance pending**.

<a id="m2--http-quality-needed-for-the-first-release"></a>
<a id="m2--首发所需的-http-下载质量"></a>

<a id="m2"></a>

### M2 — 首发所需的 HTTP 下载质量 / M2 — HTTP quality needed for the first release

- **工作**：从真实传输字节计算速率；只在长度可信时显示预计时间；定义全局/单任务限速设置并将 Scheduler 算出的上限传到 Worker；让阻塞读取中的 `task.pause` 和 `task.remove`（内部取消 Worker）及时响应，同时保持原目标文件和可恢复的部分文件安全。

  **Work:** Compute speed from actual transferred bytes; show ETA only when the length is trustworthy. Define global/per-task limit settings and pass Scheduler-calculated limits to workers. Make `task.pause` and `task.remove` (which cancels the worker internally) respond promptly during blocked reads while preserving the existing destination and recoverable partial files.
- **验收**：受控本地服务覆盖慢响应、断线、校验器变化、错误 `Content-Range`、多任务竞争、重试及目标冲突。长时间窗口内实测速率符合配置上限；本地停顿响应 fixture 下，暂停和删除各自在 5 秒内完成。成功文件哈希正确；中断、失败和删除不覆盖已有的正确目标文件。

  **Acceptance:** Controlled local servers cover slow responses, disconnects, changed validators, invalid `Content-Range`, competing tasks, retries, and destination conflicts. Measured throughput over a sustained window respects configured limits; pause and removal each finish within 5 seconds against a local stalled-response fixture. Successful files have the correct hashes; interruption, failure, and removal do not overwrite an existing good destination.
- **已记录进展**：速率改用 5 秒滑动窗口加指数平滑，从实际传输字节估算；预计时间只在响应声明可信总长时给出，未知长度（如 chunked）不显示。HTTP 引擎改为异步 reqwest，响应体读取加 3 秒无数据超时（连续 8 次空读判为停滞失败），使暂停／取消在停顿读取时于秒级内生效，而非等满 30 分钟请求超时。新增 `bandwidth` 设置（`unlimited`／`fixed <bytes/sec>`／`shared <total bytes/sec>`，可用 config 或 `--bandwidth`），Scheduler 按活跃任务数算出每连接上限并下发到 Worker，活跃数变化（入队／暂停／恢复／移除／完成）时刷新；共享策略把总带宽在并发任务间均分。Desktop 在下载行与详情里显示实时速率，长度可信时才显示 ETA。文件安全沿用完成对账：成功文件哈希正确，中断／失败／删除不覆盖已正确的目标文件。引擎单元（速率／ETA／限速／停顿打断）、Server 集成（fixed 限速实测、shared 双任务竞争、停顿下暂停／删除 5 秒内返回、目标冲突、无覆盖）与 Desktop 类型及逻辑检查全部通过（2026-10-13）。**历史切片检查通过（2026-10-13）；当前 HTTP 路径待复验。** 后续 M4/M5 改动已引入范围验证、文件保护、最终进度和完成恢复回归，历史结果不能作为当前提交的验收证据。

  **Recorded progress:** Speed now uses a 5-second sliding window with light exponential smoothing, estimated from bytes actually transferred; ETA is reported only when the response declares a trustworthy total length (not for unknown-length/chunked bodies). The HTTP engine moved to async reqwest, with a 3-second no-data read timeout (eight consecutive empty reads fail the transfer as stalled) so pause/cancel take effect within seconds on a stalled read instead of waiting out the full 30-minute request timeout. A new `bandwidth` setting (`unlimited` / `fixed <bytes/sec>` / `shared <total bytes/sec>`, via config or `--bandwidth`) lets the Scheduler compute a per-connection cap from the live active-task count and push it to workers, re-applied whenever the active count changes (queue/pause/resume/remove/complete); the shared policy divides the total across concurrent tasks. Desktop shows live speed in the download row and inspector, and ETA only when the length is trustworthy. File safety reuses completion reconciliation: successful files hash correctly, and interruption/failure/removal never overwrite an existing good destination. Engine unit tests (speed/ETA/pacing/stall-break), Server integration tests (fixed-cap measurement, shared two-task competition, stalled pause/remove within 5s, destination conflict, no-clobber), and the Desktop type-check and logic tests all pass (2026-10-13). **The earlier slice passed its recorded checks on 2026-10-13; the current HTTP path awaits revalidation.** Subsequent M4/M5 changes introduced regressions in range validation, file protection, final progress, and completion recovery; historical results do not establish acceptance of the current commit.

<a id="m3--installable-and-trusted-macos-distribution"></a>
<a id="m3--可安装可信任的-macos-发布"></a>

<a id="m3"></a>

### M3 — 可安装、可信任的 macOS 发布 / M3 — Installable and trusted macOS distribution

- **工作**：固定构建输入和版本，产出适合声明架构的安装包；完成 Developer ID 签名、公证、Staple、Gatekeeper、安装/升级/卸载与干净机器冒烟。发布工作流和更新策略须与实际支持范围一致。

  **Work:** Pin build inputs and versions, produce an installer for each claimed architecture, and complete Developer ID signing, notarization, stapling, Gatekeeper, install/upgrade/uninstall, and clean-machine smoke checks. Release workflow and update policy must match the actual support scope.
- **验收**：用户从发布产物安装并双击启动，无需开发工具或终端，即可完成 M1–M2 的 HTTP 下载；系统验证签名和公证通过，升级保留任务数据。将版本、哈希、支持的 macOS/CPU 架构和未完成的 FTP/BT/扩展能力写入发布说明。操作步骤与证据留在[发布流程](RELEASE.zh-CN.md)。

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
- **记录进度（部分实现、待验收）**：已有 `probe_range()`、`split_segments()` 和 `download_to_segments()`，后者预分配部分文件并为各段启动线程和 current-thread Tokio runtime，共享 `HttpTransferControl`，段失败时中止其他工作线程。Server 已有超过 1 MiB 且探测到 Range 支持时选择分段的接线；未满足条件时选单连接。现有拆段和探测辅助测试不能证明真实分段执行正确，原“246+ 项全通过/M4 完成”声明撤回。**缺口**：没有段级持久状态、已完成段的跨重启恢复，或段失败后的安全单连接回退；范围/校验器验证、提交前取消、文件保护和完成对账仍需真实分段及 Server 调用路径的复现验证。PR #58 的修复工作不等于这些验收已完成。

  **Recorded progress (partial, awaiting acceptance):** `probe_range()`, `split_segments()`, and `download_to_segments()` exist. The segment path preallocates a partial file and starts a thread with a current-thread Tokio runtime for each segment, sharing `HttpTransferControl` and aborting other workers on a segment failure. The Server has wiring to select segments when a file exceeds 1 MiB and probing reports Range support, otherwise selecting one connection. Splitting/probing helper tests do not verify the real segment execution path; the former “246+ tests pass/M4 complete” claim is withdrawn. **Gaps:** no durable segment-level state, restart reuse of completed segments, or safe single-connection fallback after a segment failure. Range/validator checks, cancellation before publication, file protection, and completion reconciliation still require direct segment and Server-path reproductions. Fixes being prepared in PR #58 do not constitute acceptance.

<a id="m5--general-transfer-runtime-contract-and-ftp"></a>
<a id="m5--通用传输运行契约与-ftp"></a>

<a id="m5"></a>

### M5 — 通用传输运行契约与 FTP / M5 — General transfer runtime contract and FTP

- **先决设计**：为长期运行的 Engine 定义能力声明、异步启动/进度、暂停/取消、重试、恢复元数据、错误和 Server 派发契约。现有 `EngineAdapter::start` 同步返回，而实际 HTTP 控制位于 Server Worker；先让 HTTP 通过新契约并保持 M2–M4 行为，再增加新协议。明确 FTP/FTPS 与凭据的支持范围。

  **Design first:** Define capability declarations, asynchronous start/progress, pause/cancel, retry, recovery metadata, errors, and server dispatch for long-running engines. The current `EngineAdapter::start` returns synchronously while real HTTP control lives in a separate server worker. Move HTTP through the new contract without regressing M2–M4 before adding another protocol. Decide the FTP/FTPS and credential support scope explicitly.
- **验收**：真实 FTP 来源可通过 CLI 和 Desktop 创建、排队、下载、暂停、恢复、取消及重启恢复；服务端不支持续传时安全地从零开始。认证失败、网络断开和目标冲突有可见错误，完成文件可校验。HTTP 回归测试保持通过。

  **Acceptance:** A real FTP source can be created, queued, downloaded, paused, resumed, canceled, and recovered after restart from CLI and Desktop; restart safely from byte zero when the server does not support resume. Authentication failures, disconnects, and destination conflicts have visible errors; completed files can be verified. HTTP regression tests remain green.
- **记录进度（契约与 HTTP 初步实现，待验收）**：[ADR 0008](decisions/0008-long-running-transfer-contract.zh-CN.md) 已记录长期传输契约：`EngineCapabilities` 新增 `supports_range`、`supports_retry`、`supported_protocols`、`max_concurrent_segments`，`TransferSession` 提供 `progress()`、`state()`、`error()`、`pause()`、`resume()`、`cancel()`、`wait()`，`ResumableEngine` 与 `ResumeMetadata` 已定义；同步 `EngineAdapter::start()` 弃用但保留给简单引擎。HTTP 会话和 Server 初步调用已存在。**缺口**：`ResumableEngine` 尚无可用实现；Server 仍直接选择 HTTP/HTTPS，没有按能力声明路由到 Engine Registry；会话控制、进度和完成恢复必须先修复并通过当前 SHA 的完整 HTTP 回归；FTP/FTPS 及其凭据、续传、控制与重启恢复尚未实现。设计记录和类型声明不等于 M5 完成。

  **Recorded progress (contract and initial HTTP implementation, awaiting acceptance):** [ADR 0008](decisions/0008-long-running-transfer-contract.md) records the long-running contract: `EngineCapabilities` adds `supports_range`, `supports_retry`, `supported_protocols`, and `max_concurrent_segments`; `TransferSession` provides `progress()`, `state()`, `error()`, `pause()`, `resume()`, `cancel()`, and `wait()`; `ResumableEngine` and `ResumeMetadata` are defined. Synchronous `EngineAdapter::start()` is deprecated but retained for simple engines. Initial HTTP session and Server calls exist. **Gaps:** `ResumableEngine` has no usable implementation; the Server still directly selects HTTP/HTTPS without routing through Engine Registry capability declarations; session controls, progress, and completion recovery need fixes and complete HTTP regressions on the current SHA; FTP/FTPS credentials, resume, controls, and restart recovery are unimplemented. Recorded design and type declarations do not complete M5.

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

Motrix 清单中的自动 Tracker 列表、UPnP/NAT-PMP、上传限速、登录时启动、统计 Dashboard 和远程/headless 控制仍需单独决定范围；本计划不把它们算作 M0–M7 的已承诺交付。Media/Automation、AI/MCP、远程设备、手动主题与强调色也不在首个 macOS 下载器发布门槛中。本地文件来源是否需要导入/传输路径另作产品决策。可靠事件回放、显式 CA/证书固定和更严格的协议兼容性检查按多客户端或远程连接的实际需求立项；当前事件断线后通过完整任务快照恢复，[ADR 0005](decisions/0005-tls-transport.zh-CN.md)记录已实现的 TLS 边界。以上项目不会借由已有数据模型或页面占位被标成可用功能。

Motrix features such as automatic Tracker lists, UPnP/NAT-PMP, upload limits, launch at login, a statistics Dashboard, and remote/headless control need separate scope decisions; this plan does not count them as committed M0–M7 deliverables. Media/Automation, AI/MCP, remote devices, and manual themes/accent colors are also outside the first macOS downloader release gate. Whether local-file sources need an import/transfer path is a separate product decision. Durable event replay, explicit CA/certificate pinning, and stricter protocol compatibility checks should be scheduled for concrete multi-client or remote-connection needs. Clients currently recover from a disconnected event stream by refreshing a full task snapshot; [ADR 0005](decisions/0005-tls-transport.md) records the implemented TLS boundary. Existing data models or page placeholders alone do not make these features usable.

**契约工作包（部分实现、待验收）**：[ADR 0008](decisions/0008-long-running-transfer-contract.zh-CN.md) 已记录 `EngineCapabilities`、`TransferSession`、`ResumableEngine` 与 `ResumeMetadata`，但能力路由与完整恢复接线仍未完成，FTP/FTPS 未实现。状态以 M5 的缺口和验收为准，不能把文档和 trait 定义当成已交付协议。

**Contract work package (partial, awaiting acceptance):** [ADR 0008](decisions/0008-long-running-transfer-contract.md) records `EngineCapabilities`, `TransferSession`, `ResumableEngine`, and `ResumeMetadata`; capability routing and complete recovery wiring remain unfinished, and FTP/FTPS is unimplemented. M5 gaps and acceptance criteria determine delivery; documentation and trait definitions do not establish a shipped protocol.

**M5 HTTP 会话初步实现**：`HttpTransferSession` 持有克隆的 `reqwest::Client`、源 URL 和目标路径，在后台线程运行；`progress()` 读取由回调更新的 `Mutex<Progress>`，控制方法使用 `HttpTransferControl`，`wait(Box<Self>)` 通过 `Condvar` 等待终态。这描述已存在的结构，不保证控制、续传和完成恢复正确；原文引用的 `download_to_with_control_sealed` 已不在当前实现中，“52 engine/51 Server 测试全部通过”也不能用作当前代码的证明。基线 `3768c3a` 的持久化复验为 40 通过、11 失败，4 项针对性复现全部失败。PR #58 需按最终 SHA 重新独立审查、跑完整门槛，并核对合并后的 `main` push CI，才可更新验收状态。

**Initial M5 HTTP session implementation:** `HttpTransferSession` owns a cloned `reqwest::Client`, source URL, and destination and runs in a background thread. `progress()` reads callback-updated `Mutex<Progress>`; controls use `HttpTransferControl`; `wait(Box<Self>)` waits for a terminal state through a `Condvar`. These structures do not establish correct controls, resume, or completion recovery. The former `download_to_with_control_sealed` reference is absent from the current implementation, and “all 52 engine/51 Server tests pass” is not evidence for current code. Revalidation of baseline `3768c3a` produced 40 passing and 11 failing persistence tests, with all four targeted reproductions failing. PR #58 needs independent review at its final SHA, complete gates, and post-merge main push CI before acceptance status changes.

**下一工作包**：先完成并独立验证 PR #58 的 HTTP 回归修复，恢复完整 Server 测试门槛；随后补齐 M4 段级持久化、跨重启段恢复和安全单连接回退，以及 M5 会话控制与能力路由，再评估 FTP/FTPS。M0 实际 VoiceOver 朗读与 M1 全新用户／中断／升级最终验收仍待完成；M3 签名、公证和分发保持计划状态。官网可按 M3 配套工作包准备，正式下载须等待实际发布产物验收。

**Next work package:** Finish and independently verify PR #58 HTTP regression fixes first, restoring the full Server gate. Then complete M4 durable segment state, cross-restart segment recovery, and safe single-connection fallback, plus M5 session controls and capability routing, before evaluating FTP/FTPS. M0 actual VoiceOver speech and M1 final clean-user/interruption/upgrade acceptance remain open; M3 signing, notarization, and distribution remain planned. Website preparation may follow its M3 work package, with official downloads waiting for acceptance of the actual release artifact.
