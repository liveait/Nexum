# ADR 0004: Validated Cross-Restart HTTP Resume

- Status: Accepted
- Date: 2026-09-28

## Context

Before this decision, the server staged each HTTP response in a process-unique `.part` file. A restart left no safe way to identify that file or verify that a remote response still represented the same bytes, so recovery started from byte zero. Long downloads need a bounded restart path that does not replace the destination with an unvalidated partial response.

## Decision

1. The server-owned HTTP worker derives one hidden, stable partial-file path beside the destination and passes it to the resumable HTTP engine. The path is tied to the destination; only one active task may reserve that destination at a time.
2. The engine writes an atomic JSON sidecar beside the partial file. The sidecar records the source, destination, response validator (`ETag` or `Last-Modified`), and expected total length. The partial-file length remains the byte offset of the next request.
3. When a valid partial file and validator exist, the engine sends `Range: bytes=<offset>-` with `If-Range` and accepts only a matching `206 Partial Content` range. A `200 OK`, missing or changed validator, malformed `Content-Range`, or inconsistent length discards the partial response and starts a fresh full response.
4. A response without a validator is allowed to finish in the current process but is not resumed after restart. A resumable partial response is preserved across ordinary worker errors and process exit; cancellation, task removal, and successful destination replacement remove the partial file and sidecar.
5. SQLite continues to persist task state and progress. The sidecar is transfer-session metadata owned by the HTTP engine, so the generic `TaskRepository` and synchronous `EngineAdapter` contracts do not gain HTTP-specific fields.

## Rationale

The destination-side name makes recovery deterministic without adding a schema migration or exposing engine-specific fields through every task view. Validators prevent appending bytes from a changed representation, while the full-response fallback handles servers that ignore or reject range requests. Atomic sidecar replacement keeps metadata either old and usable or absent, never half-written.

## Consequences

- Recovery can resume eligible HTTP/HTTPS work from a verified byte offset after a server restart.
- Servers that do not provide a stable `ETag` or `Last-Modified` still restart from byte zero after interruption.
- The sidecar and partial file must remain beside the destination and are subject to the same destination ownership and cleanup rules.
- Cross-host resume, multipart responses, and resumable magnet or local-file transfers remain outside this decision.

## Alternatives Considered

- **Persist validators in the task schema:** would couple generic task storage and every client view to HTTP-specific state; the sidecar keeps the storage contract stable.
- **Resume by file length without a validator:** could append bytes from a changed remote representation and corrupt the final file.
- **Use a random temporary file per process:** cannot identify or validate interrupted work after restart.

For the Chinese version, see [0004-cross-restart-http-resume.zh-CN.md](0004-cross-restart-http-resume.zh-CN.md).
