use super::*;

#[test]
fn legacy_ingress_rejects_before_completion_cleanup_or_storage() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let local = dir.root.join("SOURCE_SECRET.torrent");
    fs::write(&local, b"legacy local input").unwrap();
    for (index, source) in [
        "magnet:?xt=urn:btih:abc&tr=https://user:SOURCE_SECRET@tracker.invalid".to_owned(),
        local.to_str().unwrap().to_owned(),
        "SOURCE_SECRET://host.invalid/file".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("rejected-{index}");
        let receipt = completion_receipt_path(&CompletionContext::new(id.as_str(), dir.path()));
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, b"preserve unrelated completion evidence").unwrap();
        let response = server.request(
            "task.create",
            Some(json!({"id":id,"source":source,"destination":dir.output_path("unused")})),
        );
        assert_eq!(response["error"]["code"], -32602);
        assert!(!response.to_string().contains("SOURCE_SECRET"));
        assert_eq!(
            fs::read(receipt).unwrap(),
            b"preserve unrelated completion evidence"
        );
    }

    let receipt = completion_receipt_path(&CompletionContext::new("notification", dir.path()));
    fs::write(&receipt, b"preserve notification evidence").unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(stream, "{}", json!({"jsonrpc":"2.0","method":"task.create","params":{"id":"notification","source":"magnet:?xt=urn:btih:abc","destination":"unused"}})).unwrap();
    // A reply on this same connection proves the prior notification finished.
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0","id":9,"method":"server.version"})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 9);
    assert_eq!(
        fs::read(receipt).unwrap(),
        b"preserve notification evidence"
    );

    let receipt = completion_receipt_path(&CompletionContext::new("malformed-http", dir.path()));
    fs::write(&receipt, b"preserve malformed request evidence").unwrap();
    let response = server.request(
        "task.create",
        Some(json!({"id":"malformed-http","source":"https://example.invalid/file"})),
    );
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(
        fs::read(receipt).unwrap(),
        b"preserve malformed request evidence"
    );
    assert_eq!(server.call("task.list", None), json!([]));
    server.stop();
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    assert!(repository.list().unwrap().is_empty());
    assert!(!dir.output_path("unused").exists());
}

#[test]
fn legacy_records_are_private_and_inert_across_restart_with_http_download() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let local = dir.root.join("SOURCE_SECRET.torrent");
    fs::write(&local, b"preserve legacy local bytes").unwrap();
    let mut repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let mut originals = Vec::new();
    for (index, source) in [
        "magnet:?xt=urn:btih:abc&tr=https://user:SOURCE_SECRET@tracker.invalid".to_owned(),
        local.to_str().unwrap().to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut task = DownloadTask::new(
            format!("legacy-{index}"),
            DownloadSource::new(source),
            Destination::new(
                dir.output_path(&format!("legacy-{index}"))
                    .to_str()
                    .unwrap(),
            ),
        );
        task.state = TaskState::Queued;
        task.progress = Progress::new(123, Some(456));
        task.last_error = Some("ERROR_SECRET from the old source".to_owned());
        let stored = task.into();
        repository.insert(stored).unwrap();
        originals.push(
            repository
                .get(&TaskId::from(format!("legacy-{index}")))
                .unwrap()
                .unwrap(),
        );
    }
    drop(repository);
    for phase in 0..2 {
        let mut server = ServerProcess::start(dir.path());
        for record in &originals {
            let view = server.call("task.get", Some(json!({"id":record.id.as_str()})));
            assert!(!view.to_string().contains("SOURCE_SECRET"));
            assert!(!view.to_string().contains("ERROR_SECRET"));
            assert_eq!(view["state"], "Queued");
            assert_eq!(view["downloaded_bytes"], 123);
            assert_eq!(view["total_bytes"], 456);
        }
        let views = server.call("task.list", None).to_string();
        assert!(!views.contains("SOURCE_SECRET"));
        assert!(!views.contains("ERROR_SECRET"));
        if phase == 0 {
            let fixture = HttpFixture::new(b"HTTP still completes");
            server.call("task.create", Some(json!({"id":"http-control","source":fixture.source,"destination":dir.output_path("http-control")})));
            server.call("task.queue", Some(json!({"id":"http-control"})));
            fixture.wait_until_connected();
            fixture.finish();
            server.wait_for_state("http-control", "Completed");
            assert_eq!(
                fs::read(dir.output_path("http-control")).unwrap(),
                b"HTTP still completes"
            );
        } else {
            assert_eq!(server.task_state("http-control"), "Completed");
        }
        server.stop();
        let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
        for original in &originals {
            assert_eq!(
                repository.get(&original.id).unwrap().as_ref(),
                Some(original)
            );
            assert!(!Path::new(original.destination.as_str()).exists());
        }
        assert_eq!(fs::read(&local).unwrap(), b"preserve legacy local bytes");
    }
}

#[test]
fn legacy_boundary_is_shared_by_tls_rpc_and_https_bridge() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, certificate) = ServerProcess::start_with_tls(dir.path(), &dir.root);
    let request = json!({"jsonrpc":"2.0","id":1,"method":"task.create","params":{"id":"tls-rejected","source":"magnet:?xt=urn:btih:abc&tr=SOURCE_SECRET","destination":"unused"}});
    let response = tls_rpc_request(server.port, &certificate, request.clone());
    assert_eq!(response["error"]["code"], -32602);
    assert!(!response.to_string().contains("SOURCE_SECRET"));
    let (status, _, response) = tls_http_request(
        server.port,
        &certificate,
        "POST",
        "/jsonrpc",
        Some(request),
        None,
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(response["error"]["code"], -32602);
    assert!(!response.to_string().contains("SOURCE_SECRET"));
    let response = tls_rpc_request(
        server.port,
        &certificate,
        json!({"jsonrpc":"2.0","id":2,"method":"task.list"}),
    );
    assert_eq!(response["result"], json!([]));
    server.stop();
    assert!(
        SqliteRepository::open(dir.path().join("nexum.sqlite"))
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
}
