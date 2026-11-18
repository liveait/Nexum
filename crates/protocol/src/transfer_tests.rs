use super::*;
use serde_json::json;

#[test]
fn generic_request_debug_redacts_raw_credential_parameters() {
    let request = RpcRequest::new(
        1,
        "task.create_transfer",
        Some(json!({"password":"PROTOCOL_PASSWORD_SENTINEL","username":"PROTOCOL_USER_SENTINEL"})),
    );
    let debug = format!("{request:?}");
    assert!(!debug.contains("PROTOCOL_PASSWORD_SENTINEL"));
    assert!(!debug.contains("PROTOCOL_USER_SENTINEL"));
}

fn input(url: &str) -> Value {
    json!({"version":1,"id":"ftp-one","destination":"/tmp/ftp-one", "source":{"kind":"ftp","source_url":url,"security":"plain","authentication":{"kind":"anonymous"}}})
}
#[test]
fn typed_ftp_preserves_literal_path_and_explicit_root_semantics() {
    let request =
        transfer::CreateTransferRequest::parse(&input("ftp://EXAMPLE.com:21/pub/a%20b.bin"))
            .unwrap();
    assert_eq!(
        request.origin.canonical_url,
        "ftp://example.com/pub/a%20b.bin"
    );
    assert_eq!(request.origin.command_path, "pub/a b.bin");
    let mut value = input("ftp://example.com/pub/a.bin");
    value["source"]["path_mode"] = json!("absolute");
    assert_eq!(
        transfer::CreateTransferRequest::parse(&value)
            .unwrap()
            .origin
            .command_path,
        "/pub/a.bin"
    );
}
#[test]
fn typed_ftp_rejects_hostile_urls_and_unknown_fields_without_reflection() {
    for url in [
        "ftp://USER_SENTINEL:PASS_SENTINEL@example.com/a",
        "ftps://example.com/a",
        "ftp://example.com/a%2fb",
        "ftp://example.com/a%5cb",
        "ftp://example.com/a%00b",
        "ftp://example.com/a%0db",
        "ftp://example.com/a%ff",
        "ftp://example.com/a/../b",
        "ftp://example.com/%2e%2e/b",
        "ftp://example.com/a\\b",
        "ftp://example.com:0/a",
        "ftp://example.com/a?q=SECRET_SENTINEL",
        "ftp://example.com/a#SECRET_SENTINEL",
    ] {
        let error = transfer::CreateTransferRequest::parse(&input(url)).unwrap_err();
        assert!(!format!("{error:?}").contains("SENTINEL"));
    }
    let mut value = input("ftp://example.com/a");
    value["SECRET_SENTINEL"] = json!(true);
    assert!(transfer::CreateTransferRequest::parse(&value).is_err());
    value = input("ftp://example.com/a");
    value["version"] = json!(2);
    assert!(transfer::CreateTransferRequest::parse(&value).is_err());
}
fn client_input(url: &str) -> Value {
    let mut value = input(url);
    value["source"]
        .as_object_mut()
        .unwrap()
        .remove("authentication");
    value
}

#[test]
fn client_normalization_extracts_userinfo_and_redacts_typed_secrets() {
    let value = client_input("ftp://USER_SENTINEL:p%40ss%3F%23%5C@example.com/a");
    let normalized = transfer::CreateTransferRequest::normalize_client(&value).unwrap();
    assert_eq!(normalized["source"]["source_url"], "ftp://example.com/a");
    let request = transfer::CreateTransferRequest::parse(&normalized).unwrap();
    assert!(!format!("{request:?}").contains("USER_SENTINEL"));
    assert!(!format!("{request:?}").contains("p@ss"));
    let secret = request.secret.unwrap();
    assert_eq!(secret.username, "USER_SENTINEL");
    assert_eq!(secret.password, "p@ss?#\\");
}

#[test]
fn client_normalization_rejects_raw_delimiters_before_extracting_userinfo() {
    for url in [
        "ftp://example.org:21?QUERY_SENTINEL=@mirror.example/file",
        "ftp://example.org:21#FRAGMENT_SENTINEL@mirror.example/file",
        "ftp://user:BACKSLASH_SENTINEL\\pass@mirror.example/file",
        "ftp://user:CONTROL_SENTINEL\u{001f}@mirror.example/file",
    ] {
        let result = transfer::CreateTransferRequest::normalize_client(&client_input(url));
        assert!(result.is_err(), "accepted malformed FTP URL");
        assert!(!result.unwrap_err().message.contains("SENTINEL"));
    }
}

#[test]
fn double_slash_cannot_bypass_explicit_absolute_path_mode() {
    assert!(
        transfer::CreateTransferRequest::parse(&input("ftp://example.com//pub/a.bin")).is_err()
    );
    let request =
        transfer::CreateTransferRequest::parse(&input("ftp://[::1]:2121/pub/%252F.bin")).unwrap();
    assert_eq!(request.origin.origin, "ftp://[::1]:2121");
    assert_eq!(request.origin.command_path, "pub/%2F.bin");
    assert_eq!(
        request.origin.canonical_url,
        "ftp://[::1]:2121/pub/%252F.bin"
    );
}
