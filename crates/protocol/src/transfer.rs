//! Versioned write-only FTP input. Parsing errors never include submitted values.
use crate::RpcErrorObject;
use nexum_core::nexum_storage::origin::OriginRecord;
use serde_json::{Map, Value, json};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct OriginSecret {
    pub username: String,
    pub password: String,
}
impl fmt::Debug for OriginSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OriginSecret([REDACTED])")
    }
}
#[derive(Clone, Debug)]
pub struct CreateTransferRequest {
    pub origin: OriginRecord,
    pub secret: Option<OriginSecret>,
}
fn invalid() -> RpcErrorObject {
    RpcErrorObject::invalid_params("invalid transfer v1 input")
}
fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, RpcErrorObject> {
    value
        .as_object()
        .filter(|value| value.keys().all(|key| fields.contains(&key.as_str())))
        .ok_or_else(invalid)
}
fn string<'a>(value: &'a Map<String, Value>, key: &str) -> Result<&'a str, RpcErrorObject> {
    value.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn safe(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}
fn decode(value: &str) -> Result<String, RpcErrorObject> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut source = value.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let high = source
                .next()
                .and_then(|byte| (byte as char).to_digit(16))
                .ok_or_else(invalid)?;
            let low = source
                .next()
                .and_then(|byte| (byte as char).to_digit(16))
                .ok_or_else(invalid)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
    if decoded.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(decoded)
}
fn encode_component(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}
fn ftp_url(raw: &str, path_mode: &str) -> Result<(String, String, String), RpcErrorObject> {
    if !safe(raw) || raw.contains(['\\', '?', '#']) {
        return Err(invalid());
    }
    let (scheme, remainder) = raw.split_once("://").ok_or_else(invalid)?;
    if !scheme.eq_ignore_ascii_case("ftp") {
        return Err(invalid());
    }
    let (authority, path) = remainder.split_once('/').ok_or_else(invalid)?;
    if authority.is_empty()
        || authority.contains(['@', '%'])
        || !safe(path)
        || path.starts_with('/')
    {
        return Err(invalid());
    }
    // Validate raw components before URL libraries can normalize dot segments or backslashes.
    let mut components = Vec::new();
    for component in path.split('/') {
        let decoded = decode(component)?;
        if decoded.contains(['/', '\\']) || matches!(decoded.as_str(), "." | "..") {
            return Err(invalid());
        }
        components.push(decoded);
    }
    if components.last().is_none_or(|value| value.is_empty()) {
        return Err(invalid());
    }
    let parsed = url::Url::parse(&format!("ftp://{authority}/")).map_err(|_| invalid())?;
    if parsed.host_str().is_none()
        || parsed.port() == Some(0)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(invalid());
    }
    let host = parsed.host_str().ok_or_else(invalid)?;
    let port = parsed.port_or_known_default().ok_or_else(invalid)?;
    let canonical_authority = if port == 21 {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    };
    let command_path = components.join("/");
    let canonical_path = components
        .iter()
        .map(|value| encode_component(value))
        .collect::<Vec<_>>()
        .join("/");
    Ok((
        format!("ftp://{canonical_authority}/{canonical_path}"),
        format!("ftp://{host}:{port}"),
        if path_mode == "absolute" {
            format!("/{command_path}")
        } else {
            command_path
        },
    ))
}
impl CreateTransferRequest {
    pub fn parse(value: &Value) -> Result<Self, RpcErrorObject> {
        let value = object(value, &["version", "id", "destination", "source"])?;
        if value.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(invalid());
        }
        let id = string(value, "id")?;
        let destination = string(value, "destination")?;
        if !safe(id) || id.len() > 256 || !safe(destination) || destination.len() > 32768 {
            return Err(invalid());
        }
        let source = object(
            value.get("source").ok_or_else(invalid)?,
            &[
                "kind",
                "source_url",
                "security",
                "path_mode",
                "authentication",
            ],
        )?;
        if string(source, "kind")? != "ftp" {
            return Err(invalid());
        }
        let security = string(source, "security")?;
        if security != "plain" {
            return Err(RpcErrorObject::invalid_params(
                "FTP security mode unavailable",
            ));
        }
        let path_mode = match source.get("path_mode") {
            None => "login_relative",
            Some(Value::String(value))
                if matches!(value.as_str(), "login_relative" | "absolute") =>
            {
                value
            }
            _ => return Err(invalid()),
        };
        let (canonical_url, origin, command_path) =
            ftp_url(string(source, "source_url")?, path_mode)?;
        let authentication = object(
            source.get("authentication").ok_or_else(invalid)?,
            &["kind", "username", "password"],
        )?;
        let secret = match string(authentication, "kind")? {
            "anonymous" if authentication.len() == 1 => None,
            "password" if authentication.len() == 3 => {
                let username = string(authentication, "username")?;
                let password = string(authentication, "password")?;
                if !safe(username)
                    || username.len() > 4096
                    || password.len() > 65536
                    || password.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
                Some(OriginSecret {
                    username: username.to_owned(),
                    password: password.to_owned(),
                })
            }
            _ => return Err(invalid()),
        };
        Ok(Self {
            origin: OriginRecord {
                version: 1,
                task_id: id.into(),
                destination: destination.into(),
                canonical_url,
                origin,
                command_path,
                security: security.into(),
                path_mode: path_mode.into(),
                authenticated: secret.is_some(),
                operation_id: String::new(),
                credential: None,
            },
            secret,
        })
    }
    /// Client-only ingress accepts URL userinfo and produces the strict wire contract.
    pub fn normalize_client(value: &Value) -> Result<Value, RpcErrorObject> {
        let mut normalized = value.clone();
        let source = normalized
            .get_mut("source")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid)?;
        let raw = string(source, "source_url")?.to_owned();
        if let Some((scheme, remainder)) = raw.split_once("://") {
            let (authority, path) = remainder.split_once('/').ok_or_else(invalid)?;
            if let Some((userinfo, host)) = authority.rsplit_once('@') {
                if source.contains_key("authentication") {
                    return Err(invalid());
                }
                let (username, password) = userinfo.split_once(':').ok_or_else(invalid)?;
                source.insert(
                    "authentication".into(),
                    json!({"kind":"password","username":decode(username)?,"password":decode(password)?}),
                );
                source.insert(
                    "source_url".into(),
                    json!(format!("{scheme}://{host}/{path}")),
                );
            }
        }
        Self::parse(&normalized)?;
        Ok(normalized)
    }
}
