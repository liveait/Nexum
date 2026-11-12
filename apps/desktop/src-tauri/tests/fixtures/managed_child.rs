use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpListener,
    time::Duration,
};
fn frame(input: &mut impl Read) -> Option<String> {
    let mut header = [0u8; 4];
    input.read_exact(&mut header).ok()?;
    let mut payload = vec![0u8; u32::from_be_bytes(header) as usize];
    input.read_exact(&mut payload).ok()?;
    String::from_utf8(payload).ok()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(&args[1], "--managed");
    assert_eq!(&args[2], "--data-dir");
    let dir = std::path::PathBuf::from(&args[3]);
    let mode = std::fs::read_to_string(dir.join("mode")).unwrap();
    std::fs::write(dir.join("pid"), std::process::id().to_string()).unwrap();
    let mut input = io::stdin();
    let start = frame(&mut input).unwrap();
    let start: serde_json::Value = serde_json::from_str(&start).unwrap();
    assert_eq!(start["type"], "start_tls");
    assert_eq!(start["startup"]["control_version"], 2);
    let token = start["startup"]["bearer_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(token.len(), 64);
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    if mode == "exit" {
        return;
    }
    if mode == "late-probe" {
        std::thread::sleep(Duration::from_millis(5200));
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let pid = if mode == "wrong-pid" {
        std::process::id() + 1
    } else {
        std::process::id()
    };
    let protocol = if mode == "wrong-version" { "99" } else { "1" };
    let generated = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    let cert = generated.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der());
    let config = std::sync::Arc::new(
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key.into())
            .unwrap(),
    );
    let generation = if mode == "wrong-generation" {
        serde_json::json!("0".repeat(64))
    } else {
        start["generation"].clone()
    };
    let payload = serde_json::json!({
        "control_version": 2,
        "server_version": "__VERSION__",
        "protocol_version": protocol,
        "process_id": pid,
        "endpoint": listener.local_addr().unwrap().to_string(),
        "generation": generation,
        "certificate_der": cert.as_ref(),
    })
    .to_string();
    let mut output = io::stdout();
    output
        .write_all(&(payload.len() as u32).to_be_bytes())
        .unwrap();
    output.write_all(payload.as_bytes()).unwrap();
    output.flush().unwrap();
    let mode2 = mode.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut socket = stream.unwrap();
            if mode2 == "stall-handshake" || mode2 == "trickle-handshake" {
                let mut hello = [0; 4096];
                socket.read(&mut hello).unwrap();
                std::fs::write(dir.join("probe-started"), "handshake").unwrap();
                socket.write_all(&[22, 3, 3, 16, 0, 2]).unwrap();
                if mode2 == "trickle-handshake" {
                    for _ in 0..3000 {
                        if socket.write_all(&[0]).is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                } else {
                    std::thread::sleep(Duration::from_secs(30));
                }
                continue;
            }
            let mut stream = rustls::StreamOwned::new(
                rustls::ServerConnection::new(config.clone()).unwrap(),
                socket,
            );
            let mut line = String::new();
            if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                continue;
            }
            assert!(line.contains(&token));
            if mode2 == "trickle-probe" || mode2 == "late-probe" {
                std::fs::write(dir.join("probe-started"), "response").unwrap();
                for byte in b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"1\"}\n" {
                    if stream
                        .write_all(&[*byte])
                        .and_then(|_| stream.flush())
                        .is_err()
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(70));
                }
                continue;
            }
            let reply = if mode2 == "reject-auth" {
                format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32001,\"message\":\"{}\"}}}}\n",
                    token
                )
            } else {
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"1\"}\n".to_owned()
            };
            stream.write_all(reply.as_bytes()).unwrap();
            if mode2 == "crash" {
                std::thread::sleep(Duration::from_millis(100));
                std::process::exit(7);
            }
        }
    });
    if mode == "stall-shutdown" {
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    frame(&mut input); // Shutdown or parent EOF both end the owned child.
}
