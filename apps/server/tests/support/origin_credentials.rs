use super::*;
fn input(id: &str, destination: &Path) -> Value {
    json!({"version":1,"id":id,"destination":destination,"source":{"kind":"ftp","source_url":"ftp://EXAMPLE.com/pub/a%20b.bin","security":"plain","authentication":{"kind":"anonymous"}}})
}
#[test]
fn typed_anonymous_create_retry_restart_and_remove_use_real_ingress() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let value = input("ftp", &dir.output_path("ftp"));
    let created = server.call("task.create_transfer", Some(value.clone()));
    assert_eq!(created["source"], "ftp://example.com/pub/a%20b.bin");
    assert_eq!(created["state"], "Created");
    assert_eq!(
        server.call("task.create_transfer", Some(value.clone())),
        created
    );
    let mut conflict = value.clone();
    conflict["destination"] = json!(dir.output_path("other"));
    assert_eq!(
        server.request("task.create_transfer", Some(conflict))["error"]["code"],
        -32009
    );
    let caps = server.call("server.capabilities", None);
    assert_eq!(caps["create_transfer"]["versions"], json!([1]));
    assert_eq!(caps["ftp"]["transfer"], false);
    server.stop();
    let mut server = ServerProcess::start(dir.path());
    assert_eq!(server.call("task.get", Some(json!({"id":"ftp"}))), created);
    assert_eq!(server.call("task.list", None), json!([created]));
    assert!(
        server.request("task.queue", Some(json!({"id":"ftp"})))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("FTP transfer unavailable")
    );
    assert_eq!(server.task_state("ftp"), "Created");
    let receipt = completion_receipt_path(&CompletionContext::new("ftp", dir.path()));
    fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    fs::write(&receipt, b"unrelated HTTP evidence").unwrap();
    server.call("task.remove", Some(json!({"id":"ftp"})));
    assert_eq!(fs::read(receipt).unwrap(), b"unrelated HTTP evidence");
    assert_eq!(server.call("task.list", None), json!([]));
    server.stop();
}
#[test]
fn passwords_reject_plain_tcp_and_http_before_storage() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let mut value = input("password", &dir.output_path("file"));
    value["source"]["authentication"] =
        json!({"kind":"password","username":"USER_SENTINEL","password":"PASSWORD_SENTINEL"});
    let response = server.request("task.create_transfer", Some(value.clone()));
    assert_eq!(response["error"]["code"], -32602);
    assert!(!response.to_string().contains("SENTINEL"));
    let payload = json!({"jsonrpc":"2.0","id":1,"method":"task.create_transfer","params":value});
    let response = server
        .http_request("POST", "/jsonrpc", Some(payload))
        .2
        .to_string();
    assert!(response.contains("password credentials require TLS"));
    assert!(!response.contains("SENTINEL"));
    assert_eq!(server.call("task.list", None), json!([]));
    server.stop();
    let repo = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    assert!(repo.pending_origins().unwrap().is_empty());
    assert!(repo.cleanup_records().unwrap().is_empty());
}
#[test]
fn tls_and_https_anonymous_creation_uses_typed_projection() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, cert) = ServerProcess::start_with_tls(dir.path(), &dir.root);
    let request = json!({"jsonrpc":"2.0","id":1,"method":"task.create_transfer","params":input("tls-ftp",&dir.output_path("tls"))});
    let response = tls_rpc_request(server.port, &cert, request);
    assert_eq!(
        response["result"]["source"],
        "ftp://example.com/pub/a%20b.bin"
    );
    let request = json!({"jsonrpc":"2.0","id":2,"method":"task.create_transfer","params":input("https-ftp",&dir.output_path("https"))});
    let response = tls_http_request(server.port, &cert, "POST", "/jsonrpc", Some(request), None)
        .2
        .to_string();
    assert!(response.contains("ftp://example.com/pub/a%20b.bin"));
    server.stop();
}

#[test]
fn unavailable_security_and_unknown_input_have_no_durable_side_effects() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    for change in [
        json!("required_explicit_tls"),
        json!("unknown_PASSWORD_SENTINEL"),
    ] {
        let mut value = input("unavailable", &dir.output_path("file"));
        value["source"]["security"] = change;
        let response = server.request("task.create_transfer", Some(value));
        assert_eq!(response["error"]["code"], -32602);
        assert!(!response.to_string().contains("SENTINEL"));
    }
    assert_eq!(server.call("task.list", None), json!([]));
    server.stop();
    let repo = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    assert!(repo.pending_origins().unwrap().is_empty());
    assert!(repo.cleanup_records().unwrap().is_empty());
}

#[test]
fn typed_notifications_keep_tcp_silent_and_http_no_content() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let mut socket = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let value = input("notification", &dir.output_path("file"));
    writeln!(
        socket,
        "{}",
        json!({"jsonrpc":"2.0","method":"task.create_transfer","params":value})
    )
    .unwrap();
    let mut password = input("rejected-notification", &dir.output_path("rejected"));
    password["source"]["authentication"] =
        json!({"kind":"password","username":"USER_SENTINEL","password":"PASSWORD_SENTINEL"});
    writeln!(
        socket,
        "{}",
        json!({"jsonrpc":"2.0","method":"task.create_transfer","params":password})
    )
    .unwrap();
    writeln!(
        socket,
        "{}",
        json!({"jsonrpc":"2.0","id":77,"method":"task.list"})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 77);
    assert_eq!(response["result"].as_array().unwrap().len(), 1);
    assert!(!line.contains("SENTINEL"));
    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc":"2.0","method":"task.create_transfer","params":password})),
    );
    assert!(status.contains("204"));
    assert_eq!(response, Value::Null);
    assert_eq!(server.call("task.list", None).as_array().unwrap().len(), 1);
    server.stop();
}
