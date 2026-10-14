<a id="adr-0008-long-running-transfer-contract"></a>
<a id="adr-0008-长期传输契约"></a>

# ADR 0008：长期传输契约 / Long-Running Transfer Contract

- 状态：设计已接受；实现与验收须分别验证<br>
  Status: Accepted design; implementation and acceptance must be verified separately
- 日期 / Date：2026-10-14
- 取代：无<br>
  Supersedes: none

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

当前 `EngineAdapter` trait 是同步的：`start()` 立即返回句柄，而实际传输在独立 worker 中运行。Server 层拥有 HTTP worker 并派发任务。同步契约适用于 HTTP（Server 管理 worker 线程），但无法表达 FTP（可能需要数分钟连接、认证和列出目录）或涉及多步协商的未来协议所需的语义。

The current `EngineAdapter` trait is synchronous: `start()` returns immediately with a handle, while the real transfer runs in a separate worker. The server layer owns the HTTP worker and dispatches tasks. The synchronous contract works for HTTP (where the server manages a worker thread) but cannot express the semantics needed for FTP (which may take minutes to connect, authenticate, and list directories) or future protocols that involve multi-step negotiations.

M5 计划要求：“为长期运行的引擎定义能力声明、异步启动／进度、暂停／取消、重试、恢复元数据、错误和 Server 派发。现有 `EngineAdapter::start` 同步返回，而实际 HTTP 控制位于独立 Server worker；在添加新协议前，先让 HTTP 通过新契约，并保持 M2–M4 无回归。”

The plan for M5 says: “Define capability declarations, asynchronous start/progress, pause/cancel, retry, recovery metadata, errors, and server dispatch for long-running engines. The current `EngineAdapter::start` returns synchronously while real HTTP control lives in a separate server worker. Move HTTP through the new contract without regressing M2–M4 before adding another protocol.”

<a id="decision"></a>
<a id="决定"></a>

## 决定 / Decision

1. **增强 `EngineCapabilities`**：添加字段描述引擎能力。<br>
   **Enhanced `EngineCapabilities`**: Add fields to describe engine capabilities.

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

   - 引擎是否支持将下载分割为并发 Range 段。<br>
     `supports_range` — whether the engine can split a download into concurrent Range segments.
   - 引擎是否在瞬态错误时自动重试，而非让 Scheduler 重试。<br>
     `supports_retry` — whether the engine automatically retries on transient errors, rather than letting the scheduler retry.
   - 引擎支持的协议标识数组，例如 HTTP 引擎的 `["http", "https"]`。<br>
     `supported_protocols` — array of supported protocol identifiers, such as `["http", "https"]` for the HTTP engine.
   - 多连接的最大段数；`0` 表示仅支持单连接。<br>
     `max_concurrent_segments` — maximum segment count for multi-connection; `0` means single-connection only.

2. **异步 Session API**：长期运行的引擎使用拥有传输生命周期的 session，替代同步 `start()` 契约。启动传输时返回用于观察和控制的句柄；句柄在传输完成、失败或被删除前有效。下面的可选 API 按第 6 项决策与同步 Adapter 共存。<br>
   **Asynchronous session API**: For long-running engines, use a session that owns the transfer lifecycle instead of the synchronous `start()` contract. Starting a transfer returns a handle to observe and control it; the handle is valid until the transfer completes, fails, or is removed. The opt-in API below coexists with the synchronous adapter under decision 6.

   ```rust
   fn start(
       &mut self,
       task_id: TaskId,
       source: String,
       destination: String,
   ) -> Result<Box<dyn TransferSession>, EngineError>;

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

   - 返回当前进度：已传字节、已知总字节数和速率。<br>
     `progress()` — returns current progress: bytes transferred, total if known, and rate.
   - 返回当前状态：`Queued`、`Downloading`、`Paused`、`Completed` 或 `Failed`。<br>
     `state()` — returns the current state: `Queued`, `Downloading`, `Paused`, `Completed`, or `Failed`.
   - 返回导致传输失败的错误；仍在运行或已完成时为 `None`。<br>
     `error()` — returns the error that caused transfer failure, or `None` if it is still running or completed.
   - 请求在块边界暂停。`Ok(())` 表示请求被接受；不支持暂停时返回错误。传输继续到下一个 chunk 边界后暂停。<br>
     `pause()` — requests a boundary pause. `Ok(())` means the request was accepted; an unsupported pause returns an error. The transfer continues until the next chunk boundary.
   - 恢复暂停的传输。`Ok(())` 表示引擎支持恢复；不支持时返回错误。传输在下一个 chunk 边界从暂停点继续。<br>
     `resume()` — resumes a paused transfer. `Ok(())` means the engine supports resume; an unsupported resume returns an error. The transfer continues from the pause point on the next chunk boundary.
   - 请求取消。引擎应尽快停止，清理部分文件，并保持目标文件不变。<br>
     `cancel()` — requests cancellation. The engine should stop as soon as possible, clean up partial files, and leave the destination untouched.
   - 阻塞等待传输到达终态（`Completed` 或 `Failed`），返回最终进度和／或错误；等待持续到传输完成、超时或 session 被取消。<br>
     `wait()` — blocks until the transfer reaches a terminal state (`Completed` or `Failed`) and returns final progress and/or error. It waits until the transfer completes, times out, or the session is cancelled.

3. **协议级恢复元数据**：为支持跨重启续传的引擎添加 trait。`resume_metadata()` 返回恢复本次传输所需的元数据；尚未开始或无法续传时返回 `None`。`ResumeMetadata` 描述进程重启后需要恢复的传输。<br>
   **Protocol-level recovery metadata**: Add a trait for engines that support cross-restart resume. `resume_metadata()` returns the metadata needed to resume this transfer, or `None` if it has not started or cannot be resumed. `ResumeMetadata` describes the transfer to recover after a process restart.

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

4. **Server 派发扩展**：现有派发器从队列领取 HTTP 任务并创建 Server 自己拥有的 worker。它还应：<br>
   **Server dispatch extension**: The existing dispatcher claims HTTP tasks from the queue and spawns Server-owned workers. It should also:

   - 检查 `EngineCapabilities.supported_protocols` 是否匹配任务来源 URL 的 scheme。<br>
     Check `EngineCapabilities.supported_protocols` against each task's source URL scheme.
   - 跳过不支持所需协议的引擎。<br>
     Skip engines that do not support the required protocols.
   - 没有引擎支持该来源时，报告清晰的错误。<br>
     Report a clear error when no engine supports a source.

5. **HTTP 引擎迁移**：重构现有 `HttpEngine`，使其实现新的 `TransferSession` trait 或相应 Adapter，并保持可观察行为不变。Server RPC 接口上的现有 `task.pause`、`task.resume` 和 `task.remove` 继续正常工作。<br>
   **HTTP engine migration**: Refactor the current `HttpEngine` to implement the new `TransferSession` trait or a corresponding adapter without changing its observable behavior. Existing `task.pause`, `task.resume`, and `task.remove` operations on the Server RPC interface continue to work as before.

6. **向后兼容**：为 `InMemoryEngine` 及其他同步引擎保留现有同步 `EngineAdapter::start()`。新的 session API 为可选；实现它的引擎获得增强的能力声明和长期运行支持。<br>
   **Backward compatibility**: Retain the existing synchronous `EngineAdapter::start()` for `InMemoryEngine` and other synchronous engines. The new session API is opt-in; engines that implement it get enhanced capability declarations and long-running support.

<a id="rationale"></a>
<a id="理由"></a>

## 理由 / Rationale

- **FTP 和其他长期协议**需要在开始下载前连接、认证、协商，甚至列出目录。这些步骤可能耗时数分钟，无法由立即返回的同步 `start()` 建模。<br>
  **FTP and other long-running protocols** need to connect, authenticate, negotiate, and potentially list directories before starting a download. These steps can take minutes and cannot be modeled by a synchronous `start()` that returns immediately.
- **能力声明**使 Scheduler 能根据来源协议和引擎能力，将任务路由到合适的引擎。<br>
  **Capability declarations** enable the scheduler to route tasks to appropriate engines based on source protocol and engine capabilities.
- **异步 Session API**在管理队列、任务领取和重试预算的 Scheduler 与管理实际传输生命周期的 Engine 之间提供边界。<br>
  **Asynchronous session API** provides a boundary between the scheduler, which manages queues, claims, and retry budgets, and the engine, which manages the actual transfer lifecycle.
- **恢复元数据**使任何支持该功能的协议都能跨重启续传，不限于 HTTP。<br>
  **Recovery metadata** enables cross-restart resume for any protocol that supports it, not just HTTP.
- **向后兼容**确保 `InMemoryEngine` 等现有引擎无需修改即可继续工作。<br>
  **Backward compatibility** ensures existing engines such as `InMemoryEngine` continue to work without changes.

<a id="consequences"></a>
<a id="后果"></a>

## 后果 / Consequences

- Server 派发器必须检查能力声明，将任务路由到合适的引擎。<br>
  The Server dispatcher must check capability declarations to route tasks to appropriate engines.
- HTTP 引擎必须实现 session API，同时保持当前行为。<br>
  The HTTP engine must implement the session API while preserving current behavior.
- FTP、BT 等新引擎可实现 session API 并被自动发现。<br>
  New engines such as FTP and BT can implement the session API and be discovered automatically.
- 本设计弃用但保留同步 `EngineAdapter`，供不需要长期运行支持的简单引擎使用。<br>
  Under this design, the synchronous `EngineAdapter` is deprecated but retained for simple engines that do not need long-running support.

<a id="alternatives-considered"></a>
<a id="考虑的替代方案"></a>

## 考虑的替代方案 / Alternatives Considered

- **基于事件的进度**：使用通道或回调提供进度流，替代阻塞 `wait()`。更灵活但增加复杂性；阻塞 `wait()` 对初始契约更简单。<br>
  **Event-based progress**: Provide a progress stream via channels or callbacks instead of blocking `wait()`. It is more flexible but adds complexity; blocking `wait()` is simpler for the initial contract.
- **每个引擎的异步 API**：让每个引擎定义自己的异步 API，不统一 trait。可避免抽象，但使派发器更复杂。<br>
  **Per-engine async APIs**: Let each engine define its own async API instead of a uniform trait. It avoids abstractions but makes the dispatcher more complex.
- **保持同步 start**：继续对所有引擎使用 Server 自己拥有的 worker 模型。契约受限于 Server 已有支持，无法实现计划中添加 FTP 的目标。<br>
  **Keep synchronous start**: Continue using the Server-owned worker model for all engines. It limits the contract to what the Server already supports and does not address the plan's goal of adding FTP.
