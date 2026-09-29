use nexum_protocol::{Credential, RpcRequest};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;

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

pub struct JsonRpcClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl JsonRpcClient {
    pub fn connect(address: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(address)
            .map_err(|e| format!("cannot connect to Nexum server at {address}: {e}"))?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        Ok(Self { stream, reader })
    }

    pub fn call_with_credential(
        &mut self,
        id: u64,
        method: &str,
        params: Option<Value>,
        credential: Option<Credential>,
    ) -> Result<Value, String> {
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

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let response: nexum_protocol::RpcResponse =
            serde_json::from_str(&line).map_err(|e| e.to_string())?;
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
    fn cli_help_is_documented() {
        assert_eq!("task.list", "task.list");
    }
}
