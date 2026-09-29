# 开发指南

## 环境

Nexum 是使用 Rust 2024 edition 的工作区，桌面客户端采用 Tauri 2。桌面前端使用 TypeScript 和 React；浏览器扩展使用 TypeScript。

- Rust stable（1.85 或更新版本）、Cargo、`rustfmt` 和 Clippy
- Node.js LTS 和 pnpm，用于两个前端包
- 运行或打包原生桌面应用时所需的 Tauri 2 系统依赖及 Tauri CLI

Ubuntu CI 在检查 Rust 工作区前安装 `libgtk-3-dev` 和 `libwebkit2gtk-4.1-dev`。本地构建还需满足对应平台的 Tauri 依赖要求。

## 工作区

根目录 `Cargo.toml` 包含下列全部 crate 和 `apps/desktop/src-tauri`。浏览器扩展是独立的前端包。

```text
crates/
  core/ domain/ task/ scheduler/ storage/ resolver/
  protocol/ plugin/ security/ media/ engine/
apps/
  server/                 # nexum-server 可执行文件
  cli/                    # nexum-cli 可执行文件
  desktop/                # React/Vite 和 src-tauri/
  extension/              # Manifest V3 扩展
```

保持 UI 逻辑与 Core 分离，避免将具体 Engine 的实现细节放入 Domain Model。行为变化应补充针对性测试，并同步更新受影响文档的中英文版本。需要 RFC 的变更类型见[贡献指南](../CONTRIBUTING.zh-CN.md)。

## Rust 检查

在仓库根目录运行与 `.github/workflows/ci.yml` 相同的命令：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

CI 在 Ubuntu 上执行这些 Rust 检查。目前未构建或检查 TypeScript 包，也不生成发布产物。

## Server 与 CLI

在一个终端启动本地 Server：

```bash
cargo run -p nexum-server -- --port 39100
```

默认绑定 `127.0.0.1:39100`，通过 TCP 提供以换行符分隔的 JSON-RPC 2.0 服务，并提供每个连接一个请求的 HTTP `POST /jsonrpc`；通过校验的浏览器扩展 Origin 会收到 CORS 头。在另一个终端运行 CLI：

```bash
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
```

编译出的可执行文件分别名为 `nexum-server` 和 `nexum-cli`。使用 `cargo run -p nexum-server -- --help` 和 `cargo run -p nexum-cli -- --help` 查看当前参数与命令。CLI 默认连接 `127.0.0.1:39100`；如需覆盖地址，将 `--server ADDR` 放在 `task` 或 `server` 前面。

Server 将任务元数据和进度保存到 `--data-dir` 下的 `nexum.sqlite`（默认 `./data`，相对于 Server 工作目录）。启动时会按需创建目录，并在退出前一直持有同目录下 `nexum.lock` 的锁，因此同一数据目录只能由一个 Server 使用。Server 在监听前打开数据库并恢复任务；目录、锁、数据库或恢复失败会阻止启动。显式传入的 `--config` 文件无法读取时也会阻止启动。重启后，原先下载中、暂停或重试中的任务会在内存和 SQLite 中变为排队状态。恢复完成并开始监听后，符合条件的排队 HTTP/HTTPS 任务会自动派发；不支持的来源或受阻的目标会继续排队。`--max-connections` 限制活动 TCP 连接处理器；超过上限的连接会在请求处理前立即关闭。认证默认关闭。要强制认证，需同时设置 `require_auth=true`、`auth_scheme=Bearer|ApiKey` 和非空 `auth_token`；`server.auth` 只返回当前 scheme，不返回 secret。认证配置不完整或不受支持时会阻止启动。

`task queue` 会持久化任务并触发 Server 派发器。派发器会持续选取符合条件的排队 HTTP/HTTPS 任务，直到达到 Scheduler 并发上限或目标路径规则不允许继续领取。`task start` 仍是手动 kick，会选取一个排队 HTTP/HTTPS 任务，启动 Worker 后返回任务 ID，然后调用同一派发器填充其他可用槽位。Worker 每写入一个响应块就报告进度；Server 在新增至少 1 MiB 或经过 250 ms 时持久化中间快照，并在任务标记为 `Completed` 前刷新最终快照。传输异步完成，可再次执行 `task list` 或 `task get ID` 查看字节数和完成状态。Server 会拒绝数据目录内的目标、符号链接目标，以及与活动传输重叠的目标。Worker 会将响应写入目标目录的隐藏稳定部分文件，并原子维护记录来源、目标、校验器和预期长度的 JSON sidecar，完整后再重命名到目标路径。传输失败会保留已有目标文件，并将错误文本保存到任务视图的 `error` 字段；在重试策略允许时重新排队，有可用槽位时派发器会自动启动重试。默认策略允许重试三次。领取新一轮传输时会清除旧错误；如果 sidecar 匹配，Server 会在领取后恢复部分字节进度。活跃 HTTP 传输可以在响应块边界暂停，并在同一 Server 进程内恢复；Worker 暂停时保留临时文件和 HTTP 响应。`task remove` 会取消活跃 HTTP Worker，最多等待其退出 30 秒后删除任务，并保留已有目标文件。超时会返回错误，Worker 可能继续阻塞到 30 分钟 HTTP 请求超时，退出后可以重试删除。阻塞中的响应读取可能让暂停等到 HTTP 请求超时；Server 重启后，只有来源、目标、ETag 或 Last-Modified 校验器、预期长度以及服务端确认的 `206 Partial Content` 范围都匹配时，才会复用稳定的部分文件和 sidecar。收到 `200`、校验器变化或缺失、范围格式错误或长度不一致时，会丢弃部分响应并从零开始；没有校验器的响应无法在重启后续传。Magnet 和本地文件来源仍无传输 Engine。


## 认证

Server 接受 `Bearer` 和 `ApiKey` 凭据。通过 `--config` 传入的 key-value 文件配置认证：

```ini
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

也可以用命令行参数覆盖：

```bash
cargo run -p nexum-server -- \
  --require-auth true \
  --auth-scheme ApiKey \
  --auth-token replace-with-a-secret
```

`require_auth=true` 要求受支持的 scheme 和非空 token；缺少任一字段、token 为空或 scheme 不受支持都会导致启动失败。启用后，Server 会在分发每个 RPC 前以及接受 `events.subscribe` 前校验请求凭据。scheme 和 secret 必须同时匹配。缺少或无效凭据会返回 JSON-RPC 错误 `-32001`，消息为 `authentication required`。`server.auth` 只返回已配置的 scheme，Server 日志和 Debug 输出会隐藏 token。

CLI 会在保存默认 Server 地址的同一配置目录中以明文保存凭据。使用以下命令设置或清除：

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- auth clear
```

`auth set` 只接受 `Bearer` 或 `ApiKey`，并拒绝空 token。`auth clear` 会将 CLI 配置中的两个凭据字段值清空。在 macOS 上，先到 Desktop 设置保存回环 Server 地址，再在“通用”中选择 `Bearer` 或 `ApiKey`、输入匹配的 secret 并保存。Tauri 后端按该地址将 scheme 与 secret 保存到 Keychain，为普通 RPC 和 `events.subscribe` 附加凭据；查询 Keychain 配置状态时只把已配置的 scheme 返回给 React。保存或清除凭据会重启事件订阅并刷新 Task 快照。secret 输入框只写；清除操作会删除 Keychain 条目。由于尚无 TLS，Desktop 不会为非回环地址保存或附加凭据，但仍可尝试无认证的 RPC 连接。其他平台的 Desktop 可以无认证连接，但不支持保存和清除凭据。Browser Extension 仍没有凭据设置。

## RPC 限流

限流默认关闭。要启用它，在 Server 的 key-value 配置文件中同时设置持续每秒请求数与令牌桶突发容量：

```ini
rate_limit_rps=10
rate_limit_burst=20
```

也可以使用对应的命令行参数：

```bash
cargo run -p nexum-server -- --rate-limit-rps 10 --rate-limit-burst 20
```

两个参数必须成对提供且取有效正值；配置无效或不完整时，Server 会拒绝启动。显式传入的 `--config` 文件无法读取时也会拒绝启动。同一进程的令牌桶由逐行 TCP 和 HTTP `POST /jsonrpc` 共用，计入已解析且通过认证的请求，包括 `events.subscribe`。认证失败不消耗令牌；Server 生成的事件 heartbeat 和 HTTP `OPTIONS` 预检请求也不消耗令牌。带 `id` 的请求超额时返回 JSON-RPC 错误 `-32002`；HTTP `/jsonrpc` 仍返回状态码 `200`，错误位于 JSON 响应体中。没有 `id` 的通知会被丢弃，不返回 JSON-RPC 响应；HTTP 使用状态码 `204`。CLI 在执行命令前会先查询版本，Desktop 启动时也会连续调用多次，因此应设置足以覆盖这些调用的突发容量（示例使用 20）。

## Desktop

在 `apps/desktop` 启动 Vite 前端：

```bash
cd apps/desktop
pnpm install
pnpm dev
```

Vite 使用 `1420` 端口。前端调用 Tauri 命令、macOS 上基于 Keychain 的凭据命令和 Tauri dialog plugin，因此测试完整应用还需要运行中的 Nexum Server 和原生 Tauri 窗口。安装 Tauri 2 CLI 后，保持 Vite 运行，并在另一个终端从 `apps/desktop` 执行 `cargo tauri dev`。`pnpm build` 会执行 TypeScript 编译和 Vite 构建；这是 Rust CI 之外的检查。在 `apps/desktop` 执行 `pnpm dlx @tauri-apps/cli@2 build --debug --bundles app` 可打包本地 macOS 开发版，产物位于仓库根目录的 `target/debug/bundle/macos/Nexum.app`。打包后可在 `apps/desktop` 执行 `codesign --force --deep --sign - ../../target/debug/bundle/macos/Nexum.app` 为本机开发版添加临时签名。对外分发所需的签名与公证仍属于后续发布工作。更新依赖时应让 Tauri 的 JavaScript 与 Rust 包保持相同次版本。

macOS 优先的信息架构、任务状态、添加流程、设置、事件订阅、轮询回退和实现切片见 [Desktop UI 设计](DESKTOP_UI_DESIGN.zh-CN.md)。

## 浏览器扩展

在 `apps/extension` 执行：

```bash
cd apps/extension
pnpm install
pnpm lint
pnpm build
```

扩展的 `pnpm dev` 会监听文件变化并重新构建。以 `apps/extension` 为目录加载未打包扩展：根目录的 `manifest.json` 引用 `dist/` 下的构建产物和 `icons/` 下的图标。

Server 的 HTTP 桥接在 `/jsonrpc` 接受每个连接一个 JSON-RPC 请求，复用同一认证门，要求 `Content-Type: application/json`，并只向通过校验的浏览器扩展 Origin 返回 CORS 头；网页 Origin、分块请求和超过 1 MiB 的请求体会被拒绝。扩展发送流程会先调用 `task.create`，再调用 `task.queue`；它使用保存的 `server` 地址，处理 Content Script 消息，并在发送前解析相对链接。事件订阅仍只走 TCP，扩展没有凭据设置，因此使用扩展时请保持 `require_auth` 关闭。

英文版见 [DEVELOPMENT.md](DEVELOPMENT.md)。
