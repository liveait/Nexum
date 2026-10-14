# 开发指南

## 环境

Nexum 是使用 Rust 2024 edition 的工作区，桌面客户端采用 Tauri 2。桌面前端使用 TypeScript 和 React；浏览器扩展使用 TypeScript。

- Rust stable（1.85 或更新版本）、Cargo、`rustfmt` 和 Clippy
- Node.js（macOS Desktop CI job 使用 Node.js 22）和 pnpm；Desktop 固定 pnpm 11.25.0，并有独立的冻结依赖锁文件
- 运行或打包原生桌面应用时所需的 Tauri 2 系统依赖；Desktop 包会安装项目内固定版本的 Tauri CLI

Ubuntu CI 在检查 Rust 工作区前安装 `libgtk-3-dev` 和 `libwebkit2gtk-4.1-dev`。本地构建还需满足对应平台的 Tauri 依赖要求。

当前 Linux Tauri 依赖图通过 GTK 3（`gtk 0.18.2`）引入 `glib 0.18.5`。Dependabot 将其报告为 `GHSA-wrw7-89jp-8q8g` / RustSec `RUSTSEC-2024-0429`；不能把 `glib 0.20` 强制 patch 进依赖图，因为 GTK 3 的 `glib = ^0.18` 约束不兼容。Tauri 2.12.0 和 wry 0.57.0 仍声明 GTK 3 / `glib = 0.18`，常规小版本升级无法移除该告警。在上游 Tauri/wry 的 Linux runtime 迁移到已修复的 GTK binding 版本线，或项目明确迁移到兼容的 GTK 4 栈之前，应保留该告警；macOS 构建不包含这条仅 Linux 的依赖路径。

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

## 开发流程入口

任务定义、独立 worktree、多工具协作、审查、测试门槛、合并及清理规则统一见[贡献指南](../CONTRIBUTING.zh-CN.md#开发流程)。本指南维护环境和运行命令；开始任务时同时阅读根目录 `AGENTS.md`。

验收状态：本指南的操作说明和保护约定不能替代某个提交的验收证据。草案基线 `3768c3a` 的 CI 排除了 Server unit/persistence，审查已发现范围验证、文件保护、最终进度与完成恢复回归；这些行为应视为待修复/复验，而非已验收。修复后需用最终 SHA 的测试与 CI 记录更新此状态。

## Rust 检查

在仓库根目录运行完整 Rust 门槛：

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

仓库会提交 `Cargo.lock`，因为此工作区包含 Server、CLI 和 Desktop 可执行程序；修改 Rust 依赖时应同步更新它。CI 的 Rust job 应在 Ubuntu 上执行完整门槛，必须包含 Server 单元、`managed` 和 `persistence` 测试；是否实际执行以当前 workflow 与运行日志为准，不以绿色状态代替覆盖核对。另有 macOS job 使用 `--frozen-lockfile` 和 `apps/desktop/pnpm-lock.yaml` 安装 Desktop 依赖，执行 `pnpm build`、`pnpm test`，构建 `.app`，并检查 App 内的语言声明、签名和托管 Server 资源。它不构建 Browser 扩展，也不生成完成分发签名和公证的发布产物。

如需排查 Server 回归，可以先在仓库根目录运行：

```bash
cargo test --locked -p nexum-server
cargo test --locked -p nexum-server --test persistence -- --test-threads=1
```

后一命令用于隔离定位持久化/进程场景，不能替代完整门槛。环境或时序问题需保留完整测试集合与断言，并记录执行方式调整的原因。

Desktop 前端改动还需从 `apps/desktop` 运行 `pnpm install --frozen-lockfile`、`pnpm build` 和 `pnpm test`。涉及原生行为或打包时，还需本指南 Desktop 节中的 `.app` 构建、包验证与原生验收。纯文档改动执行 diff、内容和链接检查，不要求本地重跑无关构建。

## 合并后的 CI 核对

完整规则见[贡献指南](../CONTRIBUTING.zh-CN.md#合并前与合并后都要检查-ci)。使用 GitHub CLI 时，在仓库根目录查询 PR 的真实合并提交：

```bash
gh pr view PR_NUMBER --json state,headRefOid,baseRefName,mergeCommit,url
```

确认 `state` 为 `MERGED`、base 为 `main`，将 `mergeCommit.oid` 赋给任务变量；不要用源分支 head 代替 squash/rebase 后的 SHA：

```bash
task_merge_sha='ACTUAL_MERGED_SHA'
gh run list --workflow CI --branch main --event push --commit "$task_merge_sha" --json databaseId,headSha,status,conclusion,url
gh run view RUN_ID --json headSha,event,status,conclusion,jobs,url
```

核对 SHA、触发事件及必需 job 的实际结论。没有找到运行或仍在等待时，保持“已合并，等待主分支验收”；成功后在 PR/交接记录补充 run 链接。`main` 又更新时，还需验证最新 head。合并后失败的处理见贡献指南，不能用合并前或旧提交的成功替代。

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
要启用 Server TLS 监听器，请在 key-value 配置中同时设置 `tls_cert_path` 与 `tls_key_path`，或传入 `--tls-cert-path PATH --tls-key-path PATH`。Server 会在监听器接受连接前加载 PEM 证书和私钥。TLS 使用同一流承载 TCP JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc`；握手失败会关闭连接，绝不会降级到明文。CLI 和 Desktop 使用 `--server tls://host:port` 或保存的 `tls://host:port` 地址选择 TLS，加载平台根证书（也支持 `rustls-native-certs` 提供的 `SSL_CERT_FILE`/`SSL_CERT_DIR` 来源），校验 Server 名称，并且只在握手验证成功后发送凭据；Browser Extension 将 `tls://host:port` 映射到 `https://host:port/jsonrpc`，使用浏览器正常的信任库，TLS 请求失败时不会改用 HTTP 重试。

`task queue` 会持久化任务并触发 Server 派发器。派发器会持续选取符合条件的排队 HTTP/HTTPS 任务，直到达到 Scheduler 并发上限或目标路径规则不允许继续领取。`task start` 仍是手动 kick，会选取一个排队 HTTP/HTTPS 任务，启动 Worker 后返回任务 ID，然后调用同一派发器填充其他可用槽位。Worker 每写入一个响应块就报告进度；Server 在新增至少 1 MiB 或经过 250 ms 时持久化中间快照，并在任务标记为 `Completed` 前刷新最终快照。传输异步完成，可再次执行 `task list` 或 `task get ID` 查看字节数和完成状态。Server 会拒绝数据目录内的目标、符号链接目标，以及与活动传输重叠的目标。Worker 会将响应写入目标目录的隐藏稳定部分文件，并原子维护记录来源、目标、校验器和预期长度的 JSON sidecar，完整后再重命名到目标路径。传输失败会保留已有目标文件，并将错误文本保存到任务视图的 `error` 字段；在重试策略允许时重新排队，有可用槽位时派发器会自动启动重试。默认策略允许重试三次。领取新一轮传输时会清除旧错误；如果 sidecar 匹配，Server 会在领取后恢复部分字节进度。活跃 HTTP 传输可以在响应块边界暂停，并在同一 Server 进程内恢复；Worker 暂停时保留临时文件和 HTTP 响应。`task remove` 会取消活跃 HTTP Worker，最多等待其退出 30 秒后删除任务，并保留已有目标文件。超时会返回错误，Worker 可能继续阻塞到 30 分钟 HTTP 请求超时，退出后可以重试删除。阻塞中的响应读取可能让暂停等到 HTTP 请求超时；Server 重启后，只有来源、目标、ETag 或 Last-Modified 校验器、预期长度以及服务端确认的 `206 Partial Content` 范围都匹配时，才会复用稳定的部分文件和 sidecar。收到 `200`、校验器变化或缺失、范围格式错误或长度不一致时，会丢弃部分响应并从零开始；没有校验器的响应无法在重启后续传。Magnet 和本地文件来源仍无传输 Engine。


### 托管 Server 协议测试

`--managed` 是 App 子进程的私有启动入口，要求绝对数据目录和 stdin 启动帧，不能通过 `--auth-token` 或外部配置文件提供密钥。Desktop 已通过拥有句柄的子进程管理者调用此入口；令牌只由后端持有。控制帧、就绪响应、启动等待和退出边界见 [ADR 0006](decisions/0006-managed-macos-server.zh-CN.md#首个实现切片server-托管启动协议)。运行隔离的协议与进程测试：

```bash
cargo test --locked -p nexum-protocol managed::tests
cargo test --locked -p nexum-server --test managed
cargo test --locked -p nexum-desktop --lib managed_server::tests
```

进程测试自动创建并清理临时数据和下载目录，不需要停止现有的开发 Server。

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

`auth set` 只接受 `Bearer` 或 `ApiKey`，并拒绝空 token。`auth clear` 会将 CLI 配置中的两个凭据字段值清空。CLI 可在已验证的 `tls://` 连接上附加已保存凭据；对于不带 scheme 的明文地址，CLI 会先确认实际 peer 为回环地址，否则拒绝发送凭据。在 macOS 上，先到 Desktop 通用设置选择外部模式并保存回环或 `tls://` Server 地址，再在“通用”中选择 `Bearer` 或 `ApiKey`、输入匹配的 secret 并保存。Tauri 后端按包含传输身份的账户将 scheme 与 secret 保存到 Keychain，为普通 RPC 和 `events.subscribe` 附加凭据；查询 Keychain 配置状态时只把已配置的 scheme 返回给 React。保存或清除凭据会重启事件订阅并刷新 Task 快照。secret 输入框只写；清除操作会删除 Keychain 条目。明文凭据仍只允许回环地址，已校验的 TLS 凭据可以用于配置的 Server 地址。其他平台的 Desktop 可以无认证连接，但不支持保存和清除凭据。Browser Extension 仍没有凭据设置。

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
pnpm install --frozen-lockfile
pnpm exec tauri dev
```

Vite 使用 `1420` 端口；浏览器预览不能代替 Tauri 命令验收。从 `apps/desktop` 执行 `pnpm exec tauri dev`：Hook 会准备显式目标架构的 Server、启动 Vite，再打开原生窗口。只有在设置中选择外部模式时才需要单独启动 Server。`pnpm exec tauri build --bundles app` 会准备 Server 并构建前端，生成 `target/release/bundle/macos/Nexum.app`；添加 `--debug` 可生成 debug 包。Hook 在计算哈希前为暂存 macOS Server 签名，`build.rs` 嵌入哈希并检查架构／包版本。普通 debug Workspace 检查允许没有暂存 Server；release 构建必须有匹配资源。使用固定 CLI 和 frozen pnpm 锁文件。在仓库根目录执行 `node scripts/verify-desktop-bundle.mjs target/release/bundle/macos/Nexum.app` 检查最终包。计算哈希后不能再签嵌套 Server；分发签名须先准备最终已签名的可执行文件，再计算哈希和编译 Desktop。

本机构建 release App 后，如需验证完整包，可在 `apps/desktop` 显式添加并检查临时签名：

```bash
codesign --force --sign - ../../target/release/bundle/macos/Nexum.app
codesign --verify --deep --strict ../../target/release/bundle/macos/Nexum.app
```

这个本地签名不能代替分发签名、公证、Gatekeeper 放行或原生界面验收。

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

Server 的 HTTP 桥接在 `/jsonrpc` 接受每个连接一个 JSON-RPC 请求，复用同一认证门，要求 `Content-Type: application/json`，并只向通过校验的浏览器扩展 Origin 返回 CORS 头；网页 Origin、分块请求和超过 1 MiB 的请求体会被拒绝。扩展发送流程会先调用 `task.create`，再调用 `task.queue`；它使用保存的 `server` 地址，将不带 scheme 的地址映射到 HTTP、将 `tls://` 地址映射到 HTTPS，处理 Content Script 消息，并在发送前解析相对链接。事件订阅仍只走 TCP，扩展没有凭据设置，因此使用扩展时请保持 `require_auth` 关闭。

英文版见 [DEVELOPMENT.md](DEVELOPMENT.md)。
