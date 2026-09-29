# Nexum

[English](README.md) · [简体中文](README.zh-CN.md)

> 用于下载任务管理与客户端集成的 Rust Workspace。

Nexum 仍在开发中。目前可运行的主线是用于创建和管理任务的本地 TCP JSON-RPC 服务。Server 使用 SQLite 保存任务。排队 HTTP/HTTPS 任务后，只要有可用的 Scheduler 槽位和目标路径，就会自动派发到后台 Worker。

[![CI](https://github.com/liveait/Nexum/actions/workflows/ci.yml/badge.svg)](https://github.com/liveait/Nexum/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## 当前状态

CLI 和早期 Tauri Desktop 客户端可调用本地 Server；Desktop 需要单独运行 Server。Server 在数据目录（默认 `./data`）下打开 `nexum.sqlite`，启动时恢复已保存的任务。排队支持的 HTTP/HTTPS 任务、带排队任务重启 Server，或活动 HTTP 传输完成/失败后，Server 都会自动填充可用的 Scheduler 槽位。中断的 HTTP 响应在稳定的部分文件、sidecar 以及 ETag 或 Last-Modified 校验器仍与服务端确认的范围匹配时，可以在重启后续传。`task.start` 仍可作为手动 kick 和兼容接口，用于启动一个排队的 HTTP/HTTPS 任务，之后同一派发器会继续填充其他可用槽位。Desktop 现在通过独立 TCP 事件流订阅进行去抖的实时任务刷新；事件流不可用时按已配置的刷新策略回退到轮询。Magnet 和本地文件来源可以创建任务，但尚无传输路径。Browser Extension 现在通过 Server 回环 HTTP `/jsonrpc` 桥接发送 `task.create` 和 `task.queue`；事件订阅仍只走 TCP。

Server 可以在启用 `require_auth` 时执行 Bearer 或 ApiKey 认证。运行中的 Server 尚未接入 TLS、限流、可执行插件或真实媒体处理。CLI 可以保存并发送凭据；当前 Desktop 客户端不会发送凭据，因此使用该客户端时应保持认证关闭。代码边界与调用路径见[架构设计](docs/ARCHITECTURE.zh-CN.md)，后续集成工作见[开发计划](docs/DEVELOPMENT_PLAN.zh-CN.md)。

## 运行本地任务流程

需要支持 Rust 2024 edition 的 Rust（1.85 或更新版本）及 Cargo。在仓库根目录的一个终端启动 Server：

```bash
cargo run -p nexum-server
```

在另一个终端使用 CLI：

```bash
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
cargo run -p nexum-cli -- task list
```

要保护 Server，请提供完整的认证配置。scheme 和 token 必须同时配置，Server 支持 `Bearer` 和 `ApiKey`：

```ini
# server.conf
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

使用 `cargo run -p nexum-server -- --config server.conf` 启动，然后为 CLI 保存相同的凭据（token 会写入 CLI 配置目录）：

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- auth clear
```

缺少、无效或不匹配的凭据会返回 JSON-RPC 错误 `-32001`（`authentication required`）。`server.auth` 只返回已配置的 scheme，不会返回 secret。若设置 `require_auth=true` 却没有有效 scheme 和非空 token，Server 会拒绝启动。认证会覆盖普通 RPC 请求和 `events.subscribe`；客户端必须为这两类请求附加匹配凭据，CLI 会为自己的任务和 Server 请求附加保存的凭据。

`task.queue` 在任务持久化并完成一次可用 HTTP Worker 槽位的派发尝试后返回。`task.start` 仍可作为手动 kick，启动一个 HTTP Worker 后返回。Worker 每写入一个响应块就报告进度；Server 自上次写入起累计至少 1 MiB 或经过 250 ms 时持久化一次中间快照，并在任务标记为 `Completed` 前刷新最终快照。成功后会写入 `./example.html` 并记录最终下载字节数；此期间 `task list` 可能显示 `Downloading`。`task.get` 和 `task.list` 返回持久化进度，以及表示最近一次传输错误的 `error` 字段。完整响应先写入目标旁的隐藏稳定部分文件，sidecar 保存来源、目标、校验器和预期长度，完成后才替换目标文件。传输失败会保留已有目标文件、记录 Server 错误、将错误保存到任务，并在重试策略允许时重新排队；有可用槽位时 Server 会自动派发该重试。默认策略允许重试三次。领取新一轮传输时会重置持久化快照并清除旧错误；如果有可用的校验部分文件，Server 随后会恢复其进度。活跃 HTTP 传输可以在响应块边界暂停，并在同一 Server 进程内恢复；暂停期间会保留临时文件和 HTTP 响应。`task.remove` 会取消活跃 HTTP Worker、删除任务，并保留原有目标文件。`task.pause` 可能因阻塞的响应读取等待到 30 分钟 HTTP 请求超时。`task.remove` 最多等待 Worker 30 秒；超时会返回错误，Worker 可能继续阻塞到该 HTTP 超时，退出后可以重试删除。Server 重启后，只有 sidecar 校验器匹配且服务端返回匹配的 `206 Partial Content` 时，中断的 HTTP 响应才会使用 `Range` 与 `If-Range` 续传。收到 `200`、校验器变化或缺失、`Content-Range` 无效或长度不一致时，会删除部分响应并从零开始；没有 ETag 或 Last-Modified 的响应在中断后也会从零开始。Server 默认绑定 `127.0.0.1:39100`，可用 `--port PORT` 修改端口，CLI 可用 `--server ADDR` 修改连接地址。认证默认关闭；启用后，Server 会在分发任何 RPC 前同时比较配置的 scheme 和 secret，并且不会记录 secret。客户端设置、认证配置、完整 Rust 检查命令和平台依赖见[开发指南](docs/DEVELOPMENT.zh-CN.md)。

## 文档与贡献

- [架构设计](docs/ARCHITECTURE.zh-CN.md)
- [开发指南](docs/DEVELOPMENT.zh-CN.md)
- [开发计划](docs/DEVELOPMENT_PLAN.zh-CN.md)
- [Desktop UI 设计](docs/DESKTOP_UI_DESIGN.zh-CN.md)
- [变更日志](CHANGELOG.zh-CN.md)
- [架构决策](docs/decisions/README.zh-CN.md)
- [贡献指南](CONTRIBUTING.zh-CN.md)
- [治理规则](GOVERNANCE.zh-CN.md)
- [English README](README.md)

Nexum 采用 [MIT License](LICENSE)。第三方依赖仍受其各自许可证约束。
