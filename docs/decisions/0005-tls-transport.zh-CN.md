# ADR 0005：分阶段接入 TLS 传输

状态：进行中

## 背景

Nexum 当前使用一个逐行 TCP 监听器承载 JSON-RPC、事件订阅和 Browser HTTP 桥接。由于客户端仍使用明文传输，认证凭据只允许用于 Desktop 的回环连接。Server 已完成第一段 TLS：在监听前校验并加载 PEM 材料，让 TCP、事件和 HTTP 桥接共用同一 rustls 流；CLI、Desktop 和 Browser 的地址解析与信任处理仍待接入。如果只改一个客户端，其他客户端会无法连接，或形成不安全的降级路径。

## 决策

TLS 将作为 Server、CLI、Desktop 和 Browser 桥接共用的一套版本化传输契约接入：

1. **显式选择传输。** 现有不带 scheme 的 `host:port` 地址继续表示明文连接，以保持兼容。客户端使用 `tls://host:port` 选择 TLS。Server 配置 `tls_cert_path` 和 `tls_key_path` 后，在该监听端口使用 TLS，不接受明文；TLS 握手或证书失败后不会自动回退明文。
2. **Server 配置。** `tls_cert_path` 和 `tls_key_path` 必须同时提供。Server 在打开监听器前加载 PEM 证书和私钥；文件缺失、PEM 无效、私钥不匹配或只配置其中一项都会阻止启动。同一加密流承载 JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc` 桥接。两个路径都缺失时保持 TLS 关闭。
3. **Server 鉴权模型。** 第一阶段只做单向 Server 身份验证。双向 TLS 和客户端证书留待后续配对方案；未来可以引入 `tls_ca_path` 作为显式客户端信任配置，但不能因此隐式启用客户端证书鉴权。
4. **客户端校验。** CLI 和 Desktop 默认使用系统根证书，可通过明确的设置指定 CA bundle 或固定证书。客户端必须校验 Server 名称和证书链，不提供“接受任意证书”开关。Browser Extension 使用浏览器正常的 HTTPS 信任库，现有 CORS Origin 校验保持不变。
5. **凭据策略。** 对任意 Server 地址，只有在证书校验通过的 TLS 连接上才允许附加凭据。明文凭据仍只允许发送给实际回环 peer。Desktop Keychain 账户会包含传输身份，避免同一主机和端口的明文条目被 TLS 端点复用。Keychain 或证书失败都必须显示错误；客户端不能静默改为匿名 RPC。
6. **实施顺序。** 先实现并测试传输基础，再依次接入 Server 监听器和 HTTP 桥接、CLI、Desktop RPC/事件流及设置中的信任控制，最后接入 Browser HTTPS 配置和凭据配对。每一步在启用 TLS 前都保持现有明文回环路径可用。

## 验收标准

- 证书和私钥配置必须成对出现，并在 Server 监听前完成校验。
- TLS 客户端可以完成普通 RPC、`events.subscribe` 和 HTTPS `/jsonrpc`；明文客户端不能连接 TLS-only 监听器。
- 证书链、主机名或显式 CA 校验失败时，客户端不会发送凭据，也不会回退到匿名 RPC。
- TLS 关闭时，现有明文回环 CLI、Desktop 和 Browser 流程继续通过。
- 测试覆盖握手成功/失败、无效配置、禁止降级、凭据传输策略、事件订阅及 HTTPS CORS，不提交私钥文件。
- 文档说明地址语法、信任来源、迁移行为，以及第一阶段不提供双向 TLS 和自动回退。

## 第一阶段不包含

- 自动签发或续期证书。
- 公共中继、远程设备发现或共享证书注册表。
- 双向 TLS、客户端证书或不安全的证书绕过。
- 改动下载 HTTP/HTTPS Engine；它的来源 URL 与 Server 控制面传输相互独立。
