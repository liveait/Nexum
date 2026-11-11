use nexum_resolver::torrent::*;
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn string(bytes: &[u8]) -> Vec<u8> {
    [format!("{}:", bytes.len()).as_bytes(), bytes].concat()
}
fn integer(number: i64) -> Vec<u8> {
    format!("i{number}e").into_bytes()
}
fn dict(mut fields: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    fields.sort_by_key(|(key, _)| *key);
    let mut out = b"d".to_vec();
    for (key, value) in fields {
        out.extend(string(key.as_bytes()));
        out.extend(value);
    }
    out.push(b'e');
    out
}
fn list(items: Vec<Vec<u8>>) -> Vec<u8> {
    [b"l".to_vec(), items.concat(), b"e".to_vec()].concat()
}
fn single_fields() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("length", integer(3)),
        ("name", string(b"a.txt")),
        ("piece length", integer(4)),
        ("pieces", string(&[0; 20])),
    ]
}
fn metainfo(info: Vec<u8>) -> Vec<u8> {
    dict(vec![("info", info)])
}
fn inspect(bytes: &[u8]) -> Result<MetainfoSummary, MetainfoError> {
    inspect_metainfo(
        bytes,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
    )
}
fn multi(paths: &[(&[&str], i64, bool)]) -> Vec<u8> {
    let files = paths
        .iter()
        .map(|(path, size, pad)| {
            let mut fields = vec![
                ("length", integer(*size)),
                (
                    "path",
                    list(path.iter().map(|s| string(s.as_bytes())).collect()),
                ),
            ];
            if *pad {
                fields.push(("attr", string(b"p")));
            }
            dict(fields)
        })
        .collect();
    let total = paths
        .iter()
        .map(|(_, length, _)| *length as usize)
        .sum::<usize>();
    metainfo(dict(vec![
        ("files", list(files)),
        ("name", string(b"root")),
        ("piece length", integer(4)),
        ("pieces", string(&vec![0; total.div_ceil(4) * 20])),
    ]))
}

#[test]
fn exact_raw_info_identity_includes_unknown_binary_fields() {
    let mut fields = single_fields();
    fields.push(("extra", string(&[0xff, 0xfe])));
    let info = dict(fields);
    let original = metainfo(info.clone());
    let result = inspect(&original).unwrap();
    // Independently computed with Python hashlib.sha1 on this exact raw span.
    assert_eq!(result.info_hash, "ceebd007914a244e3273ef980f026f2547d8b355");
    let changed = dict(vec![
        (
            "announce",
            string(b"https://tracker.invalid/sentinel-passkey"),
        ),
        ("info", info),
    ]);
    let second = inspect(&changed).unwrap();
    assert_eq!(result.info_hash, second.info_hash);
    assert_ne!(result.metainfo_sha256, second.metainfo_sha256);
    assert_eq!(result.files[0].path, ["a.txt"]);
    assert_eq!(result.files[0].length, 3);
    assert!(!format!("{second:?}").contains("sentinel-passkey"));
}

#[test]
fn preserves_stable_multifile_indices_padding_and_empty_files() {
    let bytes = multi(&[
        (&["dir", "a"], 3, false),
        (&[".pad", "1"], 1, true),
        (&["empty"], 0, false),
    ]);
    let result = inspect(&bytes).unwrap();
    assert_eq!(result.total_bytes, 4);
    assert_eq!(result.piece_count, 1);
    assert_eq!(
        result.files.iter().map(|f| f.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(result.files[1].padding);
    assert_eq!(result.files[2].length, 0);
    let mut fields = single_fields();
    fields.retain(|(key, _)| !matches!(*key, "length" | "pieces"));
    fields.extend([("length", integer(0)), ("pieces", string(b""))]);
    assert_eq!(inspect(&metainfo(dict(fields))).unwrap().piece_count, 0);
}

#[test]
fn rejects_noncanonical_and_ambiguous_bencode_without_echo() {
    for input in [
        b"i-0e".as_slice(),
        b"i01e",
        b"i+1e",
        b"i9223372036854775808e",
        b"01:x",
        b"2:x",
        b"d1:bi1e1:ai2ee",
        b"d1:ai1e1:ai2ee",
        b"di1ei2ee",
        b"l",
        b"999999999999999999999999:",
    ] {
        assert_eq!(
            inspect(input),
            Err(MetainfoError::InvalidEncoding),
            "{input:?}"
        );
    }
    let mut bytes = metainfo(dict(single_fields()));
    bytes.extend(b"sentinel-passkey");
    let error = inspect(&bytes).unwrap_err();
    assert_eq!(error, MetainfoError::InvalidEncoding);
    assert_eq!(error.to_string(), "invalid_bencode");
}

#[test]
fn checks_lengths_piece_geometry_private_and_versions() {
    for (key, value) in [
        ("length", integer(-1)),
        ("piece length", integer(0)),
        ("pieces", string(b"bad")),
        ("pieces", string(&[0; 40])),
        ("private", integer(2)),
    ] {
        let mut fields = single_fields();
        fields.retain(|(k, _)| *k != key);
        fields.push((key, value));
        assert_eq!(
            inspect(&metainfo(dict(fields))),
            Err(MetainfoError::InvalidMetainfo)
        );
    }
    let mut fields = single_fields();
    fields.push(("private", integer(1)));
    assert!(inspect(&metainfo(dict(fields))).unwrap().private);
    let mut fields = single_fields();
    fields.push(("meta version", integer(2)));
    assert_eq!(
        inspect(&metainfo(dict(fields))),
        Err(MetainfoError::UnsupportedVersion)
    );
    let mut fields = single_fields();
    fields.push(("files", list(vec![])));
    assert_eq!(
        inspect(&metainfo(dict(fields))),
        Err(MetainfoError::InvalidMetainfo)
    );
    for lengths in [vec![i64::MAX, 1], vec![i64::MAX; 3]] {
        // Model wide declared geometry in u128 without allocating file data.
        // Otherwise the fixture overflows before the product is exercised.
        let total = lengths.iter().map(|size| *size as u128).sum::<u128>();
        let count = total.div_ceil(i64::MAX as u128) as usize;
        let files = lengths
            .iter()
            .enumerate()
            .map(|(index, length)| {
                dict(vec![
                    ("length", integer(*length)),
                    ("path", list(vec![string(format!("f{index}").as_bytes())])),
                ])
            })
            .collect();
        let bytes = metainfo(dict(vec![
            ("files", list(files)),
            ("name", string(b"root")),
            ("piece length", integer(i64::MAX)),
            ("pieces", string(&vec![0; count * 20])),
        ]));
        assert_eq!(inspect(&bytes), Err(MetainfoError::InvalidMetainfo));
    }
}

#[test]
fn rejects_unsafe_paths_symlinks_and_canonical_aliases() {
    for name in [
        "", ".", "..", "/a", "a/b", "a\\b", "C:a", "a\0b", "a\nb", "a.", "a ", "CON.txt", "LPT1",
    ] {
        let mut fields = single_fields();
        fields.retain(|(k, _)| *k != "name");
        fields.push(("name", string(name.as_bytes())));
        assert_eq!(
            inspect(&metainfo(dict(fields))),
            Err(MetainfoError::UnsafePath),
            "{name:?}"
        );
    }
    for paths in [
        vec![(&["a"][..], 0, false), (&["A"][..], 0, false)],
        vec![(&["dir"][..], 0, false), (&["dir", "a"][..], 0, false)],
        vec![(&["é"][..], 0, false), (&["e\u{301}"][..], 0, false)],
        vec![(&["Straße"][..], 0, false), (&["STRASSE"][..], 0, false)],
    ] {
        assert_eq!(inspect(&multi(&paths)), Err(MetainfoError::UnsafePath));
    }
    let mut fields = single_fields();
    fields.push(("attr", string(b"l")));
    assert_eq!(
        inspect(&metainfo(dict(fields))),
        Err(MetainfoError::UnsafePath)
    );
    let mut fields = single_fields();
    fields.retain(|(k, _)| *k != "name");
    fields.push(("name", string(&[0xff])));
    assert_eq!(
        inspect(&metainfo(dict(fields))),
        Err(MetainfoError::UnsafePath)
    );
}

#[test]
fn enforces_raw_file_piece_and_component_limits() {
    assert_eq!(
        inspect(&vec![b'x'; MAX_METAINFO_BYTES + 1]),
        Err(MetainfoError::TooLarge)
    );
    let mut fields = single_fields();
    fields.retain(|(k, _)| !matches!(*k, "length" | "pieces"));
    fields.extend([
        ("length", integer((MAX_PIECES * 4) as i64)),
        ("pieces", string(&vec![0; MAX_PIECES * 20])),
    ]);
    assert_eq!(
        inspect(&metainfo(dict(fields))).unwrap().piece_count,
        MAX_PIECES
    );
    let mut fields = single_fields();
    fields.retain(|(k, _)| !matches!(*k, "length" | "pieces"));
    fields.extend([
        ("length", integer(((MAX_PIECES + 1) * 4) as i64)),
        ("pieces", string(&vec![0; (MAX_PIECES + 1) * 20])),
    ]);
    assert_eq!(
        inspect(&metainfo(dict(fields))),
        Err(MetainfoError::ResourceLimit)
    );
    let names = (0..=MAX_FILES).map(|i| format!("f{i}")).collect::<Vec<_>>();
    let paths = names.iter().map(|s| vec![s.as_str()]).collect::<Vec<_>>();
    let entries = paths
        .iter()
        .map(|p| (p.as_slice(), 0, false))
        .collect::<Vec<_>>();
    assert_eq!(
        inspect(&multi(&entries[..MAX_FILES])).unwrap().files.len(),
        MAX_FILES
    );
    assert_eq!(inspect(&multi(&entries)), Err(MetainfoError::ResourceLimit));
    let long = "x".repeat(MAX_COMPONENT_BYTES + 1);
    assert!(inspect(&multi(&[(&[&long[..MAX_COMPONENT_BYTES]], 0, false)])).is_ok());
    assert_eq!(
        inspect(&multi(&[(&[&long], 0, false)])),
        Err(MetainfoError::UnsafePath)
    );
    let path = vec!["dir"; MAX_PATH_DEPTH + 1];
    assert!(inspect(&multi(&[(&path[..MAX_PATH_DEPTH], 0, false)])).is_ok());
    assert_eq!(
        inspect(&multi(&[(&path, 0, false)])),
        Err(MetainfoError::UnsafePath)
    );
}

#[test]
fn enforces_nesting_nodes_deadlines_and_cancellation() {
    let at_depth = [
        vec![b'l'; MAX_DEPTH - 1],
        b"0:".to_vec(),
        vec![b'e'; MAX_DEPTH - 1],
    ]
    .concat();
    assert_eq!(inspect(&at_depth), Err(MetainfoError::InvalidMetainfo));
    let deep = [vec![b'l'; MAX_DEPTH], b"0:".to_vec(), vec![b'e'; MAX_DEPTH]].concat();
    assert_eq!(inspect(&deep), Err(MetainfoError::ResourceLimit));
    let many = [b"l".to_vec(), b"0:".repeat(MAX_NODES), b"e".to_vec()].concat();
    assert_eq!(inspect(&many), Err(MetainfoError::ResourceLimit));
    let at_nodes = [b"l".to_vec(), b"0:".repeat(MAX_NODES - 1), b"e".to_vec()].concat();
    assert_eq!(inspect(&at_nodes), Err(MetainfoError::InvalidMetainfo));
    let bytes = metainfo(dict(single_fields()));
    assert_eq!(
        inspect_metainfo(
            &bytes,
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(1)
        ),
        Err(MetainfoError::Cancelled)
    );
    assert_eq!(
        inspect_metainfo(&bytes, &AtomicBool::new(false), Instant::now()),
        Err(MetainfoError::DeadlineExceeded)
    );
}
