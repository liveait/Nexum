<a id="development-guide"></a>
<a id="开发指南"></a>

# 开发指南 / Development Guide

导航 / Contents: [环境 / Environment](#environment) · [工作区 / Workspace](#workspace) · [开发流程入口 / Development Workflow Entry](#development-workflow-entry) · [Rust 检查 / Rust Checks](#rust-checks) · [合并后的 CI 核对 / Checking CI After Merge](#checking-ci-after-merge) · [Server 与 CLI / Server and CLI](#server-and-cli) · [认证 / Authentication](#authentication) · [RPC 限流 / RPC Rate Limiting](#rpc-rate-limiting) · [Desktop](#desktop) · [浏览器扩展 / Browser Extension](#browser-extension)

<a id="environment"></a>
<a id="环境"></a>

## 环境 / Environment

Nexum 是使用 Rust 2024 edition 的工作区，桌面客户端采用 Tauri 2。桌面前端使用 TypeScript 和 React；浏览器扩展使用 TypeScript。

Nexum is a Rust 2024-edition workspace with a Tauri 2 desktop client. The desktop frontend uses TypeScript and React; the browser extension is written in TypeScript.

- Rust stable（1.85 或更新版本）、Cargo、`rustfmt` 和 Clippy

  Rust stable (1.85 or newer), Cargo, `rustfmt`, and Clippy
- Node.js（macOS Desktop CI job 使用 Node.js 22）和 pnpm；Desktop 固定 pnpm 11.25.0，并有独立的冻结依赖锁文件

  Node.js (the macOS Desktop CI job uses Node.js 22) and pnpm; Desktop pins pnpm 11.25.0 and has its own frozen dependency lockfile
- 运行或打包原生桌面应用时所需的 Tauri 2 系统依赖；Desktop 包会安装项目内固定版本的 Tauri CLI

  Tauri 2 system dependencies when running or bundling the native desktop app; the Desktop package installs its pinned Tauri CLI locally

Ubuntu CI 在检查 Rust 工作区前安装 `libgtk-3-dev` 和 `libwebkit2gtk-4.1-dev`。本地构建还需满足对应平台的 Tauri 依赖要求。

The Ubuntu CI job installs `libgtk-3-dev` and `libwebkit2gtk-4.1-dev` before checking the Rust workspace. Platform-specific Tauri prerequisites also apply to local builds.

当前 Linux Tauri 依赖图通过 GTK 3（`gtk 0.18.2`）引入 `glib 0.18.5`。Dependabot 将其报告为 `GHSA-wrw7-89jp-8q8g` / RustSec `RUSTSEC-2024-0429`；不能把 `glib 0.20` 强制 patch 进依赖图，因为 GTK 3 的 `glib = ^0.18` 约束不兼容。Tauri 2.12.0 和 wry 0.57.0 仍声明 GTK 3 / `glib = 0.18`，常规小版本升级无法移除该告警。在上游 Tauri/wry 的 Linux runtime 迁移到已修复的 GTK binding 版本线，或项目明确迁移到兼容的 GTK 4 栈之前，应保留该告警；macOS 构建不包含这条仅 Linux 的依赖路径。

The Linux Tauri dependency graph currently includes `glib 0.18.5` through GTK 3 (`gtk 0.18.2`). This is reported by Dependabot as `GHSA-wrw7-89jp-8q8g` / RustSec `RUSTSEC-2024-0429`; do not force `glib 0.20` into the graph because GTK 3's `glib = ^0.18` requirement is incompatible. Tauri 2.12.0 and wry 0.57.0 still declare GTK 3 / `glib = 0.18`, so a routine minor upgrade does not remove the advisory. Keep the alert open until the upstream Tauri/wry Linux runtime moves to a fixed GTK binding line or the Linux desktop build is intentionally migrated to a compatible GTK 4 stack; macOS builds do not include this Linux-only dependency path.

<a id="workspace"></a>
<a id="工作区"></a>

## 工作区 / Workspace

根目录 `Cargo.toml` 包含下列全部 crate 和 `apps/desktop/src-tauri`。浏览器扩展是独立的前端包。

The root `Cargo.toml` includes all crates below and `apps/desktop/src-tauri`. The browser extension is a separate frontend package.

```text
crates/
  core/ domain/ task/ scheduler/ storage/ resolver/
  protocol/ plugin/ security/ media/ engine/
apps/
  server/                 # nexum-server 可执行文件 / nexum-server binary
  cli/                    # nexum-cli 可执行文件 / nexum-cli binary
  desktop/                # React/Vite 和 src-tauri/ / React/Vite and src-tauri/
  extension/              # Manifest V3 扩展 / Manifest V3 extension
```

保持 UI 逻辑与 Core 分离，避免将具体 Engine 的实现细节放入 Domain Model。行为变化应补充针对性测试，并同步更新受影响单篇文档中的中英文内容。需要 RFC 的变更类型见[贡献指南](../CONTRIBUTING.md)。

Keep UI logic out of Core and engine-specific details out of the Domain Model. Add focused tests and update the Chinese and English content in affected shared documents when behavior changes. Check [Contributing](../CONTRIBUTING.md) for changes that require an RFC.

<a id="development-workflow-entry"></a>
<a id="开发流程入口"></a>

## 开发流程入口 / Development Workflow Entry

任务定义、独立 worktree、多工具协作、审查、测试门槛、合并及清理规则统一见[贡献指南](../CONTRIBUTING.md#开发流程)。本指南维护环境和运行命令；开始任务时同时阅读根目录 `AGENTS.md`。

See the [Contribution Workflow](../CONTRIBUTING.md#development-workflow) for task definition, separate worktrees, collaboration, review, test gates, merging, and cleanup. This guide maintains environment and execution commands; also read root `AGENTS.md` when starting a task.

验收状态（2026-10-15）：PR #58 已合并为 `f1817f4f4a73db4c51a3e60f085491ddf6cb936b`，[该提交的 main push CI](https://github.com/liveait/Nexum/actions/runs/38600813846) 中 Rust 与 Desktop macOS 均通过。本轮在此基线复验 Server，29 项 unit、10 项 managed 和 51 项 persistence 全部通过。修复覆盖单连接 HTTP session 的续传进度、响应长度、具体错误、暂停／取消及启动完成对账；它不代表 M4 分段下载已验收。本轮将 Server 收敛到单连接 session，移除自动分段／探测及按 fixture URL 绕过执行路径的逻辑；本次改动同样须满足独立审查、完整门槛及合并前后 CI。M0 VoiceOver、M1 全新用户／中断／升级及 M3 分发验收仍待完成，M4/M5 的剩余范围见[开发计划](DEVELOPMENT_PLAN.md)。

Acceptance status (2026-10-15): PR #58 merged as `f1817f4f4a73db4c51a3e60f085491ddf6cb936b`; Rust and Desktop macOS both passed in [that commit's main push CI](https://github.com/liveait/Nexum/actions/runs/38600813846). Baseline revalidation for this task passed all 29 Server unit, 10 managed, and 51 persistence tests. The fixes cover single-connection HTTP session resume progress, response lengths, actionable errors, pause/cancel, and startup completion reconciliation; they do not accept M4 segment downloads. This change confines the Server to the single-connection session, removing automatic segmentation/probing and fixture-URL execution bypasses; it must also satisfy independent review, complete gates, and CI before and after merge. M0 VoiceOver, M1 clean-user/interruption/upgrade, and M3 distribution acceptance remain open; see the [Development Plan](DEVELOPMENT_PLAN.md) for the remaining M4/M5 scope.

启动局限：`f1817f4` 的 `open_core` 在监听前同步对账，待恢复大文件的完整摘要也会在此阶段读取，偏离 [ADR 0007](decisions/0007-http-completion-reconciliation.md#implementation-status) 规定的“就绪后 Worker 校验”。这可能超出托管 Server 六秒就绪预算；大文件恢复的启动延迟仍需修复并验收，当前小型 fixture 和 CI 不能证明该预算已满足。

Startup limitation: `open_core` at `f1817f4` reconciles synchronously before listening, including full-file hashing for pending large-file recovery. This deviates from [ADR 0007](decisions/0007-http-completion-reconciliation.md#implementation-status), which requires hashing in workers after readiness. It can exceed the managed Server's six-second readiness budget. Large-file recovery startup latency still needs a fix and acceptance; current small fixtures and CI do not establish that the budget is met.

<a id="rust-checks"></a>
<a id="rust-检查"></a>

## Rust 检查 / Rust Checks

在仓库根目录运行完整 Rust 门槛：

From the repository root, run the complete Rust gates:

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

仓库会提交 `Cargo.lock`，因为此工作区包含 Server、CLI 和 Desktop 可执行程序；修改 Rust 依赖时应同步更新它。CI 的 Rust job 应在 Ubuntu 上执行完整门槛，必须包含 Server 单元、`managed` 和 `persistence` 测试；是否实际执行以当前 workflow 与运行日志为准，不以绿色状态代替覆盖核对。另有 macOS job 使用 `--frozen-lockfile` 和 `apps/desktop/pnpm-lock.yaml` 安装 Desktop 依赖，执行 `pnpm build`、`pnpm test`，构建 `.app`，并检查 App 内的语言声明、签名和托管 Server 资源。它不构建 Browser 扩展，也不生成完成分发签名和公证的发布产物。

`Cargo.lock` is committed because this workspace ships Server, CLI, and Desktop binaries; keep it updated when changing Rust dependencies. The Rust CI job should run the complete gates on Ubuntu, including Server unit, `managed`, and `persistence` tests. Verify the actual workflow and logs; green status alone does not establish coverage. A separate macOS job installs the Desktop package from `apps/desktop/pnpm-lock.yaml` with `--frozen-lockfile`, runs `pnpm build` and `pnpm test`, builds the `.app` bundle, and verifies localization declarations, signatures, and managed Server resources. It does not build the Browser extension or produce signed and notarized release artifacts.

如需排查 Server 回归，可以先在仓库根目录运行：

To investigate Server regressions, run from the repository root:

```bash
cargo test --locked -p nexum-server
cargo test --locked -p nexum-server --test persistence -- --test-threads=1
```

后一命令用于隔离定位持久化/进程场景，不能替代完整门槛。环境或时序问题需保留完整测试集合与断言，并记录执行方式调整的原因。

The latter isolates persistence/process scenarios; it does not replace the full gate. Preserve the complete test set and assertions when resolving environment/timing issues and record the reason for execution-mode changes.

Desktop 前端改动还需从 `apps/desktop` 运行 `pnpm install --frozen-lockfile`、`pnpm build` 和 `pnpm test`。涉及原生行为或打包时，还需本指南 Desktop 节中的 `.app` 构建、包验证与原生验收。纯文档改动执行 diff、内容和链接检查，不要求本地重跑无关构建。

For Desktop frontend changes, also run `pnpm install --frozen-lockfile`, `pnpm build`, and `pnpm test` from `apps/desktop`. Native behavior or packaging changes additionally require the Desktop section's app build, bundle verification, and native acceptance. Documentation-only changes need diff, content, and link inspection without unrelated local builds.

<a id="checking-ci-after-merge"></a>
<a id="合并后的-ci-核对"></a>

## 合并后的 CI 核对 / Checking CI After Merge

完整规则见[贡献指南](../CONTRIBUTING.md#合并前与合并后都要检查-ci)。使用 GitHub CLI 时，在仓库根目录查询 PR 的真实合并提交：

See the [Contribution Workflow](../CONTRIBUTING.md#verify-ci-before-and-after-merging) for the full rules. With GitHub CLI, query the actual merged commit from the repository root:

```bash
gh pr view PR_NUMBER --json state,headRefOid,baseRefName,mergeCommit,url
```

确认 `state` 为 `MERGED`、base 为 `main`，将 `mergeCommit.oid` 赋给任务变量；不要用源分支 head 代替 squash/rebase 后的 SHA：

Confirm `state` is `MERGED` and base is `main`. Put `mergeCommit.oid` in a task-specific variable; do not substitute the source head for a squash/rebase result:

```bash
task_merge_sha='ACTUAL_MERGED_SHA'
gh run list --workflow CI --branch main --event push --commit "$task_merge_sha" --json databaseId,headSha,status,conclusion,url
gh run view RUN_ID --json headSha,event,status,conclusion,jobs,url
```

核对 SHA、触发事件及必需 job 的实际结论。没有找到运行或仍在等待时，保持“已合并，等待主分支验收”；成功后在 PR/交接记录补充 run 链接。`main` 又更新时，还需验证最新 head。合并后失败的处理见贡献指南，不能用合并前或旧提交的成功替代。

Verify the SHA, event, and actual required-job conclusions. If a run is missing or pending, keep the status “merged, awaiting main acceptance.” Record the successful run link in the PR/handoff. Also verify the latest head if `main` advances. Follow the contribution guide for failures; pre-merge or older successes cannot replace this check.

<a id="server-and-cli"></a>
<a id="server-与-cli"></a>

## Server 与 CLI / Server and CLI

在一个终端启动本地 Server：

Start the local server in one terminal:

```bash
cargo run -p nexum-server -- --port 39100
```

默认绑定 `127.0.0.1:39100`，通过 TCP 提供以换行符分隔的 JSON-RPC 2.0 服务，并提供每个连接一个请求的 HTTP `POST /jsonrpc`；通过校验的浏览器扩展 Origin 会收到 CORS 头。在另一个终端运行 CLI：

It binds to `127.0.0.1:39100` by default and serves newline-delimited JSON-RPC 2.0 over TCP plus one-request HTTP `POST /jsonrpc` calls. Validated browser-extension origins receive CORS headers. Run the CLI in another terminal:

```bash
cargo run -p nexum-cli -- task list
cargo run -p nexum-cli -- task create task-1 https://example.com/ ./example.html
cargo run -p nexum-cli -- task queue task-1
```

编译出的可执行文件分别名为 `nexum-server` 和 `nexum-cli`。使用 `cargo run -p nexum-server -- --help` 和 `cargo run -p nexum-cli -- --help` 查看当前参数与命令。CLI 默认连接 `127.0.0.1:39100`；如需覆盖地址，将 `--server ADDR` 放在 `task` 或 `server` 前面。

The built binaries are named `nexum-server` and `nexum-cli`. Use `cargo run -p nexum-server -- --help` and `cargo run -p nexum-cli -- --help` for current flags and commands. The CLI defaults to `127.0.0.1:39100`; put `--server ADDR` before `task` or `server` to override it.

Server 将任务元数据和进度保存到 `--data-dir` 下的 `nexum.sqlite`（默认 `./data`，相对于 Server 工作目录）。启动时会按需创建目录，并在退出前一直持有同目录下 `nexum.lock` 的锁，因此同一数据目录只能由一个 Server 使用。Server 在监听前打开数据库并恢复任务；目录、锁、数据库或恢复失败会阻止启动。显式传入的 `--config` 文件无法读取时也会阻止启动。重启后，原先下载中、暂停或重试中的任务会在内存和 SQLite 中变为排队状态。恢复完成并开始监听后，符合条件的排队 HTTP/HTTPS 任务会自动派发；不支持的来源或受阻的目标会继续排队。`--max-connections` 限制活动 TCP 连接处理器；超过上限的连接会在请求处理前立即关闭。认证默认关闭。要强制认证，需同时设置 `require_auth=true`、`auth_scheme=Bearer|ApiKey` 和非空 `auth_token`；`server.auth` 只返回当前 scheme，不返回 secret。认证配置不完整或不受支持时会阻止启动。
要启用 Server TLS 监听器，请在 key-value 配置中同时设置 `tls_cert_path` 与 `tls_key_path`，或传入 `--tls-cert-path PATH --tls-key-path PATH`。Server 会在监听器接受连接前加载 PEM 证书和私钥。TLS 使用同一流承载 TCP JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc`；握手失败会关闭连接，绝不会降级到明文。CLI 和 Desktop 使用 `--server tls://host:port` 或保存的 `tls://host:port` 地址选择 TLS，加载平台根证书（也支持 `rustls-native-certs` 提供的 `SSL_CERT_FILE`/`SSL_CERT_DIR` 来源），校验 Server 名称，并且只在握手验证成功后发送凭据；Browser Extension 将 `tls://host:port` 映射到 `https://host:port/jsonrpc`，使用浏览器正常的信任库，TLS 请求失败时不会改用 HTTP 重试。

The server stores task metadata and progress in `nexum.sqlite` under `--data-dir` (default `./data`, relative to the server's working directory). It creates the directory if needed and holds a lock on `nexum.lock` there until exit, so only one server can use that directory. It opens the database and recovers tasks before listening; directory, lock, database, or recovery errors stop startup. An explicitly supplied `--config` file that cannot be read also stops startup. On restart, previously downloading, paused, or retrying tasks become queued in memory and in SQLite. Once recovery and listening complete, eligible queued HTTP/HTTPS tasks are dispatched automatically; unsupported sources and blocked destinations remain queued. `--max-connections` limits active TCP connection handlers; a connection over the limit is accepted by the OS and immediately closed before request processing. Authentication is disabled by default. Set `require_auth=true` together with `auth_scheme=Bearer|ApiKey` and a non-empty `auth_token` to require credentials; `server.auth` reports the active scheme without the secret. An incomplete or unsupported credential configuration stops startup.
To enable the Server TLS listener, set both `tls_cert_path` and `tls_key_path` in the key-value config or pass `--tls-cert-path PATH --tls-key-path PATH`. The PEM certificate and private key are loaded before the listener accepts connections. TLS carries TCP JSON-RPC, `events.subscribe`, and HTTP `/jsonrpc` over the same stream; a TLS handshake failure closes the connection and never downgrades to plaintext. The CLI and Desktop select TLS with `--server tls://host:port` or the saved `tls://host:port` address, load the platform root store (or the `SSL_CERT_FILE`/`SSL_CERT_DIR` sources supported by `rustls-native-certs`), validate the server name, and send credentials only after a verified handshake. The Browser extension maps `tls://host:port` to `https://host:port/jsonrpc` and uses the browser's normal trust store; it does not retry failed TLS requests over HTTP.

`task queue` 会持久化任务并触发 Server 派发器，持续领取符合条件的排队 HTTP/HTTPS 任务，直到达到 Scheduler 并发上限或目标路径规则不允许继续领取。`task start` 仍是手动 kick，领取一个任务并启动 Worker 后返回任务 ID，再调用同一派发器填充其他槽位。所有 HTTP/HTTPS Worker 都使用单连接 `HttpTransferSession`，不预先探测 Range 或自动分段，也不根据 URL 地址选择测试专用路径。session 从实际传输字节计算进度、速率和可信长度下的 ETA；Server 在新增至少 1 MiB 或经过 250 ms 时持久化中间快照，并通过一次 repository update 同时保存最终进度和 `Completed`。Scheduler 带宽上限下发到 Worker，活跃任务数变化时重新计算。传输异步完成，可用 `task list` 或 `task get ID` 查询。

`task queue` persists a task and triggers the Server dispatcher, which claims eligible queued HTTP/HTTPS tasks until scheduler concurrency or destination rules prevent another claim. `task start` remains a manual kick: it returns the task ID after launching one worker, then invokes the same dispatcher to fill other slots. Every HTTP/HTTPS worker uses a single-connection `HttpTransferSession`, without a preliminary Range probe, automatic segments, or a URL-based test-only path. The session calculates progress, speed, and ETA for trustworthy lengths from actual transfer bytes. The Server persists intermediate snapshots after 1 MiB or 250 ms, then saves final progress and `Completed` together in one repository update. Scheduler bandwidth caps are passed to workers and recalculated as the active-task count changes. Completion is asynchronous; query it with `task list` or `task get ID`.

Server 拒绝数据目录内的目标、符号链接目标及与活动传输重叠的目标。单连接 session 写入隐藏稳定部分文件，并原子维护包含来源、目标、校验器和预期长度的 JSON sidecar；完整响应先生成私有完成凭证，再不覆盖地发布目标文件。启动时在网络派发前按凭证校验已发布文件或重新发布完整暂存文件，并原子保存最终状态；无法安全对账的凭证保留可见失败，不进入普通网络重试。普通传输失败保留已有目标和可恢复的部分数据，将具体错误写入任务视图的 `error`；默认最多重试三次，新一轮领取清除旧错误。

The Server rejects destinations inside its data directory, symbolic-link destinations, and overlapping active destinations. The single-connection session writes a stable hidden partial and atomically maintains a JSON sidecar containing the source, destination, validator, and expected length. A complete response receives a private completion receipt before no-clobber publication. Before network dispatch at startup, receipts verify published files or republish complete staging bytes, then persist the final state atomically. A receipt that cannot reconcile safely produces a visible failure without ordinary network retries. Normal transfer failures preserve existing destinations and recoverable partial bytes, and retain the specific cause in the task view's `error` field. The default retry budget is three; a new claim clears the prior error.

活跃 HTTP 传输支持协作式暂停、同进程恢复及删除取消。响应体读取以 1 秒无数据窗口检查控制请求，连续 8 次无数据才判为停滞；暂停时保留部分文件和响应。`task remove` 最多等待 Worker 退出 30 秒，超时返回错误；已有目标文件保持不变。重启续传只复用来源、目标、校验器和预期长度匹配的部分文件，并校验服务端 `206 Partial Content` 的范围和响应长度；进度包含已保存偏移，表示整个文件。`200`、校验器缺失／变化或不合法的范围使单连接路径丢弃旧部分响应并从零开始。没有校验器的未完成响应不能跨重启续传；已证明完整的字节仍可通过凭证本地对账。Magnet 和本地文件来源仍无传输 Engine。

Active HTTP transfers support cooperative pause, same-process resume, and cancellation through removal. Response-body reads check controls at one-second no-data windows and fail as stalled after eight consecutive empty windows; pause retains the partial file and response. `task remove` waits up to 30 seconds for the worker and returns an error on timeout, preserving an existing destination. Restart resume reuses partials only when the source, destination, validator, and expected length match, and validates the Server's `206 Partial Content` range and response length. Progress includes the saved offset and represents the whole file. A `200`, missing/changed validator, or invalid range discards stale partial bytes and restarts the single-connection path from zero. Incomplete responses without validators cannot resume across restart; proven-complete bytes can still reconcile locally through receipts. Magnet and local-file sources have no transfer engine yet.

<a id="managed-server-protocol-tests"></a>
<a id="托管-server-协议测试"></a>

### 托管 Server 协议测试 / Managed Server protocol tests

`--managed` 是 App 子进程的私有启动入口，要求绝对数据目录和 stdin 启动帧，不能通过 `--auth-token` 或外部配置文件提供密钥。Desktop 已通过拥有句柄的子进程管理者调用此入口；令牌只由后端持有。控制帧、就绪响应、启动等待和退出边界见 [ADR 0006](decisions/0006-managed-macos-server.md#首个实现切片server-托管启动协议)。运行隔离的协议与进程测试：

`--managed` is a private App-child entry point requiring an absolute data directory and a startup frame on stdin. Neither `--auth-token` nor an external configuration file can supply its secret. The Desktop now calls it through the owned-child supervisor; its token remains backend-only. See [ADR 0006](decisions/0006-managed-macos-server.md#first-implementation-slice-managed-server-bootstrap) for control frames, readiness, startup waiting, and exit boundaries. Run the isolated protocol and process tests:

```bash
cargo test --locked -p nexum-protocol managed::tests
cargo test --locked -p nexum-server --test managed
cargo test --locked -p nexum-desktop --lib managed_server::tests
```

进程测试自动创建并清理临时数据和下载目录，不需要停止现有的开发 Server。

The process tests create and clean their own temporary data/download directories without stopping an existing development Server.

<a id="authentication"></a>
<a id="认证"></a>

## 认证 / Authentication

Server 接受 `Bearer` 和 `ApiKey` 凭据。通过 `--config` 传入的 key-value 文件配置认证：

The server accepts `Bearer` and `ApiKey` credentials. Configure authentication in a key-value file passed with `--config`:

```ini
require_auth=true
auth_scheme=ApiKey
auth_token=replace-with-a-secret
```

也可以用命令行参数覆盖：

The same values can be supplied as command-line overrides:

```bash
cargo run -p nexum-server -- \
  --require-auth true \
  --auth-scheme ApiKey \
  --auth-token replace-with-a-secret
```

`require_auth=true` 要求受支持的 scheme 和非空 token；缺少任一字段、token 为空或 scheme 不受支持都会导致启动失败。启用后，Server 会在分发每个 RPC 前以及接受 `events.subscribe` 前校验请求凭据。scheme 和 secret 必须同时匹配。缺少或无效凭据会返回 JSON-RPC 错误 `-32001`，消息为 `authentication required`。`server.auth` 只返回已配置的 scheme，Server 日志和 Debug 输出会隐藏 token。

`require_auth=true` requires a supported scheme and non-empty token; a missing pair, empty token, or unsupported scheme makes startup fail. When enabled, the server checks the request credential before dispatching every RPC and before accepting `events.subscribe`. Both the scheme and secret must match. Missing or invalid credentials return JSON-RPC error `-32001` with message `authentication required`. `server.auth` returns only the configured scheme, and server logs and debug output redact the token.

CLI 会在保存默认 Server 地址的同一配置目录中以明文保存凭据。使用以下命令设置或清除：

The CLI stores its credential as plain text in the same config directory as the default server address. Set or clear it with:

```bash
cargo run -p nexum-cli -- auth set ApiKey replace-with-a-secret
cargo run -p nexum-cli -- auth clear
```

`auth set` 只接受 `Bearer` 或 `ApiKey`，并拒绝空 token。`auth clear` 会将 CLI 配置中的两个凭据字段值清空。CLI 可在已验证的 `tls://` 连接上附加已保存凭据；对于不带 scheme 的明文地址，CLI 会先确认实际 peer 为回环地址，否则拒绝发送凭据。在 macOS 上，先到 Desktop 通用设置选择外部模式并保存回环或 `tls://` Server 地址，再在“通用”中选择 `Bearer` 或 `ApiKey`、输入匹配的 secret 并保存。Tauri 后端按包含传输身份的账户将 scheme 与 secret 保存到 Keychain，为普通 RPC 和 `events.subscribe` 附加凭据；查询 Keychain 配置状态时只把已配置的 scheme 返回给 React。保存或清除凭据会重启事件订阅并刷新 Task 快照。secret 输入框只写；清除操作会删除 Keychain 条目。明文凭据仍只允许回环地址，已校验的 TLS 凭据可以用于配置的 Server 地址。其他平台的 Desktop 可以无认证连接，但不支持保存和清除凭据。Browser Extension 仍没有凭据设置。

`auth set` accepts only `Bearer` or `ApiKey` and rejects an empty token. `auth clear` clears both credential field values in the CLI config. The CLI may attach the saved credential to a verified `tls://` connection; for a bare plaintext address it first checks that the connected peer is loopback and refuses to send credentials otherwise. On macOS, first select External under Desktop General settings and save a loopback or `tls://` Server address. Under General, select `Bearer` or `ApiKey`, enter the matching secret, and save it. The Tauri backend keeps both values in Keychain under a transport-scoped account, sends them with ordinary RPC and `events.subscribe`, and returns only the configured scheme to React when reading Keychain state. Saving or clearing the credential restarts the event subscription and refreshes the task snapshot. The secret field is write-only; clearing removes the Keychain item. Plaintext credentials remain loopback-only; verified TLS credentials may use any configured Server address. Desktop on other platforms can connect without authentication, but saving and clearing credentials are unsupported. The Browser extension still has no credential setting.

<a id="rpc-rate-limiting"></a>
<a id="rpc-限流"></a>

## RPC 限流 / RPC Rate Limiting

限流默认关闭。要启用它，在 Server 的 key-value 配置文件中同时设置持续每秒请求数与令牌桶突发容量：

Rate limiting is disabled by default. To enable it, set both the sustained requests per second and the token-bucket burst capacity in the server's key-value configuration file:

```ini
rate_limit_rps=10
rate_limit_burst=20
```

也可以使用对应的命令行参数：

The equivalent command-line flags are:

```bash
cargo run -p nexum-server -- --rate-limit-rps 10 --rate-limit-burst 20
```

两个参数必须成对提供且取有效正值；配置无效或不完整时，Server 会拒绝启动。显式传入的 `--config` 文件无法读取时也会拒绝启动。同一进程的令牌桶由逐行 TCP 和 HTTP `POST /jsonrpc` 共用，计入已解析且通过认证的请求，包括 `events.subscribe`。认证失败不消耗令牌；Server 生成的事件 heartbeat 和 HTTP `OPTIONS` 预检请求也不消耗令牌。带 `id` 的请求超额时返回 JSON-RPC 错误 `-32002`；HTTP `/jsonrpc` 仍返回状态码 `200`，错误位于 JSON 响应体中。没有 `id` 的通知会被丢弃，不返回 JSON-RPC 响应；HTTP 使用状态码 `204`。CLI 在执行命令前会先查询版本，Desktop 启动时也会连续调用多次，因此应设置足以覆盖这些调用的突发容量（示例使用 20）。

The two settings must be supplied together and have valid positive values; invalid or incomplete settings stop startup. An explicitly supplied `--config` file that cannot be read also stops startup. One process-wide token bucket is shared by parsed, authenticated requests on both line-delimited TCP and HTTP `POST /jsonrpc`, including `events.subscribe`. Authentication failures do not consume tokens. Server-generated event heartbeats and HTTP `OPTIONS` preflight requests do not consume tokens. An over-limit call with an `id` receives JSON-RPC error `-32002`; HTTP `/jsonrpc` still responds with status `200` and the error in its JSON body. A notification without an `id` is dropped without a JSON-RPC response; HTTP uses status `204`. The CLI performs a version request before its command, and the Desktop makes multiple calls at startup, so use a burst size comfortably above these client call sequences (the example uses 20).

<a id="desktop"></a>

## Desktop

在 `apps/desktop` 启动 Vite 前端：

Start the Vite frontend from `apps/desktop`:

```bash
cd apps/desktop
pnpm install --frozen-lockfile
pnpm exec tauri dev
```

Vite 使用 `1420` 端口；浏览器预览不能代替 Tauri 命令验收。从 `apps/desktop` 执行 `pnpm exec tauri dev`：Hook 会准备显式目标架构的 Server、启动 Vite，再打开原生窗口。只有在设置中选择外部模式时才需要单独启动 Server。`pnpm exec tauri build --bundles app` 会准备 Server 并构建前端，生成 `target/release/bundle/macos/Nexum.app`；添加 `--debug` 可生成 debug 包。Hook 在计算哈希前为暂存 macOS Server 签名，`build.rs` 嵌入哈希并检查架构／包版本。普通 debug Workspace 检查允许没有暂存 Server；release 构建必须有匹配资源。使用固定 CLI 和 frozen pnpm 锁文件。在仓库根目录执行 `node scripts/verify-desktop-bundle.mjs target/release/bundle/macos/Nexum.app` 检查最终包。计算哈希后不能再签嵌套 Server；分发签名须先准备最终已签名的可执行文件，再计算哈希和编译 Desktop。

Vite uses port `1420`; browser preview alone cannot exercise Tauri commands. From `apps/desktop`, use `pnpm exec tauri dev`: its hook prepares an explicit target-architecture Server and starts Vite, then opens the native window. A separate Server is needed only when Settings selects External. `pnpm exec tauri build --bundles app` runs the Server preparation hook and frontend build, then creates `target/release/bundle/macos/Nexum.app`. Add `--debug` for a debug bundle. The hook signs the staged macOS Server before hashing, and `build.rs` embeds its hash and checks its target/package version. A plain debug workspace check may have no staged Server; a release build requires it. Use the pinned CLI and frozen pnpm lockfile. Verify the finished bundle with `node scripts/verify-desktop-bundle.mjs target/release/bundle/macos/Nexum.app` from the repository root. Do not re-sign the nested Server after hashing; a distribution signature requires preparing and hashing that final signed executable before compiling Desktop.

本机构建 release App 后，如需验证完整包，可在 `apps/desktop` 显式添加并检查临时签名：

For local bundle validation after a release build, explicitly apply and verify an ad-hoc signature from `apps/desktop`:

```bash
codesign --force --sign - ../../target/release/bundle/macos/Nexum.app
codesign --verify --deep --strict ../../target/release/bundle/macos/Nexum.app
```

这个本地签名不能代替分发签名、公证、Gatekeeper 放行或原生界面验收。

This local signature does not replace distribution signing, notarization, Gatekeeper acceptance, or native UI validation.

macOS 优先的信息架构、任务状态、添加流程、设置、事件订阅、轮询回退和实现切片见 [Desktop UI 设计](DESKTOP_UI_DESIGN.md)。

The macOS-first information architecture, task states, add flow, settings, event subscription, polling fallback, and implementation slices are documented in [Desktop UI Design](DESKTOP_UI_DESIGN.md).

<a id="browser-extension"></a>
<a id="浏览器扩展"></a>

## 浏览器扩展 / Browser Extension

在 `apps/extension` 执行：

From `apps/extension`:

```bash
cd apps/extension
pnpm install
pnpm lint
pnpm build
```

扩展的 `pnpm dev` 会监听文件变化并重新构建。以 `apps/extension` 为目录加载未打包扩展：根目录的 `manifest.json` 引用 `dist/` 下的构建产物和 `icons/` 下的图标。

The extension's `pnpm dev` watches and rebuilds files. Load `apps/extension` as the unpacked extension: its root `manifest.json` references assets under `dist/` and icons under `icons/`.

Server 的 HTTP 桥接在 `/jsonrpc` 接受每个连接一个 JSON-RPC 请求，复用同一认证门，要求 `Content-Type: application/json`，并只向通过校验的浏览器扩展 Origin 返回 CORS 头；网页 Origin、分块请求和超过 1 MiB 的请求体会被拒绝。扩展发送流程会先调用 `task.create`，再调用 `task.queue`；它使用保存的 `server` 地址，将不带 scheme 的地址映射到 HTTP、将 `tls://` 地址映射到 HTTPS，处理 Content Script 消息，并在发送前解析相对链接。事件订阅仍只走 TCP，扩展没有凭据设置，因此使用扩展时请保持 `require_auth` 关闭。

The server's HTTP bridge accepts one JSON-RPC request per connection at `/jsonrpc`, enforces the same authentication gate, requires `Content-Type: application/json`, returns CORS headers to validated browser-extension origins, and rejects web-page origins, chunked bodies, or bodies larger than 1 MiB. The extension's send flow calls `task.create` and then `task.queue`; it uses the saved `server` address, maps bare addresses to HTTP and `tls://` addresses to HTTPS, handles content-script messages, and resolves relative links before sending. Event subscriptions remain TCP-only, and the extension has no credential settings, so leave `require_auth` disabled for extension use.
