# ADR 0003: Cooperative Controls for Server HTTP Transfers

- Status: Accepted
- Date: 2026-09-26

## Context

The server runs blocking HTTP workers directly instead of through the synchronous `EngineAdapter::start` path. Until now it rejected pause, resume, and remove while a worker was active. The task protocol already exposes those operations, and clients need a safe way to stop a download without replacing an existing destination.

Cross-restart resume would require stable partial-file names, validators, Range requests, and persisted transfer metadata. A smaller same-process control boundary can provide useful pause and cancellation semantics without changing the task state model or storage schema.

## Decision

1. Give each server HTTP worker a shared `HttpTransferControl` backed by a mutex and condition variable. The worker checks it at response chunk boundaries.
2. `task.pause` requests a boundary pause, persists the latest progress, and transitions the task to `Paused`. The worker keeps its temporary file and response open while paused.
3. `task.resume` uses the existing Scheduler slot accounting and wakes the same worker. It returns `false` when no slot is available, matching the existing protocol contract.
4. `task.remove` marks the active transfer for destructive cancellation, wakes the worker, waits up to 30 seconds for completion, and then removes the task through `Core`. If the worker remains blocked, the request returns an error and can be retried after it exits. Cancellation drops the temporary file and never enters the normal failed/retry path; an existing destination is left untouched.
5. Keep the generic synchronous `HttpEngine` adapter pause/resume capabilities disabled. These controls belong to the server-owned worker path until the adapter supports an asynchronous session API.

## Rationale

The server already owns the worker, Core lock, scheduler slot, destination reservation, and RPC request. A cooperative boundary preserves those invariants while avoiding a second task state or a storage migration. Waiting for worker completion before removal prevents a deleted task from receiving progress, retrying, or renaming a completed temporary file afterward.

## Consequences

- Pause and resume work only while the same server process and HTTP response remain alive. A peer or network that closes an idle response can still make resume fail.
- A blocking response read cannot observe a condition variable until the 30-minute HTTP request timeout expires, so `task.pause` may be delayed by that timeout. `task.remove` waits up to 30 seconds for the worker and returns an error if it is still blocked; the request can be retried after the worker exits.
- Restart recovery still normalizes interrupted work to `Queued`; validated cross-restart Range resume is defined by [ADR 0004](0004-cross-restart-http-resume.md), while responses without a usable validator still begin from byte zero.
- The existing JSON-RPC shapes remain unchanged: pause and remove return `true`, while resume returns `true` or `false`.

## Alternatives Considered

- **Cancel and requeue on every pause:** would lose the same-process partial response and make pause indistinguishable from a retry.
- **Implement Range resume now:** would require persisted partial-file identity, validators, response validation, and recovery changes beyond this slice.
- **Expose controls through the synchronous adapter:** would require turning `EngineAdapter::start` into an asynchronous session boundary and affect all existing engines.

For the Chinese version, see [0003-http-transfer-controls.zh-CN.md](0003-http-transfer-controls.zh-CN.md).
