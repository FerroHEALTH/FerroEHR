// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The token file: a cleartext-signed licence followed by the public
//! certificate that signed it.
//!
//! One armored file, two blocks, in this order:
//!
//! 1. `-----BEGIN PGP SIGNED MESSAGE-----` … `-----END PGP SIGNATURE-----`:
//!    the canonical licence JSON, signed by a signing subkey (RFC 9580 §7,
//!    the cleartext signature framework).
//! 2. `-----BEGIN PGP PUBLIC KEY BLOCK-----` … `-----END PGP PUBLIC KEY BLOCK-----`:
//!    the issuer's public certificate, primary plus subkeys with their binding
//!    signatures (RFC 9580 §10.1, transferable public keys).
//!
//! Bundling the certificate is what makes subkey rotation free: the verifier
//! embeds only the primary, learns the current subkeys from the token, and
//! checks the bindings itself.

use pgp::composed::{CleartextSignedMessage, Deserializable as _, SignedPublicKey};

/// Armor header opening the signed licence.
pub const SIGNED_MESSAGE_BEGIN: &str = "-----BEGIN PGP SIGNED MESSAGE-----";
/// Armor footer closing the signed licence.
pub const SIGNATURE_END: &str = "-----END PGP SIGNATURE-----";
/// Armor header opening the issuer certificate.
pub const CERTIFICATE_BEGIN: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";
/// Armor footer closing the issuer certificate.
pub const CERTIFICATE_END: &str = "-----END PGP PUBLIC KEY BLOCK-----";

/// Why a token file could not be read as a token.
///
/// Structural only: a token that parses may still fail [`crate::licence::verify`].
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// No cleartext-signed block was found.
    #[error("token carries no signed licence block")]
    MissingSignedMessage,
    /// No public key block was found.
    #[error("token carries no issuer certificate block")]
    MissingCertificate,
    /// The signed block is not a well-formed cleartext signature.
    #[error("signed licence block is malformed")]
    Message(#[source] pgp::errors::Error),
    /// The certificate block is not a well-formed transferable public key.
    #[error("issuer certificate block is malformed")]
    Certificate(#[source] pgp::errors::Error),
}

/// A parsed, not yet verified, token.
#[derive(Debug)]
pub struct Token {
    message: CleartextSignedMessage,
    certificate: SignedPublicKey,
}

impl Token {
    /// Parse a token file.
    ///
    /// # Errors
    /// [`TokenError`] when either block is absent or malformed.
    pub fn parse(text: &str) -> Result<Self, TokenError> {
        let signed = block(text, SIGNED_MESSAGE_BEGIN, SIGNATURE_END)
            .ok_or(TokenError::MissingSignedMessage)?;
        let certificate = block(text, CERTIFICATE_BEGIN, CERTIFICATE_END)
            .ok_or(TokenError::MissingCertificate)?;
        let (message, _headers) =
            CleartextSignedMessage::from_string(signed).map_err(TokenError::Message)?;
        let (certificate, _headers) =
            SignedPublicKey::from_string(certificate).map_err(TokenError::Certificate)?;
        Ok(Self {
            message,
            certificate,
        })
    }

    /// The signed licence block.
    #[must_use]
    pub fn message(&self) -> &CleartextSignedMessage {
        &self.message
    }

    /// The issuer certificate bundled with the token.
    #[must_use]
    pub fn certificate(&self) -> &SignedPublicKey {
        &self.certificate
    }

    /// The text the signature covers, normalised as the signer hashed it.
    #[must_use]
    pub fn signed_text(&self) -> String {
        self.message.signed_text()
    }
}

/// Compose a token file from its two armored blocks.
#[must_use]
pub fn assemble(signed_message: &str, certificate: &str) -> String {
    format!(
        "{}\n{}\n",
        signed_message.trim_end(),
        certificate.trim_end()
    )
}

/// The first `begin` … `end` span of `text`, inclusive of both markers.
fn block<'a>(text: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let start = text.find(begin)?;
    let tail = text.get(start..)?;
    let end_rel = tail.find(end)?;
    tail.get(..end_rel.checked_add(end.len())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_blocks_are_named() {
        assert!(matches!(
            Token::parse("nothing here"),
            Err(TokenError::MissingSignedMessage)
        ));
        let only_message = format!("{SIGNED_MESSAGE_BEGIN}\nHash: SHA512\n\nx\n{SIGNATURE_END}\n");
        assert!(matches!(
            Token::parse(&only_message),
            Err(TokenError::MissingCertificate)
        ));
    }

    #[test]
    fn block_extraction_is_inclusive() {
        let text = "junk\nBEGIN\nbody\nEND\nmore";
        assert_eq!(block(text, "BEGIN", "END"), Some("BEGIN\nbody\nEND"));
        assert_eq!(block(text, "BEGIN", "NOPE"), None);
    }

    #[test]
    fn assemble_puts_one_newline_between_blocks() {
        assert_eq!(assemble("A\n\n", "B"), "A\nB\n");
    }
}
