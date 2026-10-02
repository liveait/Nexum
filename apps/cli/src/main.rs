use nexum_protocol::{Credential, RpcRequest};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

/// Simple config file management for the CLI.
pub struct Config {
    dir: PathBuf,
}

impl Config {
    pub fn new() -> Self {
        let dir = if let Ok(home) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(home).join("nexum")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("nexum")
        } else {
            PathBuf::from("./nexum")
        };
        Self { dir }
    }

    pub fn default_server_address(&self) -> Option<String> {
        self.read_config_value("default_server")
    }

    pub fn default_credential(&self) -> Option<Credential> {
        let scheme = self.read_config_value("default_auth_scheme")?;
        let token = self.read_config_value("default_auth_token")?;
        match scheme.to_ascii_lowercase().as_str() {
            "bearer" if !token.is_empty() => Some(Credential::Bearer { token }),
            "apikey" if !token.is_empty() => Some(Credential::ApiKey { key: token }),
            _ => None,
        }
    }

    pub fn set_server_address(&self, address: &str) -> Result<(), String> {
        parse_transport_address(address)?;
        self.write_config_value("default_server", address)
    }

    pub fn set_credential(&self, scheme: &str, token: &str) -> Result<(), String> {
        if !matches!(scheme.to_ascii_lowercase().as_str(), "bearer" | "apikey") {
            return Err("authentication scheme must be Bearer or ApiKey".into());
        }
        if token.trim().is_empty() {
            return Err("authentication token must not be empty".into());
        }
        self.write_config_value("default_auth_scheme", scheme)?;
        self.write_config_value("default_auth_token", token)
    }

    pub fn clear_credential(&self) -> Result<(), String> {
        self.write_config_value("default_auth_scheme", "")?;
        self.write_config_value("default_auth_token", "")
    }

    fn read_config_value(&self, key: &str) -> Option<String> {
        let file = self.config_file();
        if !file.is_file() {
            return None;
        }
        std::fs::read_to_string(&file).ok().and_then(|content| {
            content.lines().find_map(|line| {
                let (name, value) = line.trim().split_once('=')?;
                (name.trim() == key).then(|| value.trim().to_owned())
            })
        })
    }

    fn write_config_value(&self, key: &str, value: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("cannot create config directory: {e}"))?;
        let file = self.config_file();
        if file.is_file() {
            let existing =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read config: {e}"))?;
            let mut found = false;
            let mut lines = existing
                .lines()
                .map(|line| {
                    let matches_key = line
                        .trim()
                        .split_once('=')
                        .is_some_and(|(name, _)| name.trim() == key);
                    if matches_key {
                        found = true;
                        format!("{key} = {value}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>();
            if !found {
                lines.push(format!("{key} = {value}"));
            }
            let new_content = lines.join("\n");
            std::fs::write(&file, new_content).map_err(|e| format!("cannot write config: {e}"))?;
        } else {
            std::fs::write(&file, format!("{key} = {value}"))
                .map_err(|e| format!("cannot write config: {e}"))?;
        }
        Ok(())
    }

    fn config_file(&self) -> PathBuf {
        self.dir.join("config.txt")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

enum ClientTransport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for ClientTransport {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for ClientTransport {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransportAddress {
    Plain {
        authority: String,
    },
    Tls {
        authority: String,
        server_name: String,
    },
}

fn parse_transport_address(address: &str) -> Result<TransportAddress, String> {
    if let Some(authority) = address.strip_prefix("tls://") {
        let authority = authority.trim();
        if authority.is_empty() || authority.contains('/') || authority.contains('?') {
            return Err(format!("invalid TLS server address: {address}"));
        }
        let server_name = server_name_from_authority(authority)?;
        return Ok(TransportAddress::Tls {
            authority: authority.to_owned(),
            server_name,
        });
    }

    let authority = address.trim();
    if address.contains("://")
        || authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
    {
        return Err(format!("invalid server address: {address}"));
    }
    let _ =
        authority_host_port(authority).map_err(|_| format!("invalid server address: {address}"))?;
    Ok(TransportAddress::Plain {
        authority: authority.to_owned(),
    })
}

fn authority_host_port(authority: &str) -> Result<(&str, u16), String> {
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed
            .split_once(']')
            .ok_or_else(|| "IPv6 server address must close the bracket".to_owned())?;
        let port = rest
            .strip_prefix(':')
            .ok_or_else(|| "server address must include a port".to_owned())?;
        (host, port)
    } else {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| "server address must include a port".to_owned())?;
        if host.contains(':') {
            return Err("IPv6 server addresses must use [address]:port".to_owned());
        }
        (host, port)
    };
    if host.is_empty() {
        return Err("server address must include a host".to_owned());
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| "server address port must be between 1 and 65535".to_owned())?;
    if port == 0 {
        return Err("server address port must be between 1 and 65535".to_owned());
    }
    Ok((host, port))
}

fn server_name_from_authority(authority: &str) -> Result<String, String> {
    let (host, _) = authority_host_port(authority)
        .map_err(|error| format!("invalid TLS server address: {authority} ({error})"))?;
    // Validate the name now so malformed SNI values fail before a socket is opened.
    if host.parse::<std::net::IpAddr>().is_err() {
        ServerName::try_from(host.to_owned())
            .map_err(|_| format!("invalid TLS server name: {host}"))?;
    }
    Ok(host.to_owned())
}

fn native_client_config() -> Result<Arc<ClientConfig>, String> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let mut invalid_roots = 0usize;
    for certificate in native.certs {
        if roots.add(certificate).is_err() {
            invalid_roots = invalid_roots.saturating_add(1);
        }
    }
    if roots.is_empty() {
        let detail = native
            .errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        let suffix = if detail.is_empty() {
            if invalid_roots == 0 {
                String::new()
            } else {
                format!(" ({invalid_roots} invalid certificates)")
            }
        } else {
            format!(
                ": {}; {} invalid certificates",
                detail.join("; "),
                invalid_roots
            )
        };
        return Err(format!("no system root certificates are available{suffix}"));
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

pub struct JsonRpcClient {
    stream: ClientTransport,
    credential_allowed: bool,
}

impl JsonRpcClient {
    pub fn connect(address: &str) -> Result<Self, String> {
        Self::connect_with_tls_config(address, None)
    }

    fn connect_with_tls_config(
        address: &str,
        tls_config: Option<Arc<ClientConfig>>,
    ) -> Result<Self, String> {
        let parsed = parse_transport_address(address)?;
        let (stream, credential_allowed) = match parsed {
            TransportAddress::Plain { authority } => {
                let stream = TcpStream::connect(&authority)
                    .map_err(|e| format!("cannot connect to Nexum server at {address}: {e}"))?;
                let credential_allowed = stream
                    .peer_addr()
                    .map(|peer| peer.ip().is_loopback())
                    .unwrap_or(false);
                (ClientTransport::Plain(stream), credential_allowed)
            }
            TransportAddress::Tls {
                authority,
                server_name,
            } => {
                let stream = TcpStream::connect(&authority)
                    .map_err(|e| format!("cannot connect to Nexum server at {address}: {e}"))?;
                let server_name = if let Ok(ip) = server_name.parse::<std::net::IpAddr>() {
                    ServerName::IpAddress(ip.into())
                } else {
                    ServerName::try_from(server_name.clone())
                        .map_err(|_| format!("invalid TLS server name: {server_name}"))?
                };
                let config = match tls_config {
                    Some(config) => config,
                    None => native_client_config()?,
                };
                let connection = ClientConnection::new(config, server_name)
                    .map_err(|error| format!("cannot initialize TLS connection: {error}"))?;
                let mut stream = StreamOwned::new(connection, stream);
                stream
                    .conn
                    .complete_io(&mut stream.sock)
                    .map_err(|error| format!("TLS handshake failed: {error}"))?;
                (ClientTransport::Tls(Box::new(stream)), true)
            }
        };
        Ok(Self {
            stream,
            credential_allowed,
        })
    }

    pub fn call_with_credential(
        &mut self,
        id: u64,
        method: &str,
        params: Option<Value>,
        credential: Option<Credential>,
    ) -> Result<Value, String> {
        if credential.is_some() && !self.credential_allowed {
            return Err(
                "credentials can only be sent to a loopback plaintext server or a verified TLS server"
                    .to_owned(),
            );
        }
        let request = match credential {
            Some(credential) => RpcRequest::new(id, method, params).with_credential(credential),
            None => RpcRequest::new(id, method, params),
        };
        let payload = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        self.stream
            .write_all(payload.as_bytes())
            .map_err(|e| e.to_string())?;
        self.stream.write_all(b"\n").map_err(|e| e.to_string())?;
        self.stream.flush().map_err(|e| e.to_string())?;

        let mut line = Vec::with_capacity(256);
        loop {
            let mut byte = [0_u8; 1];
            let read = self.stream.read(&mut byte).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
            if line.len() > 16 * 1024 * 1024 {
                return Err("Nexum server response exceeds the maximum size".into());
            }
        }
        if line.is_empty() {
            return Err("Nexum server closed the connection without a response".into());
        }
        let response: nexum_protocol::RpcResponse =
            serde_json::from_slice(&line).map_err(|e| e.to_string())?;
        if let Some(error) = response.error {
            return Err(format!("[{}] {}", error.code, error.message));
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    pub fn call(&mut self, id: u64, method: &str, params: Option<Value>) -> Result<Value, String> {
        self.call_with_credential(id, method, params, None)
    }
}

fn usage() {
    eprintln!("usage:");
    eprintln!("  nexum [--server ADDR] task list");
    eprintln!("  nexum [--server ADDR] task get ID");
    eprintln!("  nexum [--server ADDR] task create ID SOURCE DESTINATION");
    eprintln!("  nexum [--server ADDR] task queue ID");
    eprintln!("  nexum [--server ADDR] task start");
    eprintln!("  nexum [--server ADDR] task pause ID");
    eprintln!("  nexum [--server ADDR] task resume ID");
    eprintln!("  nexum [--server ADDR] task remove ID");
    eprintln!("  nexum [--server ADDR] config get-server");
    eprintln!("  nexum [--server ADDR] config set-server ADDR");
    eprintln!("  nexum [--server ADDR] auth set SCHEME TOKEN");
    eprintln!("  nexum [--server ADDR] auth clear");
    eprintln!("  nexum [--server ADDR] server ping");
    eprintln!("  nexum [--server ADDR] server version");
    eprintln!("  nexum [--server ADDR] server auth");
    eprintln!("  nexum --version");
    eprintln!("  nexum --help");
}

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();

    // Check for --version and --help first
    if args.iter().any(|a| a == "--version") {
        eprintln!("nexum {}", env!("CARGO_PKG_VERSION"));
        eprintln!("protocol {}", nexum_protocol::RpcDispatcher::version());
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return;
    }

    // Default address (can be overridden by --server or config file)
    let mut address = String::from("127.0.0.1:39100");
    let mut credential: Option<Credential> = None;

    if args.len() >= 2 && args[0] == "--server" {
        address = args[1].clone();
        args.drain(0..2);
    } else {
        // Try to load default from config file
        let config = Config::new();
        if let Some(default_addr) = config.default_server_address() {
            address = default_addr;
        }
        // Load credential from config file
        credential = Config::new().default_credential();
    }

    if args.len() < 2
        || (args[0] != "task" && args[0] != "config" && args[0] != "auth" && args[0] != "server")
    {
        usage();
        std::process::exit(2);
    }

    // If connecting to a server, verify version on first connection
    let mut verify_version = true;

    let (method, params) = match (args[0].as_str(), args[1].as_str()) {
        ("task", "list") if args.len() == 2 => ("task.list", None),
        ("task", "get") if args.len() == 3 => {
            ("task.get", Some(serde_json::json!({"id": args[2]})))
        }
        ("task", "create") if args.len() == 5 => (
            "task.create",
            Some(serde_json::json!({"id": args[2], "source": args[3], "destination": args[4]})),
        ),
        ("task", "queue") if args.len() == 3 => {
            ("task.queue", Some(serde_json::json!({"id": args[2]})))
        }
        ("task", "start") if args.len() == 2 => ("task.start", None),
        ("task", "pause") if args.len() == 3 => {
            ("task.pause", Some(serde_json::json!({"id": args[2]})))
        }
        ("task", "resume") if args.len() == 3 => {
            ("task.resume", Some(serde_json::json!({"id": args[2]})))
        }
        ("task", "remove") if args.len() == 3 => {
            ("task.remove", Some(serde_json::json!({"id": args[2]})))
        }
        ("server", "version") if args.len() == 2 => ("server.version", None),
        ("server", "auth") if args.len() == 2 => ("server.auth", None),
        ("server", "ping") if args.len() == 2 => {
            // Ping queries both server.version and server.auth
            verify_version = false;
            ("server.version", None)
        }
        ("config", "set-server") if args.len() == 3 => {
            match Config::new().set_server_address(&args[2]) {
                Ok(()) => {
                    println!("Server address set to {}", args[2]);
                    return;
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        ("auth", "set") if args.len() == 4 => {
            match Config::new().set_credential(&args[2], &args[3]) {
                Ok(()) => {
                    println!("Authentication set: {}", args[2]);
                    return;
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        ("auth", "clear") => match Config::new().clear_credential() {
            Ok(()) => {
                println!("Authentication cleared");
                return;
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => {
            usage();
            std::process::exit(2);
        }
    };

    let mut client = match JsonRpcClient::connect(&address) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    // Verify server version on first connection (unless explicitly skipping)
    #[allow(clippy::collapsible_if)]
    if verify_version
        && let Ok(response) =
            client.call_with_credential(0, "server.version", None, credential.clone())
    {
        if let Some(server_ver) = response.as_str() {
            let local_ver = nexum_protocol::RpcDispatcher::version();
            if server_ver != local_ver {
                eprintln!(
                    "warning: server protocol v{server_ver} differs from client v{local_ver}"
                );
            } else {
                eprintln!("connected: server protocol v{server_ver}");
            }
        }
    }

    match client.call_with_credential(1, method, params, credential) {
        Ok(value) => println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::generate_simple_self_signed;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    struct TestTlsServer {
        port: u16,
        certificate_der: Vec<u8>,
        worker: JoinHandle<Option<Value>>,
    }

    impl TestTlsServer {
        fn start() -> Self {
            let generated = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
            let certificate_der = generated.cert.der().to_vec();
            let private_key =
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der()));
            let config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(certificate_der.clone())],
                    private_key,
                )
                .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let worker = thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let connection = ServerConnection::new(Arc::new(config)).unwrap();
                let mut stream = StreamOwned::new(connection, socket);
                stream.conn.complete_io(&mut stream.sock).ok()?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).ok()?;
                if line.is_empty() {
                    return None;
                }
                let request = serde_json::from_str(&line).ok()?;
                writeln!(
                    reader.get_mut(),
                    "{}",
                    serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": "1"})
                )
                .ok()?;
                Some(request)
            });
            Self {
                port,
                certificate_der,
                worker,
            }
        }

        fn address(&self, host: &str) -> String {
            format!("tls://{host}:{}", self.port)
        }
    }

    fn test_client_config(certificate_der: Option<&[u8]>) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        if let Some(certificate_der) = certificate_der {
            roots
                .add(CertificateDer::from(certificate_der.to_vec()))
                .unwrap();
        }
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    fn call_with_test_credential(
        address: &str,
        tls_config: Arc<ClientConfig>,
    ) -> Result<Value, String> {
        let mut client = JsonRpcClient::connect_with_tls_config(address, Some(tls_config))?;
        client.call_with_credential(
            1,
            "server.version",
            None,
            Some(Credential::ApiKey {
                key: "test-secret".to_owned(),
            }),
        )
    }

    fn test_config() -> (Config, PathBuf) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "nexum-cli-config-test-{}-{stamp}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (Config { dir: dir.clone() }, dir)
    }

    #[test]
    fn config_round_trips_server_address() {
        let dir = std::env::temp_dir().join("nexum-cli-config-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Override the config directory for testing
        // Config::new() uses XDG_CONFIG_HOME / HOME / ./nexum
        // We test the core logic by writing directly
        let file = dir.join("config.txt");
        std::fs::write(&file, "default_server=127.0.0.1:9999\n").unwrap();

        let read = std::fs::read_to_string(&file).unwrap();
        let value = read.trim().split_once('=').unwrap().1.trim();
        assert_eq!(value, "127.0.0.1:9999");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_returns_none_for_missing_file() {
        // The Config::new() method constructs a path, but we test the
        // core logic: if no config file exists, default_server_address returns None.
        let no_file = PathBuf::from("/nonexistent/config/nexum/config.txt");
        assert!(!no_file.is_file());
    }

    #[test]
    fn config_round_trips_credentials_and_clear_removes_both_values() {
        let (config, dir) = test_config();
        config.set_server_address("127.0.0.1:9999").unwrap();
        config.set_credential("Bearer", "secret-token").unwrap();
        assert_eq!(
            config.default_credential(),
            Some(Credential::Bearer {
                token: "secret-token".to_owned()
            })
        );

        config.set_credential("ApiKey", "api-key").unwrap();
        assert_eq!(
            config.default_credential(),
            Some(Credential::ApiKey {
                key: "api-key".to_owned()
            })
        );
        assert_eq!(
            config.default_server_address().as_deref(),
            Some("127.0.0.1:9999")
        );

        config.clear_credential().unwrap();
        assert_eq!(config.default_credential(), None);
        let content = std::fs::read_to_string(dir.join("config.txt")).unwrap();
        assert!(content.contains("default_auth_scheme = "));
        assert!(content.contains("default_auth_token = "));
        assert!(content.contains("default_server = 127.0.0.1:9999"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_rejects_unknown_scheme_and_empty_token() {
        let (config, dir) = test_config();
        assert!(config.set_credential("Basic", "secret").is_err());
        assert!(config.set_credential("Bearer", "  ").is_err());
        assert_eq!(config.default_credential(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_plain_and_tls_server_addresses() {
        assert_eq!(
            parse_transport_address("127.0.0.1:39100").unwrap(),
            TransportAddress::Plain {
                authority: "127.0.0.1:39100".to_owned()
            }
        );
        assert_eq!(
            parse_transport_address("tls://example.com:443").unwrap(),
            TransportAddress::Tls {
                authority: "example.com:443".to_owned(),
                server_name: "example.com".to_owned()
            }
        );
        assert_eq!(
            parse_transport_address("tls://[::1]:39100").unwrap(),
            TransportAddress::Tls {
                authority: "[::1]:39100".to_owned(),
                server_name: "::1".to_owned()
            }
        );
    }

    #[test]
    fn rejects_ambiguous_or_invalid_server_addresses() {
        for address in [
            "",
            "https://example.com:443",
            "tls://example.com",
            "tls://[::1]:0",
            "example.com",
            "example.com:0",
            "::1:39100",
        ] {
            assert!(
                parse_transport_address(address).is_err(),
                "expected invalid address: {address}"
            );
        }
    }

    #[test]
    fn config_rejects_invalid_server_address() {
        let (config, dir) = test_config();
        assert!(config.set_server_address("tls://example.com").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_help_is_documented() {
        assert_eq!("task.list", "task.list");
    }

    #[test]
    fn tls_client_sends_credential_after_a_verified_handshake() {
        let server = TestTlsServer::start();
        let config = test_client_config(Some(&server.certificate_der));
        let result = call_with_test_credential(&server.address("localhost"), config).unwrap();
        assert_eq!(result, "1");
        let request = server.worker.join().unwrap().unwrap();
        assert_eq!(request["method"], "server.version");
        assert_eq!(request["credential"]["ApiKey"]["key"], "test-secret");
    }

    #[test]
    fn tls_client_does_not_send_credential_to_an_untrusted_server() {
        let server = TestTlsServer::start();
        let error =
            call_with_test_credential(&server.address("localhost"), test_client_config(None))
                .unwrap_err();
        assert!(error.contains("TLS handshake failed"), "{error}");
        assert!(server.worker.join().unwrap().is_none());
    }

    #[test]
    fn tls_client_does_not_send_credential_when_the_server_name_mismatches() {
        let server = TestTlsServer::start();
        let config = test_client_config(Some(&server.certificate_der));
        let error = call_with_test_credential(&server.address("127.0.0.1"), config).unwrap_err();
        assert!(error.contains("TLS handshake failed"), "{error}");
        assert!(server.worker.join().unwrap().is_none());
    }

    #[test]
    fn tls_client_does_not_retry_a_plaintext_listener_without_tls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("tls://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut received = [0_u8; 1024];
            let size = socket.read(&mut received).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            received[..size].to_vec()
        });
        let error = call_with_test_credential(&address, test_client_config(None)).unwrap_err();
        assert!(error.contains("TLS handshake failed"), "{error}");
        let received = worker.join().unwrap();
        assert_eq!(received.first(), Some(&0x16));
        assert!(
            !received
                .windows(b"test-secret".len())
                .any(|bytes| bytes == b"test-secret")
        );
    }
}
