//! TLS transport for Bollard, including the contract's `verify = false` mode.
use std::sync::Arc;

use bollard::{API_DEFAULT_VERSION, BollardRequest, Docker};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject as _};

use crate::{ConnectError, TlsConfig};

pub(super) fn client(url: &str, config: &TlsConfig) -> Result<Docker, ConnectError> {
    let address = if let Some(host) = url
        .strip_prefix("tcp://")
        .or_else(|| url.strip_prefix("http://"))
    {
        format!("https://{host}")
    } else if url.starts_with("https://") {
        url.to_owned()
    } else {
        return Err(ConnectError::InvalidConfig(
            "TLS endpoint must use tcp://, http:// or https://".into(),
        ));
    };
    let invalid = |e: String| ConnectError::InvalidConfig(e);
    let certs = |name: &str| -> Result<Vec<CertificateDer<'static>>, ConnectError> {
        let path = config.cert_dir.join(name);
        let certificates = CertificateDer::pem_file_iter(&path)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        if certificates.is_empty() {
            return Err(invalid(format!(
                "{} contains no certificates",
                path.display()
            )));
        }
        Ok(certificates)
    };
    let mut roots = RootCertStore::empty();
    for certificate in certs("ca.pem")? {
        roots.add(certificate).map_err(|e| invalid(e.to_string()))?;
    }
    let key_path = config.cert_dir.join("key.pem");
    let key = PrivateKeyDer::from_pem_file(&key_path)
        .map_err(|e| invalid(format!("{}: {e}", key_path.display())))?;
    // An explicit provider avoids a process-global provider installation or a
    // panic when another workspace crate also enables rustls's aws-lc feature.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| invalid(e.to_string()))?
        .with_root_certificates(roots)
        .with_client_auth_cert(certs("cert.pem")?, key)
        .map_err(|e| invalid(e.to_string()))?;
    if !config.verify {
        tls.dangerous()
            .set_certificate_verifier(Arc::new(UnverifiedServer { provider }));
    }
    let connector = HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_only()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new()).build(connector);
    Docker::connect_with_custom_transport(
        move |request: BollardRequest| {
            let client = client.clone();
            async move {
                client
                    .request(request)
                    .await
                    .map_err(bollard::errors::Error::from)
            }
        },
        Some(address),
        120,
        API_DEFAULT_VERSION,
    )
    .map_err(|e| invalid(e.to_string()))
}

/// Explicitly requested encryption without certificate identity verification.
/// Handshake signatures are still validated against the peer's public key.
#[derive(Debug)]
struct UnverifiedServer {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for UnverifiedServer {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
