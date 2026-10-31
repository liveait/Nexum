<a id="nexum-macos-desktop-ui-design"></a>
<a id="nexum-macos-desktop-ui-设计"></a>

# Nexum macOS Desktop UI 设计 / Nexum macOS Desktop UI Design

导航 / Contents: [1. 产品目标 / 1. Product goal](#1-product-goal) · [2. 设计决定 / 2. Design decisions](#2-design-decisions) · [3. 任务列表与 Inspector / 3. Task list and inspector](#3-task-list-and-inspector) · [4. Add Download Sheet / 4. Add Download sheet](#4-add-download-sheet) · [5. 设置 / 5. Settings](#5-settings) · [6. 状态模型与数据新鲜度 / 6. State model and data freshness](#6-state-model-and-data-freshness) · [7. macOS 交互与可访问性 / 7. macOS interaction and accessibility](#7-macos-interaction-and-accessibility) · [8. 实现切片 / 8. Implementation slices](#8-implementation-slices) · [9. 验收标准 / 9. Acceptance criteria](#9-acceptance-criteria)

状态：已实现 macOS Desktop UI 基线；剩余项目已标注为计划项。

Status: implemented macOS Desktop UI baseline; remaining items are called out as planned.

本文定义 Tauri Desktop 的 macOS 优先信息架构和交互模型。当前 `apps/desktop/src/App.tsx` 与 `App.css` 已实现侧边栏、Downloads、Inspector、带 macOS Keychain 凭据操作的 Settings 卡片、带原生文件夹选择器的 Add Download Sheet、状态栏、事件驱动刷新、键盘交互、可访问性标签、焦点管理、减少动效行为、跟随系统的明暗配色、中英文 UI 文案和计划能力占位。

This document defines the macOS-first information architecture and interaction model for the Tauri desktop client. The current `apps/desktop/src/App.tsx` and `App.css` implement the sidebar, Downloads surface, Inspector, Settings cards with macOS Keychain credential controls, Add Download sheet with a native folder picker, status bar, event-driven refresh, keyboard interaction, accessibility labels, focus management, reduced-motion behavior, system light/dark palettes, English/Simplified Chinese UI strings, and planned-capability placeholders described here.

<a id="1-product-goal"></a>
<a id="1-产品目标"></a>

## 1. 产品目标 / 1. Product goal

桌面端应让本地常用流程一眼可懂：

The desktop app should make the common local workflow understandable at a glance:

1. 连接 Nexum Server。

   Connect to a Nexum Server.
2. 添加 HTTP/HTTPS 下载。

   Add an HTTP/HTTPS download.
3. 排队后无需手动刷新即可看到进度。

   Queue it and see progress without manually refreshing.
4. 安全地暂停、恢复、重试或删除任务。

   Pause, resume, retry, or remove it safely.
5. 任务需要处理时，可以查看目标路径和最近一次错误。

   Inspect the destination and the latest error when a task needs attention.

Desktop 默认使用“本地（推荐）”连接方式，自动托管随包下载服务；通用设置的“远程”也支持连接本机独立运行的服务。尚未支持的 Magnet 和本地文件来源不展示成可用功能。

Desktop defaults to the “Local (recommended)” connection mode and manages its bundled download service automatically. “Remote” under General settings also supports a service running separately on this Mac. Unsupported Magnet and local-file sources remain unavailable.

<a id="2-design-decisions"></a>
<a id="2-设计决定"></a>

## 2. 设计决定 / 2. Design decisions

<a id="21-navigationsplitview-style-shell"></a>
<a id="21-macos-风格的侧边栏壳层"></a>

### 2.1 macOS 风格的侧边栏壳层 / 2.1 NavigationSplitView-style shell

使用 macOS 风格的侧边栏和单一任务工作区，替换当前顶部 Tab 布局。

Use a macOS-style sidebar and a single main task surface rather than tabs across the top.

```text
┌────────────────────────────────────────────────────────────────┐
│ Sidebar       │ Downloads: filter  search  Inspector           │
│ Dashboard     ├────────────────────────────────────────────────┤
│ Downloads     │ Task list                         Inspector     │
│ Trackers      │ name | status | progress | destination          │
│ Plugins       │                                                │
│               │                                                │
│ Notifications │                                                │
│ Settings      │                                                │
└───────────────┴────────────────────────────────────────────────┘
```

图示术语 / Diagram terms: 侧边栏 = Sidebar；过滤 = filter；搜索 = search；任务列表 = Task list；名称/状态/进度/目标路径 = name/status/progress/destination。

- 原生窗口默认尺寸为 1200×800，有下载任务时可同时显示任务列表与 Inspector。窗口仍可调整大小；目前未配置原生最小尺寸。

  Default native window: 1200×800, so the task list and Inspector can appear together when downloads are present. The window remains resizable; no native minimum size is configured.
- 侧边栏目前固定为 226 px（视口宽度不超过 980 px 时为 194 px）；收起控制仍属于计划项。

  The sidebar currently has a fixed width of 226 px (194 px at viewport widths of 980 px or less). A collapse control remains planned.
- 主区域以任务列表为主。视口宽度超过 980 px 时，已开启的 Inspector 显示在任务列表右侧；宽度不超过 980 px 时，详情紧跟选中的任务行，即使列表很长也能就近查看。两处详情按钮在任意宽度下均可显示或隐藏详情；关闭后列表使用全部可用宽度。

  Main content has a list-first layout. Above the 980 px viewport breakpoint, the enabled Inspector appears beside the task list. At 980 px or below, it appears directly after the selected task row so details remain close to that row even in a long list. Both detail controls show or hide it at either width; closing it also lets the list use the full width.
- 使用系统字体、Nexum 紫色强调色、系统明暗外观和清晰的键盘焦点环。跟随系统强调色的选项仍待实现。

  Use the system font stack, Nexum's purple accent, light/dark system appearance, and visible keyboard focus rings. A system accent-color option remains planned.

<a id="211-appearance-and-language-implementation-decision"></a>
<a id="211-外观和语言实施决定"></a>

### 2.1.1 外观和语言实施决定 / 2.1.1 Appearance and language implementation decision

- Desktop 壳层通过 `prefers-color-scheme` 跟随 macOS 明暗外观，提供两套可读配色，并保留已有的减少动效行为。手动主题或强调色选择仍待实现。

  The Desktop shell follows macOS light and dark appearance through `prefers-color-scheme`, with two readable palettes and the existing reduced-motion behavior. A manual theme or accent-color selector remains planned.
- 语言默认选择**跟随系统**：中文系统语言使用简体中文，其他语言使用英文。设置中可改为 **English** 或 **简体中文**；选择与 Server 地址和刷新策略一起持久化，旧设置默认按**跟随系统**处理。

  Language defaults to **System**, resolving a Chinese system language to Simplified Chinese and other languages to English. Settings can override it with **English** or **简体中文**; the choice is persisted alongside the Server address and refresh policy, with older settings defaulting to **System**.
- 集中的类型化文案目录覆盖可见文字、提示、可访问名称、任务状态、校验与客户端生成的状态消息。语言选择只影响显示，不改变协议值或 Server 请求。Server 或操作系统返回的原始诊断信息保持原文，以免丢失技术细节。

  A central typed message catalog supplies visible UI text, tooltips, accessible names, task-state labels, validation, and client-generated status messages. The language choice changes presentation only: protocol values and Server requests stay unchanged. Raw diagnostics returned by the Server or operating system remain verbatim so technical details are not lost.

<a id="22-sidebar-sections"></a>
<a id="22-侧边栏分组"></a>

### 2.2 侧边栏分组 / 2.2 Sidebar sections

侧边栏采用 Motrix 风格的顶层导航。任务过滤器放在 `Downloads` 内，Server 连接信息放在设置内部：

The sidebar follows the Motrix-style top-level navigation. Task filters stay inside `Downloads`; Server connection details belong inside Settings:

| 分组<br>Section | 用途<br>Purpose | 数据来源<br>Data source |
| --- | --- | --- |
| Dashboard | 概览和最近活动<br>Overview and recent activity | `task.list` 快照<br>`task.list` snapshot |
| Downloads | 全部、活动、排队、已完成和失败过滤<br>All, Active, Queued, Completed, and Failed filters | 对 `task.list` 做客户端过滤<br>client-side filter over `task.list` |
| Trackers | 预留 BitTorrent Tracker 健康状态<br>Reserved for BitTorrent tracker health | 计划中的 Engine 能力<br>planned engine capability |
| Plugins | 预留插件目录和运行时<br>Reserved for plugin catalog and runtime | 计划中的 Plugin 能力<br>planned plugin capability |
| Notifications | 本次 Desktop 会话的任务和连接事件<br>Session task and connection events | Desktop 会话状态<br>Desktop session state |
| Settings | Server 连接、外观、刷新和未来的客户端选项<br>Server connection, appearance, refresh, and future client options | Desktop 设置<br>Desktop settings |

数据新鲜时显示每个过滤器的数量。连接不可用时隐藏数量，避免把过期数据当成当前状态。

Each filter shows a count when the data is fresh. Counts are hidden while the connection is unavailable so stale values are not presented as current.

<a id="23-toolbar"></a>
<a id="23-工具栏"></a>

### 2.3 工具栏 / 2.3 Toolbar

Downloads 页头提供过滤器和紧凑控制：

The Downloads header has a filter selector and compact controls:

- **添加**：下载页右上工具栏固定为“添加（＋）→ 详情 → 搜索”，`⌘N` 提供键盘入口。侧边栏、悬浮按钮和概览不再重复提供添加按钮；空列表引导使用工具栏／快捷键，首次欢迎只跳转到下载页，筛选无结果只提示调整筛选。

  **Add**: the Downloads upper-right toolbar uses “Add (+) → Details → Search”, with `⌘N` as the keyboard entry. Sidebar, floating and Overview buttons do not duplicate Add; empty lists point to the toolbar/shortcut, welcome only navigates to Downloads, and empty filtered results suggest adjusting filters.
- **过滤**：选择全部、活动、排队、已完成或失败任务。

  **Filter**: selects All, Active, Queued, Completed, or Failed downloads.
- **Inspector**：宽窗口切换列表右侧详情，窄窗口切换选中任务行下方详情。

  **Inspector**: toggles task details beside the list in wide windows or below the selected row in narrow windows.
- **搜索**：在显示的文件名、来源、目标路径和状态中做本地过滤。

  **Search**: filters the displayed filename, source, destination, and state locally.
- 更多操作占位、Inspector 和搜索图标在等大的工具栏按钮中居中。

  The more-actions placeholder, Inspector, and Search icons sit at the center of equal-size toolbar buttons.
- 任务行根据状态提供暂停、恢复、排队、启动或删除操作。

  Row actions pause, resume, queue, start, or remove a task when its state permits it.
- 底部状态栏显示 HTTP/HTTPS、连接状态和最近刷新时间。

  The footer shows HTTP/HTTPS, connection state, and the latest refresh time.

请求进行中只禁用对应任务行。常用流程已有快捷键；原生上下文菜单仍属于计划项。

Actions are disabled per row while their request is in flight. Keyboard shortcuts are available for common flows; native context menus remain planned.

<a id="3-task-list-and-inspector"></a>
<a id="3-任务列表与-inspector"></a>

## 3. 任务列表与 Inspector / 3. Task list and inspector

<a id="31-row-content"></a>
<a id="31-任务行内容"></a>

### 3.1 任务行内容 / 3.1 Row content

每行展示基于 Server 任务数据的内容：

Every row shows server-backed task data:

- 任务名称：从目标路径提取文件名，必要时回退到来源 URL 的文件名或主机名。任务 ID 只用于内部 RPC 和选中状态。

  Task name: derive the filename from the destination path, falling back to the source URL's filename or host. Task IDs remain internal to RPC and selection state.
- 状态徽标：Created、Queued、Downloading、Paused、Completed 或 Failed。徽标在每行标题的右侧对齐，不受可用操作数量影响。

  State badge: Created, Queued, Downloading, Paused, Completed, or Failed. Align badges at the right edge of each row header regardless of which actions are available.
- 进度条：`Completed` 总是显示满条，因为终态表示传输已结束，即使总大小缺失或与已下载量不一致也一样。字节文字仍保留实际报告的值，不虚构总大小。其他任务在已知 `total_bytes` 时显示确定进度；总大小未知时使用不确定进度条，仅在 `Downloading` 状态滚动，其余状态保持静止；Inspector 遵循同一规则。

  Progress bar: `Completed` always shows a full bar, since the terminal state confirms the transfer finished, even when the reported total is missing or differs from downloaded bytes. Keep the byte text as reported rather than inventing a total. Other tasks show determinate progress when `total_bytes` is known, or an indeterminate bar when it is unknown; animate the indeterminate bar only while `Downloading` and keep it stationary in other states. The Inspector follows the same rule.
- 来源和进度条使用卡片的内部宽度。元数据与任务操作放在可随窄窗口换行的底部区域，操作仅占自身宽度；不再预留贯穿整行的操作列。

  The source and progress track use the card's inner width. Put metadata and row actions in a footer that can wrap at narrow widths, with actions using only their own width; do not reserve a full-height action column.
- 字节文本：`downloaded_bytes / total_bytes`，使用可读单位。

  Byte text: `downloaded_bytes / total_bytes` with a human-readable unit.
- 目标路径，过长时截断并提供 Tooltip。

  Destination path, truncated with a tooltip.
- `error` 存在时在行下显示最近一次错误。

  Latest error beneath the row when `error` is present.

Server 尚未提供速度、ETA、重试次数和校验器，因此暂不显示。不要从稀疏轮询数据估算这些值并造成精确承诺。

Speed, ETA, retry count, and validator are not shown until the protocol exposes those values. The UI must not estimate them from sparse polling data and imply precision that the server does not provide.

<a id="32-inspector"></a>

### 3.2 Inspector

当前 Inspector 按以下顺序显示选中任务：

The current Inspector presents the selected task in this order:

1. 状态与进度。

   State and progress.
2. 来源 URL。

   Source URL.
3. 目标路径。

   Destination path.
4. 连接标识：托管显示“这台 Mac”，外部显示保存的 Server 地址。

   Connection label: This Mac for Managed, saved Server address for External.
5. 最近错误和复制操作。

   Last error with a copy action.

Inspector 只读展示任务元数据。任务行提供当前任务操作；原生显示文件和破坏性确认仍属于计划项。

The Inspector is read-only for task metadata. Row actions currently provide the task commands; native reveal and destructive confirmation are planned.

<a id="33-empty-loading-and-error-states"></a>
<a id="33-空加载和错误状态"></a>

### 3.3 空、加载和错误状态 / 3.3 Empty, loading, and error states

- **未连接：** 保留任务页面，在底部状态栏/错误条显示不可用状态，托管模式显示本机服务启动／恢复或重试提示，外部模式引导修改地址和凭据。

  **No connection:** keep the task surface available, show the unavailable state in the footer/banner, and show local service startup/recovery or Retry guidance in Managed mode; External directs the user to its address and credential settings.
- **已连接但没有任务：** 引导使用工具栏“＋”或 `⌘N`，不再增加重复按钮。

  **Connected with no tasks:** point to the toolbar plus or `⌘N` without another duplicate button.
- **加载中：** 同一 Server 与凭据上下文内刷新时保留现有列表。切换 Server 或凭据时，立即隐藏旧任务列表、选中项、上次更新时间和正在执行的行操作；只有新上下文的快照成功后才显示任务。旧请求即使迟到，也不能恢复旧任务行或对新 Server 执行行操作。

  **Loading:** keep the existing list visible for refreshes of the same Server and credential context. When either changes, hide the old task list, selection, last-update time, and in-flight row actions immediately; show tasks only after the new context's snapshot succeeds. A late response from the previous context cannot restore its rows or trigger a row action against the new Server.
- **请求失败：** 保留最近一次成功列表，标记数据过期，并显示可重试错误条。瞬时错误不能把有用列表替换成空状态。

  **Request failed:** keep the last successful list, mark it stale, and show a recoverable error banner with Retry. Do not replace a useful list with an empty state after a transient error.
- **操作消息：** 成功操作在 Downloads 页显示可手动关闭的提示，并在五秒后自动消失。错误提示可手动关闭；可通过刷新恢复时保留“重试”入口。后续成功会清除旧错误，后台轮询不会立刻抹掉它。

  **Operation messages:** successful task actions appear in a dismissible Downloads banner for five seconds. Errors remain dismissible with Retry when a refresh can recover the view. A later success clears the previous error; background polling does not immediately erase it.
- **不支持来源：** 提交前校验 HTTP/HTTPS，并说明 Magnet/本地文件传输尚未可用。

  **Unsupported source:** validate HTTP/HTTPS before submission and explain that Magnet/local-file transfers are not available yet.

<a id="4-add-download-sheet"></a>

## 4. Add Download Sheet / 4. Add Download sheet

Sheet 只承担一条清晰流程：

The sheet has one focused flow:

1. 较高的 Source URL 文本框会自动软换行。每次只接收一个 HTTP/HTTPS URL；实际换行视为输入错误。

   A taller Source URL text area soft-wraps long links. It accepts one HTTP/HTTPS URL; a hard line break is an error.
2. 先根据 URL 路径最后一段解码并立即显示可编辑的文件名建议，再短时探测 HTTP 响应头中的 `Content-Disposition`（优先使用有效的 UTF-8 `filename*`，其次使用 `filename`）。响应头没有安全名称时，可使用重定向后的最终 URL 路径。探测先发 HEAD，必要时再发只读取响应头的带单字节 Range 的 GET；不读取响应正文。探测有较短的总超时，失败时保留路径建议。未经手动编辑的文件名正在探测时禁用提交，避免快速提交把 GitHub codeload 链接的 `main` 路径段当作文件名。手动修改立即优先，后续探测和 URL 改动不得覆盖；URL 改动或关闭 Sheet 后忽略过期结果。

   Show an immediate editable suggestion from the decoded last URL path segment, then briefly probe HTTP response headers for `Content-Disposition` (preferring a valid UTF-8 `filename*`, then `filename`). If the header has no safe name, use the final redirected URL path when available. The probe uses HEAD and, when needed, a headers-only GET with a one-byte Range request; it never reads the response body. It has a short overall timeout and failures keep the path suggestion. Disable submission while an unedited name is being checked so a quick submit cannot save a misleading path basename such as `main` from a GitHub codeload URL. A manual edit takes precedence immediately; later probes and URL changes cannot replace it. Ignore stale results after editing the URL or closing the sheet.
3. 托管或回环外部模式下，每次新建 Sheet 使用保存的默认目录或系统 Downloads 目录，可用原生目录选择器为当前任务另选文件夹。连接远程 Server 时，需输入 Server 主机上的绝对文件夹路径。提交前显示最终完整路径。

   In Managed or loopback External mode, a new sheet uses the saved default folder or this Mac's system Downloads directory. The native directory picker can choose another folder for this task. With a remote Server, enter an absolute folder path on the Server host. Show the resulting full path before submission.
4. Advanced 折叠区预留 headers、优先级和带宽策略；Server 支持前保持隐藏。

   Advanced disclosure for future headers, priority, and bandwidth policy; hidden until those server features exist.
5. Cancel 与 Add Download 按钮。

   Cancel and Add Download buttons.

提交时创建任务并立即排队。只有两个 RPC 都成功后才关闭 Sheet。创建成功但排队失败后，不重复创建任务的重试流程仍待实现。

Submit creates the task and immediately queues it. The sheet closes only after both RPC calls succeed. Retrying queueing without creating a duplicate after a partial failure remains planned.

Server RPC 要求 ID 和目标路径；Desktop Rust 后端生成 ID，校验 HTTP(S) URL、单一安全文件名和目录，再分配目标。托管模式默认使用系统 Downloads 或已保存的默认目录，可用原生选择器为单项另选文件夹；文件名会绕开任务占用、完整文件、部分文件和 sidecar 自动添加序号。远程外部目录属于 Server 主机，支持绝对 POSIX、Windows 驱动器和 UNC 路径；Mac 不检查远程文件系统。分配不创建最终文件，Engine 的不覆盖提交继续防止后续竞态。

The Server RPC requires an ID and destination; the Desktop Rust backend generates the ID, validates the HTTP(S) URL, single safe filename and folder, and chooses the destination. Managed mode defaults to system Downloads or the saved default folder, permits a native per-task folder override, and allocates a numbered filename around task reservations, final files, partial files and sidecars. A remote External folder belongs to the Server host, including absolute POSIX, Windows drive and UNC paths; the Mac does not inspect that filesystem. Allocation does not create a final file; the Engine retains no-clobber commit protection against later races.

<a id="5-settings"></a>
<a id="5-设置"></a>

## 5. 设置 / 5. Settings

设置是一个独立页面，按分区组织内容，不是第二个任务工作流：

Settings is a dedicated page with grouped sections. It is not a second task workflow:

- **连接：** “连接方式”提供“本地（推荐）”和“远程”。本地自动启动随包下载服务，显示启动中、就绪、恢复中或失败状态和重试。远程连接指定下载服务，也支持本机独立运行的服务，显示保存的服务地址（默认 `127.0.0.1:39100`）及 Keychain 操作。托管端点和凭据不展示。界面名称分别对应既有 `managed` 和 `external` 模式，不改变连接或凭据行为。默认下载目录选择和重置为 Downloads 随设置保存。

  **Connection:** “Connection mode” offers “Local (recommended)” and “Remote”. Local starts the bundled download service automatically and shows starting, running, recovering or failed status with Retry. Remote connects to a specified download service, including one running separately on this Mac, and exposes the saved service address (default `127.0.0.1:39100`) and Keychain controls. The managed endpoint and credential are not shown. These UI names map to the existing `managed` and `external` modes without changing connection or credential behavior. Default download folder selection and Reset to Downloads are saved with the settings.
- **认证：** 在通用设置中针对当前已保存的 Server 显示 `Bearer`/`ApiKey` scheme 选择器、只写 secret 输入框、已配置 scheme 状态，以及保存/清除操作。将 scheme 和 secret 一起按 Server 地址保存在 macOS Keychain，不能把 secret 回读到 React。保存、清除或切换当前 Server 后重启事件订阅并刷新 Task 快照。允许为回环明文地址和显式 `tls://` 地址保存凭据。明文凭据仍只允许回环地址；TLS 客户端会先校验 Server 名称和系统根证书链，再发送凭据。

  **Authentication:** in General settings for the active saved Server, show a `Bearer`/`ApiKey` scheme selector, write-only secret field, configured-scheme status, and Save/Clear actions. Save the scheme and secret together in macOS Keychain under that Server address; never read the secret back into React. Saving, clearing, or changing the active Server restarts the event subscription and refreshes the task snapshot. Enable credential storage for loopback plaintext addresses and explicit `tls://` addresses. Plaintext credentials remain loopback-only; the TLS client validates the server name and system root chain before sending a credential.
- **更新：** 自动策略使用事件刷新；事件流断开时回退到空闲每五秒、下载中每秒的轮询。手动策略会禁用该轮询回退。

  **Updates:** Automatic uses event refreshes and a five-second idle or one-second active polling fallback when the stream is disconnected; Manual disables that polling fallback.
- **外观：** 壳层跟随系统明暗外观；跟随系统/英文/简体中文语言选择即时保存，并与 Server 地址、刷新策略一起持久化。手动主题和强调色选择仍待实现。

  **Appearance:** the shell follows the system light/dark appearance. A System/English/Simplified Chinese language selector saves immediately and persists with the Server address and refresh policy. Manual theme and accent-color choices remain planned.
- **通知：** 在本次 Desktop 会话的活动中心显示任务操作、完成、失败和重试事件。卡片使用内容区可用宽度，不留下无用的右侧空列。侧边栏角标只计未读项，打开通知页即标记为已读。若相关任务仍在当前 Server，点击通知会切换到 Downloads、清除遮挡该任务的过滤或搜索条件、选中该任务、将其滚动到可见区域并聚焦，同时打开 Inspector；已移除或不可用的任务不显示失效的跳转操作。每条通知可单独清除，也可全部清除。内存中只保留最新 30 条，重启应用后不持久化；通知偏好与静音控制留待后续切片。

  **Notifications:** the session Activity Center shows task actions, completion, failure, and retry events. Its cards use the available content width instead of leaving an unused right column. The sidebar badge counts unread entries and clears when the page opens. Entries for tasks still present on the current Server open Downloads, clear filters and search, select the task, scroll its row into view, focus it, and show the Inspector. Removed or unavailable tasks have no dead navigation action. Each entry can be cleared, and Clear all removes the session list. Only the newest 30 entries are retained in memory; they are not persisted across app restarts. Delivery preferences and mute controls remain planned.
- 托管模式说明关闭窗口继续下载、退出 App 停止；外部模式不停止外部 Server。

  Managed mode explains that downloads continue when the window closes and stop when the App quits; External mode leaves that Server running.

通过 Tauri 命令将地址、刷新策略和语言偏好保存到 macOS Application Support 目录。凭据只保存在 macOS Keychain。Tauri 后端会为普通 RPC 和独立的 `events.subscribe` 请求附加凭据，Keychain 失败时显示不含 secret 的错误。其他平台的 Desktop 可以无认证连接，但不支持保存或清除凭据。

Persist the address, refresh policy, and language preference in the macOS application support directory through Tauri commands. Keep credentials only in macOS Keychain. The Tauri backend attaches the credential to ordinary RPC calls and the dedicated `events.subscribe` request, and reports Keychain failures without revealing the secret. Desktop on other platforms can connect without authentication, but credential Save and Clear are unsupported.

<a id="6-state-model-and-data-freshness"></a>
<a id="6-状态模型与数据新鲜度"></a>

## 6. 状态模型与数据新鲜度 / 6. State model and data freshness

React 层使用小型客户端状态模型：

The React layer should consume a small client state model:

```text
ConnectionState = disconnected | connecting | connected | error
TaskData = { items, selectedId, lastUpdatedAt }
OperationState = { [taskId]: idle | running }
```

Desktop 通过 Tauri 后台线程建立独立的 `events.subscribe` TCP 连接。Server 发送带增量 Task 或 Scheduler 数据的 `events.event` JSON-RPC 通知，并每 15 秒发送一次 heartbeat 保持连接。Tauri 会忽略 heartbeat 更新，在每条连接内跟踪 sequence，并将重复或过期通知视为无害。如果发现前进方向的 sequence 缺口，Tauri 会关闭流，由重连流程触发完整快照；由于更新通知不包含完整 `TaskView`，React 会对 Task 和 Scheduler 通知去抖后刷新完整 Task 快照。

The Desktop opens a dedicated `events.subscribe` TCP connection through a Tauri background thread. The Server sends `events.event` JSON-RPC notifications with incremental Task or Scheduler data, plus a heartbeat every 15 seconds to keep the connection alive. Tauri ignores heartbeat updates, tracks the sequence within each connection, and treats duplicate or stale notifications as harmless. A forward sequence gap closes the stream so reconnect logic can trigger a full snapshot; React debounces task and scheduler notifications and refreshes the full task snapshot because an update does not contain a complete `TaskView`.

事件流连接时使用事件驱动刷新：

While the event stream is connected, use event-driven refreshes:

- 订阅有效时显示实时更新状态。

  Show a live-updates status when the subscription is active.
- 对进度事件做短暂去抖，避免并发发起多个 `task.list` 请求。

  Refresh after a short debounce so progress bursts do not create overlapping `task.list` calls.
- 保留手动 Refresh 作为恢复手段。

  Keep manual Refresh as a recovery action.

事件流断开时使用自适应轮询回退：

When the stream is disconnected, use adaptive polling as a fallback:

- 有任务处于 Downloading 或有请求进行中时，每 1 秒轮询。

  1 second while any task is Downloading or a request is active.
- 已连接且空闲时，每 5 秒轮询。

  5 seconds while connected and idle.
- 窗口隐藏时停止轮询。

  Stop polling when the window is hidden.
- 保存 `lastUpdatedAt`，在工具栏显示“刚刚更新 / X 前更新”。

  Keep `lastUpdatedAt` and show “Updated just now / X ago” in the toolbar.

事件流没有回放缓冲。重连、订阅被丢弃或检测到 sequence 缺口后先获取完整 Task 快照；刷新失败时仍保留最近一次成功快照。

The stream has no replay buffer. A reconnect, dropped subscription, or detected sequence gap starts with a full task snapshot, and the UI continues to keep the last successful snapshot when a refresh fails.

RPC 边界留在 Rust/Tauri 命令中。React 只负责展示状态，不能直接打开 TCP Socket 或解析 JSON-RPC 错误。

The RPC boundary stays in Rust/Tauri commands. React owns presentation state and never opens a TCP socket or interprets JSON-RPC errors directly.

<a id="7-macos-interaction-and-accessibility"></a>
<a id="7-macos-交互与可访问性"></a>

## 7. macOS 交互与可访问性 / 7. macOS interaction and accessibility

- `⌘N` 打开 Add Download；`⌘F` 打开并聚焦 Downloads 搜索。Escape 关闭已打开的 Sheet 或搜索框。侧边栏和操作控件仍按正常 Tab 顺序访问。

  `⌘N` opens Add Download; `⌘F` opens and focuses Downloads search. Escape closes the open sheet or search field. Sidebar and action controls remain in the normal Tab order.
- 任务选择控件是按钮：Enter 或 Space 选中任务，上/下方向键在可见任务之间移动，Home/End 移到首个或最后一个可见任务。任务操作使用独立按钮。

  Task selection is a button: Enter or Space selects it, Up/Down moves between visible tasks, and Home/End moves to the first or last visible task. Row actions are separate buttons.
- Add Sheet 首先聚焦 Source URL，Tab 和 Shift+Tab 保持在弹窗内，关闭后将焦点还给打开弹窗的控件。原生目标选择器关闭后焦点回到 Choose 按钮。

  The Add sheet focuses Source URL, keeps Tab and Shift+Tab inside the dialog, and returns focus to the invoking control when closed. The native destination picker returns focus to Choose after it closes.
- 导航使用 `aria-current`；任务选择控件读出状态与进度，Inspector 进度有进度条语义，并在总大小未知时读出任务状态；任务操作有可访问名称。错误和操作消息使用 alert/status 语义，不播报每次进度变化。

  Navigation uses `aria-current`; task selection announces task state and progress, Inspector progress has progress-bar semantics and includes the task state when total size is unknown, and task actions have accessible names. Error and operation messages use alert/status semantics without announcing every progress tick.
- CSS 针对 `prefers-reduced-motion` 关闭过渡、悬停位移和活跃任务的不确定进度动画，并通过 `prefers-color-scheme` 跟随系统明暗外观。

  CSS respects `prefers-reduced-motion` for transitions, hover movement, and active indeterminate progress animation, and `prefers-color-scheme` for light/dark palettes.
- 类型化中英文文案目录覆盖 Desktop 自有的可见及无障碍文案。任务原生上下文菜单和明确的删除确认文案仍待实现。

  A typed English/Simplified Chinese message catalog covers Desktop-owned visible and accessibility-facing strings. Native context menus for task actions and destructive confirmation wording remain planned.

<a id="8-implementation-slices"></a>
<a id="8-实现切片"></a>

## 8. 实现切片 / 8. Implementation slices

<a id="slice-a--shell-and-reliable-state-implemented"></a>
<a id="slice-a--壳层与可靠状态已实现"></a>

### Slice A — 壳层与可靠状态（已实现） / Slice A — shell and reliable state (implemented)

- 用侧边栏、工具栏、列表和 Inspector 替换顶部 Tab。[x]

  Replace top tabs with sidebar, toolbar, list, and inspector. [x]
- 在 React 中建立类型化任务、连接和操作状态。[x]

  Introduce typed task/connection/operation state in React. [x]
- 修复按操作独立 loading，并在错误时保留最近一次成功列表。[x]

  Fix per-action loading and preserve the last successful list on errors. [x]
- 保持当前 TCP RPC 命令不变。[x]

  Keep current TCP RPC commands unchanged. [x]

<a id="slice-b--add-sheet-and-native-settings-partially-implemented"></a>
<a id="slice-b--add-sheet-与原生设置部分实现"></a>

### Slice B — Add Sheet 与原生设置（部分实现） / Slice B — Add sheet and native settings (partially implemented)

- 增加 URL 校验。[x]

  Add URL validation. [x]
- 内部生成任务 ID，并改用文件名展示任务标题。[x]

  Generate task IDs internally and show filename-based task titles instead. [x]
- 增加原生文件夹选择器、系统 Downloads 默认位置、可编辑的推测文件名和最终路径预览。[x]

  Add a native folder picker, system Downloads default, editable suggested file name, and visible target path. [x]
- 增加读取/保存 Desktop 设置的 Tauri 命令。[x]

  Add Tauri commands for loading and saving desktop settings. [x]
- 增加按 Server 地址存储的 macOS Keychain 凭据保存/清除操作，并为 RPC 和事件订阅附加凭据。[x]

  Add per-Server macOS Keychain credential Save/Clear controls and attach credentials to RPC and event subscriptions. [x]
- Add 流程在创建任务后自动排队。[x]

  Queue a newly created task as part of the Add flow. [x]
- 在 Add 流程部分失败后保留已创建 ID 和实际目标，原位重试排队而不重复创建任务；关闭 Sheet 或切换 Server、凭据、托管进程代次后废弃该重试上下文。[x]

  Retain the created ID and allocated destination after a partial Add failure, retrying queueing in the sheet without creating another task. Closing the sheet or changing the Server, credential or managed-child generation discards that retry context. [x]

<a id="slice-c--live-task-surface-partially-implemented"></a>
<a id="slice-c--实时任务界面部分实现"></a>

### Slice C — 实时任务界面（部分实现） / Slice C — live task surface (partially implemented)

- 增加自适应轮询和刷新时间。[x]

  Add adaptive polling and refresh timestamps. [x]
- 增加进度条、状态徽标和 Inspector 详情。[x]

  Add progress bars, state badges, and inspector details. [x]
- 增加独立 Server 事件订阅、Tauri 事件桥接和去抖任务刷新。[x]

  Add the dedicated Server event subscription, Tauri event bridge, and debounced task refresh. [x]
- 为请求错误后保留的快照增加明确的过期数据标记；重试期间保留，成功取得同一上下文的新快照后清除。[x]

  Mark retained snapshots stale after request errors, keeping that indicator during retries and clearing it after a fresh snapshot succeeds in the same context. [x]
- 增加键盘导航、快捷键、焦点管理和可访问性标签。[x]

  Add keyboard navigation, shortcuts, focus management, and accessibility labels. [x]
- 桌面端过渡动画遵循减少动效设置。[x]

  Respect reduced-motion preferences for desktop transitions. [x]
- 为任务操作增加原生上下文菜单。[ ]

  Add native context menus for task actions. [ ]

<a id="slice-d--macos-release-polish"></a>
<a id="slice-d--macos-发布打磨"></a>

### Slice D — macOS 发布打磨 / Slice D — macOS release polish

- 跟随系统明暗外观，并增加中英文 UI 文案。[x]

  Follow the system light/dark appearance and add English/Simplified Chinese UI strings. [x]
- 增加明暗模式截图和视觉回归检查。

  Add light/dark screenshots and visual regression checks.
- 在 macOS 上构建前端和 Tauri 应用。

  Build the frontend and Tauri app on macOS.
- 具备发布签名与身份后生成签名/公证的 `.app`/`.dmg`。

  Produce a signed/notarized `.app`/`.dmg` when release credentials and identity are available.

<a id="9-acceptance-criteria"></a>
<a id="9-验收标准"></a>

## 9. 验收标准 / 9. Acceptance criteria

当前实现已满足以下基线行为：

The current implementation satisfies these baseline behaviors:

- 不打开详情也能识别连接状态和任务状态。

  A user can identify connection state and task state without opening a detail view.
- 添加下载不需要手写 JSON-RPC 或执行 CLI。

  Adding a download requires no manual JSON-RPC or CLI step.
- 事件流连接时，活动进度无需按 Refresh 就会更新；事件流不可用时回退到轮询。

  Active progress updates without pressing Refresh when the event stream is connected, with polling fallback when it is not.
- 请求失败不会清空最近一次有用的任务列表。

  A failed request does not erase the last useful task list.
- 只有合法状态显示暂停、恢复和删除操作，并报告执行结果。

  Pause, resume, and remove actions are available only in valid states and report their result.
- 重启应用后保留 Server 地址和刷新策略。

  Restarting the app preserves the Server address and refresh policy.
- 传输 Engine 尚未实现前，UI 不宣称支持 Magnet/本地文件。

  The UI never claims Magnet/local-file support before the transfer engines exist.
- 核心流程具备跟随系统的明暗配色、中英文 UI 文案、键盘导航、可访问控件、焦点管理和减少动效。原生 macOS 视觉和 VoiceOver 发布检查仍待执行。

  The same flows render with system light/dark palettes, English/Simplified Chinese UI strings, keyboard navigation, accessible controls, managed focus, and reduced-motion behavior. Native macOS visual and VoiceOver release checks remain pending.
