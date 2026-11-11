//! Input resolution boundaries for Nexum.

pub mod torrent;

use std::{fmt, path::Path, str::FromStr};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolveKind {
    Http,
    Https,
    Magnet,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveRequest {
    source: String,
}

impl ResolveRequest {
    pub fn new(source: impl Into<String>) -> Result<Self, ResolverError> {
        let source = source.into().trim().to_owned();
        if source.is_empty() {
            return Err(ResolverError::EmptySource);
        }
        Ok(Self { source })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn kind(&self) -> Result<ResolveKind, ResolverError> {
        let scheme = self.scheme();
        match scheme.as_str() {
            "http" => Ok(ResolveKind::Http),
            "https" => Ok(ResolveKind::Https),
            "magnet" => Ok(ResolveKind::Magnet),
            "" => Ok(ResolveKind::Local),
            _ => Err(ResolverError::UnsupportedScheme(scheme)),
        }
    }

    fn scheme(&self) -> String {
        self.source
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveResult {
    pub kind: ResolveKind,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolverError {
    EmptySource,
    UnsupportedScheme(String),
    InvalidSource(String),
}

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => f.write_str("source is empty"),
            Self::UnsupportedScheme(scheme) => write!(f, "unsupported source scheme: {scheme}"),
            Self::InvalidSource(message) => write!(f, "invalid source: {message}"),
        }
    }
}

impl std::error::Error for ResolverError {}

pub trait Resolver: Send {
    fn supports(&self, request: &ResolveRequest) -> bool;
    fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError>;
}

#[derive(Clone, Debug, Default)]
pub struct HttpResolver;

impl Resolver for HttpResolver {
    fn supports(&self, request: &ResolveRequest) -> bool {
        matches!(request.kind(), Ok(ResolveKind::Http | ResolveKind::Https))
    }

    fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
        let kind = request.kind()?;
        if !matches!(kind, ResolveKind::Http | ResolveKind::Https) {
            return Err(ResolverError::InvalidSource("not an HTTP source".into()));
        }
        let authority = request
            .source()
            .split_once("://")
            .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
            .unwrap_or("");
        if authority.is_empty() || authority.starts_with(':') {
            return Err(ResolverError::InvalidSource(
                "HTTP source is missing a valid host".into(),
            ));
        }
        Ok(ResolveResult {
            kind,
            source: request.source().to_owned(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MagnetResolver;

impl Resolver for MagnetResolver {
    fn supports(&self, request: &ResolveRequest) -> bool {
        matches!(request.kind(), Ok(ResolveKind::Magnet))
    }

    fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
        if !self.supports(request) {
            return Err(ResolverError::InvalidSource("not a magnet source".into()));
        }
        let query = request.source().strip_prefix("magnet:?").unwrap_or("");
        if query
            .split('&')
            .all(|part| !part.starts_with("xt=urn:btih:"))
        {
            return Err(ResolverError::InvalidSource(
                "magnet source is missing an xt=urn:btih parameter".into(),
            ));
        }
        Ok(ResolveResult {
            kind: ResolveKind::Magnet,
            source: request.source().to_owned(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct LocalResolver;

impl Resolver for LocalResolver {
    fn supports(&self, request: &ResolveRequest) -> bool {
        matches!(request.kind(), Ok(ResolveKind::Local))
    }

    fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
        if !self.supports(request) {
            return Err(ResolverError::InvalidSource("not a local source".into()));
        }
        if !Path::new(request.source()).exists() {
            return Err(ResolverError::InvalidSource(
                "local source does not exist".into(),
            ));
        }
        Ok(ResolveResult {
            kind: ResolveKind::Local,
            source: request.source().to_owned(),
        })
    }
}

pub struct ResolverRegistry {
    resolvers: Vec<Box<dyn Resolver + Send>>,
}

impl Default for ResolverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolverRegistry {
    pub fn new() -> Self {
        Self {
            resolvers: vec![
                Box::new(HttpResolver),
                Box::new(MagnetResolver),
                Box::new(LocalResolver),
            ],
        }
    }

    pub fn with_resolver(mut self, resolver: Box<dyn Resolver + Send>) -> Self {
        self.resolvers.push(resolver);
        self
    }

    /// Registers a resolver at runtime (e.g. from a plugin).
    pub fn register_resolver(&mut self, resolver: Box<dyn Resolver + Send>) {
        self.resolvers.push(resolver);
    }

    pub fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
        for resolver in &self.resolvers {
            if resolver.supports(request) {
                return resolver.resolve(request);
            }
        }
        Err(ResolverError::UnsupportedScheme(request.scheme()))
    }
}

#[derive(Default)]
pub struct DefaultResolver(ResolverRegistry);

impl DefaultResolver {
    pub fn new() -> Self {
        Self(ResolverRegistry::new())
    }
}

impl Resolver for DefaultResolver {
    fn supports(&self, request: &ResolveRequest) -> bool {
        self.0
            .resolvers
            .iter()
            .any(|resolver| resolver.supports(request))
    }

    fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
        self.0.resolve(request)
    }
}

impl FromStr for ResolveRequest {
    type Err = ResolverError;
    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::new(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_dispatches_supported_sources() {
        let registry = ResolverRegistry::new();
        for (source, kind) in [
            ("http://example.com/file", ResolveKind::Http),
            ("HTTPS://example.com/file", ResolveKind::Https),
            ("magnet:?xt=urn:btih:abc", ResolveKind::Magnet),
        ] {
            let request = ResolveRequest::new(source).unwrap();
            assert_eq!(registry.resolve(&request).unwrap().kind, kind);
        }
    }

    #[test]
    fn rejects_invalid_http_and_magnet_sources() {
        let registry = ResolverRegistry::new();
        for source in ["https://", "https://:443/file", "magnet:?dn=file"] {
            let request = ResolveRequest::new(source).unwrap();
            assert!(matches!(
                registry.resolve(&request),
                Err(ResolverError::InvalidSource(_))
            ));
        }
    }

    #[test]
    fn validates_local_sources() {
        let registry = ResolverRegistry::new();
        let path = std::env::temp_dir().join("nexum-resolver-test");
        std::fs::write(&path, b"nexum").unwrap();
        let request = ResolveRequest::new(path.to_string_lossy()).unwrap();
        assert_eq!(registry.resolve(&request).unwrap().kind, ResolveKind::Local);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unsupported_schemes() {
        let request = ResolveRequest::new("ftp://example.com/file").unwrap();
        assert_eq!(
            request.kind(),
            Err(ResolverError::UnsupportedScheme("ftp".into()))
        );
    }

    // ——— Resolver extension tests ——

    #[test]
    fn registry_accepts_runtime_resolver_registration() {
        let mut registry = ResolverRegistry::new();

        struct PrefixResolver(String);
        impl Resolver for PrefixResolver {
            fn supports(&self, request: &ResolveRequest) -> bool {
                request.source().starts_with(&self.0)
            }
            fn resolve(&self, request: &ResolveRequest) -> Result<ResolveResult, ResolverError> {
                self.supports(request)
                    .then(|| ResolveResult {
                        kind: ResolveKind::Http,
                        source: request.source().to_owned(),
                    })
                    .ok_or(ResolverError::InvalidSource("not supported".into()))
            }
        }

        registry.register_resolver(Box::new(PrefixResolver("http://prefix".into())));
        assert!(
            registry
                .resolve(&ResolveRequest::new("http://prefix/file").unwrap())
                .is_ok()
        );
    }
}
