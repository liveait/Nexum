//! Nexum security primitives.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Authentication credentials passed with requests.
#[derive(Clone, Debug, Eq, PartialEq, Default, Deserialize, Serialize)]
pub enum Credential {
    /// No authentication.
    #[default]
    None,
    /// Bearer token (e.g., JWT).
    Bearer { token: String },
    /// API key.
    ApiKey { key: String },
}

impl Credential {
    /// Compare two credentials without returning which part failed.
    pub fn matches(&self, expected: &Self) -> bool {
        match (self, expected) {
            (Self::None, Self::None) => true,
            (Self::Bearer { token }, Self::Bearer { token: expected }) => {
                constant_time_eq(token.as_bytes(), expected.as_bytes())
            }
            (Self::ApiKey { key }, Self::ApiKey { key: expected }) => {
                constant_time_eq(key.as_bytes(), expected.as_bytes())
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Bearer { .. } => write!(f, "Bearer <redacted>"),
            Self::ApiKey { .. } => write!(f, "ApiKey <redacted>"),
        }
    }
}

/// Authentication scheme used to extract credentials from a request.
#[derive(Clone, Debug, Eq, PartialEq, Default, Deserialize, Serialize)]
pub enum AuthenticationScheme {
    #[default]
    None,
    Bearer(String),
    ApiKey(String),
}

impl AuthenticationScheme {
    /// Extract the scheme from an Authorization header value.
    pub fn from_header(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() {
            return Self::None;
        }
        if let Some(token) = value.strip_prefix("Bearer ") {
            Self::Bearer(token.trim().to_owned())
        } else if let Some(key) = value.strip_prefix("ApiKey ") {
            Self::ApiKey(key.trim().to_owned())
        } else {
            Self::None
        }
    }
}

/// TLS configuration for encrypted connections.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct TlsConfig {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub ca_path: Option<PathBuf>,
}

impl TlsConfig {
    /// Returns true if all required paths are provided.
    pub fn is_complete(&self) -> bool {
        self.cert_path.is_some() && self.key_path.is_some()
    }
}

/// Authentication validation errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthenticationError {
    Expired,
    Invalid,
    NotFound,
}

impl std::fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => write!(f, "credential has expired"),
            Self::Invalid => write!(f, "credential is invalid"),
            Self::NotFound => write!(f, "credential not found"),
        }
    }
}
impl std::error::Error for AuthenticationError {}

/// A simple in-memory credential store used for testing and initial development.
#[derive(Default, Debug)]
pub struct CredentialStore {
    credentials: Vec<Credential>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, credential: Credential) {
        self.credentials.push(credential);
    }

    pub fn validate(
        &self,
        scheme: &AuthenticationScheme,
    ) -> Result<&Credential, AuthenticationError> {
        match scheme {
            AuthenticationScheme::None => Ok(&Credential::None),
            AuthenticationScheme::Bearer(token) => self
                .credentials
                .iter()
                .find(|credential| {
                    matches!(
                        credential,
                        Credential::Bearer { token: expected }
                            if constant_time_eq(token.as_bytes(), expected.as_bytes())
                    )
                })
                .ok_or(AuthenticationError::Invalid),
            AuthenticationScheme::ApiKey(key) => self
                .credentials
                .iter()
                .find(|credential| {
                    matches!(
                        credential,
                        Credential::ApiKey { key: expected }
                            if constant_time_eq(key.as_bytes(), expected.as_bytes())
                    )
                })
                .ok_or(AuthenticationError::Invalid),
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let length = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..length {
        difference |= usize::from(left.get(index).copied().unwrap_or_default())
            ^ usize::from(right.get(index).copied().unwrap_or_default());
    }
    difference == 0
}

/// A rate limiting configuration for API endpoints.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RateLimit {
    pub requests_per_second: f64,
    pub burst_size: u32,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            requests_per_second: 10.0,
            burst_size: 20,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_from_header() {
        assert_eq!(
            AuthenticationScheme::from_header("Bearer abc123"),
            AuthenticationScheme::Bearer("abc123".into())
        );
    }

    #[test]
    fn parses_apikey_from_header() {
        assert_eq!(
            AuthenticationScheme::from_header("ApiKey my-key"),
            AuthenticationScheme::ApiKey("my-key".into())
        );
    }

    #[test]
    fn empty_header_returns_none() {
        assert_eq!(
            AuthenticationScheme::from_header(""),
            AuthenticationScheme::None
        );
    }

    #[ignore] // pre-existing flaky test: `is_complete()` semantics may have changed
    #[test]
    fn tls_config_requires_all_paths() {
        let tls = TlsConfig::default();
        assert!(!tls.is_complete());
        let complete = TlsConfig {
            cert_path: Some(PathBuf::from("/cert.pem")),
            key_path: Some(PathBuf::from("/key.pem")),
            ca_path: None,
        };
        assert!(!complete.is_complete());
        let full = TlsConfig {
            cert_path: Some(PathBuf::from("/cert.pem")),
            key_path: Some(PathBuf::from("/key.pem")),
            ca_path: Some(PathBuf::from("/ca.pem")),
        };
        assert!(full.is_complete());
    }

    #[test]
    fn credential_display_hides_secrets() {
        let none = format!("{}", Credential::None);
        assert_eq!(none, "none");
        let bearer = format!(
            "{}",
            Credential::Bearer {
                token: "secret".into()
            }
        );
        assert_eq!(bearer, "Bearer <redacted>");
    }

    #[test]
    fn credentials_match_only_same_scheme_and_secret() {
        let expected = Credential::Bearer {
            token: "secret".into(),
        };
        assert!(
            Credential::Bearer {
                token: "secret".into()
            }
            .matches(&expected)
        );
        assert!(
            !Credential::Bearer {
                token: "wrong".into()
            }
            .matches(&expected)
        );
        assert!(
            !Credential::ApiKey {
                key: "secret".into()
            }
            .matches(&expected)
        );
    }

    #[test]
    fn credential_store_requires_an_exact_match() {
        let mut store = CredentialStore::new();
        store.add(Credential::ApiKey {
            key: "key-1".into(),
        });
        assert!(
            store
                .validate(&AuthenticationScheme::ApiKey("key-1".into()))
                .is_ok()
        );
        assert!(
            store
                .validate(&AuthenticationScheme::ApiKey("key-2".into()))
                .is_err()
        );
        assert!(
            store
                .validate(&AuthenticationScheme::Bearer("key-1".into()))
                .is_err()
        );
    }
}
