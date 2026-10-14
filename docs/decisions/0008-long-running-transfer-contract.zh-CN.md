# ADR 0008: 长期传输契约

- 状态：已接受
- 日期：2026-10-14
- 取代：无

## 背景

当前 `EngineAdapter` trait 是同步的：`start()` 立即返回一个句柄，而实际传输在独立 worker 中运行。Server 层拥有 HTTP worker 并分派任务。同步契约适用于 HTTP（server 管理 worker 线程），但无法表达 FTP（可能需要数分钟连接、认证、列出目录）或涉及多步协商的未来协议的语义。

M5 计划称：“定义能力声明、异步启动/进度、暂停/取消、重试、恢复元数据、错误和 Server 派发长期运行引擎契约。现有 `EngineAdapter::start` 同步返回，而实际 HTTP 控制位于独立 server worker；在添加新协议前先让 HTTP 通过新契约。”

## 决定

1. **增强 `EngineCapabilities`**：添加字段描述引擎能力：

   ```rust
   pub struct EngineCapabilities {
       pub supports_pause: bool,
       pub supports_resume: bool,
       pub supports_remove: bool,
       pub supports_progress: bool,
       pub supports_range: bool,
       pub supports_retry: bool,
       pub supported_protocols: &'static [&'static str],
       pub max_concurrent_segments: usize,
   }
   ```

   - `supports_range` — 引擎是否支持将下载分割为并发 Range 段。
   - `supports_retry` — 引擎是否在瞬态错误时自动重试（而非让 scheduler 重试）。
   - `supported_protocols` — 引擎支持的协议标识数组（如 HTTP 引擎为 `["http", "https"]`）。
   - `max_concurrent_segments` — 多连接的最大段数（0 = 仅单连接）。

2. **异步 Session API**：用 session 替换 `start()`，该 session 拥有传输生命周期：

   ```rust
   fn start(&mut self, task_id: TaskId, source: String, destination: String) -> Result<Box<dyn TransferSession>, EngineError>;
   ```

   ```rust
   pub trait TransferSession: Send + Sync {
       fn progress(&self) -> Progress;
       fn state(&self) -> EngineTaskState;
       fn error(&self) -> Option<String>;
       fn pause(&self) -> Result<(), EngineError>;
       fn resume(&self) -> Result<(), EngineError>;
       fn cancel(&self);
       fn wait(self: Box<Self>) -> (EngineTaskState, Progress);
   }
   ```

   - `progress()` — 返回当前进度（已传字节、已知总字节、速率）。
   - `state()` — 返回当前状态（Queued, Downloading, Paused, Completed, Failed）。
   - `error()` — 返回导致传输失败的错误（仍在运行或已完成时为 None）。
   - `pause()` — 请求边界暂停。返回 Ok(()) 表示接受，或 Err 表示引擎不支持。传输在下一chunk边界前继续。
   - `resume()` — 恢复暂停的传输。返回 Ok(()) 表示引擎支持恢复，或 Err 表示不支持。传输从暂停点继续。
   - `cancel()` — 请求取消。引擎应尽快停止，清理部分文件，保留目标文件不变。
   - `wait()` — 阻塞等待传输到达终止状态（Completed 或 Failed），返回最终进度和/或错误。

3. **协议级恢复元数据**：为支持跨重启续传的工程添加 trait：

   ```rust
   pub trait ResumableEngine {
       fn resume_metadata(&self) -> Option<ResumeMetadata>;
   }

   pub struct ResumeMetadata {
       pub source: String,
       pub destination: String,
       pub downloaded_bytes: u64,
       pub expected_total: Option<u64>,
       pub validator: Option<HttpResumeValidator>,
       pub partial_path: PathBuf,
   }
   ```

4. **Server 派发扩展**：现有分发器从队列声明 HTTP 任务并生成 server 拥有的 worker。分发器还应：
   - 检查 `EngineCapabilities.supported_protocols` 以匹配任务源 URL 方案。
   - 跳过不支持所需协议的引擎。
   - 当没有引擎支持某源时返回清晰错误。

5. **HTTP 引擎迁移**：现有 `HttpEngine` 必须重构以实现新的 `TransferSession` trait（或对应适配器），且不改变其可观察行为。server RPC 接口上的现有 `task.pause`、`task.resume`、`task.remove` 继续正常工作。

6. **向后兼容**：现有同步 `EngineAdapter::start()` 保留给 `InMemoryEngine` 及其他同步引擎。新的 session API 为可选；实现它的引擎获得增强的能力声明和长期运行支持。

## 理由

- **FTP 和其他长期协议** 需要连接、认证、协商，甚至列出目录后才能开始下载。这些步骤可能耗时数分钟，无法由立即返回的同步 `start()` 建模。
- **能力声明** 使 scheduler 能根据源协议和引擎能力路由任务。
- **异步 Session API** 在 scheduler（管理队列、声明和重试预算）和 engine（管理实际传输生命周期）之间提供了清晰的边界。
- **恢复元数据** 使任何支持该功能的协议都能实现跨重启续传，不仅限于 HTTP。
- **向后兼容** 确保现有引擎（InMemoryEngine）无需修改继续工作。

## 后果

- Server 分发器必须检查能力声明以路由任务到合适的引擎。
- HTTP 引擎必须重构以实现 session API，同时保持当前行为。
- 新引擎（FTP、BT）可实现 session API 并被自动发现。
- 同步 `EngineAdapter` 已弃用但未移除；它服务于不需要长期支持的简单引擎。

## 考虑的替代方案

- **基于事件的进度**：不用阻塞 `wait()`，提供进度流（通道或回调）。这更灵活但增加复杂性；阻塞 `wait()` 对初始契约更简单。
- **每个引擎的异步 API**：不统一 trait，让每个引擎定义自己的异步 API。这避免了抽象，但使分发器更复杂。
- **保持同步 start**：继续对所有引擎使用 server 拥有的 worker 模型。这限制了契约到 server 已支持的内容，未触及计划添加 FTP 的目标。

中文版本见 [0008-long-running-transfer-contract.zh-CN.md](0008-long-running-transfer-contract.zh-CN.md)。
