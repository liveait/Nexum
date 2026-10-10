# Architecture Decision Records

ADR 用于记录已经作出的重要技术决策，以及决策发生时的背景。

建议记录：

- 背景
- 决策
- 原因
- 影响
- 被考虑的替代方案

当前已记录的决策有：

- [0001：持久化传输进度与错误](0001-persist-transfer-progress-and-errors.zh-CN.md)
- [0002：由 Server 负责 HTTP 传输自动派发](0002-auto-dispatch-http-transfers.zh-CN.md)
- [0003：Server HTTP 传输的协作式控制](0003-http-transfer-controls.zh-CN.md)
- [0004：带校验的跨重启 HTTP 续传](0004-cross-restart-http-resume.zh-CN.md)
- [0005：分阶段接入 TLS 传输](0005-tls-transport.zh-CN.md)
- [0006：由 macOS App 托管本地 Server](0006-managed-macos-server.zh-CN.md)

ADR 0002 取代了 ADR 0001 中“重试派发保持手动”的临时决策。后续决策请使用递增的编号文件名：

```text
docs/decisions/
├── 0001-use-rust-core.md
├── 0002-use-sqlite-storage.md
└── 0003-protocol-versioning.md
```
