use nexum_resolver::magnet::{MagnetError, parse_magnet};
use nexum_resolver::{ResolveKind, ResolveRequest, ResolveResult, ResolverRegistry};

const HEX: &str = "0123456789abcdef0123456789abcdef01234567";
const BASE32: &str = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH";
const URI: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

#[test]
fn normalizes_independent_hex_and_base32_vectors() {
    let expected = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef, 0x01, 0x23, 0x45, 0x67,
    ];
    for source in [
        URI.to_owned(),
        format!("MAGNET:?xt=URN:BTIH:{}", HEX.to_uppercase()),
        format!("magnet:?xt=urn:btih:{BASE32}"),
        format!("magnet:?xt=urn:btih:{}", BASE32.to_lowercase()),
    ] {
        let parsed = parse_magnet(&source).unwrap();
        assert_eq!(parsed.info_hash(), &expected);
        assert_eq!(parsed.info_hash_hex(), HEX);
    }
    let parsed = parse_magnet("magnet:?xt=urn:btih:AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQT").unwrap();
    assert_eq!(
        parsed.info_hash(),
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19
        ]
    );
    assert_eq!(
        parsed.info_hash_hex(),
        "000102030405060708090a0b0c0d0e0f10111213"
    );
}

#[test]
fn coalesces_equal_topics_and_rejects_conflicts_in_both_orders() {
    let parsed = parse_magnet(&format!(
        "{URI}&xt=URN:BTIH:{}&xt=urn:btih:{BASE32}",
        HEX.to_uppercase()
    ))
    .unwrap();
    assert_eq!(parsed.info_hash_hex(), HEX);
    let other = "000102030405060708090a0b0c0d0e0f10111213";
    for source in [
        format!("{URI}&xt=urn:btih:{other}"),
        format!("magnet:?xt=urn:btih:{other}&xt=urn:btih:{HEX}"),
        format!("magnet:?xt=urn:btih:{BASE32}&xt=urn:btih:{other}"),
    ] {
        assert_eq!(
            parse_magnet(&source),
            Err(MagnetError::ConflictingInfoHashes)
        );
    }
    assert_eq!(
        parse_magnet(&format!("{URI}&xt=urn:btih:abc")),
        Err(MagnetError::InvalidInfoHash)
    );
    assert_eq!(
        parse_magnet(&format!("magnet:?xt=urn:btih:abc&xt=urn:btih:{HEX}")),
        Err(MagnetError::InvalidInfoHash)
    );
}

#[test]
fn rejects_malformed_hash_alphabets_lengths_and_namespaces() {
    for value in [
        "",
        "abc",
        "urn:btih:",
        "urn:btih:abc",
        "urn:sha1:0123456789abcdef0123456789abcdef01234567",
        "x:btih:0123456789abcdef0123456789abcdef01234567",
        "urn:btih:0123456789abcdef0123456789abcdef0123456g",
        "urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRL0",
        "urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRL=",
        "urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRL8",
    ] {
        assert_eq!(
            parse_magnet(&format!("magnet:?xt={value}")),
            Err(MagnetError::InvalidInfoHash)
        );
    }
    for length in [31, 33, 39, 41] {
        assert_eq!(
            parse_magnet(&format!("magnet:?xt=urn:btih:{}", "a".repeat(length))),
            Err(MagnetError::InvalidInfoHash)
        );
    }
}

#[test]
fn rejects_v2_mixed_and_indexed_topics_without_dropping_an_identity() {
    for source in [
        "magnet:?xt=urn:btmh:1220aa".to_owned(),
        format!("{URI}&xt=urn:btmh:1220aa"),
        format!("magnet:?xt=urn:btmh:1220aa&xt=urn:btih:{HEX}"),
    ] {
        assert_eq!(parse_magnet(&source), Err(MagnetError::UnsupportedV2));
    }
    for key in ["xt.1", "xt.0", "xt.01", "%78t.1", "x%74.2", "xt."] {
        for prefix in ["magnet:?".to_owned(), format!("{URI}&")] {
            for hash in [HEX, "000102030405060708090a0b0c0d0e0f10111213"] {
                assert_eq!(
                    parse_magnet(&format!("{prefix}{key}=urn:btih:{hash}")),
                    Err(MagnetError::UnsupportedIndexedTopic)
                );
            }
        }
    }
}

#[test]
fn decodes_once_after_splitting_delimiters_and_keeps_values_opaque() {
    let encoded = "magnet:?%78%74=urn%3Abtih%3A0123456789abcdef0123456789abcdef01234567";
    assert_eq!(parse_magnet(encoded).unwrap().info_hash_hex(), HEX);
    assert_eq!(
        parse_magnet(&format!(
            "{URI}&dn=a=b&x=%26xt%3Durn%3Abtih%3Aother&so=0-18446744073709551615"
        ))
        .unwrap()
        .info_hash_hex(),
        HEX
    );
    for source in [
        format!("magnet:?xt=urn%253Abtih%253A{HEX}"),
        format!("magnet:?xt=urn:btih:{HEX}%26xt%3Durn:btih:other"),
        format!("magnet:?xt=urn:btih:{}+", &HEX[..39]),
    ] {
        assert_eq!(parse_magnet(&source), Err(MagnetError::InvalidInfoHash));
    }
    assert_eq!(
        parse_magnet(&format!("magnet:?%2578t=urn:btih:{HEX}")),
        Err(MagnetError::InvalidParameter)
    );
}

#[test]
fn validates_ignored_parameters_and_requires_well_formed_utf8() {
    for suffix in [
        "x=%",
        "x=%0",
        "x=%ZZ",
        "x=%FF",
        "x=%C0%80",
        "x=%ED%A0%80",
        "x=%F4%90%80%80",
        "x=%00",
        "x=%0A",
        "x=%7F",
        "x=%C2%85",
        "%FF=x",
        "%0A=x",
    ] {
        assert_eq!(
            parse_magnet(&format!("{URI}&{suffix}")),
            Err(MagnetError::InvalidEncoding)
        );
    }
    for suffix in ["&", "=x", "x", "x=%26&", "x%26y=z", "x+y=z"] {
        assert_eq!(
            parse_magnet(&format!("{URI}&{suffix}")),
            Err(MagnetError::InvalidParameter)
        );
    }
    assert_eq!(
        parse_magnet(&format!(
            "{URI}&dn=&tr=udp%3A%2F%2Ftracker.invalid%3A80&x.pe=%5B%3A%3A1%5D%3A6881"
        ))
        .unwrap()
        .info_hash_hex(),
        HEX
    );
}

#[test]
fn rejects_fragments_authority_paths_controls_and_raw_query_whitespace() {
    for source in [
        format!("{URI}#fragment"),
        format!("magnet://host/?xt=urn:btih:{HEX}"),
        format!("magnet:path?xt=urn:btih:{HEX}"),
        format!("{URI}&dn=a b"),
        format!("{URI}&dn=\\path"),
        format!("{URI}&dn=\"name\""),
        format!("{URI}&tr=http://[::1]"),
        format!("{URI}\n"),
        format!("\t{URI}"),
        format!("{URI}&dn=\u{85}"),
        format!("{URI}&dn=a\u{2000}b"),
    ] {
        assert_eq!(parse_magnet(&source), Err(MagnetError::InvalidUri));
    }
    for source in ["", "http:?xt=urn:btih:abc", "magnet:", "magnet"] {
        assert_eq!(parse_magnet(source), Err(MagnetError::InvalidUri));
    }
    assert_eq!(parse_magnet("magnet:?"), Err(MagnetError::MissingInfoHash));
}

#[test]
fn accepts_unicode_parameters_and_bounded_outer_whitespace() {
    for suffix in [
        "dn=下载文件",
        "dn=%E4%B8%AD%E6%96%87",
        "dn=a+b",
        "dn=a%2Bb",
        "dn=a%20b",
        "x=ignored&x=again",
    ] {
        assert_eq!(
            parse_magnet(&format!("{URI}&{suffix}"))
                .unwrap()
                .info_hash_hex(),
            HEX
        );
    }
    let padded = format!(" \u{2000}{URI}\u{a0} ");
    assert_eq!(parse_magnet(&padded).unwrap().info_hash_hex(), HEX);
    let request = ResolveRequest::new(padded).unwrap();
    assert_eq!(request.source(), URI);
    assert_eq!(
        ResolverRegistry::new().resolve(&request).unwrap().source,
        URI
    );
}

#[test]
fn applies_the_raw_byte_cap_before_trimming_in_both_public_entrypoints() {
    let prefix = format!("{URI}&dn=");
    let exact = format!("{prefix}{}", "a".repeat(8192 - prefix.len()));
    assert_eq!(exact.len(), 8192);
    assert_eq!(parse_magnet(&exact).unwrap().info_hash_hex(), HEX);
    let request = ResolveRequest::new(exact.clone()).unwrap();
    assert!(ResolverRegistry::new().resolve(&request).is_ok());
    for oversized in [
        format!("{exact}a"),
        format!("{}{}", " ".repeat(8193 - URI.len()), URI),
        format!("{}{}", URI, " ".repeat(8193 - URI.len())),
        format!("{prefix}{}", "中".repeat(2731)),
    ] {
        assert!(oversized.len() > 8192);
        assert_eq!(parse_magnet(&oversized), Err(MagnetError::TooLong));
        assert!(matches!(
            ResolveRequest::new(oversized),
            Err(nexum_resolver::ResolverError::InvalidSource(_))
        ));
    }
    assert!(ResolveRequest::new(format!("{URI}\n")).is_err());
    assert!(ResolveRequest::new(format!(" \t{URI}")).is_err());
}

#[test]
fn treats_query_keys_as_case_sensitive_and_requires_one_supported_identity() {
    for source in [
        "magnet:?dn=name".to_owned(),
        format!("magnet:?XT=urn:btih:{HEX}"),
        format!("magnet:?xXt=urn:btih:{HEX}"),
    ] {
        assert_eq!(parse_magnet(&source), Err(MagnetError::MissingInfoHash));
    }
    assert_eq!(
        parse_magnet(&format!("{URI}&XT=unknown&xs=ignored"))
            .unwrap()
            .info_hash_hex(),
        HEX
    );
}

#[test]
fn debug_and_errors_hide_magnet_secrets_but_explicit_source_access_is_unchanged() {
    let source = format!("{URI}&tr=https://tracker.invalid/PASSKEY_SENTINEL&dn=NAME_SENTINEL");
    let request = ResolveRequest::new(source.clone()).unwrap();
    let result = ResolverRegistry::new().resolve(&request).unwrap();
    assert_eq!(request.source(), source);
    assert_eq!(result.source, source);
    for rendered in [
        format!("{request:?}"),
        format!("{result:?}"),
        format!("{:?}", parse_magnet(&source).unwrap()),
        format!(
            "{:?}",
            ResolveResult {
                kind: ResolveKind::Magnet,
                source: "PASSKEY_SENTINEL".into()
            }
        ),
    ] {
        assert!(!rendered.contains("PASSKEY_SENTINEL"));
        assert!(!rendered.contains("NAME_SENTINEL"));
        assert!(!rendered.contains("tracker.invalid"));
    }
    let invalid = "magnet:?xt=urn:btih:PASSKEY_SENTINEL&dn=NAME_SENTINEL";
    let request = ResolveRequest::new(invalid).unwrap();
    let error = ResolverRegistry::new().resolve(&request).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("PASSKEY_SENTINEL"));
    assert!(!format!("{error:?} {error}").contains("NAME_SENTINEL"));
    assert_eq!(
        ResolveRequest::new(" \tHTTPS://example.invalid/file\n")
            .unwrap()
            .source(),
        "HTTPS://example.invalid/file"
    );
    let http = ResolveRequest::new("  HtTpS://example.invalid/file  ").unwrap();
    let result = ResolverRegistry::new().resolve(&http).unwrap();
    assert_eq!(result.kind, ResolveKind::Https);
    assert_eq!(result.source, "HtTpS://example.invalid/file");
    assert!(format!("{result:?}").contains("example.invalid"));
}

#[test]
fn rejects_control_obfuscated_magnet_schemes_and_redacts_their_results() {
    let source = format!("{URI}&tr=https://tracker.invalid/PASSKEY_SENTINEL");
    let suffix = source.split_once(':').unwrap().1;
    for code in 0..=0x9f {
        let character = char::from_u32(code).unwrap();
        if !character.is_control() {
            continue;
        }
        for boundary in 0..=6 {
            let mut scheme = "MaGnEt".to_owned();
            scheme.insert(boundary, character);
            let invalid = format!("{scheme}:{suffix}");
            assert_eq!(parse_magnet(&invalid), Err(MagnetError::InvalidUri));
            assert!(
                ResolveRequest::new(invalid.clone()).is_err(),
                "control-obfuscated Magnet must be rejected before constructing a request"
            );
            let result = ResolveResult {
                kind: ResolveKind::Http,
                source: invalid,
            };
            assert!(!format!("{result:?}").contains("PASSKEY_SENTINEL"));
        }
        let padded = format!(" \u{2000}{character}{source}");
        assert!(ResolveRequest::new(padded).is_err());
    }
    for invalid in [
        format!("ma gnet:{suffix}"),
        format!("ma\u{2000}gnet:{suffix}"),
    ] {
        assert_eq!(parse_magnet(&invalid), Err(MagnetError::InvalidUri));
        let request = ResolveRequest::new(invalid).unwrap();
        assert!(ResolverRegistry::new().resolve(&request).is_err());
        assert!(!format!("{request:?}").contains("PASSKEY_SENTINEL"));
    }
    let oversized = format!("{}\0{source}", " ".repeat(8193));
    assert_eq!(parse_magnet(&oversized), Err(MagnetError::TooLong));
    assert!(ResolveRequest::new(oversized).is_err());
}
