// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Outbound TLS material for one terminology-server provider: the client
//! certificate the CDR presents (mutual TLS) and the trust anchors it verifies
//! the server with.
//!
//! NOTE: no openEHR spec governs terminology-server transport security — our own
//! design/extension; the certificate formats are PEM-encoded X.509 (RFC 7468)
//! handled by the pinned `rustls` and `reqwest` stack.
//!
//! The client identity is configured per
//! `[terminology.external.providers.<name>]` rather than once per process,
//! because a client certificate is a credential issued by the peer's PKI, as is
//! the CA signing each server's certificate. Repeating the same paths degenerates
//! to a shared identity; `[server.tls]` is inbound-only.
//!
//! There is no insecure or skip-verification switch: this module only supplies
//! trust anchors and a client identity to the `reqwest` builder, never touching
//! `danger_accept_invalid_certs` and never installing a custom verifier, so
//! server-certificate and hostname verification stay on for every provider. A
//! configured [`FhirProviderConfig::ca_bundle_path`] replaces the default trust
//! anchors for that provider (`reqwest::ClientBuilder::tls_certs_only`) rather
//! than widening them, pinning a privately issued terminology server to its own
//! PKI.
//!
//! The material applies to the connection to the terminology server itself. The
//! `OAuth2` token endpoint ([`super::oauth2::TokenSource`]) is a different host
//! in a different trust domain and keeps the default TLS stack; presenting the
//! terminology server's client certificate to an identity provider would be a
//! credential leak.

use std::path::{Path, PathBuf};

use super::config::FhirProviderConfig;

/// A failure loading a provider's TLS material.
///
/// Every variant is a boot failure: a route to a terminology server whose
/// identity cannot be assembled must fail loudly at startup, never silently
/// at the first validated code.
#[derive(Debug, thiserror::Error)]
pub enum TlsMaterialError {
    /// `client_cert_path` was set without `client_key_path`, or the reverse.
    #[error(
        "client_cert_path and client_key_path must be set together (a client certificate is \
         useless without its private key)"
    )]
    IncompleteIdentity,
    /// A configured PEM file could not be read.
    #[error("reading {role} '{}': {source}", path.display())]
    Read {
        /// Which configured key pointed at the unreadable file.
        role: MaterialRole,
        /// The configured path.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A PEM file that must hold certificates holds none, or unparseable ones.
    #[error("{role} '{}' holds no usable PEM certificate: {reason}", path.display())]
    NoCertificate {
        /// Which configured key pointed at the file.
        role: MaterialRole,
        /// The configured path.
        path: PathBuf,
        /// Why nothing usable came out of it.
        reason: String,
    },
    /// The key file holds no parseable PEM private key.
    #[error("client_key_path '{}' contains no PEM private key: {reason}", path.display())]
    NoPrivateKey {
        /// The configured path.
        path: PathBuf,
        /// The parser's rejection reason.
        reason: String,
    },
    /// The HTTP client rejected the assembled material.
    #[error("{role} was rejected by the TLS stack: {source}")]
    Rejected {
        /// Which configured key produced the rejected material.
        role: MaterialRole,
        /// The underlying `reqwest` failure.
        #[source]
        source: reqwest::Error,
    },
}

/// Which configured key a piece of TLS material came from — the discriminant a
/// boot error names, so an operator knows which line of the TOML to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialRole {
    /// `client_cert_path` — the client certificate (chain) presented to the TS.
    ClientCertificate,
    /// `client_key_path` — its private key.
    ClientKey,
    /// `ca_bundle_path` — the trust anchors the TS certificate is verified with.
    CaBundle,
}

impl std::fmt::Display for MaterialRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ClientCertificate => "client_cert_path",
            Self::ClientKey => "client_key_path",
            Self::CaBundle => "ca_bundle_path",
        })
    }
}

/// Apply a provider's configured TLS material to its HTTP-client builder.
///
/// A provider that configures neither an identity nor a CA bundle gets the
/// builder back untouched — the default TLS stack, byte-identical to the
/// behaviour before mutual TLS existed.
///
/// # Errors
///
/// [`TlsMaterialError`] when the identity is half-configured, a PEM file is
/// unreadable, or its contents are not the certificates/key the key promises.
pub(super) fn apply(
    builder: reqwest::ClientBuilder,
    cfg: &FhirProviderConfig,
) -> Result<reqwest::ClientBuilder, TlsMaterialError> {
    let builder = match (&cfg.client_cert_path, &cfg.client_key_path) {
        (Some(cert), Some(key)) => builder.identity(load_identity(cert, key)?),
        (None, None) => builder,
        _ => return Err(TlsMaterialError::IncompleteIdentity),
    };
    let Some(ca) = &cfg.ca_bundle_path else {
        return Ok(builder);
    };
    Ok(builder.tls_certs_only(load_trust_anchors(ca)?))
}

/// Load the client certificate chain + private key as one `reqwest` identity.
///
/// Both PEM files are parsed first with `rustls`' own reader so a mistake names
/// the offending key (`client_cert_path` vs `client_key_path`) instead of
/// surfacing as one opaque "invalid identity" from the concatenated buffer.
fn load_identity(cert_path: &Path, key_path: &Path) -> Result<reqwest::Identity, TlsMaterialError> {
    use rustls::pki_types::pem::PemObject;

    let cert_pem = read(MaterialRole::ClientCertificate, cert_path)?;
    let key_pem = read(MaterialRole::ClientKey, key_path)?;

    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| TlsMaterialError::NoCertificate {
            role: MaterialRole::ClientCertificate,
            path: cert_path.to_path_buf(),
            reason: e.to_string(),
        })?;
    if certs.is_empty() {
        return Err(TlsMaterialError::NoCertificate {
            role: MaterialRole::ClientCertificate,
            path: cert_path.to_path_buf(),
            reason: "the file contains no PEM certificate section".to_owned(),
        });
    }
    rustls::pki_types::PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| {
        TlsMaterialError::NoPrivateKey {
            path: key_path.to_path_buf(),
            reason: e.to_string(),
        }
    })?;

    // `reqwest::Identity::from_pem` takes the key and the chain in one buffer.
    let mut combined = key_pem;
    combined.extend_from_slice(b"\n");
    combined.extend_from_slice(&cert_pem);
    reqwest::Identity::from_pem(&combined).map_err(|e| TlsMaterialError::Rejected {
        role: MaterialRole::ClientCertificate,
        source: e,
    })
}

/// Load the trust anchors a provider verifies its terminology server with.
fn load_trust_anchors(path: &Path) -> Result<Vec<reqwest::Certificate>, TlsMaterialError> {
    let pem = read(MaterialRole::CaBundle, path)?;
    let anchors =
        reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| TlsMaterialError::Rejected {
            role: MaterialRole::CaBundle,
            source: e,
        })?;
    if anchors.is_empty() {
        return Err(TlsMaterialError::NoCertificate {
            role: MaterialRole::CaBundle,
            path: path.to_path_buf(),
            reason: "the file contains no PEM certificate section".to_owned(),
        });
    }
    Ok(anchors)
}

/// Read a configured PEM file, naming the configuration key in the error.
fn read(role: MaterialRole, path: &Path) -> Result<Vec<u8>, TlsMaterialError> {
    std::fs::read(path).map_err(|source| TlsMaterialError::Read {
        role,
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::terminology::config::test_provider_config;

    /// Neither key set: the builder is untouched — a deployment that never
    /// mentions TLS material keeps the default stack.
    #[test]
    fn no_material_is_a_no_op() {
        let cfg = test_provider_config("https://ts.example.org/fhir");
        assert!(apply(reqwest::Client::builder(), &cfg).is_ok());
    }

    /// A certificate without its key (or the reverse) is a loud failure — never
    /// a connection that silently presents no identity.
    #[test]
    fn a_half_configured_identity_is_rejected() {
        let mut cfg = test_provider_config("https://ts.example.org/fhir");
        cfg.client_cert_path = Some(PathBuf::from("/nonexistent/client.crt.pem"));
        assert!(matches!(
            apply(reqwest::Client::builder(), &cfg),
            Err(TlsMaterialError::IncompleteIdentity)
        ));

        let mut cfg = test_provider_config("https://ts.example.org/fhir");
        cfg.client_key_path = Some(PathBuf::from("/nonexistent/client.key.pem"));
        assert!(matches!(
            apply(reqwest::Client::builder(), &cfg),
            Err(TlsMaterialError::IncompleteIdentity)
        ));
    }

    /// An unreadable file names the configuration key that pointed at it.
    #[test]
    fn an_unreadable_file_names_its_config_key() {
        let mut cfg = test_provider_config("https://ts.example.org/fhir");
        cfg.ca_bundle_path = Some(PathBuf::from("/nonexistent/ca.pem"));
        let err = apply(reqwest::Client::builder(), &cfg).expect_err("must fail");
        assert!(
            matches!(
                &err,
                TlsMaterialError::Read {
                    role: MaterialRole::CaBundle,
                    ..
                }
            ),
            "got {err}"
        );
        assert!(err.to_string().contains("ca_bundle_path"), "got {err}");
    }
}
