# Nexum 开发计划

本计划区分“代码中已有基础能力”和“当前 Server/客户端可端到端使用的功能”。勾选表示仓库中已有对应实现；未勾选表示仍需实现或接线。计划项目仅说明方向，不承诺固定的发布时间。当前调用路径见[架构设计](ARCHITECTURE.zh-CN.md)，工程和提交规范见[贡献指南](../CONTRIBUTING.zh-CN.md)。

## 1. 开发阶段

### Phase 0 - 项目基础

- [x] 仓库结构、MIT License、贡献与治理文档
- [x] 英文和简体中文文档
- [x] 包含 Server、CLI、Desktop Tauri crate 和 Core crates 的 Rust workspace
- [x] GitHub CI 工作流已配置格式检查、workspace check/test 和 Clippy

### Phase 1 - Domain 与 Task Core

- [x] Domain 值类型和下载任务模型
- [x] 带校验的任务状态机
- [x] 内存任务服务与 Task Events
- [x] 任务生命周期和状态转换单元测试

### Phase 2 - Scheduler

- [x] 优先级队列与任务并发上限
- [x] 重试策略和暂停/恢复操作
- [x] 带宽策略接口与限速值计算
- [x] Scheduler Events 与受控单元测试
- [ ] 将计算出的带宽限制应用到传输
- [x] Server 派发器在有可用槽位时自动填充排队任务和重试；Scheduler 仍不负责传输

### Phase 3 - Storage

- [x] `TaskRepository` 与内存实现
- [x] 带 Schema Version 和 Migration 的 SQLite 任务元数据/进度 Repository
- [x] 重建排队任务并持久化恢复状态的 `Core::recover`
- [x] 在 Server 启动路径打开 SQLite 并执行恢复
- [x] 验证真实 Server 重启后任务连续性

### Phase 4 - Resolver

- [x] Resolver 请求/结果/错误模型与 Registry
- [x] HTTP/HTTPS 校验、Magnet `xt=urn:btih:` 参数存在性检查、已有本地路径校验
- [x] Resolver 测试与 Core 创建任务时的来源校验
- [x] 在 `task.queue`、启动恢复或 Worker 完成/失败后，通过 Server 派发器将符合条件的 HTTP/HTTPS 任务路由到 Worker
- [ ] 为 Magnet 和本地文件来源接入兼容的传输 Engine
- [ ] 增加真实 Magnet 与本地来源传输路径

### Phase 5 - Engine Adapter

- [x] Adapter Capability、Task Mapping 与 Engine Registry
- [x] 模拟 InMemory Engine 和支持重定向的阻塞式 HTTP GET Engine
- [x] 受控 Engine 测试
- [x] 由 Server 派发器运行 HTTP Engine；CLI 与 Desktop 使用同一个 RPC，并仍可用 `task.start` 手动 kick
- [x] 将 HTTP 下载暂存到 `.part` 文件，仅在响应完整后重命名到目标路径
- [x] 为真实传输增加增量进度报告与持久化
- [x] 为 Server HTTP 传输增加协作式取消和同进程暂停/恢复
- [x] 增加跨重启 HTTP 续传：目标旁稳定的部分文件、原子 sidecar 元数据、ETag/Last-Modified 校验和 `Range`/`If-Range` 请求；校验器或响应范围不匹配时从零重试

### Phase 6 - Nexum Protocol 与 Security

- [x] JSON-RPC 2.0 请求/响应与错误对象
- [x] Task 创建/查询/列表/排队/启动/暂停/恢复/删除，以及 Server 信息查询方法
- [x] V1 版本类型、请求字段和 `server.version` 方法
- [x] Task/Scheduler 事件信封与缓存
- [x] Credential、TLS、限流类型及 Protocol/Security 单元测试
- [ ] 在信封校验之外执行 Protocol 兼容性检查
- [x] 通过独立 TCP 订阅发布事件并让客户端接收；事件流没有回放缓冲，重连时先获取完整 Task 快照
- [x] 按配置校验并执行 Bearer/ApiKey Credential，覆盖 RPC 请求和事件订阅
- [x] 按配置对 TCP 和 HTTP `/jsonrpc` 执行可选的 RPC 限流
- [ ] 按配置执行 TLS

### Phase 7 - Server 与 CLI

- [x] 使用按行 JSON-RPC 的本地回环 TCP Server
- [x] 支持任务控制、地址配置和 Server 信息查询的 CLI TCP 客户端
- [x] Server 命令行选项与 key-value 配置解析
- [x] 在 TCP 接入时执行 `max_connections`；超出上限的连接在处理请求前关闭
- [x] 在 RPC 分发和事件订阅前执行 `require_auth`、`auth_scheme` 与 `auth_token` 校验
- [x] 利用 `data_dir` 实现 SQLite 持久化与重启恢复
- [x] 让普通 `task.start` 对支持的 HTTP/HTTPS 来源启动真实下载
- [x] 持久化传输错误并通过任务视图返回，而不只写入 Server 日志
- [x] 在有可用 Scheduler 槽位时自动派发排队重试和新排队的 HTTP/HTTPS 任务

### Phase 8 - Desktop

- [x] Tauri 2 + React 应用与 TCP JSON-RPC 命令桥接
- [x] Task 列表、添加、排队/启动、暂停/恢复与删除 UI
- [x] 可编辑 Server 地址，操作或地址变更后刷新
- [x] 记录 macOS 优先的侧边栏、任务列表、Inspector、Add Sheet 与设置流程（[Desktop UI 设计](DESKTOP_UI_DESIGN.zh-CN.md)）
- [x] 用文档中的侧边栏/列表/Inspector 壳层替换原型 Tab 布局
- [x] 增加创建并排队任务的 Add Download Sheet
- [x] 通过 Tauri dialog plugin 为 Add Download Sheet 增加原生目标选择器
- [x] 增加 Tauri 事件订阅和去抖任务刷新；事件流断开时保留定时轮询作为回退
- [x] 通过 Tauri 命令持久化 Server 地址和刷新策略
- [x] 在不丢弃最近一次成功任务列表的情况下呈现连接/操作错误
- [x] 为每个 Server 将 Bearer/ApiKey 凭据保存到 macOS Keychain，让普通 RPC 和事件订阅都携带凭据，并在 Desktop 设置中保存/清除；Desktop TLS 客户端和信任设置接入前仅允许回环地址使用凭据
- [ ] 增加键盘导航、可访问性标签、系统外观和中英文 UI 文案

### Phase 9 - Browser 集成

- [x] Manifest V3 扩展骨架、链接右键菜单和可下载链接标记启发式逻辑
- [x] 保存单个 Server 地址的 Popup 字段
- [x] 使用回环 HTTP `/jsonrpc` 桥接完成 Send-to-Nexum；桥接接受 CORS POST，并执行 RPC 认证门
- [x] 在后台脚本使用已保存的 `server` 地址，并将创建的任务排队
- [x] 处理 Content Script 的发送消息、解析绝对链接，并覆盖端到端任务创建桥接测试
- [ ] 若多设备投递仍是产品需求，增加设备选择

### Phase 10 - 可扩展性

- [x] Plugin Manifest、Permission 和 Capability 数据模型
- [x] `PluginManager` 状态转换与测试
- [x] `EngineProvider` 和 `ResolverProvider` Trait
- [ ] 调用插件生命周期实现并加载可执行插件入口
- [ ] 向 Core 注册插件提供的 Engine/Resolver；当前初始化只改变 Manager 状态
- [ ] 执行 Permission 约束，并定义稳定的 SDK/Runtime 合约

### Phase 11 - Media 与 Automation

- [x] Media、Job、Workflow 与 MCP Request 基础类型
- [x] Workflow 依赖排序和模拟的内存 Job API
- [ ] 探测真实媒体并解析 Manifest
- [ ] 实现 Track Selection、Segment Scheduling、Mux 与后处理
- [ ] 执行并持久化真实 Job/Workflow，而不是构造模拟完成结果
- [ ] 对外提供 Automation 端点，并按需集成 AI/MCP
- [ ] 实现远程设备管理

## 2. 基于当前代码的实施顺序

1. 补齐剩余的 Server/客户端合约：有界事件流的可靠交付语义，以及按配置启用的 TLS。
   - [x] 明确 v1 断线重连和事件丢弃行为：Desktop 忽略重复/过期 sequence，将前进方向的缺口或断线视为状态过期，重连并刷新完整快照；Server 仍没有回放缓冲。
   - [ ] 在需要可靠事件投递时增加持久回放缓冲和显式 after-sequence 订阅。
   - [x] 增加可选的进程级 RPC 限流，让 TCP 和 HTTP `/jsonrpc` 共用令牌桶。计入已解析且通过认证的请求（含事件订阅），不计入 heartbeat 和 HTTP 预检；带 `id` 的超额调用返回 JSON-RPC `-32002`，无 `id` 的超额通知不响应。默认关闭，配置无效或不完整时拒绝启动。
   - [x] 增加基于 macOS Keychain 的逐 Server Desktop 凭据设置；普通 RPC 和事件订阅都携带凭据，凭据变化时重启事件流。
   - [ ] 在确定合适的存储和配对流程后增加 Browser 凭据设置。

### TLS 实施计划（进行中；Server 传输切片已启用）

- [ ] 定义唯一且显式的传输语法：不带 scheme 的 `host:port` 为兼容保留的明文连接，`tls://host:port` 选择 TLS；证书或握手失败时绝不回退到匿名明文。
- [x] 增加 Server `tls_cert_path`/`tls_key_path` 成对校验，并在监听前加载 PEM 材料；TCP JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc` 桥接共用加密流。客户端信任和地址解析仍待接入。
- [ ] 增加共用客户端信任处理，默认使用系统根证书，并支持显式 CA 或证书固定；主机名、证书链或信任失败时不得发送凭据。
- [ ] 分别接入 CLI、Desktop RPC/事件流和 Browser HTTPS 桥接；TLS 关闭时保持现有明文回环路径。
- [ ] 将传输身份纳入 Desktop 凭据作用域；已校验的 TLS 端点可以使用凭据，明文凭据仍只允许发给实际回环 peer。
- [ ] 使用生成证书增加握手、无效配置、禁止降级、RPC/事件/HTTPS、凭据策略和 CORS 集成测试；私钥不得进入仓库。

分阶段传输决策及其不包含项见 [ADR 0005](decisions/0005-tls-transport.zh-CN.md)。
2. 完成 Desktop 发布层：键盘导航、可访问性标签、减少动效/系统外观行为，以及中英文文案。
3. 接入 Plugin Provider 并执行其声明的权限。
4. 用真实处理替换模拟的 Media 操作，再对外提供 Automation 与远程设备工作流。

## 3. 当前重点

Phase 0-9 各自具有不同程度的脚手架和库级覆盖。运行中的 Server 使用 SQLite 持久化任务并在重启后恢复，在任务入队、启动恢复以及 Worker 完成或失败后自动派发符合条件的 HTTP/HTTPS 工作。`task.start` 仍是手动 kick 和兼容接口。Server 会节流持久化中间进度，通过任务视图返回最近一次传输错误，并在传输成功后写入最终进度与完成状态。活跃的 Server HTTP 传输支持协作式块边界暂停、同进程恢复和破坏性删除取消；阻塞中的响应读取可能让 `task.pause` 等到 30 分钟 HTTP 超时，`task.remove` 无法及时停止时会在等待 Worker 30 秒后返回错误。Server 现在会在进程重启后保留并校验 HTTP 部分响应。只有 sidecar 匹配且服务端返回 `206 Partial Content` 时才续传；无效或没有校验器的响应会被丢弃并从零下载。Task 与 Scheduler Events 现在由 Server Event Pump 从 Core 取出，并通过独立的 `events.subscribe` TCP 流发送；Desktop 对通知去抖后刷新完整 Task 快照，事件流不可用时按已配置的刷新策略回退到轮询。Server 还通过同一认证门提供单请求 CORS HTTP `/jsonrpc`，Browser Extension 可用它创建并排队任务。可选的进程级限流现在覆盖通过认证的 TCP 与 HTTP RPC 请求。Server 现在可通过成对配置的 PEM 证书和私钥启用 TLS，同一加密流承载 TCP JSON-RPC、事件订阅和 HTTP 桥接；客户端地址解析和信任设置仍待接入。Magnet 和本地文件传输仍不受支持。Plugin 与 Media crates 已包含数据类型之外的代码，但 Provider 回调和真实处理尚未接入产品路径。macOS Desktop 信息架构已记录在[Desktop UI 设计](DESKTOP_UI_DESIGN.zh-CN.md)中；React 页面现在已有侧边栏/列表/Inspector 壳层、设置页、带原生目标选择器的 Add Download Sheet、明确的 RPC 错误展示、自适应事件刷新、持久化 Server 设置，以及针对已保存回环 Server 的 Keychain 凭据。键盘/可访问性打磨、本地化、客户端 TLS 和 Browser 凭据设置仍待完成。
