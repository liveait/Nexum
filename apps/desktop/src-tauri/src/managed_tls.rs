//! A pipe-delivered leaf pin supplements normal WebPKI and handshake validation.

use nexum_protocol::managed::MANAGED_CERTIFICATE_MAX_BYTES;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{Resumption, WebPkiServerVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use std::sync::Arc;

#[derive(Debug)]
struct PinnedCertificate {
    certificate: CertificateDer<'static>,
    verifier: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if end_entity.as_ref() != self.certificate.as_ref() {
            return Err(Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        self.verifier
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.verifier.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.verifier.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.verifier.supported_verify_schemes()
    }
}

pub(super) fn client_config(certificate_der: Vec<u8>) -> Result<Arc<ClientConfig>, String> {
    let invalid = || "The local service returned an invalid TLS certificate.".to_owned();
    if certificate_der.is_empty() || certificate_der.len() > MANAGED_CERTIFICATE_MAX_BYTES {
        return Err(invalid());
    }
    let certificate = CertificateDer::from(certificate_der);
    let mut roots = RootCertStore::empty();
    roots.add(certificate.clone()).map_err(|_| invalid())?;
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|_| invalid())?;
    let mut config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCertificate {
            certificate,
            verifier,
        }))
        .with_no_client_auth();
    config.resumption = Resumption::disabled();
    config.enable_early_data = false;
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_leaf_pin_rejects_another_certificate_using_the_same_key() {
        let key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()]).unwrap();
        let first = params.clone().self_signed(&key).unwrap();
        let mut next = params;
        next.serial_number = Some(42.into());
        let second = next.self_signed(&key).unwrap();
        assert_ne!(first.der(), second.der());
        let mut roots = RootCertStore::empty();
        roots.add(first.der().clone()).unwrap();
        let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .unwrap();
        let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
        // A trust anchor alone accepts this differently encoded certificate.
        assert!(
            verifier
                .verify_server_cert(second.der(), &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        let pin = PinnedCertificate {
            certificate: first.der().clone(),
            verifier,
        };
        assert!(
            pin.verify_server_cert(first.der(), &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        assert!(
            pin.verify_server_cert(second.der(), &[], &name, &[], UnixTime::now())
                .is_err()
        );
    }

    #[test]
    fn pinned_certificate_still_requires_valid_name_time_and_der() {
        assert!(client_config(Vec::new()).is_err());
        assert!(client_config(vec![1; MANAGED_CERTIFICATE_MAX_BYTES + 1]).is_err());
        assert!(client_config(vec![1, 2, 3]).is_err());
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params =
            rcgen::CertificateParams::new(vec!["elsewhere.example".to_owned()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2019, 1, 1);
        params.not_after = rcgen::date_time_ymd(2020, 1, 1);
        let certificate = params.self_signed(&key).unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(certificate.der().clone()).unwrap();
        let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .unwrap();
        let pin = PinnedCertificate {
            certificate: certificate.der().clone(),
            verifier,
        };
        let right_name = ServerName::try_from("elsewhere.example").unwrap();
        let valid_time = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_560_000_000));
        assert!(
            pin.verify_server_cert(certificate.der(), &[], &right_name, &[], valid_time)
                .is_ok()
        );
        assert!(
            pin.verify_server_cert(certificate.der(), &[], &right_name, &[], UnixTime::now())
                .is_err()
        );
        let wrong_name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
        assert!(
            pin.verify_server_cert(certificate.der(), &[], &wrong_name, &[], valid_time)
                .is_err()
        );
    }
}
