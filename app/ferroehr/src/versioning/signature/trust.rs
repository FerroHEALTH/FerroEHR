// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `OpenPGP` certificates of other systems this deployment trusts to have
//! signed what it receives from them, such as the checkpoint of an imported
//! access log.
//!
//! Which systems a deployment trusts is its own decision, so the certificates
//! are trust configuration read at boot. No openEHR spec governs it: our own
//! design. The signature form is the keyring's (RM common
//! `master06-change_control_package.adoc` §Digital Signature, RFC 4880).

use std::path::{Path, PathBuf};

use pgp::composed::{Deserializable as _, DetachedSignature, SignedPublicKey};

use super::key::{KeyError, verify_against_certificate};

/// Armored `OpenPGP` public certificates trusted to verify signatures.
#[derive(Default)]
pub struct TrustedCertificates {
    certificates: Vec<SignedPublicKey>,
}

impl std::fmt::Debug for TrustedCertificates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedCertificates")
            .field("certificates", &self.certificates.len())
            .finish()
    }
}

impl TrustedCertificates {
    /// Read the armored public certificates at `paths`.
    ///
    /// # Errors
    /// [`KeyError::Read`] for a file that does not read and
    /// [`KeyError::Parse`] for one that holds no armored certificate.
    pub fn load(paths: &[PathBuf]) -> Result<Self, KeyError> {
        let certificates = paths
            .iter()
            .map(|path| certificate(path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { certificates })
    }

    /// Trust the armored public certificates `armored`.
    ///
    /// # Errors
    /// [`KeyError::Parse`] for a text that holds no armored certificate.
    pub fn from_armored<'a>(armored: impl IntoIterator<Item = &'a str>) -> Result<Self, KeyError> {
        let certificates = armored
            .into_iter()
            .map(|text| {
                SignedPublicKey::from_string(text)
                    .map(|(certificate, _headers)| certificate)
                    .map_err(KeyError::Parse)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { certificates })
    }

    /// How many certificates are trusted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.certificates.len()
    }

    /// Whether no certificate is trusted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.certificates.is_empty()
    }

    /// Whether `armored` is a detached signature over `data` by one of the
    /// trusted certificates, its primary key or a subkey.
    #[must_use]
    pub fn verifies(&self, data: &[u8], armored: &str) -> bool {
        let Ok((signature, _headers)) = DetachedSignature::from_string(armored) else {
            return false;
        };
        self.certificates
            .iter()
            .any(|certificate| verify_against_certificate(&signature, certificate, data))
    }
}

/// The armored public certificate at `path`.
fn certificate(path: &Path) -> Result<SignedPublicKey, KeyError> {
    let armored = std::fs::read_to_string(path).map_err(|source| KeyError::Read {
        path: path.display().to_string(),
        source,
    })?;
    SignedPublicKey::from_string(&armored)
        .map(|(certificate, _headers)| certificate)
        .map_err(KeyError::Parse)
}
