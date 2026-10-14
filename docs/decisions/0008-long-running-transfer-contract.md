# ADR 0008: Long-Running Transfer Contract

- Status: Accepted
- Date: 2026-10-14
- Supersedes: none

## Context

The current `EngineAdapter` trait is synchronous: `start()` returns immediately with a handle, while the real transfer runs in a separate worker. The server layer owns the HTTP worker and dispatches tasks. The synchronous contract works for HTTP (where the server manages a worker thread) but cannot express the semantics needed for FTP (which may take minutes to connect, authenticate, and list directories) or future protocols that involve multi-step negotiations.

The plan for M5 says: "Define capability declarations, asynchronous start/progress, pause/cancel, retry, recovery metadata, errors, and server dispatch for long-running engines. The current `EngineAdapter::start` returns synchronously while real HTTP control lives in a separate server worker. Move HTTP through the new contract without regressing M2–M4 before adding another protocol."

## Decision

1. **Enhanced `EngineCapabilities`**: Add fields to describe engine capabilities:

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

   - `supports_range` — whether the engine can split a download into concurrent Range segments.
   - `supports_retry` — whether the engine automatically retries on transient errors (vs. letting the scheduler retry).
   - `supported_protocols` — array of protocol identifiers the engine supports (e.g. `["http", "https"]` for the HTTP engine).
   - `max_concurrent_segments` — maximum segment count for multi-connection (0 = single-connection only).

2. **Asynchronous session API**: Replace `start()` with a session that owns the transfer lifecycle:

   ```rust
   /// Starts a transfer and returns a handle to observe and control it.
   /// The returned handle is valid until the transfer completes, fails, or is removed.
   fn start(
       &mut self,
       task_id: TaskId,
       source: String,
       destination: String,
   ) -> Result<Box<dyn TransferSession>, EngineError>;
   ```

   ```rust
   /// Trait for a long-running transfer session.
   pub trait TransferSession: Send + Sync {
       /// Returns the current progress (bytes transferred, total if known, rate).
       fn progress(&self) -> Progress;

       /// Returns the current state (Queued, Downloading, Paused, Completed, Failed).
       fn state(&self) -> EngineTaskState;

       /// Returns any error that caused the transfer to fail (None if still running or completed).
       fn error(&self) -> Option<String>;

       /// Requests a boundary pause. Returns Ok(()) if the pause was accepted,
       /// or Err if the engine does not support pause. The transfer continues
       /// until the next chunk boundary.
       fn pause(&self) -> Result<(), EngineError>;

       /// Resumes a paused transfer. Returns Ok(()) if the engine supports resume,
       /// or Err if the engine does not support resume. The transfer continues
       /// from the pause point on the next chunk boundary.
       fn resume(&self) -> Result<(), EngineError>;

       /// Requests cancellation. The engine should stop as soon as possible,
       /// cleaning up any partial files and leaving the destination untouched.
       fn cancel(&self);

       /// Waits for the transfer to reach a terminal state (Completed or Failed)
       /// and returns the final progress and/or error. This blocks until the
       /// transfer completes, times out, or the session is cancelled.
       fn wait(self: Box<Self>) -> (EngineTaskState, Progress);
   }
   ```

3. **Protocol-level recovery metadata**: Add a trait for engines that support cross-restart resume:

   ```rust
   /// Trait for engines that can resume from a previously interrupted transfer.
   pub trait ResumableEngine {
       /// Returns the metadata needed to resume this transfer, or None if
       /// the transfer has not yet started or cannot be resumed.
       fn resume_metadata(&self) -> Option<ResumeMetadata>;
   }

   /// Metadata needed to resume a transfer after a process restart.
   pub struct ResumeMetadata {
       pub source: String,
       pub destination: String,
       pub downloaded_bytes: u64,
       pub expected_total: Option<u64>,
       pub validator: Option<HttpResumeValidator>,
       pub partial_path: PathBuf,
   }
   ```

4. **Server dispatch extension**: The existing dispatcher claims HTTP tasks from the queue and spawns server-owned workers. The dispatcher should also:
   - Check `EngineCapabilities.supported_protocols` against each task's source URL scheme.
   - Skip engines that do not support the required protocols.
   - Report a clear error when no engine supports a source.

5. **HTTP engine migration**: The current `HttpEngine` must be refactored to implement the new `TransferSession` trait (or a corresponding adapter), without changing its observable behavior. The existing `task.pause`, `task.resume`, `task.remove` on the server RPC interface continue to work as before.

6. **Backward compatibility**: The existing synchronous `EngineAdapter::start()` remains for the `InMemoryEngine` and any other synchronous engines. The new session API is opt-in; engines that implement it get the enhanced capability declarations and long-running support.

## Rationale

- **FTP and other long-running protocols** need to connect, authenticate, negotiate, and potentially list directories before starting a download. These steps can take minutes and cannot be modeled by a synchronous `start()` that returns immediately.
- **Capability declarations** enable the scheduler to route tasks to appropriate engines based on source protocol and engine capabilities.
- **Asynchronous session API** provides a clean boundary between the scheduler (which manages queues, claims, and retry budgets) and the engine (which manages the actual transfer lifecycle).
- **Recovery metadata** enables cross-restart resume for any protocol that supports it, not just HTTP.
- **Backward compatibility** ensures existing engines (InMemoryEngine) continue to work without changes.

## Consequences

- The server dispatcher must check capability declarations to route tasks to appropriate engines.
- The HTTP engine must be refactored to implement the session API while preserving current behavior.
- New engines (FTP, BT) can implement the session API and be discovered automatically.
- The synchronous `EngineAdapter` is deprecated but not removed; it serves simple engines that do not need long-running support.

## Alternatives Considered

- **Event-based progress**: Instead of a blocking `wait()`, provide a progress stream (channels or callbacks). This is more flexible but adds complexity; a blocking `wait()` is simpler for the initial contract.
- **Per-engine async APIs**: Instead of a uniform trait, let each engine define its own async API. This avoids abstractions but makes the dispatcher more complex.
- **Keep synchronous start**: Continue using the server-owned worker model for all engines. This limits the contract to what the server already supports and does not address the plan's goal of adding FTP.

For the Chinese version, see [0008-long-running-transfer-contract.zh-CN.md](0008-long-running-transfer-contract.zh-CN.md).
