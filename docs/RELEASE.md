<a id="release-process"></a>
<a id="发布流程"></a>

# 发布流程 / Release Process

导航 / Contents: [当前状态 / Current Status](#current-status) · [手动发布检查表 / Manual Release Checklist](#manual-release-checklist)

<a id="current-status"></a>
<a id="当前状态"></a>

## 当前状态 / Current Status

仓库目前没有自动发布工作流，当前检出也没有版本标签。`.github/workflows/ci.yml` 在推送和 Pull Request 上执行 Rust 格式、检查、测试和 Clippy。新增的 macOS job 安装 pnpm 11.25.0 和 Node.js 22，使用冻结锁文件安装 Desktop 依赖，构建前端及 macOS `.app`，并检查语言声明、包内 Server 的哈希／版本／架构及嵌套签名，以及本机临时签名的 App 签名。CI 仍不构建 Browser 扩展，不进行分发签名或公证，也不会发布 GitHub Release。各包与应用清单目前声明 `0.1.0`，而变更日志仍将该版本标为 TBD。

There is no automated release workflow or version tag in this checkout. `.github/workflows/ci.yml` runs Rust formatting, check, tests, and Clippy on pushes and pull requests. Its macOS job installs pnpm 11.25.0 and Node.js 22, installs the Desktop dependencies from the frozen lockfile, builds the frontend and a macOS `.app`, and checks language declarations, the bundled Server hash/version/architecture and nested signature, and the locally ad-hoc-signed App signature. CI does not build the Browser extension, sign or notarize for distribution, or publish a GitHub Release. The packages and app manifests currently declare `0.1.0`, while the changelog still marks that release as TBD.

`Cargo.lock` 和 `apps/desktop/pnpm-lock.yaml` 已纳入 Git。Desktop 固定 pnpm 11.25.0 和项目内 Tauri CLI 2.11.5，构建时应使用冻结锁文件。Browser 扩展仍没有已跟踪的锁文件，全新检出可能解析到不同的前端依赖版本。构建候选版本前，应让两个已跟踪的锁文件与对应清单保持同步。

`Cargo.lock` and `apps/desktop/pnpm-lock.yaml` are tracked. The Desktop package pins pnpm 11.25.0 and the project-local Tauri CLI 2.11.5; use the frozen lockfile for Desktop builds. The Browser extension still has no tracked lockfile and can resolve different frontend dependency versions in a fresh checkout. Keep both tracked lockfiles synchronized with their manifests before building a release candidate.

验收基线说明（2026-10-14）：`3768c3a` 的 CI 实际排除了 Server unit/persistence，之后的流程文档提交未修复这些代码。下文有明确日期／产物身份的测试和原生记录是历史证据；范围验证、文件保护、最终进度和完成恢复当前需用修复后的最终 SHA 复验，并检查实际合并提交的 `main` push CI，才可发布。

Acceptance baseline (2026-10-14): CI at `3768c3a` excluded Server unit/persistence tests, and the subsequent workflow-documentation commit did not fix that code. Dated test/native records below identify historical artifacts. Revalidate range checks, file protection, final progress, and completion recovery at the final fixed SHA, then check main push CI on the actual merged commit before publishing.

<a id="manual-release-checklist"></a>
<a id="手动发布检查表"></a>

## 手动发布检查表 / Manual Release Checklist

以下步骤说明当前源码树生成候选版本所需的工作。完成验证后，再人工决定打标签和发布。

The following steps describe what the current source tree requires for a release candidate. Tagging and publishing are manual decisions after validation.

1. 在 Rust 包清单（`crates/*/Cargo.toml`、`apps/server/Cargo.toml`、`apps/cli/Cargo.toml` 和 `apps/desktop/src-tauri/Cargo.toml`）中设置目标版本，并保持 `apps/desktop/package.json`、`apps/desktop/src-tauri/tauri.conf.json`、`apps/extension/package.json`、`apps/extension/manifest.json` 的版本一致。JSON-RPC 协议版本是独立的兼容性编号。

   Set the intended version in the Rust package manifests (`crates/*/Cargo.toml`, `apps/server/Cargo.toml`, `apps/cli/Cargo.toml`, and `apps/desktop/src-tauri/Cargo.toml`), and keep `apps/desktop/package.json`, `apps/desktop/src-tauri/tauri.conf.json`, `apps/extension/package.json`, and `apps/extension/manifest.json` in sync. The JSON-RPC protocol version is a separate compatibility number.
2. 将发布内容从 `Unreleased` 移至 双语 `CHANGELOG.md` 中带日期的版本条目。

   Move the release notes from `Unreleased` into a dated entry in the shared bilingual `CHANGELOG.md`.
3. 在仓库根目录运行[开发指南](DEVELOPMENT.md#rust-检查)列出的四条 Rust CI 命令。

   Run the four Rust CI commands from the repository root as listed in the [Development Guide](DEVELOPMENT.md#rust-checks).
4. 使用 pnpm 11.25.0，在 `apps/desktop` 运行 `pnpm install --frozen-lockfile && pnpm build`。macOS CI job 会执行这两条命令。在 `apps/extension` 运行 `pnpm install && pnpm lint && pnpm build`；Desktop 锁文件和 CI job 均不覆盖扩展。

   With pnpm 11.25.0, run `pnpm install --frozen-lockfile && pnpm build` in `apps/desktop`. The macOS CI job runs both commands. In `apps/extension`, run `pnpm install && pnpm lint && pnpm build`; the extension is not covered by the Desktop lockfile or CI job.
5. 运行 `cargo build --locked --release -p nexum-server -p nexum-cli` 构建 Rust 命令行产物；可执行文件为 `target/release/nexum-server` 和 `target/release/nexum-cli`。在 `apps/desktop` 执行 `pnpm exec tauri build --bundles app`，通过项目内固定版本的 Tauri CLI 构建 macOS App，产物位于仓库根目录的 `target/release/bundle/macos/Nexum.app`。其他平台或安装包目标需分别构建和验收。

   Build the Rust command-line artifacts with `cargo build --locked --release -p nexum-server -p nexum-cli`. The binaries are `target/release/nexum-server` and `target/release/nexum-cli`. From `apps/desktop`, run `pnpm exec tauri build --bundles app` to build the macOS app with the pinned, project-local Tauri CLI. Its output is `target/release/bundle/macos/Nexum.app` at the repository root. Build and validate any other platform or installer target separately.
6. 打包扩展时包含根目录 `manifest.json`、`icons/` 和生成的 `dist/`。清单同时引用后两处的文件，仅有 `dist/` 无法作为扩展加载。

   Package the extension with its root `manifest.json`, `icons/`, and generated `dist/` directory. The manifest points to files in both locations, so `dist/` alone is not a loadable extension.
7. 在默认托管模式下启动原生 App，不另开终端 Server；粘贴 HTTP(S) URL，检查系统 Downloads 或所选目录、可编辑文件名、同名文件分配，以及退出／重开后的任务与文件。检查关闭窗口后台下载、Dock 重开、有限崩溃恢复和手动重试。另验外部模式的地址／Keychain，并确认切换和退出不停止外部 Server。然后使用本地 Server 对独立 CLI／Server 流程进行冒烟测试。通过 HTTP 提供一个已知文件，创建任务并将其 URL 加入队列；自动派发的 Worker 阻塞期间查询 `task.get` 或 `task.list`，确认能看到中间字节数；`task.start` 仍可作为排队 HTTP 任务的可选手动 kick。Worker 完成后核对目标文件字节和已持久化的 `Completed` 状态。确认失败传输保留已有目标文件，并通过任务 `error` 字段返回错误；在重试预算未耗尽时自动重试，并在预算耗尽后使用同一 `--data-dir` 重启 Server 仍保留最终错误。中断的 `Downloading` 任务恢复为 `Queued` 并清除旧错误；存在稳定部分文件和匹配校验器时，恢复的 HTTP/HTTPS 任务发送 `Range`/`If-Range` 并从已验证偏移完成；校验器缺失或变化、返回 `200`、`Content-Range` 无效或长度不一致时，从零重启。使用分块本地 fixture 验证活跃任务的 `task.pause` 会等待 `Paused`，`task.resume` 会继续同一个 Worker，`task.remove` 会取消 Worker，最多等待 30 秒后删除任务并保留原有目标文件。记录阻塞中的响应读取可能让 `task.pause` 等到 30 分钟 HTTP 超时；删除超时会返回错误，Worker 退出后可以重试。验证没有 ETag 或 Last-Modified 的响应在中断后会从零开始，并确认成功、取消和 `task.remove` 会删除部分文件及 sidecar。确认 Server 会拒绝数据目录内和重叠的目标。自动化 TLS 测试在运行时生成证书，覆盖 TCP RPC、HTTPS `/jsonrpc` `OPTIONS`/`POST`、Browser 扩展 CORS 允许/拒绝、TCP/HTTPS 上的配置鉴权、带鉴权的 `events.subscribe`、明文拒绝、启动时拒绝不完整或无效的证书/私钥材料，以及握手时拒绝不受信任或主机名不匹配的证书。CLI 和 Desktop 测试覆盖连接失败后的凭据门控；CLI 测试还验证 TLS 失败后不会重试明文。关闭认证时，从 `apps/extension` 加载 Browser 扩展，发送已知 HTTP 链接，确认不带 scheme 的地址使用 HTTP `/jsonrpc` 桥接；启用 TLS 的 Server 上保存 `tls://host:port`，确认扩展使用 HTTPS，浏览器拒绝不受信任的证书，且不会重试 HTTP。验证网页 `Origin` 被拒绝，而扩展 Origin 在 HTTP 和 HTTPS 上都能收到 CORS 头。

   Launch the native bundle in default Managed mode without a separate terminal Server. Paste an HTTP(S) URL, verify the system Downloads or selected folder, editable filename, collision-safe allocation, and task/file persistence across Quit/relaunch. Check window-close background behavior, Dock reopen, bounded crash recovery and manual Retry. Verify External mode separately, including its address/Keychain and that switching or quitting does not stop that Server. Then smoke-test the standalone CLI/Server path against a local server. Serve a known file over HTTP, create and queue its URL, then query `task.get` or `task.list` while the automatically dispatched worker is blocked to confirm an intermediate byte count; `task.start` remains available as an optional manual kick for a queued HTTP task. Check that the destination bytes and persisted `Completed` state match after the worker finishes. Verify that a failed transfer preserves any existing destination, exposes its message through the task `error` field, automatically retries while the retry budget remains, and retains the final error after the retry budget is exhausted and a restart with the same `--data-dir`; an interrupted `Downloading` task returns to `Queued` while clearing the previous error; when a stable partial and matching validator exist, the recovered HTTP/HTTPS task sends `Range`/`If-Range` and completes from the validated offset, while a missing or changed validator, `200` response, invalid `Content-Range`, or inconsistent length causes a byte-zero restart. With a chunked local fixture, verify that active `task.pause` waits for `Paused`, `task.resume` continues the same worker, and active `task.remove` cancels the worker, waits up to 30 seconds for it to exit, removes the task, and preserves the previous destination. Record that a blocking response read can delay `task.pause` until the 30-minute HTTP timeout; a timed-out remove returns an error and can be retried after the worker exits. Verify that a response without ETag or Last-Modified restarts from byte zero after interruption, and that success, cancellation, and `task remove` delete the partial file and sidecar. Check that the server rejects data-directory and overlapping destinations. The automated TLS tests generate certificates at runtime and cover successful TCP RPC, HTTPS `/jsonrpc` `OPTIONS`/`POST`, browser-extension CORS allow/reject behavior, configured TCP/HTTPS authentication, authenticated `events.subscribe`, plaintext rejection, startup rejection for incomplete or invalid certificate/key material, and handshake rejection for untrusted or wrong-hostname certificates. CLI and Desktop tests cover credential gating after failed connections; CLI tests also verify that TLS failures never retry plaintext. With authentication disabled, load the Browser extension from `apps/extension`, send a known HTTP link, and confirm that a bare address uses the HTTP `/jsonrpc` bridge; with a TLS-enabled server, save `tls://host:port` and confirm the extension uses HTTPS, the browser rejects an untrusted certificate, and no HTTP retry occurs. Verify that a web-page `Origin` is rejected while an extension `Origin` receives CORS headers over both HTTP and HTTPS.
8. 在 macOS 原生 Desktop 构建中验证键盘和可访问性行为：`⌘N` 打开 Add Download 并聚焦 Source URL；Tab 和 Shift+Tab 保持在 Sheet 内；Escape 关闭并恢复焦点。`⌘F` 打开或聚焦 Downloads 搜索，Escape 关闭搜索并恢复焦点。使用 Enter/Space 选择任务，并用上/下方向键及 Home/End 在可见任务间移动。通过 VoiceOver 检查任务操作、状态和 Inspector 进度；再启用 macOS“减少动态效果”，确认过渡和不确定进度动画停止。

   On the native macOS Desktop build, verify keyboard and accessibility behavior: `⌘N` opens Add Download with focus on Source URL; Tab and Shift+Tab stay inside the sheet; Escape closes it and returns focus. `⌘F` opens or focuses Downloads search, and Escape closes search and returns focus. Use Enter/Space to select a task and Up/Down/Home/End to move among visible tasks. Check task actions, state, and Inspector progress with VoiceOver, then enable macOS Reduce Motion and confirm transitions and indeterminate progress stop animating.
9. 在该原生 macOS Desktop 构建中保持应用打开，切换 macOS“浅色”和“深色”外观。检查两种模式下的侧边栏、Downloads 任务行及状态徽标、Inspector、设置卡片与输入框、Add Download Sheet 和键盘焦点环，并保存截图以供视觉比较。在“设置 → 外观”依次选择“跟随系统”、English 和“简体中文”，确认可见标签、提示、任务状态、校验消息、通知和 VoiceOver 名称即时切换；重启应用后确认语言选择保留。在 `Nexum.app/Contents/Info.plist` 中确认 `CFBundleLocalizations` 包含 `en` 和 `zh-Hans`。选择“跟随系统”时，在 macOS“语言与地区 → 应用程序”中分别将 Nexum 设为“简体中文”和 English，每次更改后重启应用并确认界面语言；验收后恢复原有的应用语言设置。确认 Server 和操作系统诊断原文不变，切换语言也不会改变任务协议值或请求。

   In that native macOS Desktop build, switch macOS Appearance between Light and Dark while the app is open. Inspect the sidebar, Downloads rows and status badges, Inspector, Settings cards and inputs, Add Download sheet, and keyboard focus rings in both modes; capture screenshots for a visual comparison. In Settings → Appearance, choose Follow system, English, and Simplified Chinese. Confirm that visible labels, tooltips, task states, validation messages, notifications, and VoiceOver names change immediately; restart the app to confirm the selected language persists. Confirm that `Nexum.app/Contents/Info.plist` lists `en` and `zh-Hans` under `CFBundleLocalizations`. With Follow system selected, use macOS Language & Region → Applications to set Nexum to Simplified Chinese and English in turn, relaunching after each change to verify the interface language; restore the original per-app language setting afterward. Verify that Server and operating-system diagnostic text stays verbatim and that changing language does not alter task protocol values or requests.
10. 验证实际产物后，人工创建版本标签和 GitHub Release；在发布说明中记录目标平台及尚未完成的集成。

    After verifying the actual artifacts, create a version tag and GitHub Release manually. Record target platforms and any incomplete integrations in the release notes.

<a id="local-release-build-evidence"></a>
<a id="本地-release-构建证据"></a>

### 本地 release 构建证据 / Local release-build evidence

2026 年 10 月 9 日，本机 macOS release 构建生成了 `target/release/bundle/macos/Nexum.app`、`target/release/nexum-server` 和 `target/release/nexum-cli`。使用固定的 Tauri CLI 2.11.5 构建 Desktop 也已通过。App 的 `Info.plist` 在 `CFBundleLocalizations` 下包含 `en` 和 `zh-Hans`。显式添加本机临时签名后，App 通过 `codesign --verify --deep --strict`；`spctl --assess --type execute` 仍拒绝它。分发签名、公证和 Gatekeeper 放行仍未验收。

On October 9, 2026, local macOS release builds produced `target/release/bundle/macos/Nexum.app`, `target/release/nexum-server`, and `target/release/nexum-cli`. The Desktop build with the pinned Tauri CLI 2.11.5 also completed. The app bundle's `Info.plist` contains `en` and `zh-Hans` under `CFBundleLocalizations`. After an explicit local ad-hoc signature, the app passed `codesign --verify --deep --strict`; `spctl --assess --type execute` rejected it. Distribution signing, notarization, and Gatekeeper acceptance remain unverified.

此前主检出目录的 release App 已显示已完成的 1 MB HTTP 下载，并验证了“跟随系统”下的 macOS 应用专属 English 覆盖：重启后，界面及任务、进度辅助功能名称切换为英文；移除覆盖并重启后恢复简体中文。该语言检查期间，应用的 `settings.json` 哈希未变化。此前隔离 worktree 的检查显示了已完成的已知和未知大小任务，确认测试通知可打开关联任务，并在启用“减弱动态效果”检查添加下载 Sheet 后恢复系统偏好。那些测试任务、文件和通知均已在检查后移除。

Earlier release-app checks from the main checkout displayed a completed 1 MB HTTP download and verified the macOS per-app English override with Follow system selected: UI and accessible task/progress names changed after relaunch, and removing the override restored Simplified Chinese. The app's `settings.json` hash was unchanged during that language check. Earlier isolated-worktree checks displayed completed known-size and unknown-size tasks, confirmed that a test notification opened its related task, and restored system preferences after checking the Add Download sheet with Reduce Motion enabled. Those test tasks, files, and notifications were removed after inspection.

<a id="october-10-release-candidate-native-qa"></a>
<a id="10-月-10-日-release-候选产物原生验收"></a>

#### 10 月 10 日 release 候选产物：原生验收 / October 10 release candidate: native QA

2026 年 10 月 10 日，以源码提交 `1df5eab8411081e0177037029025f6e2e81fdfcc` 使用固定 Tauri CLI 重新构建 release App，添加本机临时签名后通过 `codesign --verify --deep --strict`。[release 截图摘要](assets/native-macos/release-2026-10-10/release-capture-summary.json)记录二进制及前端哈希、截图时间、像素尺寸和图像哈希，用于识别下列检查所用的候选产物。分发签名与公证仍是独立的发布要求。

On October 10, 2026, the release app at source commit `1df5eab8411081e0177037029025f6e2e81fdfcc` was rebuilt with the pinned Tauri CLI, given a local ad-hoc signature, and passed `codesign --verify --deep --strict`. The [release capture summary](assets/native-macos/release-2026-10-10/release-capture-summary.json) records the binary and frontend hashes, capture timestamps, pixel dimensions, and image hashes. This identifies the candidate used for the following checks; distribution signing and notarization remain separate release requirements.

- **键盘**：`⌘N` 打开添加下载并聚焦来源 URL；Tab 和 Shift+Tab 可双向移动，并在 Sheet 边界循环。Escape 关闭 Sheet 并恢复焦点。`⌘F` 打开下载搜索并聚焦输入框，Escape 关闭搜索并恢复焦点。上/下方向键及 Home/End 可移动任务选择。恢复原 Server 连接后，Tab/Shift+Tab 可聚焦任务行；Space 和 Return 分别选中聚焦的任务，并让 Inspector 跟随，未更改任务数据。

  **Keyboard:** `⌘N` opened Add Download with Source URL focused; Tab and Shift+Tab moved in both directions and wrapped at the sheet boundaries. Escape closed the sheet and restored focus. `⌘F` opened Downloads search with its input focused, and Escape closed search and restored focus. Up/Down/Home/End moved task selection. After restoring the original Server connection, Tab/Shift+Tab focused task rows; Space and Return each selected the focused task and updated Inspector without changing task data.
- **任务和通知**：隔离本地 Server 提供了一个已完成的 65,536 字节已知大小任务，以及一个已完成的 90,112 字节、`total_bytes=null` 的任务。两个任务行的进度条均满格；未知大小任务的 Inspector 在原生辅助功能树中显示 100%。打开通知页后未读状态清除；点击暂停通知会选中关联任务。原生截图显示通知卡片使用内容区可用宽度。

  **Tasks and notifications:** An isolated local Server supplied a completed 65,536-byte known-size task and a completed 90,112-byte task with `total_bytes=null`. Both rows had full progress bars; the unknown-size task's Inspector exposed 100% in the native accessibility tree. Opening Notifications cleared its unread state. Clicking a pause notification selected the related task. Native screenshots show notification cards using the available content width.
- **视觉证据**：四组浅色/深色配对覆盖下载列表及 Inspector、设置、来源 URL 已聚焦的添加下载、通知，尺寸均为 2400 × 1602 像素。单独的深色截图记录系统半屏更窄窗口，尺寸为 2058 × 2458 像素。本轮 CUA 截图未测量 point 边界，窄窗口截图也不是同几何尺寸的明暗配对。

  **Visual evidence:** Four Light/Dark pairs cover Downloads with Inspector, Settings, Add Download with Source URL focused, and Notifications at 2400 × 1602 pixels. A separate Dark capture records the system's half-screen narrower window at 2058 × 2458 pixels. Point bounds were not measured for these CUA captures, and the narrower capture is not a matching-geometry pair.
- **动态效果采样**：关闭“减弱动态效果”时，未知大小任务在间隔 33.623 秒的两帧中均为 Paused；开启时，任务在间隔 68.773 秒的两帧中仍为 Downloading。每组的列表及 Inspector 进度轨道像素均一致。这只确认采样时刻的像素相同，不能证明两帧之间持续没有动态效果。[Desktop CSS](../apps/desktop/src/App.css)只为活跃的不确定进度设置动画，并在 `prefers-reduced-motion: reduce` 下关闭该动画；这项源码检查补充了原生采样证据。

  **Motion sampling:** With Reduce Motion off, the unknown-size task was Paused in two frames 33.623 seconds apart. With Reduce Motion on, it remained Downloading in two frames 68.773 seconds apart. The list and Inspector progress-track pixels were identical within each pair. These observations establish equality at the sampled times, not continuous absence of motion between frames. The [Desktop CSS](../apps/desktop/src/App.css) applies animation only to active indeterminate progress and disables it under `prefers-reduced-motion: reduce`; this source check complements the sampled native evidence.
- **恢复与剩余检查**：Server 设置已恢复为 `127.0.0.1:39100`，刷新间隔为 5 秒，语言为 `system`，并从设置文件确认。浅色外观和原窗口尺寸已恢复，原已知文件任务已选中，搜索已关闭，Inspector 保持可见。“减弱动态效果”和“旁白”恢复为关闭，旁白字幕偏好保持原值。工具无法采集旁白音频，两次尝试访问其原生字幕窗口均超时，尚未确认实际朗读内容。M0 仍待该朗读检查。

  **Restoration and remaining checks:** The Server setting was restored to `127.0.0.1:39100`, refresh interval to 5 seconds, and language to `system`, confirmed from the settings file. Light appearance and the previous window size were restored; the original known-file task was selected, search was closed, and Inspector remained visible. Reduce Motion and VoiceOver were restored to off, with the VoiceOver caption preference unchanged. The tools cannot capture VoiceOver audio, and two attempts to access its native caption window timed out; actual spoken output remains unconfirmed. M0 remains open for that speech check.
- **隔离 fixture 清理**：三个 QA 任务均成功移除，隔离任务列表为空。只停止了本轮独立的 39112 端口 Server 和 39113 端口 HTTP fixture；两个端口均已释放，其临时数据/下载目录已删除。原 39100 端口 Server 保持运行，清理前后两个原任务的状态、字节数和错误字段均一致，原已知文件下载仍保留。

  **Isolated fixture cleanup:** All three QA tasks were removed successfully and the isolated task list was empty. Only this run's separate Server on port 39112 and HTTP fixture on port 39113 were stopped; both ports were released and their temporary data/download directories were deleted. The original Server on port 39100 remained running. Its two original tasks had unchanged states, byte counts, and error fields across cleanup, and the original known-file download remained present.

<a id="managed-server-native-qa"></a>
<a id="托管-server-原生验收"></a>

#### 托管 Server 原生验收 / Managed Server native QA

2026 年 10 月 11 日，Desktop 托管 Server 切片构建为原生 release App，包含 `aarch64-apple-darwin` Server，两者版本均为 `0.1.0`，使用本机临时签名。App 与嵌套 Server 均通过严格签名检查；打包检查验证可执行权限、架构、版本和 SHA-256。[托管截图摘要](assets/native-macos/managed-2026-10-11/managed-capture-summary.json)通过源码文件及产物哈希识别基线 `4acbb162384dbf12bc6019b66ea4def86991ba13` 上的实现，并区分生命周期候选产物与修复快捷键依赖后重新构建的最终产物。

On October 11, 2026, the Desktop-managed Server slice was built as a native release App with a bundled `aarch64-apple-darwin` Server, local ad-hoc signatures, and matching `0.1.0` versions. The App and nested Server passed strict signature verification; the bundle checker verified executable permissions, architecture, version and SHA-256. The [managed capture summary](assets/native-macos/managed-2026-10-11/managed-capture-summary.json) identifies the implementation over baseline `4acbb162384dbf12bc6019b66ea4def86991ba13` using source-file and artifact hashes. It distinguishes the lifecycle candidate from the final rebuild after a shortcut dependency fix.

- **下载**：原生添加下载 Sheet 探测 `https://codeload.github.com/liveait/omlx/zip/refs/heads/main`，正确建议 `omlx-main.zip`。系统下载目录已有较早的部分文件和 sidecar，后端因此分配 `omlx-main (1).zip`。完整文件为 52,830,371 字节，ZIP 全部条目通过 CRC 校验。持久化任务为 Completed，字节数与文件一致，`total_bytes=null`；任务行及 Inspector 都暴露 100%。完成任务和文件保留供查看，原有部分文件及 sidecar 的哈希未变化。

  **Download:** The native Add Download sheet probed `https://codeload.github.com/liveait/omlx/zip/refs/heads/main` and suggested `omlx-main.zip`. The system Downloads directory already contained an older partial and sidecar, so the backend allocated `omlx-main (1).zip`. The completed file contains 52,830,371 bytes; every ZIP entry passed CRC verification. The persisted task is Completed with the same byte count and `total_bytes=null`; both task row and Inspector expose 100%. The completed task and file are retained for inspection, and the pre-existing partial and sidecar hashes are unchanged.
- **生命周期**：关闭窗口后 App 及拥有的 Server 继续运行，下载继续；退出停止两者，重开恢复完成任务。确认实际子进程的可执行文件与父进程关系后，终止该子进程，0.374 秒内出现替代进程；App 持续运行并恢复完整任务快照。这只检查一次意外退出，不代表已原生验收重启预算耗尽。

  **Lifecycle:** Closing the window kept the App and its owned Server alive while the transfer continued. Quit stopped both; relaunch restored the completed task. After verifying the actual child executable and parent relationship, terminating that child produced a replacement in 0.374 seconds; the App remained running and restored the full task snapshot. This checks one unexpected exit, not exhaustion of the restart budget.
- **外部模式**：切换到隔离的独立 Server 后，界面用其空任务列表替换托管快照，并停止 App 拥有的子进程；外部地址与 Keychain 控件显示。独立 Server 在 App 退出／重开以及切回托管后均保持运行。切回后托管完成任务恢复，清理时只停止独立 fixture。

  **External mode:** Switching to an isolated independently started Server replaced the Managed snapshot with its empty task list and stopped the owned child. The external address and Keychain controls appeared. That independent Server survived App Quit/relaunch and the switch back to Managed. The completed Managed task returned after switching back; only the separate fixture was stopped during cleanup.
- **默认目录及最终重建**：原生目录选择器选中临时文件夹，保存后正确持久化。该检查发现 `⌘N` 闭包读取旧设置，现已让快捷键和目录重置 effect 跟踪当前目录及本机／远程模式。前端测试及 TypeScript/Vite 构建通过，随后重建 release App 并通过签名／打包检查。最终 App 保存目录后首次 `⌘N` 即使用所选目录，重置后首次 `⌘N` 使用系统下载目录；重开也恢复完成任务及 100% 进度。

  **Default folder and final rebuild:** A native folder picker selected a temporary folder, and saving persisted it. This check exposed a stale `⌘N` closure; the shortcut and directory-reset effects now track the current folder and local/remote mode. Frontend tests and TypeScript/Vite build passed, followed by a new release App build and signature/bundle checks. In that final App, the first `⌘N` after saving used the selected folder, and the first `⌘N` after resetting used system Downloads. Relaunch also restored the completed task with 100% progress.
- **检查与恢复**：Rust 格式、workspace check、276 项 workspace 测试及禁止警告的 Clippy 均通过。Desktop 库有 29 项测试，其中 12 项覆盖托管生命周期／资源；binary 有 8 项测试，前端有 13 项。最终设置为托管，外部地址 `127.0.0.1:39100`、刷新 5 秒、语言 `system`，默认下载目录无自定义值。外部 fixture 目录已清理、端口已释放；App 停留在下载页面并显示 Inspector。

  **Checks and restoration:** Rust formatting, workspace check, 276 workspace tests and Clippy with warnings denied passed. The Desktop library has 29 tests, including 12 managed lifecycle/resource tests; its binary has 8 tests, and the frontend has 13. Final settings are Managed, external address `127.0.0.1:39100`, refresh 5 seconds, language `system`, and no custom default folder. The external fixture directory and port were released; the App is left on Downloads with Inspector visible.
- **边界**：本机临时签名不代表分发签名、公证或 Gatekeeper 放行。M0 的 VoiceOver 实际朗读仍未确认；此原生产物尚不包含下文的完成对账后续变更；全新用户／升级最终验收仍待完成。

  **Scope:** These local signatures do not establish distribution signing, notarization or Gatekeeper acceptance. M0 actual VoiceOver speech remains unconfirmed. This native artifact predates the completion-reconciliation follow-up below; final clean-user/upgrade acceptance remains open.

| 原生界面状态<br>Native state | 截图<br>Capture |
| --- | --- |
| 已连接的托管设置<br>Connected Managed settings | [查看](assets/native-macos/managed-2026-10-11/settings-managed.png)<br>[View](assets/native-macos/managed-2026-10-11/settings-managed.png) |
| 外部地址及 Keychain 控件<br>External address and Keychain controls | [查看](assets/native-macos/managed-2026-10-11/settings-external.png)<br>[View](assets/native-macos/managed-2026-10-11/settings-external.png) |
| 最终 App 的 codeload 完成任务与 Inspector<br>Completed codeload task and Inspector in the final App | [查看](assets/native-macos/managed-2026-10-11/download-completed.png)<br>[View](assets/native-macos/managed-2026-10-11/download-completed.png) |

三张图片均为浅色、简体中文，尺寸为 2400 × 1602 像素。设置截图拍摄于快捷键最终修复前；完成任务截图使用最终 App。下载图片中的两处本机路径已替换为明确标注脱敏的 `~/Downloads` 路径。未测量窗口 point 边界，这些截图不能作为新一轮明暗配对或 VoiceOver 朗读证据。

All three images are 2400 × 1602 pixels in Light appearance and Simplified Chinese. Settings captures precede the final shortcut fix; the completed-task capture uses the final App. Two local paths in that download image were replaced with annotated `~/Downloads` paths. Window bounds in points were not measured, and these captures are not new Light/Dark pairs or VoiceOver speech evidence.

<a id="http-completion-reconciliation-backend-evidence"></a>
<a id="http-完成对账后端证据"></a>

#### HTTP 完成对账：后端证据 / HTTP completion reconciliation: backend evidence

10 月 11 日的 [ADR 0007](decisions/0007-http-completion-reconciliation.md) 后续切片增加私有完成凭证和一次最终进度／Completed 持久化。本轮证据来自独立 engine／Core fixture 与真实 Server 子进程，未重启既有原生 App，也未修改其任务数据库和下载文件。

The October 11 follow-up under [ADR 0007](decisions/0007-http-completion-reconciliation.md) adds private completion receipts and one persisted final-progress/Completed update. This evidence uses independent engine/Core fixtures and real Server child processes, without restarting the existing native App or modifying its task database and downloads.

- **恢复**：本地 HTTP fixture 包括无 validator 且未知长度响应，关闭 HTTP listener 后仍可凭证恢复完成；分别检查发布前暂存文件和已发布目标。SQLite trigger 拒绝 Completed，另一个场景同时拒绝两个终态；终止／重启子进程后恢复相同完整字节及持久化状态。

  **Recovery:** Local HTTP fixtures include unknown-length responses without validators. Receipt-backed recovery completes after the HTTP listener has closed. Pre-publication staging and already-published files are checked separately. SQLite triggers reject `Completed` and, in another case, both terminal saves; terminating/restarting the child recovers the same complete bytes and durable state.
- **保护与生命周期**：覆盖内容相同但身份不同、摘要变化、符号链接／FIFO／超大／损坏记录、完成持久化后的 hard-link 残留、被替换暂存文件、删除清理错误、继续派发队列和任务 ID 复用。全量校验在 worker 中配合暂停／取消；启动清理只读取有界元数据和小型 sidecar。

  **Protection and lifecycle:** Tests cover different identities with identical bytes, changed digests, symlink/FIFO/oversized/corrupt records, hard-link residue after durable completion, replaced staging files, removal cleanup errors, continued queue dispatch, and task ID reuse. Full-file verification runs in the worker and obeys pause/cancel; startup cleanup reads only bounded metadata and small sidecars.
- **检查**：engine 有 39 项测试，其中新增 16 项凭证场景；Core 有 26 项。Server persistence 有 47 项进程测试，其中 11 项完成对账场景；`cargo fmt --all -- --check`、`cargo check --locked --workspace --all-targets`、全部 305 项 workspace 测试及 `cargo clippy --locked --workspace --all-targets -- -D warnings` 均通过。首次 macOS 执行遇到既有托管管道捕获超时；单独 EOF、串行与再次默认并发托管测试，以及最终默认并发完整 workspace 测试通过，未放宽进程期限。

  **Checks:** The engine has 39 tests, including 16 new receipt cases; Core has 26 tests. Server persistence has 47 process tests, including 11 completion cases. `cargo fmt --all -- --check`, `cargo check --locked --workspace --all-targets`, all 305 workspace tests, and `cargo clippy --locked --workspace --all-targets -- -D warnings` passed. An initial macOS run hit an existing managed pipe-capture timeout; the isolated EOF case, serial and repeated default-concurrency managed suites, and final full default-concurrency workspace run passed. No process deadline was relaxed.
- **边界**：这些是后端进程测试，不代表最终原生全新用户／升级验收或 VoiceOver 朗读证据。此前记录的原生 App 尚不包含本次后续变更。Windows 文件身份已实现但未在本机执行；当前 CI 检查 Linux Rust 和 macOS Desktop。硬件掉电保障依赖平台／文件系统同步语义。

  **Scope:** These are backend process tests, not final native clean-user/upgrade acceptance or VoiceOver speech evidence. The previously recorded native App predates this follow-up. Windows file identity is implemented but has not been executed locally; current CI validates Linux Rust and macOS Desktop. Power-loss durability depends on platform/filesystem synchronization.

#### 10 月 31 日恢复就绪：后端验证 / October 31 recovery readiness: backend verification

以 `cf88455` 为基线的恢复就绪切片已实现有界启动检查及 Ready 后可控对账。完整本地 Rust 门槛通过（410 项，0 失败／忽略，含 Server unit、managed、persistence）；验证器等待期间真实 managed 循环仍响应 RPC，生产 Server fixture 可离线恢复，并保留 SQLite 拒绝完成保存后的证明。Ready 不表示所有任务已到终态，客户端与验收脚本必须等待目标终态。完整覆盖见[开发计划 M1](DEVELOPMENT_PLAN.md#m1)和 [ADR 0007](decisions/0007-http-completion-reconciliation.md)。这不更新 10 月 10／11 日原生产物的验收身份；最终候选仍需大文件恢复、VoiceOver、干净用户和升级验收。PR 审查／CI、正式签名公证及发布另行核对。

The recovery-readiness slice from `cf88455` implements bounded startup checks and controllable reconciliation after Ready. Complete local Rust gates pass (410 tests, zero failures/ignored, including Server unit, managed and persistence). The real managed loop responds to RPC while verification waits; production Server fixtures recover offline and retain proofs after SQLite rejects completion. Ready does not mean every task is terminal; clients/acceptance scripts must wait for the desired terminal state. See [M1](DEVELOPMENT_PLAN.md#m1) and [ADR 0007](decisions/0007-http-completion-reconciliation.md) for coverage. This does not change the identity/acceptance of October 10/11 native artifacts. The final candidate still needs large-file recovery, VoiceOver, clean-user and upgrade checks. Verify PR review/CI, distribution signing/notarization and publishing separately.

<a id="native-macos-visual-evidence"></a>
<a id="macos-原生界面截图留证"></a>

### macOS 原生界面截图留证 / Native macOS visual evidence

执行检查表第 9 项时，使用已构建并正在运行的 `Nexum.app`。配置的默认窗口尺寸为 1200 × 800 point；实际截图边界应以 JSON sidecar 为准。PNG 包含原生标题栏，并以显示器的实际像素比例保存。先在应用中打开所需的下载列表、Inspector、设置、添加下载和焦点状态，并手动把 macOS 外观切换为浅色或深色。然后在仓库根目录截图：

Use a running, built `Nexum.app` for checklist step 9. The configured default window size is 1200 × 800 points; measure the captured window's actual bounds from the JSON sidecar. The PNG includes the native title bar and uses the display's pixel scale. Open the required Downloads, Inspector, Settings, Add Download, and focused-control states in the app. Set Light or Dark in macOS Appearance yourself, then capture each state from the repository root:

```sh
mkdir -p "$PWD/../nexum-native-qa"
swift scripts/capture-native-macos-window.swift \
  --app "$PWD/target/release/bundle/macos/Nexum.app" \
  --output "$PWD/../nexum-native-qa/downloads-light.png" \
  --appearance light
```

脚本只选择该精确 App 路径和 PID 所属、标题为 `Nexum` 的一个可见 layer-0 主窗口，忽略其他标题的辅助窗口，再执行 `screencapture -x -o -l<windowid>`。窗口不存在、存在多个候选、系统外观不符或输出文件已存在时会拒绝截图；不会更改窗口、系统外观或应用设置。每张 PNG 旁边会生成 `.png.json`，记录 App 路径、PID、窗口 ID、point 尺寸、macOS 当前外观、截图时间和实际像素尺寸。对应的深色截图应使用新文件名、`--appearance dark`，并通过 `--expect-pixels WIDTHxHEIGHT` 指定浅色截图报告的像素尺寸；像素尺寸不同时脚本会拒绝截图，不会缩放图片。再比较两份 sidecar 的 point 边界和像素尺寸，检查窗口几何尺寸或显示器缩放是否变化。对每组界面状态重复截图并人工比较 PNG。终端应用可能需要“屏幕录制”权限。验收后恢复原有的 macOS 外观和临时 Nexum 设置。

The script selects one visible layer-0 primary window titled `Nexum` belonging to the exact running app bundle and PID, ignoring auxiliary windows with other titles, then calls `screencapture -x -o -l<windowid>`. It refuses missing or ambiguous windows, an appearance mismatch, and existing output files. It never changes the window, system appearance, or app settings. Each PNG has a `.png.json` sidecar with the app path, PID, window ID and bounds in points, effective macOS appearance, capture time, and actual pixel dimensions. For the matching Dark capture, use a new output name, `--appearance dark`, and `--expect-pixels WIDTHxHEIGHT` with the Light capture's reported pixel dimensions; this rejects a capture with different pixel dimensions without resizing the image. Compare both sidecars' point bounds and pixel dimensions to check for window geometry or display-scale changes. Repeat for each matching UI state and review the PNGs visually. Screen Recording permission may be needed for the terminal app. Restore the original macOS Appearance and any temporary Nexum settings after validation.

<a id="release-candidate-captures"></a>
<a id="release-候选产物截图"></a>

#### release 候选产物截图 / Release candidate captures

10 月 10 日的 release 候选产物通过 CUA 截图，应用跟随系统简体中文语言设置。下表每组浅色/深色截图的尺寸均为 2400 × 1602 像素。[release 截图摘要](assets/native-macos/release-2026-10-10/release-capture-summary.json)标明源码提交及产物哈希，描述截图状态并记录进度轨道比较。添加下载图片中的本机下载路径已明确标注脱敏；其他图片显示隔离 QA 数据。本轮截图不推测窗口的 point 边界。

The October 10 release candidate was captured through CUA while following the Simplified Chinese system language. Each Light/Dark pair below has the same 2400 × 1602-pixel dimensions. The [release capture summary](assets/native-macos/release-2026-10-10/release-capture-summary.json) identifies the source commit and artifact hashes, describes the captured states, and records progress-track comparisons. Add Download images mark the redacted local download path; the other images show isolated QA data. These captures do not infer window bounds in points.

| 界面状态<br>App state | 浅色<br>Light | 深色<br>Dark |
| --- | --- | --- |
| 下载列表、未知大小已完成任务及 Inspector<br>Downloads, completed unknown-size task, and Inspector | [查看](assets/native-macos/release-2026-10-10/downloads-light.png)<br>[View](assets/native-macos/release-2026-10-10/downloads-light.png) | [查看](assets/native-macos/release-2026-10-10/downloads-dark.png)<br>[View](assets/native-macos/release-2026-10-10/downloads-dark.png) |
| 设置首页<br>Settings overview | [查看](assets/native-macos/release-2026-10-10/settings-light.png)<br>[View](assets/native-macos/release-2026-10-10/settings-light.png) | [查看](assets/native-macos/release-2026-10-10/settings-dark.png)<br>[View](assets/native-macos/release-2026-10-10/settings-dark.png) |
| 添加下载，来源 URL 已聚焦<br>Add Download with Source URL focused | [查看](assets/native-macos/release-2026-10-10/add-download-light.png)<br>[View](assets/native-macos/release-2026-10-10/add-download-light.png) | [查看](assets/native-macos/release-2026-10-10/add-download-dark.png)<br>[View](assets/native-macos/release-2026-10-10/add-download-dark.png) |
| 通知<br>Notifications | [查看](assets/native-macos/release-2026-10-10/notifications-light.png)<br>[View](assets/native-macos/release-2026-10-10/notifications-light.png) | [查看](assets/native-macos/release-2026-10-10/notifications-dark.png)<br>[View](assets/native-macos/release-2026-10-10/notifications-dark.png) |

[更窄的深色窗口](assets/native-macos/release-2026-10-10/downloads-narrow-dark.png)尺寸为 2058 × 2458 像素，显示已暂停的未知大小任务及 Inspector，与同尺寸明暗配对分别留证。动态效果采样保存为[暂停帧 C](assets/native-macos/release-2026-10-10/paused-c.png)、[暂停帧 D](assets/native-macos/release-2026-10-10/paused-d.png)、[减弱动态效果帧 C](assets/native-macos/release-2026-10-10/reduced-motion-c.png)、[减弱动态效果帧 D](assets/native-macos/release-2026-10-10/reduced-motion-d.png)。摘要记录间隔和比较的像素区域；这些静态图片不能作为连续动画记录或旁白实际朗读证据。

The [narrower Dark window](assets/native-macos/release-2026-10-10/downloads-narrow-dark.png) is 2058 × 2458 pixels and shows the paused unknown-size task with Inspector visible; it is separate from the matching Light/Dark pairs. Progress sampling is retained as [Paused frame C](assets/native-macos/release-2026-10-10/paused-c.png), [Paused frame D](assets/native-macos/release-2026-10-10/paused-d.png), [Reduce Motion frame C](assets/native-macos/release-2026-10-10/reduced-motion-c.png), and [Reduce Motion frame D](assets/native-macos/release-2026-10-10/reduced-motion-d.png). The summary records elapsed times and the compared pixel regions; these still images are not a continuous animation recording or VoiceOver speech evidence.

<a id="earlier-debug-captures"></a>
<a id="较早的-debug-截图"></a>

#### 较早的 debug 截图 / Earlier debug captures

以下原生 debug App 截图拍摄于 2026 年 10 月 8–9 日，应用跟随 macOS 的简体中文语言设置。截图时，脚本报告各组浅色与深色截图的窗口边界均为 1200 × 801 point，图像尺寸均为 2400 × 1602 像素。[脱敏 debug 截图摘要](assets/native-macos/capture-summary.json)保留尺寸与图像哈希，不包含本机路径或 PID。

The following native debug-bundle captures were taken on October 8–9, 2026, with the app following a Simplified Chinese macOS language setting. During capture, the script reported the same 1200 × 801-point window bounds and 2400 × 1602-pixel image dimensions for each pair. The [sanitized debug capture summary](assets/native-macos/capture-summary.json) records the dimensions and image hashes without local paths or PIDs.

| 界面状态<br>App state | 浅色<br>Light | 深色<br>Dark |
| --- | --- | --- |
| 下载列表、已完成任务及 Inspector<br>Downloads, completed task, and Inspector | [查看](assets/native-macos/downloads-light.png)<br>[View](assets/native-macos/downloads-light.png) | [查看](assets/native-macos/downloads-dark.png)<br>[View](assets/native-macos/downloads-dark.png) |
| 设置首页<br>Settings overview | [查看](assets/native-macos/settings-light.png)<br>[View](assets/native-macos/settings-light.png) | [查看](assets/native-macos/settings-dark.png)<br>[View](assets/native-macos/settings-dark.png) |
| 添加下载，来源 URL 已聚焦<br>Add Download with Source URL focused | [查看](assets/native-macos/add-download-light.png)<br>[View](assets/native-macos/add-download-light.png) | [查看](assets/native-macos/add-download-dark.png)<br>[View](assets/native-macos/add-download-dark.png) |

2026 年 10 月 9 日，同一个原生 debug App 的应用内语言也完成即时切换：[英文外观页](assets/native-macos/appearance-english-light.png)与[简体中文外观页](assets/native-macos/appearance-chinese-light.png)。英文选项在应用重启后仍然生效，设置、外观和语言的辅助功能名称也随界面切换。这些较早图片仍明确属于 debug App；release 候选产物证据和剩余原生检查见上文。

On October 9, 2026, the same native debug app also switched its in-app language immediately: [English Appearance](assets/native-macos/appearance-english-light.png) and [Simplified Chinese Appearance](assets/native-macos/appearance-chinese-light.png). The English choice survived an app restart, and the accessible Settings, Appearance, and Language names changed with the interface. These earlier images retain their debug-bundle identity. The release candidate evidence and remaining native checks are recorded above.
