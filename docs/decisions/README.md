<a id="architecture-decision-records"></a>

# 架构决策记录 / Architecture Decision Records

ADR 用于记录重要技术决策及其背景，例如 Rust Core、SQLite 存储、Protocol 版本管理、传输边界等需要长期保留的架构约定。

ADRs record important technical decisions and the context in which they were made. Examples include decisions about the Rust core, SQLite storage, Protocol versioning, transport boundaries, and other durable architecture decisions.

建议章节：背景、决策、原因、影响，以及被考虑的替代方案。

Recommended sections: Context, Decision, Rationale, Consequences, and Alternatives Considered.

**状态边界：**“已接受”表示设计决策已被接受，不代表当前实现已通过验收。历史实现或验证记录只适用于所注明的切片和日期；当前交付状态以[开发计划](../DEVELOPMENT_PLAN.md)以及最终提交的测试／CI 证据为准。

**Status scope:** “Accepted” means the design decision was accepted. It does not establish that the current implementation has passed acceptance. Historical implementation or verification notes apply to the named slice and date; check the [Development Plan](../DEVELOPMENT_PLAN.md) and the final commit's test/CI evidence for current delivery status.

当前已记录的决策：

The recorded decisions are:

- [0001：持久化传输进度与错误 / Persist Transfer Progress and Errors](0001-persist-transfer-progress-and-errors.md)
- [0002：由 Server 负责 HTTP 传输自动派发 / Server-Owned Automatic Dispatch for HTTP Transfers](0002-auto-dispatch-http-transfers.md)
- [0003：Server HTTP 传输的协作式控制 / Cooperative Controls for Server HTTP Transfers](0003-http-transfer-controls.md)
- [0004：带校验的跨重启 HTTP 续传 / Validated Cross-Restart HTTP Resume](0004-cross-restart-http-resume.md)
- [0005：分阶段接入 TLS 传输 / Staged TLS Transport](0005-tls-transport.md)
- [0006：由 macOS App 托管本地 Server / App-Managed Local Server for macOS](0006-managed-macos-server.md)
- [0007：HTTP 文件发布与任务完成对账 / Reconcile HTTP File Publication and Task Completion](0007-http-completion-reconciliation.md)
- [0008：长期传输契约 / Long-Running Transfer Contract](0008-long-running-transfer-contract.md)

ADR 0002 取代 ADR 0001 中“重试派发保持手动”的临时决策。后续决策使用递增的编号文件名；下列文件名仅示意编号方式，不代表现有记录：

ADR 0002 supersedes ADR 0001's temporary decision to dispatch retries manually. New decisions should use the next numbered filename; the filename examples below illustrate the numbering scheme, not existing records:

```text
docs/decisions/
├── 0001-use-rust-core.md
├── 0002-use-sqlite-storage.md
└── 0003-protocol-versioning.md
```
