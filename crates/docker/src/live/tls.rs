//! Mutual TLS for Bollard's custom transport.
//!
//! Only configuration: rustls does the TLS and certificate verification.
//! It exists because Bollard's built-in `connect_with_ssl` also trusts the
//! public web roots, while a Docker context trusts its own `ca.pem` only
//! (as the Docker CLI does).
use std::path::Path;
use std::sync::Arc;

use bollard::{API_DEFAULT_VERSION, BollardRequest, Docker};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};

use crate::{ConnectError, TlsConfig};

pub(super) fn client(url: &str, config: &TlsConfig) -> Result<Docker, ConnectError> {
    let invalid = ConnectError::InvalidConfig;
    if !config.verify {
        return Err(invalid(
            "connecting without TLS verification (SkipTLSVerify) isn't supported; \
             trust the daemon's CA in ca.pem instead"
                .into(),
        ));
    }
    let host = ["tcp://", "http://", "https://"]
        .iter()
        .find_map(|scheme| url.strip_prefix(scheme))
        .ok_or_else(|| invalid("TLS endpoint must use tcp://, http:// or https://".into()))?;
    let dir = &config.cert_dir;
    let tls = ClientConfig::builder_with_provider(Arc::new(
        // Explicit, so no process-global provider has to be installed.
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| invalid(e.to_string()))?
    .with_root_certificates(roots(&dir.join("ca.pem"))?)
    .with_client_auth_cert(
        certificates(&dir.join("cert.pem"))?,
        PrivateKeyDer::from_pem_file(dir.join("key.pem"))
            .map_err(|e| invalid(format!("{}: {e}", dir.join("key.pem").display())))?,
    )
    .map_err(|e| invalid(e.to_string()))?;
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
        Some(format!("https://{host}")),
        120,
        API_DEFAULT_VERSION,
    )
    .map_err(|e| invalid(e.to_string()))
}

fn certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, ConnectError> {
    let invalid = |e: String| ConnectError::InvalidConfig(format!("{}: {e}", path.display()));
    let certificates = CertificateDer::pem_file_iter(path)
        .map_err(|e| invalid(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(e.to_string()))?;
    if certificates.is_empty() {
        return Err(invalid("contains no certificates".into()));
    }
    Ok(certificates)
}

/// The trust store: exactly the certificates in `ca.pem`, nothing else.
fn roots(ca: &Path) -> Result<RootCertStore, ConnectError> {
    let mut roots = RootCertStore::empty();
    for certificate in certificates(ca)? {
        roots
            .add(certificate)
            .map_err(|e| ConnectError::InvalidConfig(format!("{}: {e}", ca.display())))?;
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    #[test]
    fn trust_store_is_only_the_context_ca() {
        let ca =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/live/test-certs/ca.pem");
        let expected = super::certificates(&ca).unwrap().len();
        assert_eq!(super::roots(&ca).unwrap().len(), expected);
        assert!(expected > 0);
    }
}
