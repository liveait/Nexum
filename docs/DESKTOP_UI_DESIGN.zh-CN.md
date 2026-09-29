# Nexum macOS Desktop UI 设计

状态：已实现 macOS Desktop UI 基线；剩余项目已标注为计划项。

本文定义 Tauri Desktop 的 macOS 优先信息架构和交互模型。当前 `apps/desktop/src/App.tsx` 与 `App.css` 已实现侧边栏、Downloads、Inspector、带 macOS Keychain 凭据操作的 Settings 卡片、带原生目标选择器的 Add Download Sheet、状态栏、事件驱动刷新和计划能力占位。快捷键和本地化仍属于后续切片。

英文版见 [DESKTOP_UI_DESIGN.md](DESKTOP_UI_DESIGN.md)。

## 1. 产品目标

桌面端应让本地常用流程一眼可懂：

1. 连接 Nexum Server。
2. 添加 HTTP/HTTPS 下载。
3. 排队后无需手动刷新即可看到进度。
4. 安全地暂停、恢复、重试或删除任务。
5. 任务需要处理时，可以查看目标路径和最近一次错误。

第一版 Desktop 是独立 Server 的客户端，不自动启动 Server，也不把尚未支持的 Magnet 或本地文件来源展示成可用功能。

## 2. 设计决定

### 2.1 macOS 风格的侧边栏壳层

使用 macOS 风格的侧边栏和单一任务工作区，替换当前顶部 Tab 布局。

```text
┌────────────────────────────────────────────────────────────────┐
│ 侧边栏        │ Downloads：过滤  搜索  Inspector              │
│ Dashboard     ├────────────────────────────────────────────────┤
│ Downloads     │ 任务列表                           Inspector   │
│ Trackers      │ 名称 | 状态 | 进度 | 目标路径                  │
│ Plugins       │                                                │
│               │                                                │
│ Notifications │                                                │
│ Settings      │                                                │
└───────────────┴────────────────────────────────────────────────┘
```

- 最小窗口：960×640；推荐窗口：1120×720。
- 侧边栏宽度 208–240 px，可折叠。
- 主区域以任务列表为主。窗口宽度超过 1100 px 时，选中任务在右侧打开 Inspector；较窄时使用 Sheet 或上下堆叠详情面板。
- 使用系统字体、系统强调色、系统明暗外观和原生焦点环。

### 2.2 侧边栏分组

侧边栏采用 Motrix 风格的顶层导航。任务过滤器放在 `Downloads` 内，Server 连接信息放在设置内部：

| 分组 | 用途 | 数据来源 |
| --- | --- | --- |
| Dashboard | 概览和最近活动 | `task.list` 快照 |
| Downloads | 全部、活动、排队、已完成和失败过滤 | 对 `task.list` 做客户端过滤 |
| Trackers | 预留 BitTorrent Tracker 健康状态 | 计划中的 Engine 能力 |
| Plugins | 预留插件目录和运行时 | 计划中的 Plugin 能力 |
| Notifications | 本次 Desktop 会话的任务和连接事件 | Desktop 会话状态 |
| Settings | Server 连接、外观、刷新和未来的客户端选项 | Desktop 设置 |

数据新鲜时显示每个过滤器的数量。连接不可用时隐藏数量，避免把过期数据当成当前状态。

### 2.3 工具栏

Downloads 页头提供过滤器和紧凑控制：

- **添加**：从侧边栏或悬浮按钮打开 Add Download Sheet。
- **过滤**：选择全部、活动、排队、已完成或失败任务。
- **Inspector**：切换右侧任务详情面板。
- **搜索**：在任务 ID、来源、目标路径和状态中做本地过滤。
- 任务行根据状态提供暂停、恢复、排队、启动或删除操作。
- 底部状态栏显示 HTTP/HTTPS、连接状态和最近刷新时间。

请求进行中只禁用对应任务行。快捷键和原生上下文菜单仍属于计划项。

## 3. 任务列表与 Inspector

### 3.1 任务行内容

每行只显示 Server 实际提供的值：

- 任务名称：当前使用任务 ID；协议提供文件名后再派生更友好的名称。
- 状态徽标：Created、Queued、Downloading、Paused、Completed 或 Failed。
- 进度条：知道 `total_bytes` 时使用确定进度，否则使用不确定进度。
- 字节文本：`downloaded_bytes / total_bytes`，使用可读单位。
- 目标路径，过长时截断并提供 Tooltip。
- `error` 存在时在行下显示最近一次错误。

Server 尚未提供速度、ETA、重试次数和校验器，因此暂不显示。不要从稀疏轮询数据估算这些值并造成精确承诺。

### 3.2 Inspector

当前 Inspector 按以下顺序显示选中任务：

1. 状态与进度。
2. 来源 URL。
3. 目标路径。
4. Server 地址。
5. 最近错误和复制操作。

Inspector 只读展示任务元数据。任务行提供当前任务操作；原生显示文件和破坏性确认仍属于计划项。

### 3.3 空、加载和错误状态

- **未连接：** 保留任务页面，在底部状态栏/错误条显示不可用状态，并引导用户到 Settings 修改 Server 地址。
- **已连接但没有任务：** 将“添加下载”作为主操作。
- **加载中：** 保留现有列表，只在工具栏显示小型加载指示。
- **请求失败：** 保留最近一次成功列表，标记数据过期，并显示可重试错误条。瞬时错误不能把有用列表替换成空状态。
- **不支持来源：** 提交前校验 HTTP/HTTPS，并说明 Magnet/本地文件传输尚未可用。

## 4. Add Download Sheet

Sheet 只承担一条清晰流程：

1. Source URL 输入框。
2. Destination 输入框，通过 Tauri dialog plugin 提供原生保存对话框。
3. 当前 Server 合约要求显式填写 Task ID。
4. Advanced 折叠区预留 headers、优先级和带宽策略；Server 支持前保持隐藏。
5. Cancel 与 Add Download 按钮。

提交时创建任务并立即排队。只有两个 RPC 都成功后才关闭 Sheet。如果创建成功但排队失败，保留表单并指出已创建的任务，使用户可以重试排队而不是重复创建任务。

当前 Server 要求显式任务 ID 和目标路径，因此第一版保留这两个字段。协议未来允许 ID 可选时无需改变布局。

## 5. 设置

设置是一个独立页面，按分区组织内容，不是第二个任务工作流：

- **Server：** 地址（默认 `127.0.0.1:39100`）、连接/测试按钮、最近连接结果和协议版本。
- **认证：** 在通用设置中针对当前已保存的 Server 显示 `Bearer`/`ApiKey` scheme 选择器、只写 secret 输入框、已配置 scheme 状态，以及保存/清除操作。将 scheme 和 secret 一起按 Server 地址保存在 macOS Keychain，不能把 secret 回读到 React。保存、清除或切换当前 Server 后重启事件订阅并刷新 Task 快照。允许为回环明文地址和显式 `tls://` 地址保存凭据。明文凭据仍只允许回环地址；TLS 客户端会先校验 Server 名称和系统根证书链，再发送凭据。
- **更新：** 自动策略使用事件刷新；事件流断开时回退到空闲每五秒、下载中每秒的轮询。手动策略会禁用该轮询回退。
- **外观：** 跟随系统外观；语言和强调色选择先预留给客户端设置模型。
- **通知：** 在本次 Desktop 会话的 Activity Center 显示任务完成、失败和重试事件；通知偏好与静音控制留待后续切片。
- 第一版 Server 是独立进程的说明。

通过 Tauri 命令将地址和刷新策略保存到 macOS Application Support 目录。凭据只保存在 macOS Keychain。Tauri 后端会为普通 RPC 和独立的 `events.subscribe` 请求附加凭据，Keychain 失败时显示不含 secret 的错误。其他平台的 Desktop 可以无认证连接，但不支持保存或清除凭据。

## 6. 状态模型与数据新鲜度

React 层使用小型客户端状态模型：

```text
ConnectionState = disconnected | connecting | connected | error
TaskData = { items, selectedId, lastUpdatedAt }
OperationState = { [taskId]: idle | running }
```

Desktop 通过 Tauri 后台线程建立独立的 `events.subscribe` TCP 连接。Server 发送带增量 Task 或 Scheduler 数据的 `events.event` JSON-RPC 通知，并每 15 秒发送一次 heartbeat 保持连接。Tauri 会忽略 heartbeat 更新，在每条连接内跟踪 sequence，并将重复或过期通知视为无害。如果发现前进方向的 sequence 缺口，Tauri 会关闭流，由重连流程触发完整快照；由于更新通知不包含完整 `TaskView`，React 会对 Task 和 Scheduler 通知去抖后刷新完整 Task 快照。

事件流连接时使用事件驱动刷新：

- 订阅有效时显示实时更新状态。
- 对进度事件做短暂去抖，避免并发发起多个 `task.list` 请求。
- 保留手动 Refresh 作为恢复手段。

事件流断开时使用自适应轮询回退：

- 有任务处于 Downloading 或有请求进行中时，每 1 秒轮询。
- 已连接且空闲时，每 5 秒轮询。
- 窗口隐藏时停止轮询。
- 保存 `lastUpdatedAt`，在工具栏显示“刚刚更新 / X 前更新”。

事件流没有回放缓冲。重连、订阅被丢弃或检测到 sequence 缺口后先获取完整 Task 快照；刷新失败时仍保留最近一次成功快照。

RPC 边界留在 Rust/Tauri 命令中。React 只负责展示状态，不能直接打开 TCP Socket 或解析 JSON-RPC 错误。

## 7. macOS 交互与可访问性

- 支持侧边栏、任务列表、Inspector 和 Sheet 的键盘导航。
- 为状态徽标、进度和破坏性按钮提供可访问名称。
- 连接和操作结果使用 `aria-live="polite"`；不要播报每次进度变化。
- 遵循减少动效设置和系统明暗模式。
- 除工具栏按钮外，为任务操作提供上下文菜单。
- 删除确认文案要具体指出任务，并说明 Server 会保留已有目标文件。
- 正式发布前通过小型类型化 Message Catalog 提供英文和简体中文文案。

## 8. 实现切片

### Slice A — 壳层与可靠状态（已实现）

- 用侧边栏、工具栏、列表和 Inspector 替换顶部 Tab。[x]
- 在 React 中建立类型化任务、连接和操作状态。[x]
- 修复按操作独立 loading，并在错误时保留最近一次成功列表。[x]
- 保持当前 TCP RPC 命令不变。[x]

### Slice B — Add Sheet 与原生设置（部分实现）

- 增加 URL 校验和任务 ID 推导。[ ]
- 通过 Tauri dialog plugin 增加原生目标保存对话框。[x]
- 增加读取/保存 Desktop 设置的 Tauri 命令。[x]
- 增加按 Server 地址存储的 macOS Keychain 凭据保存/清除操作，并为 RPC 和事件订阅附加凭据。[x]
- Add 流程在创建任务后自动排队。[x]

### Slice C — 实时任务界面（部分实现）

- 增加自适应轮询和刷新时间。[x]
- 增加进度条、状态徽标和 Inspector 详情。[x]
- 增加独立 Server 事件订阅、Tauri 事件桥接和去抖任务刷新。[x]
- 增加过期数据标记、快捷键、可访问性标签和上下文菜单。[ ]

### Slice D — macOS 发布打磨

- 增加明暗模式截图和视觉回归检查。
- 在 macOS 上构建前端和 Tauri 应用。
- 具备发布签名与身份后生成签名/公证的 `.app`/`.dmg`。

## 9. 验收标准

当前实现已满足以下基线行为：

- 不打开详情也能识别连接状态和任务状态。
- 添加下载不需要手写 JSON-RPC 或执行 CLI。
- 事件流连接时，活动进度无需按 Refresh 就会更新；事件流不可用时回退到轮询。
- 请求失败不会清空最近一次有用的任务列表。
- 只有合法状态显示暂停、恢复和删除操作，并报告执行结果。
- 重启应用后保留 Server 地址和刷新策略。
- 传输 Engine 尚未实现前，UI 不宣称支持 Magnet/本地文件。
- 核心流程已在深色 Motrix 风格壳层中可用；键盘导航、系统外观和本地化属于后续发布打磨。
