// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native TLS on the main listener (`[server.tls]`): the rustls server
//! configuration and the certificate reloader behind it.
//!
//! The certificate and key are not baked into the [`rustls::ServerConfig`]:
//! the config resolves them per handshake through [`CertificateReloader`],
//! which re-reads the two files on an interval and swaps a renewed pair in, so
//! a certificate rotated on disk (cert-manager renewing a mounted Secret) is
//! served without a restart. A renewed pair that fails to load is logged and
//! the previous pair stays in service. The client CA behind mutual TLS is
//! watched the same way by [`ClientCaReloader`], which builds a fresh
//! configuration around a renewed trust anchor for the listener to swap in.
//! No openEHR spec governs this — our own design/extension.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};

use ferroehr::config::server::{ClientAuth, TlsConfig, TlsVersion};
use ferroehr::service::error::ErrorChain;

/// Why the `[server.tls]` material could not be turned into a TLS listener.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// A key the enabled listener needs is not configured.
    #[error("server.tls.{key} is required when TLS is enabled")]
    Missing {
        /// The `[server.tls]` key that is unset.
        key: &'static str,
    },
    /// A configured file could not be read.
    #[error("server.tls.{key} `{}` could not be read", path.display())]
    Read {
        /// The `[server.tls]` key naming the file.
        key: &'static str,
        /// The configured path.
        path: PathBuf,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A configured file is not the PEM it should be.
    #[error("server.tls.{key} `{}` is not valid PEM", path.display())]
    Pem {
        /// The `[server.tls]` key naming the file.
        key: &'static str,
        /// The configured path.
        path: PathBuf,
        /// The PEM decoding failure.
        #[source]
        source: rustls::pki_types::pem::Error,
    },
    /// A certificate file holds no certificate.
    #[error("server.tls.{key} `{}` contains no certificate", path.display())]
    NoCertificate {
        /// The `[server.tls]` key naming the file.
        key: &'static str,
        /// The configured path.
        path: PathBuf,
    },
    /// The private key is of a kind the crypto provider cannot sign with.
    #[error("server.tls.key_file `{}` is not a usable private key", path.display())]
    Key {
        /// The configured key path.
        path: PathBuf,
        /// The provider's refusal.
        #[source]
        source: rustls::Error,
    },
    /// The end-entity certificate cannot be parsed.
    #[error("server.tls.cert_file `{}` holds an unparseable certificate", path.display())]
    Certificate {
        /// The configured certificate path.
        path: PathBuf,
        /// The parse failure.
        #[source]
        source: rustls::Error,
    },
    /// The private key does not belong to the first certificate of the chain.
    #[error(
        "server.tls.key_file `{}` does not belong to the first certificate in \
         server.tls.cert_file `{}`",
        key.display(),
        cert.display()
    )]
    KeyMismatch {
        /// The configured certificate path.
        cert: PathBuf,
        /// The configured key path.
        key: PathBuf,
    },
    /// An EC key does not match its certificate's public key encoding.
    ///
    /// rustls compares the key's `SubjectPublicKeyInfo`, which it encodes with
    /// the named curve, byte for byte with the certificate's. A certificate
    /// whose EC public key carries explicit curve parameters therefore never
    /// matches, even over the right key: RFC 5480 §2.1.1 allows only
    /// `namedCurve` in PKIX, and TLS clients refuse such a certificate anyway.
    #[error(
        "server.tls.key_file `{}` (an EC key) does not match the first certificate in \
         server.tls.cert_file `{}`; if the key is the right one, the certificate encodes \
         its curve with explicit parameters, which RFC 5480 §2.1.1 forbids (LibreSSL's \
         `genpkey` and `req -newkey ec` write them unless given \
         `-pkeyopt ec_param_enc:named_curve`): issue the key and certificate again with \
         a named curve",
        key.display(),
        cert.display()
    )]
    EcKeyMismatch {
        /// The configured certificate path.
        cert: PathBuf,
        /// The configured key path.
        key: PathBuf,
    },
    /// A client-CA certificate is not a usable trust anchor.
    #[error("server.tls.client_ca_file `{}` holds an unusable trust anchor", path.display())]
    ClientCa {
        /// The configured CA path.
        path: PathBuf,
        /// The refusal.
        #[source]
        source: rustls::Error,
    },
    /// The configured protocol versions are not offered by the provider.
    #[error("the TLS protocol versions could not be configured")]
    Versions(#[source] rustls::Error),
    /// The client-certificate verifier could not be built.
    #[error("the client-certificate verifier could not be built")]
    Verifier(#[source] rustls::server::VerifierBuilderError),
}

/// A ready TLS listener configuration and the reloader that feeds it.
#[derive(Debug, Clone)]
pub struct ServerTls {
    /// The rustls configuration the listener accepts with.
    pub config: Arc<rustls::ServerConfig>,
    /// The certificate source the configuration resolves every handshake
    /// against; [`CertificateReloader::watch`] keeps it current.
    pub certificate: Arc<CertificateReloader>,
    /// The client-CA source when `client_auth` is not `off`;
    /// [`ClientCaReloader::watch`] hands the listener a configuration built
    /// around a renewed trust anchor.
    pub client_ca: Option<Arc<ClientCaReloader>>,
}

/// Builds the TLS listener configuration for `[server.tls]`.
///
/// The certificate chain and key are loaded once here, so a boot with broken
/// material fails; afterwards they are served through a
/// [`CertificateReloader`]. When `client_auth` is not `off`, a
/// client-certificate verifier is built against the explicit `client_ca_file`
/// trust anchor, never the web PKI. The protocol floor is `min_version`.
///
/// # Errors
/// [`TlsError`] when required material is unset, unreadable or invalid, the
/// key does not belong to the certificate, or a verifier component is refused.
pub fn server_tls(tls: &TlsConfig) -> Result<ServerTls, TlsError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let certificate = Arc::new(CertificateReloader::load(tls, Arc::clone(&provider))?);

    // NOTE: 1.3-only by default — OWASP Transport Layer Security §Only Support
    // Strong Protocols; 1.1/1.0 are unreachable by construction (RFC 8996).
    let versions = match tls.min_version {
        TlsVersion::V1_3 => TLS13_ONLY,
        TlsVersion::V1_2 => TLS13_AND_12,
    };
    let shape = ConfigShape {
        provider,
        versions,
        resolver: Arc::clone(&certificate),
    };

    let (client_ca, config) = match tls.client_auth {
        ClientAuth::Off => (None, shape.build(None)?),
        ClientAuth::Optional | ClientAuth::Required => {
            let path = required(tls.client_ca_file.as_ref(), "client_ca_file")?.to_path_buf();
            let pem = read(&path, "client_ca_file")?;
            let reloader = ClientCaReloader {
                path,
                optional: tls.client_auth == ClientAuth::Optional,
                shape: shape.clone(),
                seen: tokio::sync::Mutex::new(Sha256::digest(&pem).into()),
            };
            let config = reloader.build(&pem)?;
            (Some(Arc::new(reloader)), config)
        }
    };
    Ok(ServerTls {
        config: Arc::new(config),
        certificate,
        client_ca,
    })
}

/// The protocol set of the default floor.
const TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];
/// The protocol set of the `1.2` compatibility floor.
const TLS13_AND_12: &[&rustls::SupportedProtocolVersion] =
    &[&rustls::version::TLS13, &rustls::version::TLS12];

/// What every listener configuration shares, whichever client trust anchor it
/// is built around: the crypto provider, the protocol floor and the
/// certificate source.
#[derive(Debug, Clone)]
struct ConfigShape {
    provider: Arc<CryptoProvider>,
    versions: &'static [&'static rustls::SupportedProtocolVersion],
    resolver: Arc<CertificateReloader>,
}

impl ConfigShape {
    /// Builds a listener configuration, verifying client certificates against
    /// `client` when it is given, never against the web PKI.
    ///
    /// # Errors
    /// [`TlsError::Versions`] when the protocol floor is refused and
    /// [`TlsError::Verifier`] when the verifier cannot be built.
    fn build(
        &self,
        client: Option<(rustls::RootCertStore, bool)>,
    ) -> Result<rustls::ServerConfig, TlsError> {
        let builder = rustls::ServerConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_protocol_versions(self.versions)
            .map_err(TlsError::Versions)?;
        let builder = match client {
            None => builder.with_no_client_auth(),
            Some((roots, optional)) => {
                let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                    roots.into(),
                    Arc::clone(&self.provider),
                );
                let verifier = if optional {
                    verifier.allow_unauthenticated()
                } else {
                    verifier
                };
                builder.with_client_cert_verifier(verifier.build().map_err(TlsError::Verifier)?)
            }
        };
        let resolver: Arc<dyn ResolvesServerCert> =
            Arc::<CertificateReloader>::clone(&self.resolver);
        Ok(builder.with_cert_resolver(resolver))
    }
}

/// The client CA of mutual TLS, re-read from `[server.tls] client_ca_file` and
/// turned into a fresh listener configuration when it changes.
///
/// rustls fixes the client-certificate verifier inside a
/// [`rustls::ServerConfig`], so a renewed trust anchor means a new
/// configuration, which the listener swaps in for every new handshake
/// (`axum_server::tls_rustls::RustlsConfig::reload_from_config`). A changed
/// file that fails to load is refused and the configuration in service stays;
/// the refused file is remembered by digest and reported once, the rule
/// [`CertificateReloader`] follows for the server pair.
#[derive(Debug)]
pub struct ClientCaReloader {
    /// `[server.tls] client_ca_file`.
    path: PathBuf,
    /// Whether `client_auth` is `optional`.
    optional: bool,
    /// What every configuration shares.
    shape: ConfigShape,
    /// SHA-256 of the file as last read, loaded or refused.
    seen: tokio::sync::Mutex<[u8; 32]>,
}

impl ClientCaReloader {
    /// A listener configuration verifying client certificates against the
    /// trust anchors in `pem`.
    ///
    /// # Errors
    /// [`TlsError`] when `pem` holds no usable certificate or the verifier
    /// cannot be built.
    fn build(&self, pem: &[u8]) -> Result<rustls::ServerConfig, TlsError> {
        let mut roots = rustls::RootCertStore::empty();
        for cert in parse_certificates(pem, &self.path, "client_ca_file")? {
            roots.add(cert).map_err(|source| TlsError::ClientCa {
                path: self.path.clone(),
                source,
            })?;
        }
        self.shape.build(Some((roots, self.optional)))
    }

    /// Re-reads the client CA file and, when it changed, answers the listener
    /// configuration built around it.
    ///
    /// # Errors
    /// [`TlsError`] when the file cannot be read or a changed file fails to
    /// load; the configuration in service then stays. A refused file is
    /// reported once: until it changes again, later calls answer `None`.
    pub async fn reload(&self) -> Result<Option<Arc<rustls::ServerConfig>>, TlsError> {
        let pem = read_async(&self.path, "client_ca_file").await?;
        let fresh: [u8; 32] = Sha256::digest(&pem).into();
        let mut seen = self.seen.lock().await;
        if *seen == fresh {
            return Ok(None);
        }
        *seen = fresh;
        Ok(Some(Arc::new(self.build(&pem)?)))
    }

    /// Checks the file every `interval` and swaps each configuration built
    /// around a renewed client CA into `listener`, logging each replacement
    /// and each refusal with its cause chain, for as long as anything besides
    /// this task still holds the reloader (the listener's [`ServerTls`] does).
    pub async fn watch(
        self: Arc<Self>,
        interval: Duration,
        listener: axum_server::tls_rustls::RustlsConfig,
    ) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick completes at once, and the file was loaded at boot.
        ticker.tick().await;
        while Arc::strong_count(&self) > 1 {
            ticker.tick().await;
            match self.reload().await {
                Ok(None) => {}
                Ok(Some(config)) => {
                    listener.reload_from_config(config);
                    tracing::info!(
                        client_ca_file = %self.path.display(),
                        "the TLS client CA changed on disk and now verifies new handshakes"
                    );
                }
                Err(error) => tracing::error!(
                    error = %ErrorChain::new(&error),
                    "the TLS client CA on disk could not be loaded; the previous one still verifies"
                ),
            }
        }
    }
}

/// Whether a reload put a new certificate into service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reloaded {
    /// The files are byte-identical to the pair last read; nothing changed.
    Unchanged,
    /// A new pair loaded and is now presented to every new handshake.
    Replaced,
}

/// The server certificate, re-read from its files and swapped in when they
/// change.
///
/// Implements [`ResolvesServerCert`], so every handshake presents whichever
/// pair is current. A changed pair that fails to load is refused and the
/// current one stays in service; the refused files are remembered by digest,
/// so the same broken pair is reported once rather than on every check.
#[derive(Debug)]
pub struct CertificateReloader {
    /// `[server.tls] cert_file`.
    cert_path: PathBuf,
    /// `[server.tls] key_file`.
    key_path: PathBuf,
    /// The provider the key is loaded through, the listener's own.
    provider: Arc<CryptoProvider>,
    /// The pair every new handshake presents.
    current: ArcSwap<CertifiedKey>,
    /// SHA-256 over the two files as last read, loaded or refused; a digest
    /// rather than the bytes, so no second copy of the private key is kept.
    seen: tokio::sync::Mutex<[u8; 32]>,
}

impl CertificateReloader {
    /// Loads the pair `tls` names, which becomes the current one.
    fn load(tls: &TlsConfig, provider: Arc<CryptoProvider>) -> Result<Self, TlsError> {
        let cert_path = required(tls.cert_file.as_ref(), "cert_file")?.to_path_buf();
        let key_path = required(tls.key_file.as_ref(), "key_file")?.to_path_buf();
        let cert_pem = read(&cert_path, "cert_file")?;
        let key_pem = read(&key_path, "key_file")?;
        let current = certified_key(&cert_pem, &key_pem, &cert_path, &key_path, &provider)?;
        Ok(Self {
            seen: tokio::sync::Mutex::new(digest(&cert_pem, &key_pem)),
            cert_path,
            key_path,
            provider,
            current: ArcSwap::from_pointee(current),
        })
    }

    /// The pair new handshakes present.
    #[must_use]
    pub fn current(&self) -> Arc<CertifiedKey> {
        self.current.load_full()
    }

    /// Re-reads the certificate and key files and, when they changed, puts the
    /// new pair into service.
    ///
    /// # Errors
    /// [`TlsError`] when a file cannot be read or a changed pair fails to load;
    /// the current pair then stays in service. A refused pair is reported once:
    /// until either file changes again, later calls answer
    /// [`Reloaded::Unchanged`].
    pub async fn reload(&self) -> Result<Reloaded, TlsError> {
        let cert_pem = read_async(&self.cert_path, "cert_file").await?;
        let key_pem = read_async(&self.key_path, "key_file").await?;
        let fresh = digest(&cert_pem, &key_pem);
        let mut seen = self.seen.lock().await;
        if *seen == fresh {
            return Ok(Reloaded::Unchanged);
        }
        *seen = fresh;
        let key = certified_key(
            &cert_pem,
            &key_pem,
            &self.cert_path,
            &self.key_path,
            &self.provider,
        )?;
        self.current.store(Arc::new(key));
        Ok(Reloaded::Replaced)
    }

    /// Checks the files every `interval` for as long as anything besides this
    /// task still holds the reloader (the listener's configuration does),
    /// logging each replacement and each refusal with its cause chain.
    pub async fn watch(self: Arc<Self>, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick completes at once, and the pair was loaded at boot.
        ticker.tick().await;
        while Arc::strong_count(&self) > 1 {
            ticker.tick().await;
            match self.reload().await {
                Ok(Reloaded::Unchanged) => {}
                Ok(Reloaded::Replaced) => tracing::info!(
                    cert_file = %self.cert_path.display(),
                    "the TLS certificate changed on disk and is now served"
                ),
                Err(error) => tracing::error!(
                    error = %ErrorChain::new(&error),
                    "the TLS certificate on disk could not be loaded; the previous one is still served"
                ),
            }
        }
    }
}

impl ResolvesServerCert for CertificateReloader {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.load_full())
    }
}

/// The configured path behind `key`, or the error naming the unset key.
fn required<'a>(value: Option<&'a String>, key: &'static str) -> Result<&'a Path, TlsError> {
    value.map(Path::new).ok_or(TlsError::Missing { key })
}

/// Reads one configured file at boot.
fn read(path: &Path, key: &'static str) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|source| TlsError::Read {
        key,
        path: path.to_path_buf(),
        source,
    })
}

/// Reads one configured file from the running listener through `tokio::fs`,
/// so a check never blocks a runtime worker.
async fn read_async(path: &Path, key: &'static str) -> Result<Vec<u8>, TlsError> {
    tokio::fs::read(path)
        .await
        .map_err(|source| TlsError::Read {
            key,
            path: path.to_path_buf(),
            source,
        })
}

/// Every certificate in a PEM file; none at all is an error.
fn parse_certificates(
    pem: &[u8],
    path: &Path,
    key: &'static str,
) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let certs = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| TlsError::Pem {
            key,
            path: path.to_path_buf(),
            source,
        })?;
    if certs.is_empty() {
        return Err(TlsError::NoCertificate {
            key,
            path: path.to_path_buf(),
        });
    }
    Ok(certs)
}

/// The chain and key as one servable pair, refused unless the key belongs to
/// the chain's first certificate.
///
/// The key may be PKCS#8, SEC1 or PKCS#1; the first private key in the file is
/// the one used.
fn certified_key(
    cert_pem: &[u8],
    key_pem: &[u8],
    cert_path: &Path,
    key_path: &Path,
    provider: &CryptoProvider,
) -> Result<CertifiedKey, TlsError> {
    let certs = parse_certificates(cert_pem, cert_path, "cert_file")?;
    let der = PrivateKeyDer::from_pem_slice(key_pem).map_err(|source| TlsError::Pem {
        key: "key_file",
        path: key_path.to_path_buf(),
        source,
    })?;
    let signing_key = provider
        .key_provider
        .load_private_key(der)
        .map_err(|source| TlsError::Key {
            path: key_path.to_path_buf(),
            source,
        })?;
    let is_ec = signing_key.algorithm() == rustls::SignatureAlgorithm::ECDSA;
    let pair = CertifiedKey::new(certs, signing_key);
    match pair.keys_match() {
        // A key that cannot state its public half is not a mismatch; rustls'
        // own `CertifiedKey::from_der` accepts it the same way.
        Ok(()) | Err(rustls::Error::InconsistentKeys(rustls::InconsistentKeys::Unknown)) => {
            Ok(pair)
        }
        Err(rustls::Error::InconsistentKeys(rustls::InconsistentKeys::KeyMismatch)) => {
            let cert = cert_path.to_path_buf();
            let key = key_path.to_path_buf();
            Err(if is_ec {
                TlsError::EcKeyMismatch { cert, key }
            } else {
                TlsError::KeyMismatch { cert, key }
            })
        }
        Err(source) => Err(TlsError::Certificate {
            path: cert_path.to_path_buf(),
            source,
        }),
    }
}

/// SHA-256 over the digests of both files, so no split of the bytes between
/// the two can collide.
fn digest(cert_pem: &[u8], key_pem: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(Sha256::digest(cert_pem));
    hasher.update(Sha256::digest(key_pem));
    hasher.finalize().into()
}
