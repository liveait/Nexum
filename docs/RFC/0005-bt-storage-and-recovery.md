# RFC 0005：BT 种子与 Magnet 的存储和恢复 / Torrent and Magnet storage and recovery

- 状态：提案；依赖、存储屏障和发布机制分别待验。日期：2026-11-07；基线：`503a3e79d89b8937b1373fc668209ec9fca43bef`（PR #84）。
  Status: proposed; dependency, storage barriers and publication mechanisms require separate verification. Date: 2026-11-07; baseline: `503a3e79d89b8937b1373fc668209ec9fca43bef` (PR #84).

## 目标与范围 / Goal and scope

首发支持 BitTorrent v1 单／多文件 `.torrent` 与 `btih` Magnet、文件选择、分片验证、进度、暂停／取消和重启恢复。40 位十六进制／32 位 base32 的 btih 规范化为同一 20 字节值，拒绝畸形、冲突的多个 xt 或与下载 metainfo 不符的 infohash。hybrid 仅在 v1 文件映射和 padding fixture 通过后列入兼容；v2-only 明确返回不支持，不按后缀或 parser 接受就声称可以下载。

The first release supports BitTorrent v1 single/multi-file `.torrent` and `btih` Magnet, file selection, piece verification, progress, pause/cancel and restart recovery. Normalize 40-digit hex/32-character base32 btih to the same 20-byte value. Reject malformed/conflicting xt values and metainfo with a mismatched infohash. Claim hybrid compatibility only after v1 mapping/padding fixtures pass. Explicitly reject v2-only rather than inferring download support from parser acceptance.

本 RFC 提出时，`MagnetResolver` 只检查 btih 参数存在，Local 只代表 Server 文件系统中的路径；两者当时可经旧 Server 创建任务但无 BT 引擎。后续旧入口已限制为 HTTP(S)，本次严格身份解析见下文；网络传输派发仍限 HTTP(S)，请求仍为单文件，`active_destinations` 只防精确路径冲突。尚无 BT 运行时或 M6 验收结论。保留 [RFC 0003](0003-registry-transfer-sessions.md) 和 [ADR 0007](../decisions/0007-http-completion-reconciliation.md) 的兼容、控制和原子完成语义。

When this RFC was proposed, Magnet resolution checked only for parameter presence, and Local denoted a Server filesystem path; both could then create legacy tasks without a BT engine. Legacy creation is now HTTP(S)-only, with strict identity parsing detailed below. Network dispatch remains HTTP(S), requests remain single-file and `active_destinations` detects exact-path conflicts only. BT runtime/M6 acceptance remains open. Preserve RFC 0003 and ADR 0007 compatibility, controls and atomic completion semantics.

## 固定候选与不可直接继承的保证 / Pinned candidate and guarantees to verify

先验证嵌入式 `librqbit = 9.1.0`，关闭默认特性，启用 `rust-tls`；上游 tag 对应提交 `4309d38769ef93f6ee4f01a1ab147009a197fa8f`，库为 Apache-2.0／edition 2024。公开库只作内部实现依赖，不向 RPC 暴露其 ID、类型或 HTTP API。依赖包检查锁文件、传递许可证、MSRV、macOS 构建及原生包增长；现有 HTTP 候选大小不能当作多协议产物大小。[固定清单](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/Cargo.toml)

Evaluate embedded `librqbit = 9.1.0` with defaults disabled and `rust-tls` enabled. Its upstream tag is commit `4309d38769ef93f6ee4f01a1ab147009a197fa8f`; the library uses Apache-2.0 and edition 2024. Keep it internal, without exposing library IDs/types/HTTP APIs through RPC. Check lockfiles, transitive licenses, MSRV, macOS builds and native bundle growth. The current HTTP candidate size is not the multi-protocol artifact size. [Pinned manifest](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/Cargo.toml)

固定版本提供自定义 `StorageFactory`／`TorrentStorage`；Nexum 不采用默认按路径覆盖／删除的 filesystem 存储。上游 JSON persistence 限制为 filesystem 存储，因此初版关闭上游 persistence／fastresume，使用下述自有记录及重校验。`on_piece_completed` 的错误不能单独充当持久化成功屏障，失败必须经自有 fatal-error 通道阻止发布。[存储接口](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/storage/mod.rs) · [JSON persistence](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/session_persistence/json.rs) · [分片处理](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/torrent_state/live/mod.rs)

The pinned release supports custom `StorageFactory`/`TorrentStorage`. Nexum will not use default path-based overwrite/deletion storage. Upstream JSON persistence is restricted to filesystem storage, so initially disable upstream persistence/fastresume and use Nexum records plus rechecking. A failed `on_piece_completed` callback cannot establish durable success; an independent fatal-error channel must prevent publication. [Storage API](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/storage/mod.rs) · [Persistence](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/session_persistence/json.rs)

## 导入与兼容 RPC / Import and compatible RPC

新增可取消的 `torrent.inspect.begin/status/cancel`；begin 接收 base64 种子内容或 Magnet，立即返回 Server 生成的随机 inspection ID，status 返回阶段／错误及解析完成的 infohash、稳定文件索引、相对路径和大小。inspect 不创建任务或发布输出。有效期、权限和资源绑定 Server 数据实例及认证主体；同一 bearer 的客户端共享该主体。切换 Server 立即丢弃客户端 inspection 上下文。远程导入上传内容，不把客户端路径当成 Server 本地文件。

Add cancellable `torrent.inspect.begin/status/cancel`. Begin accepts base64 metainfo or Magnet and promptly returns a random Server-generated inspection ID. Status returns phase/error and resolved infohash, stable file indices, relative paths and sizes. Inspection creates neither a task nor published output. Scope lifetime, permissions and resources to the Server data instance and authenticated principal; clients using the same bearer share that principal. Discard client inspection context on Server changes. Remote import uploads contents rather than treating client paths as Server paths.

初始硬上限：原始 metainfo 512 KiB、Magnet 8 KiB、文件 4096、分片 25000、路径深度 32、每组件 UTF-8 255 字节；整数计算检查溢出。原始种子 base64 后约 683 KiB，请求含全部开销须满足现有 HTTP 桥接的 1 MiB body 上限。新 Server 的公共 TCP 读帧层须在 JSON 解析及方法识别之前，对所有请求按字节施加 1 MiB 上限（不含行结束符）；禁止先无界 `read_line` 再按方法检查。超限以有界错误响应拒绝并关闭连接，不回显原始内容，也不无界排空。这也限制旧方法的超大 TCP 帧，是有意的兼容性收紧；正常旧 HTTP 调用保持契约，旧 TCP 尚无此保护。不得拆请求绕过上限。每 Server 最多 2 个活动 inspection、16 个未消费结果；解析预算 5 秒，Magnet 元数据期限 60 秒，结果 10 分钟过期。取消、超时和过期均停止并 join 所属工作。新阈值调整需同步文档及边界测试。

Initial hard limits: raw metainfo 512 KiB, Magnet 8 KiB, 4096 files, 25000 pieces, path depth 32 and 255 UTF-8 bytes per component, with checked integer arithmetic. Base64 metainfo is about 683 KiB; the full request must fit the HTTP bridge's existing 1 MiB body cap. The new Server's shared TCP frame reader must impose a 1 MiB byte limit on every request, excluding line endings, before JSON parsing or method detection; never call unbounded `read_line` before checking the method. Reject oversized input with a bounded error response and close the connection, without echoing raw input or draining an unbounded stream. This intentionally tightens compatibility for oversized legacy TCP frames; normal legacy HTTP calls retain their contract, and legacy TCP does not already provide this protection. Do not bypass limits through split requests. Limit each Server to two active inspections and 16 unconsumed results; use five seconds for parsing, 60 seconds for Magnet metadata and ten-minute result expiry. Cancellation/timeout/expiry stops and joins owned work. Update docs and boundary tests when changing limits.

入口前置实现记录（2026-11-09，基线 `550f9418`）：本实现包接入公共有界读帧及 HTTP header 读取，覆盖 LF／CRLF／EOF、分片到达、多字节、认证前超限、无派发副作用与 plain／TLS／HTTP 消费者。固定 SHA 的独立审查、完整检查及合并前后 CI 记录在对应 PR。此包不实现 inspection、metainfo 解析、凭据或 BT 传输；以上旧 TCP 基线描述保留为提案时的事实。

Ingress prerequisite implementation (2026-11-09, base `550f9418`): this slice wires common bounded framing and HTTP header reads, covering LF/CRLF/EOF, fragmented input, multibyte data, pre-authentication rejection, absence of dispatch side effects and plain/TLS/HTTP consumers. The corresponding PR records fixed-SHA independent review, full checks and CI before/after merge. Inspection, metainfo parsing, credentials and BT transfers remain unimplemented; the legacy TCP description above records the proposal-time baseline.

采用 [RFC 0004](0004-ftp-credentials-and-recovery.md) 的版本化 `task.create_transfer`，BT 参数引用 inspection ID、所选文件索引及专属目标目录。空、重复、越界或 padding-only 选择拒绝；一次事务保存任务、受限原始 metainfo、来源摘要、映射和选择后才返回成功。任务 ID 保证重试幂等，inspection 结果保留到期以便重试确认；同 ID 不同绑定返回冲突。新能力协商防止旧 Server 静默忽略参数；旧 HTTP `task.create` 和 TaskView 保持原样。HTTP URL 即使以 `.torrent` 结尾也继续作为普通下载，BT 导入必须显式选择。

Use RFC 0004's versioned `task.create_transfer`, referencing inspection ID, selected indices and a dedicated destination directory. Reject empty, duplicate, out-of-range or padding-only selection. Transactionally persist the task, bounded raw metainfo, source digest, mapping and selection before success. Task ID makes retries idempotent; inspection results remain until expiry for confirmation retries. Conflicting bindings fail. Negotiate capabilities to avoid silently ignored parameters on old Servers; retain legacy HTTP creation and TaskView. HTTP URLs ending in `.torrent` remain ordinary downloads unless explicitly imported as BT.

B1 在调用库前做有界 bencode 验证：嵌套深度最多 64、节点最多 65536、单字符串及累计字节受 512 KiB 总输入限制，拒绝重复／未排序字典键、非规范整数／长度、尾随数据和溢出。v1 infohash 取原始输入中 info 字典的确切编码字节区间做 SHA-1，不通过解码后重编码计算。解析循环至少每 1024 节点及处理大字符串块时检查期限／取消；`timeout(spawn_blocking)` 不能停止 CPU 任务，不可用来证明预算／退出。若候选库无法配合，先用受控验证器或维护补丁；超时仍须等所属 worker 退出再释放 inspection 资源。

Before invoking the library, B1 validates bounded bencode: depth at most 64, at most 65536 nodes, and individual/cumulative string bytes within the 512 KiB total-input cap. Reject duplicate/unsorted dictionary keys, noncanonical integers/lengths, trailing data and overflow. Compute v1 infohash from SHA-1 of the exact original encoded info-dictionary byte range, never decoded/re-encoded data. Check deadline/cancellation at least every 1024 nodes and during large string blocks. `timeout(spawn_blocking)` cannot stop CPU work or prove its budget/exit. Use a controlled validator or maintained patch if the candidate cannot cooperate; wait for worker exit before releasing timed-out inspection resources.

<a id="strict-magnet-identity"></a>

### 严格 Magnet 身份解析切片 / Strict Magnet identity-parsing slice

基线 `c247f5bc68cefdccfbaa2ed0b9068b36bfa4bd06`。新增纯解析 `magnet::parse_magnet(&str) -> Result<MagnetInfo, MagnetError>`，仅返回私有 20 字节 btih 及规范十六进制访问器，不保存原 URI、Tracker、显示名或选择范围。40 位 hex／32 位 base32 归一到相同值；同值重复 xt 合并，畸形或冲突 xt 拒绝。遵循既有 hybrid 验收门槛，本切片明确拒绝任何 btmh，包括混合主题，不丢弃另一个身份后宣称兼容。协议字段依据 [BEP 9](https://www.bittorrent.org/beps/bep_0009.html)，具体拒绝策略为本切片的保守实施选择。

At baseline `c247f5bc68cefdccfbaa2ed0b9068b36bfa4bd06`, add pure parsing with the signature above. `MagnetInfo` retains only a private 20-byte btih and canonical hex accessor, without URI, trackers, display name or selection ranges. Normalize hex/base32 and coalesce equal repeated xt values; reject malformed/conflicting topics. Under the existing hybrid gate, this slice rejects any btmh, including mixed topics, rather than silently dropping an identity and claiming compatibility. Fields follow [BEP 9](https://www.bittorrent.org/beps/bep_0009.html); rejection policy is this slice's conservative implementation choice.

原输入先按 UTF-8 字节检查 8 KiB 及控制字符，再允许两端空白裁剪；ResolveRequest 同样在 trim 前检查，HTTP 行为不变。接受大小写无关的 magnet／URN namespace 及 hash 字母，query key 在一次解码后大小写敏感，仅小写 xt 提供身份。先按 &／首个 = 分段，再严格解码一次 %HH；+ 保持字面值。每个参数（包括忽略项）均校验 UTF-8、控制字符、非空键及合法键字符。拒绝 fragment、authority／path、内部原始空白、空／无键分段与 xt.* 索引主题；其他参数只校验后忽略，不授权网络或文件选择。允许非空白／控制的原始 Unicode 参数文本及编码 UTF-8，保留显示名兼容；ASCII 原文遵循 query 字符语法。[RFC 3986](https://www.rfc-editor.org/rfc/rfc3986.html#section-2.4) · [RFC 4648](https://www.rfc-editor.org/rfc/rfc4648.html#section-6)

Check the original UTF-8 byte length and controls before outer trimming; ResolveRequest enforces the same guard before trim, retaining HTTP behavior. Scheme/URN namespace/hash letters are ASCII-insensitive, while decoded query keys are case-sensitive and only lowercase xt supplies identity. Split on & and the first = before exactly one strict %HH decode; preserve literal +. Validate every parameter, including ignored ones, for UTF-8, controls, nonempty keys and allowed key characters. Reject fragments, authority/path, internal raw whitespace, empty/keyless segments and indexed xt.* topics. Other parameters are validated then ignored, without network/selection authorization. Permit nonwhitespace/noncontrol raw Unicode text and encoded UTF-8; raw ASCII uses query character syntax. [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986.html#section-2.4) · [RFC 4648](https://www.rfc-editor.org/rfc/rfc4648.html#section-6)

入口及 Debug 的识别对 scheme 中的空白／控制字符作保守分类，仅用于拒绝／脱敏；真实 parser／kind 不接受这种变形。原文控制字符仍在 trim 前拒绝，覆盖前置 NUL 及 scheme 内插入控制字符。

Ingress/Debug classify whitespace/control-obfuscated schemes conservatively for rejection/redaction only. Actual parsing/kind never accepts that spelling. Original controls are still rejected before trim, including leading NUL and controls embedded in the scheme.

MagnetResolver 复用该 parser，保留 ResolveResult 字段；错误只含静态分类，Request／Result 的 Debug 对 Magnet 脱敏。显式 source 访问器仍返回原输入，不被当作公开 TaskView 脱敏器。Registry／Core 直接消费者验证规范值、拒绝及无落库／事件副作用；旧 RPC 继续拒绝有效 Magnet，旧数据库 Magnet 保持不派发。该包不新增依赖、RPC capability、Peer／Tracker、元数据获取或 BT 下载，不关闭 B1／M6。

MagnetResolver reuses the parser without changing ResolveResult fields. Errors contain static categories, and Request/Result Debug redact Magnet sources. Explicit source access remains raw and is not a public TaskView sanitizer. Registry/Core consumers verify normalization, rejection and no persistence/event side effects. Legacy RPC still rejects valid Magnet inputs, and recovered Magnet tasks remain undispatched. Add no dependency, capability, peer/tracker, metadata fetch or BT transfer; keep B1/M6 open.

实施顺序 / Execution order:

1. 先记录旧解析器错误接受短哈希的失败测试，再新增有界值类型和严格 codec。 / Record a failing short-hash acceptance regression, then implement bounded values/codecs.
2. 接入 Registry／Core，覆盖原始字节上限、单次解码、重复／冲突／索引主题及 Debug／错误脱敏。 / Wire Registry/Core and cover raw bounds, one decode, repeated/conflicting/indexed topics and Debug/error privacy.
3. 验证有效 Magnet 仍受旧 RPC／派发门槛约束，更新双语状态。 / Verify valid Magnet remains gated at RPC/dispatch and update bilingual status.
4. 固定 SHA 独立审查及实际消费者验证；完整 Rust 门槛、最新 base/head 与实际 main push CI 全通过后交付。 / Freeze/review the SHA, validate real consumers, run full Rust gates and verify latest-base/head plus actual-main CI before delivery.

旧入口保护沿用 RFC 0004：HTTP 调用保留，新非 HTTP 创建必须走类型化方法；旧 Magnet／Local 记录不能因新引擎注册而自动开始下载。任务领取前同时验证持久 metainfo／来源摘要／选择，缺少绑定即拒绝并要求显式重导入。

Retain RFC 0004's legacy-entry guard: preserve HTTP calls, require typed creation for new non-HTTP tasks and do not auto-start old Magnet/Local records when registering an engine. Validate durable metainfo/source digest/selection before claims; missing bindings fail with explicit reimport required.

新增只读 `task.transfer` 返回协议阶段（metadata／checking／transferring／publishing）、文件选择、可信进度、Tracker 状态和上传统计，不把库内部 initializing 扫描量当作下载量。TaskView 继续使用原任务状态；阶段不另起会与 Scheduler 冲突的状态机。新 RPC 方法、数据版本及错误代码在首个协议实现包定义并直接测试，本文的方法为实施目标而非当前 API。

Add read-only `task.transfer` for protocol phase (metadata/checking/transferring/publishing), selection, trusted progress, tracker status and upload statistics. Library initialization scanning is not downloaded data. TaskView retains existing task states; phases must not create a conflicting Scheduler state machine. Define/test new RPC methods, data versions and error codes in the first protocol slice; these are target APIs rather than existing ones.

原始 Magnet、metainfo 和 tracker URL 可能包含 passkey，保存在 Server 私有协议记录，不进入公共 TaskView／通知／日志；对外仅展示脱敏地址与摘要。数据库备份仍可能包含这些访问数据，不能声称其已加密；密码型 origin 凭据继续采用 RFC 0004 的 provider，不嵌入公共来源 URL。依赖的调试日志也须通过哨兵 passkey 测试。

Raw Magnet, metainfo and tracker URLs can contain passkeys. Keep them in private Server protocol records and out of public TaskView/notifications/logs; expose redacted addresses and digests only. Database backups can still contain this access data: no encryption claim is made. Password-style origin credentials use RFC 0004's provider rather than public source URLs. Test dependency debug logs with sentinel passkeys too.

私有种子严格限制为当前私有 Tracker 返回的 peer，元数据前已有的其他连接必须断开；同一时间只使用一个私有 Tracker，故障切换时断开旧 peer 后重新获取。手动 peer 或先前 DHT 结果不能继续绕过此规则。库默认是否满足这些条件需 fixture 证明，不能仅关闭 DHT 就宣称符合 BEP 27。

For private torrents, retain only peers returned by the current private tracker and disconnect other pre-metadata connections. Use one private tracker at a time; on failover disconnect old peers before obtaining new ones. Manual peers/prior DHT results cannot bypass this rule. Verify library behavior with fixtures rather than claiming BEP 27 compliance from disabling DHT alone.

<a id="bounded-magnet-peer-inspection"></a>
## 有界单节点元数据检查切片 / Bounded single-peer metadata inspection slice

实现基线 `5b321663496b0e04e60534ddd16424c43bef0e02`。本轮实施选择为独立的 BEP 9 元数据接收器，复用现有 Inspector，而不直接接入完整 BT 库。固定候选的私有 reader 在 Nexum 校验前允许分配 32 MiB，并 trace 完整 info；公共接口不能配置 512 KiB 上限。为这个只读切片引入整库维护分叉会扩大范围。完整 BT 仍保留上述库候选、存储与生命周期验收；这不是完整 BT 引擎选型结论。[BEP 9](https://www.bittorrent.org/beps/bep_0009.html) · [BEP 10](https://www.bittorrent.org/beps/bep_0010.html)

Implementation base: `5b321663496b0e04e60534ddd16424c43bef0e02`. This slice chooses a separate BEP 9 metadata receiver through the existing Inspector rather than directly embedding the full BT library. The pinned candidate's private reader allocates up to 32 MiB and traces full info before Nexum validation; its public API cannot configure the 512 KiB cap. Maintaining a full-library fork for this read-only slice would widen scope. Retain the full-BT candidate and its storage/lifecycle gates; this does not select a complete BT engine. [BEP 9](https://www.bittorrent.org/beps/bep_0009.html) · [BEP 10](https://www.bittorrent.org/beps/bep_0010.html)

`torrent.inspect.begin` 增加严格二字段输入 `{magnet, peer}`，peer 是一个显式 IP:port（IPv6 带方括号），拒绝端口 0／未指定地址／多播和额外字段。Magnet 仍先经严格身份 parser；tr、x.pe、dn、so 等不会授权网络或选择。只连接该 peer，不解析 DNS、不访问 Tracker／DHT／PEX／LSD，不监听、不上传数据片、不创建任务或数据文件。能力 source 为 `magnet_peer`，不声称普通无 peer 的 Magnet 已可用。CLI 为 `torrent inspect-magnet --stdin PEER` 及异步 begin 变体，原文仅从有界 stdin 读取；远程模式沿用现有敏感检查请求的 TLS 门槛。

Extend `torrent.inspect.begin` with exactly `{magnet, peer}`. Peer is one explicit IP:port (bracketed IPv6); reject port zero, unspecified/multicast addresses and extra fields. Strict Magnet identity parsing remains first; tr/x.pe/dn/so grant no networking or selection authority. Connect only to that peer: no DNS, tracker/DHT/PEX/LSD, listener, payload upload, tasks or data files. Advertise source `magnet_peer`, not general peerless Magnet availability. CLI uses `torrent inspect-magnet --stdin PEER` and an asynchronous begin variant; read the raw URI only from bounded stdin and retain the existing TLS gate for sensitive remote inspection.

接收器位于 `resolver::torrent::metadata`，消费 `MagnetInfo`、`SocketAddr` 和独立随机的 20 字节 peer ID（不得复用 inspection ID）；现有 Inspector worker 独占同步 socket，取消／过期／关闭保持登记直至真正 join。总期限 60 秒，不因小片段重置；连接和读写每次至多 100 ms 控制窗口。BEP 10 的双方扩展 ID 分开处理，后续扩展握手忽略；未知消息在预算内忽略。每帧最多 20 KiB，bencode header 最多 1 KiB，总计最多 256 帧；metadata 每块 16 KiB，单个在途请求，要求索引、total_size 和精确块长一致。先检查声明大小再分配，info 上限为 512 KiB 减 8 字节封装；随后调用既有严格 metainfo 检查（至多 5 秒且不超过总期限），验证原始 infohash。所有失败仅静态代码，无 URI／peer 消息摘录。

The receiver lives in `resolver::torrent::metadata`, consuming `MagnetInfo`, `SocketAddr` and an independent random 20-byte peer ID (never reuse an inspection ID). An existing Inspector worker exclusively owns the synchronous socket; cancellation/expiry/shutdown retain registration until actual join. Use an absolute 60-second total deadline, never reset by fragments, with at most 100 ms per connect/read/write control window. Separate both BEP 10 extension IDs, ignore subsequent extension handshakes and ignore unknown messages within budgets. Cap frames at 20 KiB, bencode headers at 1 KiB and total frames at 256. Request one 16 KiB metadata block at a time; require matching index/total_size/exact length. Check advertised size before allocation; info is capped at 512 KiB minus its eight-byte envelope. Then run the existing strict inspector within five seconds and the total deadline, validating the original infohash. Errors contain static codes, never URI or peer-message excerpts.

BEP 9 只返回原始 info 字典；私有保存的 metainfo 是 `d4:info<原始 info>e`。因此 `metainfo_sha256` 绑定该构造封装，而不是一个未收到的原始 .torrent 文件。文件摘要、版本、路径、padding 和计数等仍由同一检查器决定。metadata 阶段没有下载进度。B1／M6、Tracker 发现、BT 下载及文件发布保持开放。

BEP 9 returns only the original info dictionary. Privately retain metainfo as `d4:info<original info>e`; `metainfo_sha256` therefore binds that constructed envelope, not an unreceived original .torrent file. The same inspector governs hashes, versions, paths, padding and counts. Metadata has no payload-download progress. B1/M6, tracker discovery, BT transfers and publication remain open.

实施计划 / Implementation plan:

1. 先以真实 peer 记录旧 Server 拒绝新输入的 red；新增直接 receiver 测试及实现，覆盖握手／块／大小／哈希／控制边界。 / Record old Server rejection of the new input, then implement the receiver with direct real-peer handshake/block/size/hash/control tests.
2. 接入现有 Inspector、能力和 CLI，保留旧导入／配额／权限／退出屏障；验证真实 Server 与 CLI，不按 fixture 地址绕过。 / Wire Inspector, capabilities and CLI, preserving imports/quotas/principals/joins; validate actual Server/CLI without fixture-address bypasses.
3. 固定 SHA 独立审查及受控消费者覆盖成功、失败、取消、退出、资源释放、无任务／文件／发现副作用和诊断脱敏。 / Freeze/review the SHA and independently verify success/failure/cancel/exit/resource release, absence of tasks/files/discovery and redacted diagnostics.
4. 运行完整 Rust 门槛及最新 base/head CI，合并后验证实际 main push CI；未通过前仅记实现／待验收。 / Run full Rust gates and latest-base/head CI, then verify actual main push CI; retain implemented/pending status until accepted.

## 文件树、选择与占用 / File trees, selection and reservations

在创建任何数据文件前验证完整 metainfo 文件映射：拒绝绝对路径、空组件、`.`／`..`、NUL、分隔符混淆、链接／特殊类型、重复、文件与目录前缀冲突，以及目标文件系统的大小写／Unicode 别名冲突。首版文件名采用有效 UTF-8，不能用 lossy 转换把不同名字合并。不可信 torrent 名不直接决定目标目录；用户／安全分配器选择一个尚不存在的新目录。

Validate the complete metainfo file map before creating data files: reject absolute/empty/dot/parent/NUL/ambiguous-separator components, links/special types, duplicates, file-directory prefix conflicts and case/Unicode aliases on the destination filesystem. Initially require valid UTF-8 rather than merging names through lossy conversion. An untrusted torrent name does not determine the destination directory; the user/safe allocator chooses a new, absent directory.

自有 storage 持有经验证的目录与文件对象，用受限相对解析和定位 I/O；写入授权绑定 task／attempt generation／文件映射，暂停／终结前关闭新授权并等待已进入 I/O 的操作退出。初始化、长度设置、读、写、flush、删除和 callback 失败全部向 wrapper 报告，不能吞掉。处理跨文件分片时需要的未选字节只存内部区，永不发布为用户文件；padding 是虚拟零，不创建对应路径。

Custom storage holds verified directory/file objects and uses constrained relative resolution and positional I/O. Bind write grants to task/attempt generation/file mapping, close new grants and drain in-flight I/O before pause/termination. Propagate initialization, length, read/write, flush, deletion and callback failures to the wrapper. Store unselected bytes needed by boundary pieces internally, never as published user files. Padding is virtual zero data without filesystem entries.

目标占用扩展为文件／目录树冲突：同一、祖先、后代、别名均互斥，HTTP／FTP 单文件和 BT 共用规则。Core 锁内原子领取 Scheduler 槽位并预留目录对象；拒绝重叠，不在扫描目录时持锁。终态屏障、完成保存或可诊断失败处理后才释放预留。每个活动 task／attempt 先采用独立库 Session；不能把上游 `AlreadyManaged` 句柄隐式共享给多个 Nexum 任务。

Reservations must detect equal, ancestor, descendant and alias conflicts across HTTP/FTP files and BT trees. Atomically claim Scheduler capacity and directory-object reservations under the Core lock without scanning under that lock. Release after the terminal barrier and completion persistence or explicit failure handling. Initially isolate library Sessions per active task/attempt; never implicitly share an upstream `AlreadyManaged` handle across Nexum tasks.

## 生命周期与网络 / Lifecycle and networking

连接、Magnet metadata、检查和 peer 执行均在受控 worker 内，句柄创建后即可取消。上游 `Session::stop` 中的固定等待不是全部任务 join 的证明；wrapper 必须拥有解析、网络、定时器和阻塞 I/O 的停止／等待链。可用每次尝试专属 runtime／owner thread 验证，但 runtime 超时 shutdown 不能被当作已停止。缺少可证明的屏障就需要维护补丁或替代库，产品接线保持禁用。[Session 源码](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/session.rs)

Connect, Magnet metadata, checking and peers run in controlled workers, cancellable as soon as a handle exists. A fixed wait in upstream `Session::stop` is not proof that all tasks joined. The wrapper must own shutdown/wait chains for resolution, networking, timers and blocking I/O. A per-attempt runtime/owner thread is a candidate, but timed-out runtime shutdown cannot mean quiescence. Missing proof requires a maintained patch or alternative library, with product wiring disabled. [Session source](https://github.com/ikatson/rqbit/blob/4309d38769ef93f6ee4f01a1ab147009a197fa8f/crates/librqbit/src/session.rs)

暂停完成后文件、下载／上传计数及持久进度必须稳定；先停 I/O、快照并持久化，再返回 RPC。恢复重新取得槽位和 generation 写授权。取消、删除与 Server 退出先关授权、停止 peer 并 join，再清理／释放目标；超时保留登记并报告，不能启动第二个 writer。首发只在下载期间共享已验证片，采用有限上传预算；完成选中文件后停止上传再提交，Completed 后不继续做种。UI 展示上传行为与统计，长期做种／分享率单独设计。

After pause, files, download/upload counters and durable progress must be stable. Quiesce I/O, snapshot/persist, then reply. Resume reacquires capacity and generation grants. Cancellation/removal/Server exit revoke grants, stop/join peers, then clean/release destinations. A timeout retains registrations and reports failure rather than starting a second writer. Initially share verified pieces only while downloading with a finite upload budget; stop uploads before publishing selected completion and do not seed after Completed. Show upload behavior/statistics in the UI. Persistent seeding/ratio policy is separate.

首发使用 metainfo／Magnet 声明的 Tracker；private 种子禁止 DHT／PEX／局域发现。Magnet 元数据未知时默认仅用声明的 Tracker／显式 peer，用户可明确允许公开 DHT 发现以解析无 Tracker Magnet；该选择可能在得知 private 标志前泄漏 infohash，不能事后声称已阻止。取得 private 标志后立即停止公共发现。默认不自动端口映射、不启用局域发现、不追加公共 Tracker 清单。监听、Tracker／DHT／peer 并发与超时有上限，创建 Session 时显式设置，不继承库默认。受控试验全部只连接明确 loopback fixture，不能把 fixture 地址作为跳过产品代码的条件。

Use declared trackers and disable DHT/PEX/local discovery for private torrents. Before Magnet metadata is known, default to declared trackers/explicit peers. Users may explicitly allow public DHT discovery for trackerless Magnet; this can disclose the infohash before learning its private flag and cannot later be described as prevented. Stop public discovery when private metadata arrives. Disable automatic port mapping/local discovery and do not append public tracker lists by default. Explicitly bound listeners, tracker/DHT/peer concurrency and deadlines rather than inheriting defaults. Controlled probes use explicit loopback fixtures without fixture-address bypasses in product code.

## 持久化、进度与恢复 / Persistence, progress and recovery

新增版本化 `bt_state` 表与 task 外键，任务创建／删除和协议记录采用同一 SQLite 事务；迁移版本按实施顺序分配，保留旧 HTTP 数据读取。持久字段至少为格式版本、task／generation、原始有界 metainfo 与 SHA256、v1 infohash、脱敏显示来源与私有原始来源、选中索引和稳定映射、目标父目录／staging root／逐文件身份、协议阶段及待完成 manifest。未知版本拒绝并保留，旧 App 遇到较新 schema 应明确拒绝降级写入。

Add versioned `bt_state` with task foreign keys, transactionally creating/removing tasks and protocol records together. Allocate schema versions in implementation order and retain old HTTP reads. Bind format, task/generation, bounded raw metainfo/SHA256, v1 infohash, redacted display/private raw source, selected indices/stable mapping, destination-parent/staging-root/per-file identities, phase and pending completion manifest. Reject unknown versions with evidence retained. Older Apps must explicitly reject writes to a newer schema.

启动仅装载有界绑定元数据；大 metainfo 和数据重校验由 Ready 后领取槽位的 worker 完成。首版每次重启逐片重验，只复用片哈希正确的数据；预分配长度、bitfield 和旧进度不是证据。被替换文件身份失败时保留，不能按摘要相同就认领。完整凭证存在则先离线对账；已观察的凭证消失不得转为普通联网下载。恢复／checking 必须可暂停取消且 RPC 仍响应。

Startup loads bounded bindings only; large metainfo/data validation runs after Ready in claimed workers. Initially rehash every piece after restart and reuse only matching data. Preallocation length, bitfields and old progress are not proof. Retain identity-conflicting replacements rather than claiming them from matching digests. Reconcile pending completion offline first; a previously observed missing proof must not become a normal network download. Recovery/checking remains controllable while RPC responds.

整体下载进度定义为“通过片校验的数据与所选文件区间的交集字节”，去重并限制在选中总长内；跨文件边界额外字节、padding、重复流量及 checking 扫描量不计入。未知 Magnet 文件列表前 total 为 None，显示 metadata 阶段；检查进度单列。全零长度选择必须经 metainfo／空文件验证与安全发布，不能绕过完成协议。

Whole-task progress is the unique byte intersection of verified pieces and selected file ranges, bounded by selected total size. Exclude unselected boundary bytes, padding, retransfers and checking scans. Before Magnet file metadata, total is None and phase is metadata; checking has a separate metric. An all-zero-length selection still requires validated metainfo/empty files and safe publication through the completion protocol.

## 完成清单与平台前置 / Completion manifests and platform prerequisite

全部所选文件通过分片及整文件证明、sync 和停写屏障后，先持久化独立 `bt-completion/v1` manifest：绑定 task／generation／infohash／metainfo 摘要／选择、目标与 staging 目录对象，以及每个发布文件的相对路径、身份、长度和 SHA256。发布仅产生一个原先不存在的专属根目录，不与用户已有目录合并；未选边界数据不在发布树内。取消与发布共享决定锁，发布成功后一次事务保存最终进度与 Completed，保存失败保留 manifest。

After piece/whole-file proofs, sync and quiescence for every selected file, persist independent `bt-completion/v1`: bind task/generation/infohash/metainfo digest/selection, destination/staging directory objects and each published file's relative path, identity, length and SHA256. Publish one previously absent dedicated root without merging into user directories; keep boundary data outside that tree. Serialize cancellation/publication, then transactionally save final progress/Completed, retaining the manifest on failure.

manifest 不是多文件原子性或目录项所有权的证明。维护者于 2026-11-21 接受 [RFC 0002 的首发普通桌面范围](0002-segment-file-ownership.md#ordinary-desktop-ftp-bt)，用于首发 macOS BT：保留不覆盖、符号链接、完整性、取消和恢复，不承诺抵御同账户恶意程序持续替换目录项，不增加管理员服务。单目录 rename 仍解析来源名字，残余竞态未修复；无法按修改者意图区分竞态。树路径与对象校验、发布前 manifest、进程终止恢复、最终进度／Completed 原子保存及身份不匹配时保留仍须实现并经消费者验收。不把 HTTP v1 扩成隐含树语义，不以随机目录、0700、反复 stat 或事后删除错误目标冒充原子保证。M4／旧 HTTP 的保证不变，BT 产品路由和 M6 仍待各自验收。

A manifest does not prove multi-file atomicity or namespace ownership. On 2026-11-21 the maintainer accepted [RFC 0002's first-release ordinary-desktop scope](0002-segment-file-ownership.md#ordinary-desktop-ftp-bt) for macOS BT: retain no-clobber, symlink, integrity, cancellation and recovery without promising resistance to persistent same-account namespace tampering or adding an administrator service. Directory rename still resolves a source name; residual races remain and cannot distinguish the writer's intent. Implement and consumer-test tree paths/object checks, prepublication manifests, process-crash recovery, atomic final progress/Completed and mismatch preservation. Give HTTP v1 no implicit tree semantics; random names, 0700, repeated stat and removing incorrectly published targets establish no atomic guarantee. M4/legacy HTTP guarantees remain unchanged; BT product routing and M6 await their respective acceptance.

## 实施顺序与验收 / Implementation order and acceptance

本次 B1 导入切片（基线 `aaa6c7a6`）先实现离线 v1 metainfo inspection，不接 Peer 或 Magnet 元数据获取。`server.capabilities` 的 `torrent_inspect` 对象包含 `version: 1`、`sources: ["metainfo_base64"]` 及上文资源限额；只有声明的来源才可提交。`torrent.inspect.begin` 仅接受 `{metainfo_base64}`，立即返回 `{inspection_id}`；`status/cancel` 仅接受 `{inspection_id}`。status 的 phase 为 `parsing/ready/failed/cancelled`，ready 返回 `result`（v1 info_hash、原始 metainfo_sha256、name、private、piece_length/count、total_bytes、稳定 index/path/length/padding 文件清单），失败仅返回静态错误码。未知参数、Magnet 及客户端路径明确拒绝。资源耗尽为 RPC `-32030`，未知／过期／其他主体 ID 统一为 `-32031`；取消等待真实 worker join，保留 cancelled 记录到期以便幂等重试。

This B1 import slice (base `aaa6c7a6`) implements offline v1 metainfo inspection without peers or Magnet metadata fetching. `server.capabilities.torrent_inspect` declares `version: 1`, `sources: ["metainfo_base64"]` and the resource limits above. Begin accepts only `{metainfo_base64}` and returns `{inspection_id}` promptly; status/cancel accept only `{inspection_id}`. Phases are `parsing/ready/failed/cancelled`; ready exposes a result containing v1 info_hash, raw metainfo_sha256, name/private, piece length/count, total bytes and stable index/path/length/padding entries. Failures expose static codes only. Reject unknown parameters, Magnet and client paths explicitly. RPC errors are `-32030` for resource exhaustion and `-32031` for unknown/expired/other-principal IDs. Cancellation joins the actual worker and retains a cancelled record until expiry for idempotent retry.

CLI 对 `torrent.inspect.begin` 统一使用连接后实际 peer 的回环／已验证 TLS 门槛，匿名上传也在序列化／写入前检查；不是按设置页的“本地／远程”标签判断。不向非回环明文 peer 发送 metainfo。该 B1 边界沿用 ADR 0005 的回环明文策略，同 UID 替代 listener 的风险继续保留；F1 的托管 TLS pin 及来源密码身份机制仍须独立完成，不能据此宣称 metainfo 对本地同 UID 对手保密。

All CLI begin calls, including anonymous ones, check the actual connected peer's loopback/verified-TLS classification before serialization or writes, independent of a UI local/remote label. Never upload metainfo to non-loopback plaintext peers. B1 retains ADR 0005's loopback-plaintext policy and its same-UID replacement-listener risk. F1 still requires managed TLS pinning/origin-password identity; this slice does not claim confidentiality against same-UID local adversaries.

registry 每进程／数据实例独立，使用 128-bit 随机 ID；认证开启时绑定已验证的配置凭据主体，关闭时所有客户端共享匿名主体，不按未经验证的 credential 字段分区。最多 16 个存活记录（含最多 2 个活动 worker），输入／完成结果到期后由后台清理器自动取消并 join；托管关闭显式停止并 join 清理器及 worker。过期结果不复活，重启后旧 inspection 全部失效。B1 路径检查覆盖 UTF-8、分隔符／控制字符、深度／长度、重复／前缀冲突及规范化完整 casefold 冲突；实际目标文件系统的别名／身份和原子发布证明仍在 B2/B3，不以此只读检查替代。

Each process/data instance owns its registry and 128-bit random IDs. With authentication enabled, bind to the verified configured principal; otherwise share one anonymous principal, never partitioning by unverified credential input. Keep at most 16 live records, including two active workers. A background reaper cancels/joins expired work and removes results; managed shutdown explicitly joins the reaper and workers. Expired/restarted inspections cannot revive. B1 checks UTF-8, separators/control characters, depth/length, duplicates/prefixes and canonical full-casefold collisions. Actual target-filesystem aliases/identity and publication proofs remain B2/B3 responsibilities.

已接受的只读预检 P2：上述完整折叠使用 caseless 0.2.2 的 Unicode 16 表，NFD 使用 unicode-normalization 0.1.26 的 Unicode 18 表，不声明覆盖较新的折叠映射。例如 Unicode 17 的 U+A7D2/U+A7D3、U+16EA0/U+16EBB 成对映射可能漏检。接受原因是本切片仅返回元数据，不创建任务或目标文件；B2 接线前必须同步表版本或通过实际目标文件系统的拒绝机制回归这些字符对，不能用此预检证明安全存储／发布。

Accepted read-only precheck P2: full folding uses caseless 0.2.2's Unicode 16 tables while NFD uses unicode-normalization 0.1.26's Unicode 18 tables, without claiming newer folding coverage. Unicode 17 pairs U+A7D2/U+A7D3 and U+16EA0/U+16EBB can be missed. This is accepted only because inspection returns metadata without tasks or target files. Before B2 wiring, synchronize tables or regress actual target-filesystem rejection for these pairs; this precheck is not safe-storage/publication proof.

本切片固定 `sha1 0.10.7`（仅作 BEP 3 的 v1 标识，沿用现有 digest 0.10 依赖线）、`caseless 0.2.2` 和 `unicode-normalization 0.1.26`，base64/getrandom/sha2/serde 复用既有锁定版本。纯 Rust 检查器不引入 librqbit／额外 TLS/runtime；librqbit 的固定依赖整合、MSRV／许可及 Peer／存储验收仍属于 B1 后续。SHA-1 不承担来源可信或安全认证，完整 raw 绑定另用 SHA256。

This slice pins sha1 0.10.7 solely for BEP 3 identity on the existing digest 0.10 line, caseless 0.2.2 and unicode-normalization 0.1.26, reusing locked base64/getrandom/sha2/serde. The pure-Rust checker adds no librqbit/TLS/runtime. Librqbit integration, minimum-toolchain/license and peer/storage acceptance remain later B1 work. SHA-1 does not authenticate origins; SHA256 separately binds raw content.

2026-11-07 的隔离依赖编译已通过，环境、组合和边界见 [RFC 0004](0004-ftp-credentials-and-recovery.md#implementation-and-acceptance)。它使用独立锁文件，不改变产品依赖；未运行 peer、inspection、存储或停止屏障。候选引入的 reqwest 0.13.5 与当前 HTTP 的 0.12 是不同依赖线，集成时检查重复 TLS／runtime 与产物成本，不能据此声称完成 BT。

The isolated dependency compile passes on 2026-11-07; RFC 0004 records environment, features and limits. Its separate lockfile changes no product dependencies and exercises no peers, inspection, storage or shutdown barrier. Candidate reqwest 0.13.5 differs from current HTTP's 0.12 dependency line; integration must assess duplicate TLS/runtime and artifact costs. This does not establish BT completion.

| 切片 / Slice | 交付与验收 / Deliverable and acceptance |
| --- | --- |
| B1 依赖与导入 / Dependency and import | 固定库编译、许可／MSRV、严格 btih／metainfo、受限异步 inspection，经真实 Server／CLI；不宣称已能下载。 / Pin/build/audit, strict parsing and bounded inspection through real consumers, without claiming download support. |
| B2 自有存储与屏障 / Owned storage and barriers | 直接 I/O 与 loopback peer 下载；错误 piece、跨文件选择、路径替换、暂停／取消和退出后无 I/O；禁止默认文件存储危险回退。 / Direct I/O/loopback transfers, bad pieces, boundary selections, replacement races and no I/O after controls/exit, without default-storage fallback. |
| B3 恢复与安全发布 / Recovery and safe publication | SIGKILL 后逐片重验、损坏片重取、预分配进度、Ready／RPC、目录冲突、SQLite 拒写、离线 manifest 恢复和取消／发布竞态。 / Crash rehash, corrupt-piece fetching, preallocation progress, readiness/RPC, reservations, database failure, offline manifests and cancel/publication races. |
| B4 原生全链路 / Native end to end | 同一候选 `.torrent`／真实 Magnet metadata／文件选择、两种 Server 模式和切换隔离、控制、上传统计、重开、选中文件哈希与目录内容。 / One candidate validates imports, actual Magnet metadata, selection, both Server modes/switching, controls, upload stats, relaunch and output hashes/tree. |

fixture 使用自生成固定字节、受控 seeder／HTTP／UDP Tracker；Magnet 必须从 peer 获取 BEP 9 元数据，不能偷渡本地已知 metainfo。覆盖恶意路径、同内容不同身份、元数据超大／卡住、私有种子网络策略、空文件、padding 及未选数据不发布。Rust 实现运行完整 fmt／workspace all-targets check／workspace test／Clippy，含 Server unit／managed／persistence；固定 SHA 独立审查、最新 base/head 与实际合并 main CI，Desktop 另做前端及原生验收。

Use deterministic generated bytes and controlled seeders/HTTP/UDP trackers. Magnet must obtain BEP 9 metadata from a peer rather than receiving known local metainfo through a bypass. Cover hostile paths, equal-content replacement identities, oversized/stalled metadata, private-torrent networking, empty files, padding and unpublished unselected data. Rust implementation requires full formatting/check/tests/Clippy including all Server suites, fixed-SHA independent review and latest base/head/actual-main CI. Desktop adds frontend/native acceptance.

## 标准与替代方案 / Standards and alternatives

协议依据 [BEP 3](https://www.bittorrent.org/beps/bep_0003.html)、[BEP 9](https://www.bittorrent.org/beps/bep_0009.html)、[BEP 27](https://www.bittorrent.org/beps/bep_0027.html) 及 [BEP 52](https://www.bittorrent.org/beps/bep_0052.html)。libtorrent-rasterbar 是备选，会增加 C++ ABI、绑定和跨平台分发成本；aria2 sidecar 也需独立进程、RPC 凭据、版本与受控文件语义。自研 peer 协议会扩大首发范围。若候选库无法证明存储／退出边界，应据实更换或维护补丁，不降低验收断言。

Use BEPs 3/9/27/52 as protocol references. libtorrent-rasterbar is an alternative with C++ ABI/binding/distribution costs; an aria2 sidecar needs process/RPC credential/version/file-semantics ownership. A new peer implementation expands first-release scope. Replace or patch a candidate that cannot prove storage/shutdown boundaries rather than weakening acceptance assertions.
