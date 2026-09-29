export type BrowserServerProtocol = "http:" | "https:";

type ParsedAuthority = {
  authority: string;
};

/**
 * Convert the extension's saved Server address into the HTTP bridge URL.
 *
 * A bare `host:port` keeps the existing plaintext loopback behavior. The
 * explicit `tls://host:port` spelling selects browser HTTPS, so a failed
 * HTTPS request cannot be retried over plaintext. `http://` and `https://`
 * remain accepted for users who already saved a complete URL.
 */
export function jsonRpcUrl(server: string): string {
  const value = server.trim();
  if (!value) {
    throw new Error("Server address is empty");
  }

  if (/^tls:\/\//i.test(value)) {
    const authority = value.slice(value.indexOf("://") + 3);
    return bridgeUrl("https:", authority, value);
  }

  if (/^https?:\/\//i.test(value)) {
    const protocolEnd = value.indexOf("://");
    const protocol = `${value.slice(0, protocolEnd).toLowerCase()}:` as BrowserServerProtocol;
    const remainder = value.slice(protocolEnd + 3);
    const authority = remainder.split(/[/?#]/, 1)[0];
    const url = bridgeUrl(protocol, authority, value);
    const parsed = new URL(value);
    if (parsed.username || parsed.password) {
      throw new Error("Server address must not include credentials");
    }
    if (parsed.pathname !== "/" || parsed.search || parsed.hash) {
      throw new Error("Server address must not include a path or query");
    }
    return url;
  }

  if (value.includes("://")) {
    throw new Error("Server address must use http://, https://, tls://, or host:port");
  }

  return bridgeUrl("http:", value, value);
}

function bridgeUrl(protocol: BrowserServerProtocol, rawAuthority: string, original: string): string {
  const parsedAuthority = parseAuthority(rawAuthority, original);
  try {
    const parsed = new URL(`${protocol}//${parsedAuthority.authority}`);
    if (parsed.username || parsed.password || parsed.pathname !== "/" || parsed.search || parsed.hash) {
      throw new Error("Server address must contain only a host and port");
    }
    return `${protocol}//${parsed.host}/jsonrpc`;
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("Server address")) {
      throw error;
    }
    throw new Error(`Invalid Server address: ${original}`);
  }
}

function parseAuthority(rawAuthority: string, original: string): ParsedAuthority {
  const authority = rawAuthority.trim();
  const match = /^(?:\[([^\]]+)\]|([^:[\]/?#]+)):(\d+)$/.exec(authority);
  if (!match) {
    throw new Error(`Invalid Server address: ${original}`);
  }
  const port = Number(match[3]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid Server address: ${original}`);
  }
  const host = match[1] ? `[${match[1]}]` : match[2];
  if (!host) {
    throw new Error(`Invalid Server address: ${original}`);
  }
  return { authority: `${host}:${port}` };
}
