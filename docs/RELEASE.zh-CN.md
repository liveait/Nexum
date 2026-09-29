# 发布流程

## 当前状态

仓库目前没有自动发布工作流，当前检出也没有版本标签。`.github/workflows/ci.yml` 在推送和 Pull Request 上执行 Rust 格式、检查、测试和 Clippy；不会构建前端包、打包桌面安装程序或浏览器扩展，也不会发布 GitHub Release。各包与应用清单目前声明 `0.1.0`，而变更日志仍将该版本标为 TBD。

`Cargo.lock` 被 `.gitignore` 忽略，未纳入 Git；两个前端包也没有已跟踪的锁文件。因此，全新检出可能解析到不同的依赖版本。更新本地 `Cargo.lock` 不会改变发布标签中的内容。在宣称发布构建的依赖解析可复现之前，需要先确定并落实纳入版本控制的锁文件策略。

## 手动发布检查表

以下步骤说明当前源码树生成候选版本所需的工作。完成验证后，再人工决定打标签和发布。

1. 在 Rust 包清单（`crates/*/Cargo.toml`、`apps/server/Cargo.toml`、`apps/cli/Cargo.toml` 和 `apps/desktop/src-tauri/Cargo.toml`）中设置目标版本，并保持 `apps/desktop/package.json`、`apps/desktop/src-tauri/tauri.conf.json`、`apps/extension/package.json`、`apps/extension/manifest.json` 的版本一致。JSON-RPC 协议版本是独立的兼容性编号。
2. 将发布内容从 `Unreleased` 移至 `CHANGELOG.md` 和 `CHANGELOG.zh-CN.md` 中带日期的版本条目。
3. 在仓库根目录运行[开发指南](DEVELOPMENT.zh-CN.md#rust-检查)列出的四条 Rust CI 命令。
4. 在 `apps/desktop` 运行 `pnpm install && pnpm build`；在 `apps/extension` 运行 `pnpm install && pnpm lint && pnpm build`。这些 TypeScript 检查目前不在 CI 中。
5. 运行 `cargo build --release -p nexum-server -p nexum-cli` 构建 Rust 命令行产物；可执行文件为 `target/release/nexum-server` 和 `target/release/nexum-cli`。桌面安装包需先构建前端，再在每个目标平台使用 Tauri 2 CLI，从 `apps/desktop` 执行 `cargo tauri build`；Tauri 打包配置启用了该平台支持的全部目标。
6. 打包扩展时包含根目录 `manifest.json`、`icons/` 和生成的 `dist/`。清单同时引用后两处的文件，仅有 `dist/` 无法作为扩展加载。
7. 使用本地 Server 对可执行文件和原生桌面包进行冒烟测试。通过 HTTP 提供一个已知文件，创建任务并将其 URL 加入队列；自动派发的 Worker 阻塞期间查询 `task.get` 或 `task.list`，确认能看到中间字节数；`task.start` 仍可作为排队 HTTP 任务的可选手动 kick。Worker 完成后核对目标文件字节和已持久化的 `Completed` 状态。确认失败传输保留已有目标文件，并通过任务 `error` 字段返回错误；在重试预算未耗尽时自动重试，并在预算耗尽后使用同一 `--data-dir` 重启 Server 仍保留最终错误。使用分块本地 fixture 验证活跃任务的 `task.pause` 会等待 `Paused`，`task.resume` 会继续同一个 Worker，`task.remove` 会取消 Worker，最多等待 30 秒后删除任务并保留原有目标文件。记录阻塞中的响应读取可能让 `task.pause` 等到 30 分钟 HTTP 超时；删除超时会返回错误，Worker 退出后可以重试。验证没有 ETag 或 Last-Modified 的响应在中断后会从零开始，并确认成功、取消和 `task.remove` 会删除部分文件及 sidecar。确认 Server 会拒绝数据目录内和重叠的目标。关闭认证时，从 `apps/extension` 加载 Browser 扩展，发送已知 HTTP 链接，确认 HTTP `/jsonrpc` 桥接会创建并排队任务。验证网页 `Origin` 被拒绝，而扩展 `Origin` 会收到 CORS 头。
8. 验证实际产物后，人工创建版本标签和 GitHub Release；在发布说明中记录目标平台及尚未完成的集成。

英文版见 [RELEASE.md](RELEASE.md)。
