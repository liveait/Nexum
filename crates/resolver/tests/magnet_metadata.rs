use nexum_resolver::{
    magnet::parse_magnet,
    torrent::{
        self,
        metadata::{MetadataError, fetch_magnet_metadata},
    },
};
use sha1::{Digest, Sha1};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn rejects_unsafe_peer_addresses_including_ipv4_mapped_forms() {
    for peer in [
        "0.0.0.0:7",
        "127.0.0.1:0",
        "224.0.0.1:7",
        "255.255.255.255:7",
        "[::]:7",
        "[ff02::1]:7",
        "[::ffff:0.0.0.0]:7",
        "[::ffff:224.0.0.1]:7",
        "[::ffff:255.255.255.255]:7",
    ] {
        assert!(
            !torrent::metadata::valid_peer(peer.parse().unwrap()),
            "{peer}"
        );
    }
    assert!(torrent::metadata::valid_peer(
        "[::ffff:127.0.0.1]:7".parse().unwrap()
    ));
}

fn info(pieces: usize) -> Vec<u8> {
    [
        format!(
            "d6:lengthi{}e4:name5:a.txt12:piece lengthi16384e6:pieces{}:",
            pieces * 16384,
            pieces * 20
        )
        .into_bytes(),
        vec![0; pieces * 20],
        b"e".to_vec(),
    ]
    .concat()
}
fn hash(bytes: &[u8]) -> [u8; 20] {
    Sha1::digest(bytes).into()
}
fn identity(bytes: &[u8]) -> nexum_resolver::magnet::MagnetInfo {
    parse_magnet(&format!("magnet:?xt=urn:btih:{:x}", Sha1::digest(bytes))).unwrap()
}
fn frame(stream: &mut TcpStream, payload: &[u8]) {
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(payload).unwrap();
}
fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut size = [0; 4];
    stream.read_exact(&mut size).unwrap();
    let size = u32::from_be_bytes(size) as usize;
    assert!(size <= 20 * 1024);
    let mut data = vec![0; size];
    stream.read_exact(&mut data).unwrap();
    data
}
fn peer(
    expected: [u8; 20],
    run: impl FnOnce(&mut TcpStream) + Send + 'static,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut handshake = [0; 68];
        s.read_exact(&mut handshake).unwrap();
        assert_eq!(&handshake[..20], b"\x13BitTorrent protocol");
        assert_ne!(handshake[25] & 0x10, 0);
        assert_eq!(&handshake[28..48], &expected);
        handshake[48..].copy_from_slice(b"-FIXTUR-012345678901");
        s.write_all(&handshake).unwrap();
        assert_eq!(read_frame(&mut s), b"\x14\x00d1:md11:ut_metadatai1eee");
        run(&mut s);
    });
    (addr, worker)
}
fn negotiation(s: &mut TcpStream, size: usize) {
    frame(
        s,
        format!("\x14\x00d1:md11:ut_metadatai7ee13:metadata_sizei{size}ee").as_bytes(),
    );
}
fn request(s: &mut TcpStream, piece: usize) {
    assert_eq!(
        read_frame(s),
        format!("\x14\x07d8:msg_typei0e5:piecei{piece}ee").as_bytes()
    );
}
fn data(s: &mut TcpStream, piece: usize, total: usize, block: &[u8]) {
    frame(
        s,
        &[
            format!("\x14\x01d8:msg_typei1e5:piecei{piece}e10:total_sizei{total}ee").as_bytes(),
            block,
        ]
        .concat(),
    );
}
fn fetch(
    bytes: &[u8],
    addr: std::net::SocketAddr,
) -> Result<torrent::metadata::FetchedMetadata, MetadataError> {
    fetch_magnet_metadata(
        identity(bytes),
        addr,
        *b"-NX00-01234567890123",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(2),
    )
}

#[test]
fn receives_multiple_blocks_with_asymmetric_ids_and_exact_original_hash() {
    let bytes = info(1800);
    let raw = bytes.clone();
    let (addr, worker) = peer(hash(&bytes), move |s| {
        negotiation(s, raw.len());
        // Repeated extension handshakes, unknown IDs/types and keepalives are ignored.
        frame(s, b"\x14\x00d13:metadata_sizei999999999ee");
        frame(s, b"");
        frame(s, b"\x14\x63ignored");
        frame(s, b"\x14\x01d8:msg_typei99ee");
        for (index, chunk) in raw.chunks(16384).enumerate() {
            request(s, index);
            data(s, index, raw.len(), chunk);
        }
        let mut one = [0];
        assert_eq!(
            s.read(&mut one).unwrap(),
            0,
            "receiver must close before returning"
        );
    });
    let result = fetch(&bytes, addr).unwrap();
    assert_eq!(result.summary().info_hash, identity(&bytes).info_hash_hex());
    assert_eq!(result.summary().piece_count, 1800);
    let (envelope, summary) = result.into_parts();
    assert_eq!(envelope, [b"d4:info".as_slice(), &bytes, b"e"].concat());
    assert_eq!(
        torrent::inspect_metainfo(
            &envelope,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap(),
        summary
    );
    worker.join().unwrap();
}

#[test]
fn rejects_oversized_advertisements_and_frames_before_requesting_bytes() {
    for size in [0, torrent::MAX_METAINFO_BYTES - 7, i64::MAX as usize] {
        let bytes = info(1);
        let (addr, worker) = peer(hash(&bytes), move |s| {
            negotiation(s, size);
            let mut byte = [0];
            assert_eq!(s.read(&mut byte).unwrap(), 0);
        });
        let result = fetch(&bytes, addr);
        assert!(
            matches!(result, Err(MetadataError::TooLarge)),
            "size {size}: {result:?}"
        );
        worker.join().unwrap();
    }
    let bytes = info(1);
    let (addr, worker) = peer(hash(&bytes), |s| {
        frame(
            s,
            b"\x14\x00d1:md11:ut_metadatai7ee13:metadata_sizei18446744073709551615ee",
        );
        let mut b = [0];
        assert_eq!(s.read(&mut b).unwrap(), 0);
    });
    assert!(matches!(
        fetch(&bytes, addr),
        Err(MetadataError::InvalidMessage)
    ));
    worker.join().unwrap();
    let bytes = info(1);
    let (addr, worker) = peer(hash(&bytes), |s| {
        s.write_all(&(20_u32 * 1024 + 1).to_be_bytes()).unwrap();
        let mut b = [0];
        assert_eq!(s.read(&mut b).unwrap(), 0);
    });
    assert!(matches!(
        fetch(&bytes, addr),
        Err(MetadataError::FrameLimit)
    ));
    worker.join().unwrap();
}

#[test]
fn rejects_wrong_piece_total_length_reject_and_identity() {
    for variant in 0..5 {
        let bytes = info(1);
        let raw = bytes.clone();
        let (addr, worker) = peer(hash(&bytes), move |s| {
            negotiation(s, raw.len());
            request(s, 0);
            match variant {
                0 => data(s, 1, raw.len(), &raw),
                1 => data(s, 0, raw.len() + 1, &raw),
                2 => data(s, 0, raw.len(), &raw[..raw.len() - 1]),
                3 => frame(s, b"\x14\x01d8:msg_typei2e5:piecei0ee"),
                _ => {
                    let mut changed = raw.clone();
                    changed[20] ^= 1;
                    data(s, 0, raw.len(), &changed);
                }
            }
        });
        let expected = match variant {
            3 => MetadataError::Rejected,
            4 => MetadataError::IdentityMismatch,
            _ => MetadataError::InvalidMessage,
        };
        assert_eq!(
            fetch(&bytes, addr).unwrap_err(),
            expected,
            "variant {variant}"
        );
        worker.join().unwrap();
    }
}

#[test]
fn ignores_unknown_types_but_rejects_incoming_requests_without_uploading() {
    let bytes = info(1);
    let raw = bytes.clone();
    let (addr, worker) = peer(hash(&bytes), move |s| {
        negotiation(s, raw.len());
        request(s, 0);
        frame(s, b"\x14\x01d8:msg_typei99ee");
        frame(s, b"\x14\x01d8:msg_typei-1ee");
        frame(s, b"\x14\x01d8:msg_typei0e5:piecei4ee");
        assert_eq!(read_frame(s), b"\x14\x07d8:msg_typei2e5:piecei4ee");
        data(s, 0, raw.len(), &raw);
    });
    assert!(fetch(&bytes, addr).is_ok());
    worker.join().unwrap();
}

#[test]
fn cancellation_and_absolute_deadline_close_stalled_sockets() {
    for cancelled in [true, false] {
        let bytes = info(1);
        let raw = bytes.clone();
        let (tx, rx) = mpsc::channel();
        let (addr, worker) = peer(hash(&bytes), move |s| {
            negotiation(s, raw.len());
            request(s, 0);
            tx.send(()).unwrap();
            let mut one = [0];
            assert_eq!(s.read(&mut one).unwrap(), 0);
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let control = Arc::clone(&cancel);
        let deadline = Instant::now()
            + if cancelled {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(250)
            };
        let caller = thread::spawn(move || {
            fetch_magnet_metadata(
                identity(&bytes),
                addr,
                *b"-NX00-01234567890123",
                &control,
                deadline,
            )
        });
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        if cancelled {
            cancel.store(true, Ordering::Release);
        }
        let result = caller.join().unwrap();
        assert!(
            matches!(result, Err(MetadataError::Cancelled)) && cancelled
                || matches!(result, Err(MetadataError::DeadlineExceeded)) && !cancelled
        );
        worker.join().unwrap();
    }
}

#[test]
fn validates_before_connect_and_bounds_ignored_traffic() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let bytes = info(1);
    assert!(matches!(
        fetch_magnet_metadata(
            identity(&bytes),
            listener.local_addr().unwrap(),
            [0; 20],
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(1)
        ),
        Err(MetadataError::Cancelled)
    ));
    assert!(listener.accept().is_err());
    let (addr, worker) = peer(hash(&bytes), |s| {
        for _ in 0..256 {
            if s.write_all(&[0; 4]).is_err() {
                break;
            }
        }
    });
    assert!(matches!(
        fetch(&bytes, addr),
        Err(MetadataError::FrameLimit)
    ));
    worker.join().unwrap();
}
