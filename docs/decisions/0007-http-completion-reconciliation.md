# ADR 0007: Reconcile HTTP File Publication and Task Completion

- Status: Accepted; implemented, verification recorded below
- Date: 2026-10-11

## Context

[ADR 0004](0004-cross-restart-http-resume.md) safely publishes a complete HTTP file without replacing an existing destination. File publication and SQLite task persistence cannot be one transaction: a process can exit after publication but before saving `Completed`. Previously the recovered worker treated its own complete destination as a conflict. This also affects responses without validators or a known total length. [ADR 0006](0006-managed-macos-server.md) requires recovery across this window before M1 can pass.

## Decision

1. After a validated response reaches EOF and the staging file is synced, the engine writes an atomic, bounded JSON completion receipt before publication. The receipt lives in the Server's private `data_dir/http-completions/`, named by SHA-256 of the task ID. It binds the task ID, source, normalized destination and staging paths, actual bytes, original optional total, file identity and SHA-256. The existing HTTP validator sidecar remains separate; generic task storage gains no HTTP fields.
2. Keep the existing atomic no-clobber rename and hard-link fallback. Retain the completion receipt until final progress and `Completed` are persisted together by one Core repository update. A persistence failure retains the receipt for another attempt; an existing destination is never overwritten.
3. Before making an HTTP request, the worker tries receipt reconciliation. A regular destination with matching identity, length and digest can finish without network access. If the destination is absent, matching complete staging bytes can be published without network access. A matching digest alone cannot identify an unrelated replacement file. Reject symbolic links, special files, malformed or oversized receipts and binding mismatches. Preserve conflicting user files and report an error.
4. Startup reads only bounded receipt metadata. A failed task with a correctly bound pending receipt may be requeued for reconciliation; other failed tasks retain their existing recovery behavior. Hashing runs in the HTTP worker after Server readiness and cooperates with pause/cancel. Already completed tasks need only residual-receipt cleanup, without rehashing their files. This keeps the managed Server within its six-second readiness budget.
5. Cleanup of a leftover staging path checks file identity before unlinking it, including the hard-link case where both paths remain. Task removal stops its worker and deletes the database task before clearing the receipt. If cleanup fails, report the error and retain any remaining private receipt as orphan evidence; the removed database task is not resurrected. Reuse of an absent task ID requires successful orphan-receipt cleanup before creation, so a previous task's evidence cannot complete a new task. Cleanup never deletes the final destination.

## Verification plan

Use isolated temporary storage and local HTTP fixtures. Cover receipt written before publication, publication before SQLite completion, completion persisted before cleanup, unknown lengths without validators, persistence failures and restart, task ID reuse, hard-link fallback, cancellation, changed content or identity, and invalid/symlink/special-file receipts. Assert exact file bytes and persisted state, and that recovery needs no second HTTP request. Do not add an unconditional production crash-injection entry point or touch the running Desktop's task database.

## Consequences

- Completed bytes and task state can converge after a process crash without redownloading or claiming an unrelated file.
- Full digest validation costs one local read during preparation and pending recovery. It must not delay Server readiness.
- This closes a process-crash window. Power-loss durability still depends on platform and filesystem synchronization semantics; the receipt is not a backup of downloaded bytes.
- M0 VoiceOver speech and M1 final clean-user/upgrade acceptance remain separate gates. This backend change does not complete either milestone.

## Implemented verification

On October 11, engine tests cover receipt preparation and publication, forced hard-link fallback, digest changes with restored modification time, different file identities with identical content, empty responses, invalid bindings, oversized/corrupt records, symlink/FIFO protection, and pause/cancel. Core tests prove final progress/state and events either commit together or remain unchanged after a rejected update.

Server tests use real child processes and independent temporary databases. A local HTTP fixture supplies bytes while SQLite triggers reject completion (and, separately, both terminal states); the test terminates and restarts the child with the same data directory, then verifies the complete file and durable `Completed` state. Other fixtures reconstruct pre-publication and post-persistence boundaries, recover unknown-length responses after the HTTP listener has closed, preserve replacement files, and reject unsafe task ID reuse. See the backend evidence in [Release Process](../RELEASE.md#http-completion-reconciliation-backend-evidence). No native App restart was part of this slice. Windows file identity is implemented but has not been executed in this macOS verification.

## Alternatives considered

- **Accept any existing file by size or digest:** can claim a user-created replacement and provides no proof of publication by this task.
- **Save `Completed` before publication:** can report success while the final file is absent.
- **Put HTTP fields in every stored task:** changes the generic storage contract without making filesystem and SQLite commits atomic.
- **Hash every receipt before listening:** delays managed startup for large downloads and can trigger repeated supervisor timeouts.

For the Chinese version, see [0007-http-completion-reconciliation.zh-CN.md](0007-http-completion-reconciliation.zh-CN.md).
