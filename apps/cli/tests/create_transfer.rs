use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn run(input: &str, address: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nexum-cli"))
        .args(["--server", address, "task", "create-transfer"])
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join(format!("nexum-cli-transfer-{}", std::process::id())),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn input(authentication: Value) -> Value {
    json!({"version":1,"id":"cli","destination":"/tmp/nexum-cli-test-file","source":{"kind":"ftp","source_url":"ftp://example.com/pub/a.bin","security":"plain","authentication":authentication}})
}
#[test]
fn actual_cli_reads_anonymous_stdin_and_sends_only_typed_method_after_capabilities() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        let (socket, _) = loop {
            if let Ok(socket) = listener.accept() {
                break socket;
            }
            if Instant::now() > deadline {
                return Vec::new();
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(socket);
        let mut methods = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "server.version" => json!(nexum_protocol::RpcDispatcher::version()),
                "server.capabilities" => {
                    json!({"create_transfer":{"versions":[1]},"ftp":{"metadata_create":true,"durable_credentials":false}})
                }
                "task.create_transfer" => {
                    assert_eq!(request["params"]["id"], "cli");
                    json!({"id":"cli","source":"ftp://example.com/pub/a.bin"})
                }
                _ => panic!("unexpected method"),
            };
            writeln!(
                reader.get_mut(),
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],"result":result})
            )
            .unwrap();
        }
        methods
    });
    let output = run(&input(json!({"kind":"anonymous"})).to_string(), &address);
    let methods = worker.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        methods,
        [
            "server.version",
            "server.capabilities",
            "task.create_transfer"
        ]
    );
}
#[test]
fn actual_cli_rejects_plaintext_password_and_malformed_stdin_with_static_errors() {
    let value =
        input(json!({"kind":"password","username":"USER_SENTINEL","password":"PASSWORD_SENTINEL"}));
    let output = run(&value.to_string(), "127.0.0.1:1");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("password credentials require verified TLS")
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SENTINEL"));
    let output = run("{\"PASSWORD_SENTINEL\":", "127.0.0.1:1");
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid transfer JSON"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SENTINEL"));

    let mut value = input(json!({"kind":"anonymous"}));
    value["source"]["source_url"] =
        json!("ftp://example.org:21?QUERY_SENTINEL=@mirror.example/file");
    value["source"]
        .as_object_mut()
        .unwrap()
        .remove("authentication");
    let output = run(&value.to_string(), "127.0.0.1:1");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid transfer v1 input"), "{stderr}");
    assert!(!stderr.contains("SENTINEL"));
}

#[test]
fn actual_cli_explains_unsupported_ftps_and_legacy_absolute_paths_without_reflection() {
    for (url, guidance) in [
        ("ftps://example.com/FTPS_SENTINEL", "FTPS is unavailable"),
        (
            "ftp://example.com/%2fPATH_SENTINEL/a.bin",
            "use path_mode=absolute",
        ),
    ] {
        let mut value = input(json!({"kind":"anonymous"}));
        value["source"]["source_url"] = json!(url);
        let output = run(&value.to_string(), "127.0.0.1:1");
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(guidance), "{stderr}");
        assert!(!stderr.contains("SENTINEL"));
        assert!(output.stdout.is_empty());
    }

    let mut value =
        input(json!({"kind":"password","username":"USER_SENTINEL","password":"PASSWORD_SENTINEL"}));
    value["source"]["security"] = json!("required_explicit_tls");
    let output = run(&value.to_string(), "127.0.0.1:1");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("FTP security mode unavailable"), "{stderr}");
    assert!(!stderr.contains("SENTINEL"));
    assert!(output.stdout.is_empty());
}
