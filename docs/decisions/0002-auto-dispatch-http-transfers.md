# ADR 0002: Server-Owned Automatic Dispatch for HTTP Transfers

- Status: Accepted
- Date: 2026-09-26
- Supersedes: ADR 0001, decision 5 (retry dispatch remains manual)

## Context

The server already owns the running HTTP worker and the Core/Scheduler state, while the Scheduler deliberately remains free of network I/O. Requiring a separate `task.start` request after every `task.queue`, retry, or restart made the persisted queue dependent on client polling and left available concurrency slots idle. The server also needs to avoid starting unsupported sources or two HTTP transfers that target the same normalized destination.

## Decision

1. Keep automatic dispatch in the server layer. The Scheduler continues to provide queue ordering, claims, retry policy, and concurrency accounting; it does not spawn workers or perform network I/O.
2. Run the server dispatcher after a successful `task.queue`, after startup recovery and listener setup, and after every HTTP worker completion or failure. The dispatcher repeatedly claims eligible queued HTTP/HTTPS tasks until no scheduler slot or eligible task remains.
3. An eligible task must resolve to HTTP or HTTPS, have a permitted destination outside the data directory, and not overlap an active destination. Magnet and local-file tasks remain queued without blocking compatible HTTP work.
4. Keep `task.start` as a manual kick and compatibility method. It claims one queued HTTP/HTTPS task and then invokes the same dispatcher to fill remaining slots. The RPC returns before transfer completion.
5. Spawn workers after releasing the Core mutex. On normal completion or failure, release the active task and destination bookkeeping before dispatching again. Existing retry policy determines whether a failed task is requeued; the dispatcher starts that retry when a slot is available.

## Rationale

The server is the only current component that combines RPC operations, persistent Core state, Scheduler claims, destination safety checks, and the HTTP engine. Keeping orchestration there preserves the Scheduler's library boundary and makes queueing and recovery useful without requiring a client-side polling loop. Reusing one dispatcher for queue, startup, worker completion, worker failure, and manual kick keeps concurrency and destination checks consistent.

## Consequences

- Queueing an eligible HTTP/HTTPS task is sufficient to start it when capacity is available.
- A server restart automatically dispatches eligible queued HTTP/HTTPS tasks after recovery. The HTTP worker resumes a matching validated partial with `Range`/`If-Range`; unsupported, unvalidated, or blocked work remains queued or starts from byte zero as defined by [ADR 0004](0004-cross-restart-http-resume.md).
- A failed HTTP attempt is automatically retried while the existing retry budget permits. Retry counts and scheduler ordering remain in memory and are not persisted across restart.
- Magnet and local-file transfers still have no engine. Active HTTP control is defined separately by [ADR 0003](0003-http-transfer-controls.md); this dispatcher only starts and retires workers.
- A worker panic or poisoned server mutex can leave active bookkeeping unreleased; recovery for those process-level failures remains outside this decision.

## Alternatives Considered

- **Require a new `task.start` request for every attempt:** simple to implement, but leaves queued work idle and couples progress to client polling.
- **Put worker spawning inside Scheduler/Core:** would make the reusable scheduler perform network orchestration and cross the existing engine boundary.
- **Poll for queued work in a background timer:** would add latency and another lifecycle loop without improving the event-driven triggers already available in the server.

For the Chinese version, see [0002-auto-dispatch-http-transfers.zh-CN.md](0002-auto-dispatch-http-transfers.zh-CN.md).
