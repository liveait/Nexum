# ADR 0005: Staged TLS Transport

Status: In progress

## Context

Nexum currently uses one line-delimited TCP listener for JSON-RPC, event subscriptions, and the Browser HTTP bridge. Authentication credentials are accepted only for loopback Desktop connections because Desktop and Browser still use plaintext. The Server now validates and loads PEM material before listening and carries TCP, event, and HTTP bridge traffic through one rustls stream. The CLI has a TLS client slice: it selects `tls://host:port`, verifies the server name against the system root store, and gates credentials on the verified transport. Desktop and Browser address parsing and trust handling are still pending. Adding TLS to only one client leaves the remaining clients on their explicit plaintext compatibility path; no client may silently downgrade after a TLS failure.

## Decision

TLS will be added as one versioned transport contract shared by the Server, CLI, Desktop, and Browser bridge:

1. **Explicit transport selection.** Existing bare `host:port` addresses remain plaintext for backward compatibility. Clients select TLS with a `tls://host:port` address. A Server with `tls_cert_path` and `tls_key_path` configured accepts TLS on its listener and does not accept plaintext on that port. There is no automatic plaintext fallback after a TLS handshake or certificate failure.
2. **Server configuration.** `tls_cert_path` and `tls_key_path` must be supplied together. The Server loads PEM certificates and a private key before opening the listener; missing files, invalid PEM, a mismatched key, or an incomplete pair fail startup. The same secured stream handles JSON-RPC, `events.subscribe`, and the HTTP `/jsonrpc` bridge. TLS remains disabled when both paths are absent.
3. **Server authentication model.** The first TLS slice uses one-way Server authentication. Mutual TLS and client certificates are reserved for a later pairing decision. `tls_ca_path` may be introduced for explicit client trust configuration, but it must not silently enable client-certificate authentication.
4. **Client verification.** CLI and Desktop clients use the platform/system root store by default and accept an explicit CA bundle or pinned certificate only through a deliberate setting. They validate the Server name and certificate chain and never expose an “accept any certificate” switch. The Browser extension uses the browser's normal HTTPS trust store; its existing CORS origin checks remain unchanged.
5. **Credential policy.** A credential may be attached to a verified TLS connection for any configured Server address. Plaintext credentials remain limited to an actual loopback peer. Desktop Keychain accounts include the transport identity so a plaintext entry cannot be reused for a TLS endpoint with the same host and port. Keychain failures and certificate failures remain visible errors; clients do not retry anonymously.
6. **Rollout order.** Implement and test the transport primitives first, then wire clients in this order: Server listener and HTTP bridge, CLI, Desktop RPC/event stream and Settings trust controls, and finally Browser HTTPS configuration and credential pairing. The Server and CLI slices are now in place; each remaining step keeps the default plaintext loopback path working until its TLS feature is enabled.

## Acceptance criteria

- An all-or-nothing certificate/key configuration is validated before the Server listens.
- A TLS client can perform ordinary RPC, `events.subscribe`, and HTTPS `/jsonrpc`; a plaintext client cannot use a TLS-only listener.
- Certificate chain, hostname, and explicit CA failures are reported without sending a credential or falling back to anonymous RPC.
- Existing plaintext loopback CLI, Desktop, and Browser flows continue to pass when TLS is disabled.
- Tests cover handshake success/failure, invalid configuration, no downgrade, credential transport policy, event subscriptions, and Browser CORS over HTTPS without committing private keys.
- Documentation names the address syntax, trust-source rules, migration behavior, and the absence of mutual TLS/automatic fallback.

## Non-goals for the first rollout

- Automatic certificate issuance or renewal.
- A public relay, remote device discovery, or a shared certificate registry.
- Mutual TLS, client certificates, or an insecure certificate bypass.
- Changing the download HTTP/HTTPS engine; its source URLs are independent of the Server control-plane transport.
