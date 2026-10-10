# 发布流程

## 当前状态

仓库目前没有自动发布工作流，当前检出也没有版本标签。`.github/workflows/ci.yml` 在推送和 Pull Request 上执行 Rust 格式、检查、测试和 Clippy。新增的 macOS job 安装 pnpm 11.25.0 和 Node.js 22，使用冻结锁文件安装 Desktop 依赖，构建前端及 macOS `.app`，并检查 App 中的语言声明。CI 仍不构建 Browser 扩展，不进行分发签名或公证，也不会发布 GitHub Release。各包与应用清单目前声明 `0.1.0`，而变更日志仍将该版本标为 TBD。

`Cargo.lock` 和 `apps/desktop/pnpm-lock.yaml` 已纳入 Git。Desktop 固定 pnpm 11.25.0 和项目内 Tauri CLI 2.11.5，构建时应使用冻结锁文件。Browser 扩展仍没有已跟踪的锁文件，全新检出可能解析到不同的前端依赖版本。构建候选版本前，应让两个已跟踪的锁文件与对应清单保持同步。

## 手动发布检查表

以下步骤说明当前源码树生成候选版本所需的工作。完成验证后，再人工决定打标签和发布。

1. 在 Rust 包清单（`crates/*/Cargo.toml`、`apps/server/Cargo.toml`、`apps/cli/Cargo.toml` 和 `apps/desktop/src-tauri/Cargo.toml`）中设置目标版本，并保持 `apps/desktop/package.json`、`apps/desktop/src-tauri/tauri.conf.json`、`apps/extension/package.json`、`apps/extension/manifest.json` 的版本一致。JSON-RPC 协议版本是独立的兼容性编号。
2. 将发布内容从 `Unreleased` 移至 `CHANGELOG.md` 和 `CHANGELOG.zh-CN.md` 中带日期的版本条目。
3. 在仓库根目录运行[开发指南](DEVELOPMENT.zh-CN.md#rust-检查)列出的四条 Rust CI 命令。
4. 使用 pnpm 11.25.0，在 `apps/desktop` 运行 `pnpm install --frozen-lockfile && pnpm build`。macOS CI job 会执行这两条命令。在 `apps/extension` 运行 `pnpm install && pnpm lint && pnpm build`；Desktop 锁文件和 CI job 均不覆盖扩展。
5. 运行 `cargo build --locked --release -p nexum-server -p nexum-cli` 构建 Rust 命令行产物；可执行文件为 `target/release/nexum-server` 和 `target/release/nexum-cli`。在 `apps/desktop` 执行 `pnpm exec tauri build --bundles app`，通过项目内固定版本的 Tauri CLI 构建 macOS App，产物位于仓库根目录的 `target/release/bundle/macos/Nexum.app`。其他平台或安装包目标需分别构建和验收。
6. 打包扩展时包含根目录 `manifest.json`、`icons/` 和生成的 `dist/`。清单同时引用后两处的文件，仅有 `dist/` 无法作为扩展加载。
7. 使用本地 Server 对可执行文件和原生桌面包进行冒烟测试。通过 HTTP 提供一个已知文件，创建任务并将其 URL 加入队列；自动派发的 Worker 阻塞期间查询 `task.get` 或 `task.list`，确认能看到中间字节数；`task.start` 仍可作为排队 HTTP 任务的可选手动 kick。Worker 完成后核对目标文件字节和已持久化的 `Completed` 状态。确认失败传输保留已有目标文件，并通过任务 `error` 字段返回错误；在重试预算未耗尽时自动重试，并在预算耗尽后使用同一 `--data-dir` 重启 Server 仍保留最终错误。使用分块本地 fixture 验证活跃任务的 `task.pause` 会等待 `Paused`，`task.resume` 会继续同一个 Worker，`task.remove` 会取消 Worker，最多等待 30 秒后删除任务并保留原有目标文件。记录阻塞中的响应读取可能让 `task.pause` 等到 30 分钟 HTTP 超时；删除超时会返回错误，Worker 退出后可以重试。验证没有 ETag 或 Last-Modified 的响应在中断后会从零开始，并确认成功、取消和 `task.remove` 会删除部分文件及 sidecar。确认 Server 会拒绝数据目录内和重叠的目标。自动化 TLS 测试在运行时生成证书，覆盖 TCP RPC、HTTPS `/jsonrpc` `OPTIONS`/`POST`、Browser 扩展 CORS 允许/拒绝、TCP/HTTPS 上的配置鉴权、带鉴权的 `events.subscribe`、明文拒绝、启动时拒绝不完整或无效的证书/私钥材料，以及握手时拒绝不受信任或主机名不匹配的证书。CLI 和 Desktop 测试覆盖连接失败后的凭据门控；CLI 测试还验证 TLS 失败后不会重试明文。关闭认证时，从 `apps/extension` 加载 Browser 扩展，发送已知 HTTP 链接，确认不带 scheme 的地址使用 HTTP `/jsonrpc` 桥接；启用 TLS 的 Server 上保存 `tls://host:port`，确认扩展使用 HTTPS，浏览器拒绝不受信任的证书，且不会重试 HTTP。验证网页 `Origin` 被拒绝，而扩展 Origin 在 HTTP 和 HTTPS 上都能收到 CORS 头。
8. 在 macOS 原生 Desktop 构建中验证键盘和可访问性行为：`⌘N` 打开 Add Download 并聚焦 Source URL；Tab 和 Shift+Tab 保持在 Sheet 内；Escape 关闭并恢复焦点。`⌘F` 打开或聚焦 Downloads 搜索，Escape 关闭搜索并恢复焦点。使用 Enter/Space 选择任务，并用上/下方向键及 Home/End 在可见任务间移动。通过 VoiceOver 检查任务操作、状态和 Inspector 进度；再启用 macOS“减少动态效果”，确认过渡和不确定进度动画停止。
9. 在该原生 macOS Desktop 构建中保持应用打开，切换 macOS“浅色”和“深色”外观。检查两种模式下的侧边栏、Downloads 任务行及状态徽标、Inspector、设置卡片与输入框、Add Download Sheet 和键盘焦点环，并保存截图以供视觉比较。在“设置 → 外观”依次选择“跟随系统”、English 和“简体中文”，确认可见标签、提示、任务状态、校验消息、通知和 VoiceOver 名称即时切换；重启应用后确认语言选择保留。在 `Nexum.app/Contents/Info.plist` 中确认 `CFBundleLocalizations` 包含 `en` 和 `zh-Hans`。选择“跟随系统”时，在 macOS“语言与地区 → 应用程序”中分别将 Nexum 设为“简体中文”和 English，每次更改后重启应用并确认界面语言；验收后恢复原有的应用语言设置。确认 Server 和操作系统诊断原文不变，切换语言也不会改变任务协议值或请求。缺少 `language` 字段的旧设置应以“跟随系统”加载。
10. 验证实际产物后，人工创建版本标签和 GitHub Release；在发布说明中记录目标平台及尚未完成的集成。

### 本地 release 构建证据

2026 年 10 月 9 日，本机 macOS release 构建生成了 `target/release/bundle/macos/Nexum.app`、`target/release/nexum-server` 和 `target/release/nexum-cli`。使用固定的 Tauri CLI 2.11.5 构建 Desktop 也已通过。App 的 `Info.plist` 在 `CFBundleLocalizations` 下包含 `en` 和 `zh-Hans`。显式添加本机临时签名后，App 通过 `codesign --verify --deep --strict`；`spctl --assess --type execute` 仍拒绝它。分发签名、公证和 Gatekeeper 放行仍未验收。

此前主检出目录的 release App 已显示已完成的 1 MB HTTP 下载，并验证了“跟随系统”下的 macOS 应用专属 English 覆盖：重启后，界面及任务、进度辅助功能名称切换为英文；移除覆盖并重启后恢复简体中文。该语言检查期间，应用的 `settings.json` 哈希未变化。此前隔离 worktree 的检查显示了已完成的已知和未知大小任务，确认测试通知可打开关联任务，并在启用“减弱动态效果”检查添加下载 Sheet 后恢复系统偏好。那些测试任务、文件和通知均已在检查后移除。

#### 最新 release 候选产物：原生验收

2026 年 10 月 10 日，以源码提交 `1df5eab8411081e0177037029025f6e2e81fdfcc` 使用固定 Tauri CLI 重新构建 release App，添加本机临时签名后通过 `codesign --verify --deep --strict`。[release 截图摘要](assets/native-macos/release-2026-10-10/release-capture-summary.json)记录二进制及前端哈希、截图时间、像素尺寸和图像哈希，用于识别下列检查所用的候选产物。分发签名与公证仍是独立的发布要求。

- **键盘**：`⌘N` 打开添加下载并聚焦来源 URL；Tab 和 Shift+Tab 可双向移动，并在 Sheet 边界循环。Escape 关闭 Sheet 并恢复焦点。`⌘F` 打开下载搜索并聚焦输入框，Escape 关闭搜索并恢复焦点。上/下方向键及 Home/End 可移动任务选择。恢复原 Server 连接后，Tab/Shift+Tab 可聚焦任务行；Space 和 Return 分别选中聚焦的任务，并让 Inspector 跟随，未更改任务数据。
- **任务和通知**：隔离本地 Server 提供了一个已完成的 65,536 字节已知大小任务，以及一个已完成的 90,112 字节、`total_bytes=null` 的任务。两个任务行的进度条均满格；未知大小任务的 Inspector 在原生辅助功能树中显示 100%。打开通知页后未读状态清除；点击暂停通知会选中关联任务。原生截图显示通知卡片使用内容区可用宽度。
- **视觉证据**：四组浅色/深色配对覆盖下载列表及 Inspector、设置、来源 URL 已聚焦的添加下载、通知，尺寸均为 2400 × 1602 像素。单独的深色截图记录系统半屏更窄窗口，尺寸为 2058 × 2458 像素。本轮 CUA 截图未测量 point 边界，窄窗口截图也不是同几何尺寸的明暗配对。
- **动态效果采样**：关闭“减弱动态效果”时，未知大小任务在间隔 33.623 秒的两帧中均为 Paused；开启时，任务在间隔 68.773 秒的两帧中仍为 Downloading。每组的列表及 Inspector 进度轨道像素均一致。这只确认采样时刻的像素相同，不能证明两帧之间持续没有动态效果。[Desktop CSS](../apps/desktop/src/App.css)只为活跃的不确定进度设置动画，并在 `prefers-reduced-motion: reduce` 下关闭该动画；这项源码检查补充了原生采样证据。
- **恢复与剩余检查**：Server 设置已恢复为 `127.0.0.1:39100`，刷新间隔为 5 秒，语言为 `system`，并从设置文件确认。浅色外观和原窗口尺寸已恢复，原已知文件任务已选中，搜索已关闭，Inspector 保持可见。“减弱动态效果”和“旁白”恢复为关闭，旁白字幕偏好保持原值。工具无法采集旁白音频，两次尝试访问其原生字幕窗口均超时，尚未确认实际朗读内容。M0 仍待该朗读检查。
- **隔离 fixture 清理**：三个 QA 任务均成功移除，隔离任务列表为空。只停止了本轮独立的 39112 端口 Server 和 39113 端口 HTTP fixture；两个端口均已释放，其临时数据/下载目录已删除。原 39100 端口 Server 保持运行，清理前后两个原任务的状态、字节数和错误字段均一致，原已知文件下载仍保留。

### macOS 原生界面截图留证

执行检查表第 9 项时，使用已构建并正在运行的 `Nexum.app`。配置的默认窗口尺寸为 1200 × 800 point；实际截图边界应以 JSON sidecar 为准。PNG 包含原生标题栏，并以显示器的实际像素比例保存。先在应用中打开所需的下载列表、Inspector、设置、添加下载和焦点状态，并手动把 macOS 外观切换为浅色或深色。然后在仓库根目录截图：

```sh
mkdir -p "$PWD/../nexum-native-qa"
swift scripts/capture-native-macos-window.swift \
  --app "$PWD/target/release/bundle/macos/Nexum.app" \
  --output "$PWD/../nexum-native-qa/downloads-light.png" \
  --appearance light
```

脚本只选择该精确 App 路径和 PID 所属、标题为 `Nexum` 的一个可见 layer-0 主窗口，忽略其他标题的辅助窗口，再执行 `screencapture -x -o -l<windowid>`。窗口不存在、存在多个候选、系统外观不符或输出文件已存在时会拒绝截图；不会更改窗口、系统外观或应用设置。每张 PNG 旁边会生成 `.png.json`，记录 App 路径、PID、窗口 ID、point 尺寸、macOS 当前外观、截图时间和实际像素尺寸。对应的深色截图应使用新文件名、`--appearance dark`，并通过 `--expect-pixels WIDTHxHEIGHT` 指定浅色截图报告的像素尺寸；像素尺寸不同时脚本会拒绝截图，不会缩放图片。再比较两份 sidecar 的 point 边界和像素尺寸，检查窗口几何尺寸或显示器缩放是否变化。对每组界面状态重复截图并人工比较 PNG。终端应用可能需要“屏幕录制”权限。验收后恢复原有的 macOS 外观和临时 Nexum 设置。

#### release 候选产物截图

10 月 10 日的 release 候选产物通过 CUA 截图，应用跟随系统简体中文语言设置。下表每组浅色/深色截图的尺寸均为 2400 × 1602 像素。[release 截图摘要](assets/native-macos/release-2026-10-10/release-capture-summary.json)标明源码提交及产物哈希，描述截图状态并记录进度轨道比较。添加下载图片中的本机下载路径已明确标注脱敏；其他图片显示隔离 QA 数据。本轮截图不推测窗口的 point 边界。

| 界面状态 | 浅色 | 深色 |
| --- | --- | --- |
| 下载列表、未知大小已完成任务及 Inspector | [查看](assets/native-macos/release-2026-10-10/downloads-light.png) | [查看](assets/native-macos/release-2026-10-10/downloads-dark.png) |
| 设置首页 | [查看](assets/native-macos/release-2026-10-10/settings-light.png) | [查看](assets/native-macos/release-2026-10-10/settings-dark.png) |
| 添加下载，来源 URL 已聚焦 | [查看](assets/native-macos/release-2026-10-10/add-download-light.png) | [查看](assets/native-macos/release-2026-10-10/add-download-dark.png) |
| 通知 | [查看](assets/native-macos/release-2026-10-10/notifications-light.png) | [查看](assets/native-macos/release-2026-10-10/notifications-dark.png) |

[更窄的深色窗口](assets/native-macos/release-2026-10-10/downloads-narrow-dark.png)尺寸为 2058 × 2458 像素，显示已暂停的未知大小任务及 Inspector，与同尺寸明暗配对分别留证。动态效果采样保存为[暂停帧 C](assets/native-macos/release-2026-10-10/paused-c.png)、[暂停帧 D](assets/native-macos/release-2026-10-10/paused-d.png)、[减弱动态效果帧 C](assets/native-macos/release-2026-10-10/reduced-motion-c.png)、[减弱动态效果帧 D](assets/native-macos/release-2026-10-10/reduced-motion-d.png)。摘要记录间隔和比较的像素区域；这些静态图片不能作为连续动画记录或旁白实际朗读证据。

#### 较早的 debug 截图

以下原生 debug App 截图拍摄于 2026 年 10 月 8–9 日，应用跟随 macOS 的简体中文语言设置。截图时，脚本报告各组浅色与深色截图的窗口边界均为 1200 × 801 point，图像尺寸均为 2400 × 1602 像素。[脱敏 debug 截图摘要](assets/native-macos/capture-summary.json)保留尺寸与图像哈希，不包含本机路径或 PID。

| 界面状态 | 浅色 | 深色 |
| --- | --- | --- |
| 下载列表、已完成任务及 Inspector | [查看](assets/native-macos/downloads-light.png) | [查看](assets/native-macos/downloads-dark.png) |
| 设置首页 | [查看](assets/native-macos/settings-light.png) | [查看](assets/native-macos/settings-dark.png) |
| 添加下载，来源 URL 已聚焦 | [查看](assets/native-macos/add-download-light.png) | [查看](assets/native-macos/add-download-dark.png) |

2026 年 10 月 9 日，同一个原生 debug App 的应用内语言也完成即时切换：[英文外观页](assets/native-macos/appearance-english-light.png)与[简体中文外观页](assets/native-macos/appearance-chinese-light.png)。英文选项在应用重启后仍然生效，设置、外观和语言的辅助功能名称也随界面切换。这些较早图片仍明确属于 debug App；release 候选产物证据和剩余原生检查见上文。

英文版见 [RELEASE.md](RELEASE.md)。
