<a id="adr-0005-staged-tls-transport"></a>
<a id="adr-0005分阶段接入-tls-传输"></a>

# ADR 0005：分阶段接入 TLS 传输 / Staged TLS Transport

状态：进行中

Status: In progress

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

Nexum 当前使用一个逐行 TCP 监听器承载 JSON-RPC、事件订阅和 Browser HTTP 桥接。Browser 桥接对不带 scheme 的地址使用 HTTP，并将显式 `tls://host:port` 映射到浏览器 HTTPS；Desktop 凭据可以使用回环明文或经过校验的 TLS 端点。Server 已完成 TLS 监听器：在监听前校验并加载 PEM 材料，让 TCP、事件和 HTTP 桥接共用同一 rustls 流。CLI 和 Desktop 已完成 TLS 客户端切片：使用 `tls://host:port`，对照系统根证书校验 Server 名称，并按已校验的传输状态限制凭据。Browser 使用浏览器正常的 HTTPS 信任库，仍没有凭据配对。任何客户端都不能在 TLS 失败后静默降级。

Nexum currently uses one line-delimited TCP listener for JSON-RPC, event subscriptions, and the Browser HTTP bridge. The Browser bridge accepts bare addresses over HTTP and maps explicit `tls://host:port` addresses to browser HTTPS, while Desktop credentials can use loopback plaintext or a verified TLS endpoint. The Server now validates and loads PEM material before listening and carries TCP, event, and HTTP bridge traffic through one rustls stream. The CLI and Desktop select `tls://host:port`, verify the server name against the system root store, and gate credentials on the verified transport. The Browser uses the browser's normal HTTPS trust store and still has no credential pairing. No client may silently downgrade after a TLS failure.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

TLS 将作为 Server、CLI、Desktop 和 Browser 桥接共用的一套版本化传输契约接入：

TLS will be added as one versioned transport contract shared by the Server, CLI, Desktop, and Browser bridge:

1. **显式选择传输。** 现有不带 scheme 的 `host:port` 地址继续表示明文连接，以保持兼容。客户端使用 `tls://host:port` 选择 TLS。Server 配置 `tls_cert_path` 和 `tls_key_path` 后，在该监听端口使用 TLS，不接受明文；TLS 握手或证书失败后不会自动回退明文。<br>
   **Explicit transport selection.** Existing bare `host:port` addresses remain plaintext for backward compatibility. Clients select TLS with a `tls://host:port` address. A Server with `tls_cert_path` and `tls_key_path` configured accepts TLS on its listener and does not accept plaintext on that port. There is no automatic plaintext fallback after a TLS handshake or certificate failure.
2. **Server 配置。** `tls_cert_path` 和 `tls_key_path` 必须同时提供。Server 在打开监听器前加载 PEM 证书和私钥；文件缺失、PEM 无效、私钥不匹配或只配置其中一项都会阻止启动。同一加密流承载 JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc` 桥接。两个路径都缺失时保持 TLS 关闭。<br>
   **Server configuration.** `tls_cert_path` and `tls_key_path` must be supplied together. The Server loads PEM certificates and a private key before opening the listener; missing files, invalid PEM, a mismatched key, or an incomplete pair fail startup. The same secured stream handles JSON-RPC, `events.subscribe`, and the HTTP `/jsonrpc` bridge. TLS remains disabled when both paths are absent.
3. **Server 鉴权模型。** 第一阶段只做单向 Server 身份验证。双向 TLS 和客户端证书留待后续配对方案；未来可以引入 `tls_ca_path` 作为显式客户端信任配置，但不能因此隐式启用客户端证书鉴权。<br>
   **Server authentication model.** The first TLS slice uses one-way Server authentication. Mutual TLS and client certificates are reserved for a later pairing decision. `tls_ca_path` may be introduced for explicit client trust configuration, but it must not silently enable client-certificate authentication.
4. **客户端校验。** CLI 和 Desktop 默认使用系统根证书。未来加入 CA bundle 或证书固定时，必须通过明确的设置指定。客户端必须校验 Server 名称和证书链，不提供“接受任意证书”开关。Browser Extension 使用浏览器正常的 HTTPS 信任库，现有 CORS Origin 校验保持不变。<br>
   **Client verification.** CLI and Desktop clients use the platform/system root store by default. A future explicit CA bundle or pinned certificate must require a deliberate setting. They validate the Server name and certificate chain and never expose an “accept any certificate” switch. The Browser extension uses the browser's normal HTTPS trust store; its existing CORS origin checks remain unchanged.
5. **凭据策略。** 对任意 Server 地址，只有在证书校验通过的 TLS 连接上才允许附加凭据。明文凭据仍只允许发送给实际回环 peer。Desktop Keychain 账户会包含传输身份，避免同一主机和端口的明文条目被 TLS 端点复用。Keychain 或证书失败都必须显示错误；客户端不能静默改为匿名 RPC。<br>
   **Credential policy.** A credential may be attached to a verified TLS connection for any configured Server address. Plaintext credentials remain limited to an actual loopback peer. Desktop Keychain accounts include the transport identity so a plaintext entry cannot be reused for a TLS endpoint with the same host and port. Keychain failures and certificate failures remain visible errors; clients do not retry anonymously.
6. **实施顺序。** 先实现并测试传输基础，再依次接入 Server 监听器和 HTTP 桥接、CLI、Desktop RPC/事件流及设置中的信任控制，最后接入 Browser HTTPS 配置和凭据配对。Server、CLI、Desktop 与 Browser HTTPS 切片已完成；Browser 凭据配对和显式客户端信任控制仍是后续独立工作。每个客户端在明确选择 TLS 前都保持现有明文回环路径可用。<br>
   **Rollout order.** Implement and test the transport primitives first, then wire clients in this order: Server listener and HTTP bridge, CLI, Desktop RPC/event stream and Settings trust controls, and finally Browser HTTPS configuration and credential pairing. The Server, CLI, Desktop, and Browser HTTPS slices are now in place; Browser credential pairing and explicit client trust controls remain separate follow-up work. Each client keeps the default plaintext loopback path working until TLS is explicitly selected.

<a id="acceptance-criteria"></a>
<a id="验收标准"></a>

## 验收标准 / Acceptance criteria

- 证书和私钥配置必须成对出现，并在 Server 监听前完成校验。<br>
  An all-or-nothing certificate/key configuration is validated before the Server listens.
- TLS 客户端可以完成普通 RPC、`events.subscribe` 和 HTTPS `/jsonrpc`；明文客户端不能连接 TLS-only 监听器。<br>
  A TLS client can perform ordinary RPC, `events.subscribe`, and HTTPS `/jsonrpc`; a plaintext client cannot use a TLS-only listener.
- 证书链或主机名校验失败时，客户端不会发送凭据，也不会回退到匿名 RPC。未来加入显式 CA 设置后，该设置的校验失败也必须遵守这一规则。<br>
  Certificate chain and hostname failures are reported without sending a credential or falling back to anonymous RPC. The same behavior is required for explicit CA failures when that setting is added.
- TLS 关闭时，现有明文回环 CLI、Desktop 和 Browser 流程继续通过。<br>
  Existing plaintext loopback CLI, Desktop, and Browser flows continue to pass when TLS is disabled.
- 运行时生成证书的集成测试已覆盖 TLS 握手成功、明文拒绝、TCP RPC、HTTPS `/jsonrpc` `OPTIONS`/`POST`、Browser 扩展 CORS 允许/拒绝、TCP/HTTPS 上的配置鉴权、带鉴权的 `events.subscribe`、无效或不完整的 Server TLS 配置，以及不受信任或主机名不匹配的证书。CLI 和 Desktop 客户端测试覆盖连接失败后的凭据门控；CLI 还验证 TLS 握手失败后不会重试明文。证书和私钥只在测试运行时生成。<br>
  Generated-certificate integration tests cover successful TLS handshake, plaintext rejection, TCP RPC, HTTPS `/jsonrpc` `OPTIONS`/`POST`, browser-extension CORS allow/reject behavior, configured authentication over TCP/HTTPS, authenticated `events.subscribe`, invalid or incomplete Server TLS configuration, and untrusted or wrong-hostname certificates. CLI and Desktop client tests cover credential gating after connection failure; the CLI also verifies that a TLS handshake failure does not retry plaintext. Certificates and private keys are generated at test runtime.
- 文档说明地址语法、信任来源、迁移行为，以及第一阶段不提供双向 TLS 和自动回退。<br>
  Documentation names the address syntax, trust-source rules, migration behavior, and the absence of mutual TLS/automatic fallback.

<a id="non-goals-for-the-first-rollout"></a>
<a id="第一阶段不包含"></a>

## 第一阶段不包含 / Non-goals for the first rollout

- 自动签发或续期证书。<br>
  Automatic certificate issuance or renewal.
- 公共中继、远程设备发现或共享证书注册表。<br>
  A public relay, remote device discovery, or a shared certificate registry.
- 双向 TLS、客户端证书或不安全的证书绕过。<br>
  Mutual TLS, client certificates, or an insecure certificate bypass.
- 改动下载 HTTP/HTTPS Engine；它的来源 URL 与 Server 控制面传输相互独立。<br>
  Changing the download HTTP/HTTPS engine; its source URLs are independent of the Server control-plane transport.
