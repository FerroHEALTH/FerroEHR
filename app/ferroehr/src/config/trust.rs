// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A configured `ca_file`: certificate authorities an outbound TLS client
//! trusts beside the platform roots.
//!
//! `reqwest`'s rustls backend verifies through `rustls-platform-verifier`, and
//! `ClientBuilder::tls_certs_merge` hands extra roots to
//! `rustls_platform_verifier::Verifier::new_with_extra_roots`, so an OIDC
//! issuer, a CDR or a PIX Manager behind a private PKI is trusted on Linux and
//! macOS alike without touching the platform store. The viewer compiles this
//! file too, so its clients follow the same rule. The file is read once, when
//! the client is built, and an unreadable file, a file that is no PEM, or one
//! with no certificate refuses the start.
//!
//! NOTE: no openEHR spec governs outbound TLS trust: our own design.

use std::path::{Path, PathBuf};

/// Why a configured `ca_file` cannot serve as trust roots.
#[derive(Debug, thiserror::Error)]
pub enum TrustRootsError {
    /// The file could not be read.
    #[error("{key} `{}` could not be read", path.display())]
    Unreadable {
        /// The configuration key that named the file.
        key: &'static str,
        /// The file.
        path: PathBuf,
        /// The read failure.
        #[source]
        source: std::io::Error,
    },
    /// The file is no PEM bundle `reqwest` reads.
    #[error("{key} `{}` is not a PEM certificate bundle", path.display())]
    NotPem {
        /// The configuration key that named the file.
        key: &'static str,
        /// The file.
        path: PathBuf,
        /// The parse failure.
        #[source]
        source: reqwest::Error,
    },
    /// The file holds no certificate.
    #[error("{key} `{}` holds no certificate", path.display())]
    Empty {
        /// The configuration key that named the file.
        key: &'static str,
        /// The file.
        path: PathBuf,
    },
}

/// The certificates of the PEM bundle at `path`, named in errors by `key`.
///
/// # Errors
/// [`TrustRootsError`] when the file cannot be read, is no PEM bundle, or
/// holds no certificate.
pub fn read_trust_roots(
    key: &'static str,
    path: &Path,
) -> Result<Vec<reqwest::Certificate>, TrustRootsError> {
    let pem = std::fs::read(path).map_err(|source| TrustRootsError::Unreadable {
        key,
        path: path.to_path_buf(),
        source,
    })?;
    let roots =
        reqwest::Certificate::from_pem_bundle(&pem).map_err(|source| TrustRootsError::NotPem {
            key,
            path: path.to_path_buf(),
            source,
        })?;
    if roots.is_empty() {
        return Err(TrustRootsError::Empty {
            key,
            path: path.to_path_buf(),
        });
    }
    Ok(roots)
}

/// `builder`, trusting the bundle at `ca_file` beside the platform roots when
/// one is configured.
///
/// # Errors
/// As [`read_trust_roots`].
pub fn trusting(
    builder: reqwest::ClientBuilder,
    key: &'static str,
    ca_file: Option<&Path>,
) -> Result<reqwest::ClientBuilder, TrustRootsError> {
    match ca_file {
        Some(path) => Ok(builder.tls_certs_merge(read_trust_roots(key, path)?)),
        None => Ok(builder),
    }
}
