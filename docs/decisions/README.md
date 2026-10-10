# Architecture Decision Records

ADRs record important technical decisions and the context in which they were made.

Recommended sections: Context, Decision, Rationale, Consequences, and Alternatives Considered.

Examples include decisions about the Rust core, SQLite storage, Protocol versioning, transport boundaries, and other durable architecture decisions.

The recorded decisions are:

- [0001: Persist Transfer Progress and Errors](0001-persist-transfer-progress-and-errors.md)
- [0002: Server-Owned Automatic Dispatch for HTTP Transfers](0002-auto-dispatch-http-transfers.md)
- [0003: Cooperative Controls for Server HTTP Transfers](0003-http-transfer-controls.md)
- [0004: Validated Cross-Restart HTTP Resume](0004-cross-restart-http-resume.md)
- [0005: Staged TLS Transport](0005-tls-transport.md)
- [0006: App-Managed Local Server for macOS](0006-managed-macos-server.md)

ADR 0002 supersedes ADR 0001's temporary decision to dispatch retries manually. New decisions should use the next numbered filename.

For the Chinese version, see [README.zh-CN.md](README.zh-CN.md).
