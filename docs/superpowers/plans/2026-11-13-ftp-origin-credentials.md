# FTP 来源凭据实施计划 / FTP Origin Credentials Implementation Plan

> 代理按步骤执行本计划，使用 `superpowers:executing-plans` 或 `superpowers:subagent-driven-development`，由控制者安排固定 SHA 的独立审查。
>
> Agentic workers execute this plan with `superpowers:executing-plans` or `superpowers:subagent-driven-development`. The controller arranges independent review at a fixed SHA.

**目标 / Goal** — 为真实 CLI 提供版本化 FTP 元数据创建，完成 Server 所有的 Keychain／SQLite 创建、幂等、重启清理和删除协调。 / Provide versioned FTP metadata creation through the real CLI with Server-owned Keychain/SQLite creation, idempotency, restart cleanup and removal coordination.

**架构 / Architecture** — 持久 pending → provider 原子新增 → 任务及关联原子激活。SQLite 保存无秘密来源和引用。 / Durable pending → atomic provider add → atomic task/association activation. SQLite stores secret-free sources and references.

**技术 / Tech Stack** — Rust 2024、serde／serde_json、rusqlite、rustls、getrandom、macOS Security.framework 和 Core Foundation。 / Rust 2024, serde/serde_json, rusqlite, rustls, getrandom, macOS Security.framework and Core Foundation.

**方案 / Spec** — [RFC 0004](../../RFC/0004-ftp-credentials-and-recovery.md)。基线 / Baseline `f8b89c6cee11ddadf4abecffb8745cfbee74052b`。

## 全局约束 / Global constraints

- 本包实现来源凭据协调。能力分别报告元数据创建、持久凭据和传输。F2／F3 验收后才启用 FTP 传输。 / Implement origin-credential coordination. Advertise metadata creation, durable credentials and transfers separately. Enable FTP transfers after F2/F3 acceptance.
- 兼容旧 `task.create` 的 HTTP(S)、TaskView 字段和 HTTP 下载。历史非 HTTP 来源保持脱敏，版本化关联须与任务和数据实例匹配才能公开规范来源。 / Preserve legacy HTTP(S) creation, TaskView fields and HTTP downloads. Historical non-HTTP sources stay redacted. Canonical projection requires an association matching the task and data instance.
- 密码只经验证 Server 身份的 TLS 提交，通过 stdin 输入。SQLite、任务、事件、Debug、错误和日志保持无秘密。 / Submit passwords through identity-verified TLS using stdin. Keep SQLite, tasks, events, Debug, errors and logs secret-free.
- 独立 Keychain 服务使用随机数据实例、origin、随机 credential ID 和 revision 组成精确 account。provider 原子新增禁止覆盖。 / Use a separate Keychain service with exact accounts binding random data instance, origin, random credential ID and revision. Atomic provider adds prevent replacement.
- provider 调用在 Core 锁外，恢复清理在 Ready 后。保留六秒托管启动预算和 HTTP 控制响应性。 / Call providers outside Core and recover cleanup after Ready. Preserve the six-second managed readiness budget and responsive HTTP controls.
- 只创建 revision 1。修改秘密和在途 revision 退休随后续传输实现。冲突重试拒绝改密。 / Create revision 1. Secret updates and in-flight revision retirement accompany later transfers. Conflicting retries reject secret changes.
- 运行完整 Rust 门槛及真实消费者，固定 SHA 审查，核对最新 base/head 和实际 main push CI。 / Run complete Rust gates and real consumers, review a fixed SHA and verify latest base/head plus actual main push CI.

## 审查重点 / Review focus

- pending ID、旧 HTTP 创建和并发重试只能有一个归属。 / Pending IDs, legacy HTTP creation and concurrent retries have one owner.
- 覆盖 pending、provider 写入、激活、删除和清理之间的中断，保留失败证据。 / Cover interruptions between pending, provider writes, activation, deletion and cleanup, retaining failure evidence.
- 非法编码、UTF-8、userinfo、控制字符、错误版本和未知字段返回固定错误。 / Return static errors for invalid encoding, UTF-8, userinfo, controls, versions and unknown fields.
- 同 ID 相同请求幂等，来源、目标、认证或密码变化均冲突。 / Identical ID/input retries are idempotent. Changed source, destination, authentication or password conflicts.
- provider 缺失、锁定、拒绝或补偿失败保持 HTTP／Ready 响应及实例隔离。 / Missing, locked, denied or failed-compensation credentials preserve HTTP/Ready responsiveness and instance isolation.

### Task 1 — 版本化创建、持久凭据与真实消费者 / Versioned creation, durable credentials and real consumers

**文件 / Files**

- `crates/protocol/src/transfer.rs` 实现严格类型输入及客户端规范化，`transfer_tests.rs` 验证路径和脱敏。 / Implement strict typed input and client normalization in `transfer.rs`, with path/redaction tests in `transfer_tests.rs`.
- `crates/storage/src/origin.rs` 实现独立版本化关联、pending、原子激活及 cleanup tombstone。 / Implement independent versioned associations, pending, atomic activation and cleanup tombstones in `origin.rs`.
- `apps/server/src/origin_credentials.rs` 协调创建／恢复／删除，`origin_keychain.rs` 实现生产 provider，`origin_credentials_tests.rs` 验证受控失败。 / Coordinate creation/recovery/removal in `origin_credentials.rs`, implement production storage in `origin_keychain.rs` and test controlled failures in `origin_credentials_tests.rs`.
- 接通 Server 两种入口与托管 Ready、CLI stdin，更新相关 Cargo 文件和双语文档。 / Wire both Server ingresses, managed Ready and CLI stdin, updating applicable Cargo files and bilingual documentation.
- 使用私有临时目录运行 storage、Server 进程和实际 CLI 测试。 / Use private temporary directories for storage, Server process and actual CLI tests.

**接口 / Interfaces**

- `task.create_transfer` 接收 `{version:1,id,destination,source:{kind:"ftp",source_url,security,path_mode,authentication}}`。`path_mode` 默认 `login_relative`，支持 `absolute`。认证为 `{kind:"anonymous"}` 或 `{kind:"password",username,password}`。识别并在副作用前拒绝尚未实现的 `required_explicit_tls`，当前接受 `plain`。 / `task.create_transfer` accepts the stated version-1 shape. `path_mode` defaults to `login_relative` and supports `absolute`. Authentication uses the stated anonymous/password shapes. Recognize and reject unavailable `required_explicit_tls` before side effects. Accept `plain`.
- `CreateTransferRequest::parse` 校验 Server 输入，`normalize_client` 先拆分 URL userinfo。typed secret 和通用 `RpcRequest` 的 Debug 均脱敏。 / Validate Server input with `CreateTransferRequest::parse` and split client URL userinfo with `normalize_client`. Redact typed secrets and generic `RpcRequest` Debug.
- `SqliteRepository` 以 schema 3 保存 `OriginRecord`、精确凭据引用、随机实例及 cleanup。pending 与旧插入共用事务互斥。激活与任务新增、删除与 tombstone 分别原子提交。 / Schema 3 stores `OriginRecord`, exact references, random instance identity and cleanup in `SqliteRepository`. Fence pending and legacy inserts transactionally. Atomically activate with task insertion and delete with tombstone creation.
- provider 的 `put` 为失败时不改变已有项目的原子新增，`get` 和幂等 `remove` 只访问精确引用。macOS 使用非交互 Keychain，其他平台拒绝持久认证输入。 / Provider `put` atomically adds without modifying existing items on failure. `get` and idempotent `remove` access exact references. Use noninteractive Keychain on macOS and reject durable authenticated input on other platforms.
- 以持久 `operation_id` 作为 provider 所有权标记，随 Keychain 新增原子写入 `kSecAttrGeneric`，读取／清理在单次查询同时匹配引用和 owner。 / Use durable `operation_id` as the provider ownership marker, atomically adding it as Keychain `kSecAttrGeneric` and matching both reference and owner in each read/cleanup query.
- TCP／HTTP 从实际 Transport 传入 TLS 状态。创建在 origin 互斥下持久 pending、锁外调用 provider、原子激活，再更新已准备的内存候选。删除保留 normal Core 删除语义，并在提交后清理。 / Derive TLS from actual TCP/HTTP Transport. Under the origin mutex, persist pending, call the provider outside Core, atomically activate and install the prepared memory candidate. Preserve normal Core removal and clean the provider after commit.
- abandoned pending 原子转为持久 tombstone 后清理。失败保留精确清理证据，启动恢复在获取互斥后读取当前记录，防止过期快照删除已激活项目。 / Atomically turn abandoned pending into durable tombstones before cleanup. Retain exact evidence on failure and read current records after acquiring the recovery mutex, preventing stale snapshots from deleting activated items.
- `nexum-cli [--server ADDR] task create-transfer` 从 stdin 有界读取，先校验输入，再查询 capabilities，仅调用类型化方法。来源密码必须使用通过身份校验的 TLS。 / Read bounded stdin with the stated CLI command, validate input, query capabilities and invoke the typed method. Origin passwords require identity-verified TLS.
- capabilities 分别报告 v1、FTP 元数据创建、macOS 持久凭据、`transfer:false`。get/list 保持 TaskView 形状。FTP `task.queue`／`task.resume` 显式拒绝，任务保持 Created。 / Capabilities report v1, FTP metadata creation, macOS durable credentials and `transfer:false`. Preserve TaskView shape. Explicitly reject FTP `task.queue`/`task.resume`, keeping metadata tasks Created.

- [x] **Step 1 — 失败用例与证据 / Failing cases and evidence**

  记录严格输入、真实入口、实际 CLI、路径绕过、重复 provider 新增及 HTTP 删除等待的失败，再验证修复。使用真实 SQLite 和精确故障 double。初次测试 helper 的编译错误单独记录。 / Record strict input, real ingress, actual CLI, path bypass, duplicate provider add and blocked HTTP removal failures before verifying fixes. Use real SQLite and precise fault doubles. Record initial helper compilation mistakes separately.

- [x] **Step 2 — 来源模型和 SQLite / Source model and SQLite**

  实现实例隔离、pending 归属、激活／删除原子性、重开和失败回滚。schema 断言随 v3 更新，保留旧行和错误字段迁移覆盖。 / Implement instance isolation, pending ownership, atomic activation/removal, reopen and failure rollback. Update schema assertions for v3 while retaining old-row/error migration coverage.

- [x] **Step 3 — Provider、Server 和 CLI / Provider, Server and CLI**

  实现非交互 add-only Keychain、锁外 provider、Ready 后恢复、两种实际 TLS 入口、可信投影、无 HTTP sidecar 的 typed 删除，以及 stdin 和 capabilities。 / Implement noninteractive add-only Keychain, provider calls outside Core, post-Ready recovery, both actual TLS ingresses, trusted projection, typed removal independent of HTTP sidecars, stdin and capabilities.

- [x] **Step 4 — 完整门槛与真实消费者 / Complete gates and real consumers**

  执行 Development Guide 的五项 Rust 门槛。控制者对固定提交运行实际 CLI／随包 Server／Keychain、私有 TLS 根、重启、删除、实例隔离与秘密扫描，并执行原生候选验收。平台缺口按实际结果记录。 / Run the Development Guide's five Rust gates. At the fixed commit, the controller exercises actual CLI/bundled Server/Keychain with private TLS trust, restart, removal, instance isolation and secret scanning, plus native candidate acceptance. Record actual platform gaps.

- [x] **Step 5 — 自审、文档和提交 / Self-review, documentation and commit**

  自审调用链、更新双语文档并提交本包。控制者随后安排独立审查、最新 CI 和已授权交付。实现报告位于 `.superpowers/sdd/2026-11-13-ftp-origin-credentials/task-1-report.md`。 / Review call paths, update bilingual docs and commit this package. The controller then arranges independent review, latest CI and authorized delivery. The implementation report is at the stated path.

2026-11-14 在代码提交 `0212d4c` 完成完整 Rust 门槛（workspace 530、额外 managed 14）、实际随包 Server／CLI／Keychain 29 场景及原生 App 受控重开验证。原生观察涵盖 TLS 1.3 就绪、HTTP 完整文件、已有元数据、异常退出与 Server EOF，使用私有预置配置。界面交互、正常菜单退出、干净 OS 用户和 VoiceOver 仍按各自原生验收记录处理。固定 SHA 审查、最新 CI 与合并状态见 [PR #91](https://github.com/liveait/Nexum/pull/91)。

On 2026-11-14, code commit `0212d4c` passes the full Rust gates (workspace 530 and additional managed 14), 29 actual bundled Server/CLI/Keychain scenarios and controlled native App relaunch checks. Native observations cover TLS 1.3 readiness, complete HTTP bytes, existing metadata, abnormal App exit and Server EOF using a private seeded profile. UI interaction, normal menu quit, a clean OS user and VoiceOver retain their respective native acceptance records. See [PR #91](https://github.com/liveait/Nexum/pull/91) for fixed-SHA review, current CI and merge status.
