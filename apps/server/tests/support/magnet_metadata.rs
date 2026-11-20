use super::*;
use nexum_core::nexum_resolver::torrent;
use std::sync::atomic::AtomicBool;

fn fixture(stalled: bool) -> (String, String, Receiver<()>, Receiver<()>, JoinHandle<()>) {
    let raw = inspection_info();
    let envelope = [b"d4:info".as_slice(), &raw, b"e"].concat();
    let expected = torrent::inspect_metainfo(
        &envelope,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap()
    .info_hash;
    let identity = expected.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (connected_tx, connected) = mpsc::channel();
    let (closed_tx, closed) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let mut handshake = [0; 68];
        peer.read_exact(&mut handshake).unwrap();
        assert_eq!(&handshake[..20], b"\x13BitTorrent protocol");
        let got = handshake[28..48]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(got, identity);
        peer.write_all(&handshake).unwrap();
        fn packet(peer: &mut TcpStream) -> Vec<u8> {
            let mut len = [0; 4];
            peer.read_exact(&mut len).unwrap();
            let n = u32::from_be_bytes(len) as usize;
            assert!(n < 1024);
            let mut b = vec![0; n];
            peer.read_exact(&mut b).unwrap();
            b
        }
        fn send(peer: &mut TcpStream, b: &[u8]) {
            peer.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
            peer.write_all(b).unwrap();
        }
        assert_eq!(packet(&mut peer), b"\x14\x00d1:md11:ut_metadatai1eee");
        send(
            &mut peer,
            format!(
                "\x14\x00d1:md11:ut_metadatai7ee13:metadata_sizei{}ee",
                raw.len()
            )
            .as_bytes(),
        );
        assert_eq!(packet(&mut peer), b"\x14\x07d8:msg_typei0e5:piecei0ee");
        connected_tx.send(()).unwrap();
        if !stalled {
            send(
                &mut peer,
                &[
                    format!(
                        "\x14\x01d8:msg_typei1e5:piecei0e10:total_sizei{}ee",
                        raw.len()
                    )
                    .as_bytes(),
                    &raw,
                ]
                .concat(),
            );
        }
        let mut b = [0];
        assert_eq!(
            peer.read(&mut b).unwrap(),
            0,
            "inspection worker must close its peer"
        );
        closed_tx.send(()).unwrap();
    });
    (
        format!(
            "magnet:?xt=urn:btih:{expected}&tr=http%3A%2F%2Fignored.invalid%2FPASSKEY_SENTINEL"
        ),
        address,
        connected,
        closed,
        worker,
    )
}

#[test]
fn explicit_peer_metadata_runs_through_http_rpc_without_tasks_or_outputs() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let (magnet, peer, connected, closed, worker) = fixture(false);
    let(status,_,reply)=server.http_request("POST","/jsonrpc",Some(json!({"jsonrpc":"2.0","id":1,"method":"torrent.inspect.begin","params":{"magnet":magnet,"peer":peer}})));
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(reply.get("error").is_none(), "{reply}");
    let id = reply["result"]["inspection_id"].as_str().unwrap();
    connected.recv_timeout(Duration::from_secs(2)).unwrap();
    closed.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    let result = await_inspection(|| {
        server.call("torrent.inspect.status", Some(json!({"inspection_id":id})))
    });
    assert_eq!(result["phase"], "ready");
    assert_eq!(result["result"]["total_bytes"], 3);
    assert_eq!(result["result"]["files"][0]["path"], json!(["a.txt"]));
    assert!(!result.to_string().contains("PASSKEY_SENTINEL"));
    assert_eq!(server.call("task.list", None), json!([]));
    assert_eq!(fs::read_dir(dir.root.join("downloads")).unwrap().count(), 0);
    let db = rusqlite::Connection::open(dir.path().join("nexum.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    server.stop();
    let mut restarted = ServerProcess::start(dir.path());
    assert_eq!(
        restarted.request("torrent.inspect.status", Some(json!({"inspection_id":id})))["error"]["code"],
        -32031
    );
    restarted.stop();
}

#[test]
fn explicit_peer_cancel_joins_connections_before_quota_reuse() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let (magnet, peer, connected, closed, worker) = fixture(true);
        let begin = server.call(
            "torrent.inspect.begin",
            Some(json!({"magnet":magnet,"peer":peer})),
        );
        connected.recv_timeout(Duration::from_secs(2)).unwrap();
        jobs.push((
            begin["inspection_id"].as_str().unwrap().to_owned(),
            closed,
            worker,
        ));
    }
    let (magnet, peer, connected, closed, worker) = fixture(false);
    let input = json!({"magnet":magnet,"peer":peer});
    assert_eq!(
        server.request("torrent.inspect.begin", Some(input.clone()))["error"]["code"],
        -32030
    );
    for (id, closed, worker) in jobs {
        assert_eq!(
            server.call("torrent.inspect.cancel", Some(json!({"inspection_id":id})))["cancelled"],
            true
        );
        closed.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        assert_eq!(
            server.call("torrent.inspect.status", Some(json!({"inspection_id":id})))["phase"],
            "cancelled"
        );
    }
    let begin = server.call("torrent.inspect.begin", Some(input));
    connected.recv_timeout(Duration::from_secs(2)).unwrap();
    closed.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    assert_eq!(
        await_inspection(|| server.call(
            "torrent.inspect.status",
            Some(json!({"inspection_id":begin["inspection_id"]}))
        ))["phase"],
        "ready"
    );
    server.stop();
}

#[test]
fn explicit_peer_input_and_authentication_reject_before_connect() {
    let _guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_auth(dir.path(), "Bearer", "private-token");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let valid = json!({"magnet":"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567","peer":listener.local_addr().unwrap().to_string()});
    assert_unauthorized(&server.request("torrent.inspect.begin", Some(valid.clone())));
    let credential = Credential::Bearer {
        token: "private-token".into(),
    };
    for source in [
        "magnet:?xt=urn:btih:abc".to_owned(),
        "\0magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
        "mag\0net:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
        format!(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=PASSKEY_SENTINEL{}",
            "x".repeat(8192)
        ),
    ] {
        let mut params = valid.clone();
        params["magnet"] = json!(source);
        let reply = server.request_with_credential(
            "torrent.inspect.begin",
            Some(params),
            Some(credential.clone()),
        );
        assert_eq!(reply["error"]["code"], -32602);
        assert!(!reply.to_string().contains("PASSKEY_SENTINEL"));
    }
    for peer in [
        "localhost:1234",
        "127.0.0.1:0",
        "0.0.0.0:1234",
        "224.0.0.1:1234",
    ] {
        let mut params = valid.clone();
        params["peer"] = json!(peer);
        assert_eq!(
            server.request_with_credential(
                "torrent.inspect.begin",
                Some(params),
                Some(credential.clone())
            )["error"]["code"],
            -32602
        );
    }
    assert!(listener.accept().is_err());
    assert_eq!(
        server.call_with_credential("task.list", None, credential),
        json!([])
    );
    server.stop();
}
