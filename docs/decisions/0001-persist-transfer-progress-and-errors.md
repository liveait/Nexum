# ADR 0001: Persist Transfer Progress and Errors

- Status: Accepted; decision 5 superseded by ADR 0002
- Date: 2026-09-25

## Context

The HTTP worker used to persist only the final byte count. A long-running transfer therefore looked unchanged through the task API, and a failed transfer exposed its message only in the server log. SQLite schema version 1 had no place to retain that error across a restart. The server already owns task persistence and has an existing retry policy, so the change must preserve those boundaries and remain compatible with existing databases.

## Decision

1. Report a progress snapshot after every response chunk written by `HttpEngine`.
2. Persist intermediate progress from the server after at least 1 MiB has arrived or 250 ms have elapsed since the previous write. Always persist the final snapshot before marking a successful task `Completed`.
3. Add nullable `last_error` storage to `DownloadTask`, `StoredTask`, and SQLite schema version 2. Migrate version 1 databases by adding the column with a null value.
4. Expose the most recent transfer error as the optional `error` field on `TaskView`. A failed transfer keeps that message while the existing retry budget remains; after the budget is exhausted the task stays `Failed`. Claiming a new attempt clears the error and resets progress. Successful completion also clears it.
5. Keep retry dispatch manual for now. A queued retry still requires another `task.start` request. This temporary decision is superseded by [ADR 0002](0002-auto-dispatch-http-transfers.md), which moves retry dispatch into the server-owned HTTP dispatcher.

## Rationale

This uses the existing Core, scheduler, repository, and JSON-RPC boundaries. Throttling limits SQLite writes without hiding progress from the engine, while the final flush preserves an exact completed byte count. A nullable column makes the database migration additive and gives older tasks a defined empty error value.

## Consequences

- Clients can observe useful progress during a transfer and inspect the last failure after a restart.
- The protocol gains an additive `error` field; clients should ignore unknown response fields.
- At the time of this decision, a transfer started from byte zero after restart and active HTTP controls were not implemented. Same-process controls were later defined by [ADR 0003](0003-http-transfer-controls.md), and validated cross-restart resume is now defined by [ADR 0004](0004-cross-restart-http-resume.md).
- Intermediate progress creates additional SQLite writes, bounded by the byte and time thresholds.

## Alternatives Considered

- Persist only the final snapshot: fewer writes, but no live progress or restart-visible transfer state.
- Store errors in a separate history table: richer history, but unnecessary for the current "most recent error" contract and a larger migration.
- Add a server event stream first: useful for push clients, but it would not solve durable state for polling clients or restart recovery.

For the Chinese version, see [0001-persist-transfer-progress-and-errors.zh-CN.md](0001-persist-transfer-progress-and-errors.zh-CN.md).
