//! Per-server RPC credentials stored as macOS Keychain generic passwords.

use nexum_protocol::Credential;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.nexum.desktop.rpc-credentials.v1";
#[cfg(any(target_os = "macos", test))]
const INVALID_STORED_CREDENTIAL: &str = "stored credential has an invalid format";
#[cfg(not(target_os = "macos"))]
const UNSUPPORTED: &str = "credential storage requires macOS Keychain";

/// Plaintext accounts keep their historical canonical host and port form.
/// TLS accounts carry an explicit `tls://` prefix so a plaintext entry can
/// never be reused for the same host and port after transport changes.
#[cfg(any(target_os = "macos", test))]
fn normalize_loopback_server(server: &str) -> Result<String, String> {
    let server = server.trim();
    if let Ok(address) = server.parse::<std::net::SocketAddr>() {
        if address.ip().is_loopback() && address.port() != 0 {
            return Ok(address.to_string());
        }
    } else if let Some((host, port)) = server.rsplit_once(':')
        && host.eq_ignore_ascii_case("localhost")
        && let Ok(port) = port.parse::<u16>()
        && port != 0
    {
        return Ok(format!("localhost:{port}"));
    }
    Err("credentials require a loopback server address with a valid port".to_owned())
}

#[cfg(any(target_os = "macos", test))]
fn normalize_tls_server(server: &str) -> Result<String, String> {
    let authority = server
        .trim()
        .strip_prefix("tls://")
        .ok_or_else(|| "TLS credential requires a tls:// server address".to_owned())?;
    if authority.is_empty() || authority.contains('/') || authority.contains('?') {
        return Err("credentials require a valid TLS server address".to_owned());
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed
            .split_once(']')
            .ok_or_else(|| "TLS IPv6 server address must close the bracket".to_owned())?;
        let port = rest
            .strip_prefix(':')
            .ok_or_else(|| "TLS server address must include a port".to_owned())?;
        (host, port)
    } else {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| "TLS server address must include a port".to_owned())?;
        if host.contains(':') {
            return Err("TLS IPv6 server addresses must use [address]:port".to_owned());
        }
        (host, port)
    };
    if host.is_empty() {
        return Err("TLS server address must include a host".to_owned());
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| "TLS server address port must be between 1 and 65535".to_owned())?;
    if port == 0 {
        return Err("TLS server address port must be between 1 and 65535".to_owned());
    }
    let host = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        ip.to_string()
    } else {
        if rustls::pki_types::ServerName::try_from(host.to_owned()).is_err() {
            return Err("TLS server address contains an invalid server name".to_owned());
        }
        host.to_ascii_lowercase()
    };
    if host.contains(':') {
        Ok(format!("tls://[{host}]:{port}"))
    } else {
        Ok(format!("tls://{host}:{port}"))
    }
}

#[cfg(any(target_os = "macos", test))]
fn normalize_credential_account(server: &str) -> Result<Option<String>, String> {
    let server = server.trim();
    if server.starts_with("tls://") {
        return normalize_tls_server(server).map(Some);
    }
    normalize_loopback_server(server).map(Some).or(Ok(None))
}

#[cfg(any(target_os = "macos", test))]
fn credential_scheme(credential: &Credential) -> Result<&'static str, String> {
    match credential {
        Credential::Bearer { token } if !token.trim().is_empty() => Ok("Bearer"),
        Credential::ApiKey { key } if !key.trim().is_empty() => Ok("ApiKey"),
        _ => Err("stored credential has an invalid format".to_owned()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn new_credential(scheme: &str, secret: String) -> Result<Credential, String> {
    if secret.trim().is_empty() {
        return Err("credential secret must not be empty".to_owned());
    }
    match scheme {
        "Bearer" => Ok(Credential::Bearer { token: secret }),
        "ApiKey" => Ok(Credential::ApiKey { key: secret }),
        _ => Err("credential scheme must be Bearer or ApiKey".to_owned()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn decode_stored_credential(bytes: &[u8]) -> Result<Credential, String> {
    let credential: Credential =
        serde_json::from_slice(bytes).map_err(|_| INVALID_STORED_CREDENTIAL.to_owned())?;
    credential_scheme(&credential)?;
    Ok(credential)
}

#[cfg(target_os = "macos")]
fn read_credential(account: &str) -> Result<Option<Credential>, String> {
    use security_framework::passwords::get_generic_password;
    use security_framework_sys::base::errSecItemNotFound;

    let bytes = match get_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(bytes) => bytes,
        Err(error) if error.code() == errSecItemNotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read credential from macOS Keychain (status {})",
                error.code()
            ));
        }
    };
    decode_stored_credential(&bytes).map(Some)
}

/// Return only the configured scheme to the webview. The secret never crosses
/// the Tauri command boundary in a response.
pub fn credential_status(server: &str) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let Some(account) = normalize_credential_account(server)? else {
            return Ok(None);
        };
        read_credential(&account)?
            .map(|credential| credential_scheme(&credential).map(str::to_owned))
            .transpose()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = server;
        Ok(None)
    }
}

pub fn save_credential(server: &str, scheme: &str, secret: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::passwords::set_generic_password;

        let account = normalize_credential_account(server)?
            .ok_or_else(|| "credentials require a loopback or TLS server address".to_owned())?;
        let credential = new_credential(scheme, secret)?;
        let bytes = serde_json::to_vec(&credential)
            .map_err(|_| "could not encode credential".to_owned())?;
        set_generic_password(KEYCHAIN_SERVICE, &account, &bytes).map_err(|error| {
            format!(
                "could not save credential to macOS Keychain (status {})",
                error.code()
            )
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (server, scheme, secret);
        Err(UNSUPPORTED.to_owned())
    }
}

pub fn clear_credential(server: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::passwords::delete_generic_password;
        use security_framework_sys::base::errSecItemNotFound;

        let account = normalize_credential_account(server)?
            .ok_or_else(|| "credentials require a loopback or TLS server address".to_owned())?;
        match delete_generic_password(KEYCHAIN_SERVICE, &account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == errSecItemNotFound => Ok(()),
            Err(error) => Err(format!(
                "could not clear credential from macOS Keychain (status {})",
                error.code()
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = server;
        Err(UNSUPPORTED.to_owned())
    }
}

/// Load credentials for a canonical loopback plaintext or explicit TLS
/// address. Keychain access failures are propagated so callers cannot
/// silently retry unauthenticated.
pub fn credential_for_request(server: &str) -> Result<Option<Credential>, String> {
    #[cfg(target_os = "macos")]
    {
        let Some(account) = normalize_credential_account(server)? else {
            return Ok(None);
        };
        read_credential(&account)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = server;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_only_loopback_servers_with_nonzero_ports() {
        assert_eq!(
            normalize_loopback_server(" LOCALHOST:039100 ").unwrap(),
            "localhost:39100"
        );
        assert_eq!(
            normalize_loopback_server("127.12.3.4:39100").unwrap(),
            "127.12.3.4:39100"
        );
        assert_eq!(
            normalize_loopback_server("[0:0:0:0:0:0:0:1]:39100").unwrap(),
            "[::1]:39100"
        );
        for server in [
            "localhost:0",
            "localhost:65536",
            "localhost:-1",
            "localhost:39100:1",
            "127.0.0.1",
            "127.0.0.1:0",
            "[::1]:0",
            "127.0.0.1.evil:39100",
            "192.168.1.1:39100",
            "[::2]:39100",
            "localhost.example:39100",
        ] {
            assert!(normalize_loopback_server(server).is_err(), "{server}");
        }
    }

    #[test]
    fn credentials_have_isolated_accounts_and_valid_schemes() {
        assert_ne!(
            normalize_loopback_server("localhost:39100").unwrap(),
            normalize_loopback_server("localhost:39101").unwrap()
        );
        assert_ne!(
            normalize_loopback_server("127.0.0.1:39100").unwrap(),
            normalize_loopback_server("127.0.0.2:39100").unwrap()
        );
        assert_eq!(
            normalize_tls_server("tls://Example.COM:443").unwrap(),
            "tls://example.com:443"
        );
        assert_eq!(
            normalize_tls_server("tls://[0:0:0:0:0:0:0:1]:443").unwrap(),
            "tls://[::1]:443"
        );
        assert_ne!(
            normalize_loopback_server("localhost:39100").unwrap(),
            normalize_tls_server("tls://localhost:39100").unwrap()
        );
        assert_eq!(
            normalize_credential_account("localhost:39100").unwrap(),
            Some("localhost:39100".to_owned())
        );
        assert_eq!(
            normalize_credential_account("tls://Example.COM:443").unwrap(),
            Some("tls://example.com:443".to_owned())
        );
        assert_eq!(normalize_credential_account("example.com:443").unwrap(), None);
        assert_eq!(
            credential_scheme(&new_credential("Bearer", "secret".to_owned()).unwrap()).unwrap(),
            "Bearer"
        );
        assert_eq!(
            credential_scheme(&new_credential("ApiKey", "secret".to_owned()).unwrap()).unwrap(),
            "ApiKey"
        );
        assert!(new_credential("Bearer", String::new()).is_err());
        assert!(new_credential("Bearer", "  ".to_owned()).is_err());
        assert!(new_credential("None", "secret".to_owned()).is_err());
        assert!(credential_scheme(&Credential::None).is_err());
        assert!(
            credential_scheme(&Credential::Bearer {
                token: " \t ".to_owned()
            })
            .is_err()
        );
        for server in [
            "tls://example.com",
            "tls://example.com:0",
            "tls://example.com:443/path",
            "tls://[::1]:443",
        ] {
            if server == "tls://[::1]:443" {
                assert!(normalize_tls_server(server).is_ok());
            } else {
                assert!(normalize_tls_server(server).is_err(), "{server}");
            }
        }
        assert!(
            credential_scheme(&Credential::ApiKey {
                key: "\n".to_owned()
            })
            .is_err()
        );
    }

    #[test]
    fn status_decoding_rejects_corruption_without_exposing_secret() {
        let stored = serde_json::to_vec(&Credential::Bearer {
            token: "test-secret".to_owned(),
        })
        .unwrap();
        let decoded = decode_stored_credential(&stored).unwrap();
        assert_eq!(credential_scheme(&decoded).unwrap(), "Bearer");

        let error = decode_stored_credential(br#"{"Bearer":{"token":"test-secret"}"#).unwrap_err();
        assert_eq!(error, INVALID_STORED_CREDENTIAL);
        assert!(!error.contains("test-secret"));
        assert!(decode_stored_credential(br#""None""#).is_err());
    }
}
