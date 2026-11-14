# FTP 来源凭据实施计划 / FTP Origin Credentials Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标 / Goal** — 提供可由真实 CLI 调用的版本化 FTP 任务创建入口，完成 Server 所有的 Keychain／SQLite 创建、幂等、重启和删除协调。 / Provide versioned FTP task creation through the real CLI with Server-owned Keychain/SQLite creation, idempotency, restart and removal coordination.

**架构 / Architecture** — 使用 RFC 0004 的 pending → provider 写入 → 任务及关联原子激活流程。数据库仅保存无秘密的版本化来源和引用。现有 HTTP 创建与调度继续使用既有入口。 / Use RFC 0004's pending → provider write → atomic task/association activation flow. Persist only versioned secret-free source/reference metadata. Existing HTTP creation and scheduling retain their entry points.

**技术 / Tech Stack** — Rust 2024、现有 serde／serde_json、rusqlite、rustls、getrandom；macOS security-framework。 / Rust 2024, existing serde/serde_json, rusqlite, rustls, getrandom, and macOS security-framework.

**方案 / Spec** — [RFC 0004](../../RFC/0004-ftp-credentials-and-recovery.md)。基线 / Baseline `f8b89c6cee11ddadf4abecffb8745cfbee74052b`。

## 全局约束 / Global Constraints

- 本包交付来源凭据协调，FTP 数据传输继续等待 F2／F3 的依赖和文件安全验收。能力协商分别描述创建、持久凭据和实际传输。 / Deliver origin-credential coordination here. FTP data transfer awaits F2/F3 dependency and file-safety acceptance. Advertise creation, persistent credentials and transfer separately.
- 旧 `task.create`、TaskView 字段、HTTP 完成凭证和下载行为保持兼容。历史非 HTTP 来源继续脱敏，只有可信版本化关联可投影规范来源。 / Preserve legacy `task.create`, TaskView fields, HTTP receipts and download behavior. Historical non-HTTP sources stay redacted. Only validated versioned associations may project canonical sources.
- 密码只通过验证 Server 身份的 TLS 输入，不提供密码命令行参数，不写入 SQLite、任务、事件、日志、Debug 或错误。 / Accept passwords only through identity-verified TLS. Provide no password arguments. Exclude secrets from SQLite, tasks, events, logs, Debug and errors.
- Keychain 服务独立于 Desktop RPC 服务，account 绑定随机数据实例、origin、随机凭据 ID 和 revision；provider 必须避免静默覆盖现有秘密。 / Isolate the Keychain service from Desktop RPC credentials. Bind accounts to random data instance, origin, random credential ID and revision. Prevent implicit secret replacement.
- provider 操作在 Core 锁外执行；恢复清理在 Ready 后进行，不让 Keychain 等待占用托管六秒预算。 / Run provider operations outside the Core lock and recovery cleanup after Ready, preserving the managed six-second budget.
- 本包只创建 revision 1；已有任务的秘密修改接口与在途 revision 退休随实际传输阶段交付。冲突重试拒绝改密。 / This package creates revision 1. Secret updates and retirement of in-flight revisions accompany the transfer stage. Conflicting retries reject password changes.
- 保留完整 Rust 门槛和真实 Server／CLI 消费者验证，独立审查固定 SHA，检查最新 base/head 与合并后 main CI。 / Preserve full Rust gates and real Server/CLI consumer checks, independent fixed-SHA review, latest base/head CI and post-merge main CI.

## 审查重点 / Review Focus

- pending task ID 与旧 HTTP 创建、并发重复输入发生冲突时，只能有一个归属。 / Pending IDs racing legacy HTTP creation or concurrent retries have one owner.
- 在 pending、Keychain 写入、DB 激活、任务删除和 provider 清理之间中断，重启后可恢复且保留失败证据。 / Interruptions between pending, Keychain write, DB activation, task removal and provider cleanup recover with failure evidence retained.
- 恶意编码、非法 UTF-8、userinfo、CR/LF/NUL 和错误版本不能污染存储或反射秘密。 / Hostile encoding, invalid UTF-8, userinfo, controls and unknown versions cannot pollute storage or reflect secrets.
- 同 ID 相同请求幂等，不同来源／目标／认证或密码冲突不修改已有任务／秘密。 / Identical ID/request retries are idempotent. Conflicting source, destination, authentication or password cannot mutate existing tasks/secrets.
- Keychain 锁定、拒绝访问、缺失、补偿失败不阻塞 HTTP 或 Ready，也不清理其他实例／任务项目。 / Locked, denied, missing and failed-compensation credentials preserve HTTP/Ready responsiveness and other instances/tasks.

### Task 1: 版本化创建、持久凭据与真实消费者 / Versioned creation, durable credentials and real consumers

**文件 / Files**

- Create `crates/protocol/src/transfer.rs` for the typed request, secret-redacted input and strict FTP parser.
- Create `crates/storage/src/origin.rs` for versioned secret-free associations, pending reservations and cleanup tombstones.
- Create `apps/server/src/origin_credentials.rs` for the provider boundary, macOS Keychain implementation and coordination.
- Modify `crates/protocol/src/lib.rs`, `crates/storage/src/lib.rs`, `apps/server/src/main.rs`, `apps/cli/src/main.rs` and their Cargo files only as required by wiring.
- Add focused unit/process tests next to these modules and `apps/server/tests/` using private directories. Update RFC 0004, Development Guide and Development Plan with actual scope and acceptance status.

**接口 / Interfaces**

- New method `task.create_transfer` takes `{version:1,id,destination,source:{kind:"ftp",source_url,security,path_mode,authentication}}`. `security` is `plain` or `required_explicit_tls`; `path_mode` defaults to `login_relative`, alternatively `absolute`; authentication is `{kind:"anonymous"}` or `{kind:"password",username,password}`. Reject unknown versions/fields and unsafe inputs with static errors. No implicit FTP dispatch follows creation.
- `CreateTransferRequest::parse(&serde_json::Value) -> Result<Self, RpcErrorObject>` validates Server input. A separate client normalization entry splits URL userinfo before constructing the wire value. `Debug` redacts typed secrets and generic `RpcRequest` params that may hold them.
- Storage exposes `OriginRecord`, exact credential references and pending/active/tombstone APIs on `SqliteRepository`. Migrations preserve old rows. Reserve pending IDs transactionally, atomically activate with task insert and atomically delete with tombstones. Legacy inserts honor reservations. Existing generic repository implementations retain compatibility through default methods where required.
- Provider trait has add-only `put`, exact-reference `get` and idempotent exact-reference `remove`, returning structured missing/locked/denied/unavailable errors. Production uses macOS Keychain; test providers remain test-only. Account scope includes data-instance ID, origin, credential ID, revision. Secret equality checks support retry conflict detection without persisting secret fingerprints.
- Server passes actual TLS transport assurance from line-RPC and HTTP ingress. Coordination validates transport before side effects, serializes origin mutations outside Core lock, persists pending, writes provider, atomically commits task+origin, then updates memory from a prepared candidate. Deletion uses the normal task-removal path with transactional tombstones and post-commit cleanup. Cleanup retries only exact persisted refs and handles stale startup snapshots safely.
- `nexum-cli [--server ADDR] task create-transfer` reads a bounded JSON request from stdin, normalizes/validates before sending, checks capabilities and TLS assurance before credential submission, and invokes only the typed method. No fallback to `task.create`. Anonymous input may use plaintext. Existing command syntax remains supported.
- Capabilities explicitly advertise request version, FTP metadata creation, macOS durable credential support and disabled FTP transfer. Get/list preserve TaskView shape and project canonical FTP source only after verifying the stored association matches the task.

- [ ] **Step 1 — 编写并运行失败用例 / Write and observe failing cases**

  Cover typed input parsing and secret sentinels, atomic DB boundaries and old-schema migration, idempotent/conflicting retry, legacy/pending ID collision, both ingress transport checks, cleanup after crash/delete/compensation failure and responsiveness while provider waits. Record each observed baseline failure before implementing its behavior. Use literal expected sources/path semantics and real SQLite. Use a precise provider double only for controlled failures.

- [ ] **Step 2 — 实现来源模型及 SQLite 协调 / Implement source model and SQLite coordination**

  Implement the interfaces above. Use schema-versioned independent origin records, a durable random instance identity and exact cleanup intents surviving task deletion. Keep old tasks and public TaskView fields compatible. Verify focused protocol/storage cases pass.

- [ ] **Step 3 — 实现 provider、Server 接线和 CLI / Implement provider, Server wiring and CLI**

  Add noninteractive macOS Keychain operations with safe structured errors. Keep provider calls outside Core lock and Ready. Recover abandoned pending records by cleaning their exact provider item before removing pending evidence. Retain failed cleanup records for retry. Verify actual TLS and plaintext ingress, notification semantics, concurrent HTTP operations and typed get/list after restart.

- [ ] **Step 4 — 运行完整门槛与实际消费者 / Run full gates and real consumers**

  Run `cargo fmt --all -- --check`, `cargo check --locked --workspace --all-targets`, `cargo test --locked --workspace -- --test-threads=1`, `cargo test --locked -p nexum-server --test managed`, `cargo clippy --locked --workspace --all-targets -- -D warnings`. Run actual Server plus actual CLI with a private TLS trust root and synthetic credentials, confirming Keychain/SQLite separation, retry, restart and removal. Scan stdout/stderr/DB/WAL/SHM for the sentinel. Record native/provider and other-platform gaps accurately.

- [ ] **Step 5 — 自审、文档和提交 / Self-review, document and commit**

  Read all touched call paths. Record observed commands, failures and acceptance in the external task report. Update bilingual durable docs with implemented/verified boundaries, retaining FTP/BT first-release requirements and unverified FTP transfer gates. Commit the cohesive implementation. Independent reviewers and controller then verify the fixed SHA, latest CI and authorized delivery.
