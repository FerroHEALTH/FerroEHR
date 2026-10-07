// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The licence: what a licence token holds, how the server verifies it, and
//! the identifier stamp that records which licence was in force when a
//! record was written.
//!
//! `FerroEHR` is BUSL-1.1. Every build embeds the licensor's `non-commercial`
//! grant, what the licence gives everyone; production use in the course of a
//! business needs a `commercial` grant, installed through `[licence] file`.
//! The server behaves identically under either. Nothing here gates a
//! feature, an endpoint, a limit or a message; the only effect of the licence
//! in force is which [`stamp::StampKey`] the server mints identifiers with,
//! so that production data says which grant it was written under.
//!
//! - [`document`]: the licence document, six fields of canonical JSON.
//! - [`token`]: the file the licensee installs, a cleartext-signed licence
//!   followed by the issuer's public certificate (RFC 9580 §7 and §10.1).
//! - [`verify`]: the check of a token against the embedded master keys.
//! - [`stamp`]: sixteen keyed bits in the last two bytes of every
//!   server-minted `UUIDv7`.
//! - [`config`] and [`state`]: the `[licence]` section and the boot outcome.
//!
//! No openEHR spec governs licensing or identifier entropy beyond RFC 9562's
//! layout, which the stamp respects. Our own design.

pub mod config;
pub mod document;
pub mod stamp;
pub mod state;
pub mod token;
pub mod verify;

use pgp::composed::{Deserializable as _, SignedPublicKey};

/// The licensor's public certificates the server trusts, armored.
///
/// The two master keys of Cadasto B.V., each generated on its holder's
/// hardware token. A licence must be signed by one of them directly; whether a
/// key may sign and when it expires are read from these certificates, never
/// from the copy a token carries.
///
/// `F2D214FC30698BA65A846B2BF2EA48D4C316406D` and
/// `0BCE848225EFAECA912175292DBFB014CA506311` (Ed25519, certify and sign, no
/// expiry). Adding or dropping a key is a release.
pub const ANCHOR_CERTIFICATES: &[&str] = &[
    r"-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEasZ5hRYJKwYBBAHaRw8BAQdAuu7e0mjOwr0pPtHaTts30zbi0WvJ/JPXSqvn
ggn41nm0MEZlcnJvSEVBTFRIIExpY2Vuc2luZyA8bGljZW5zaW5nQEZlcnJvSEVB
TFRILmV1PoivBBMWCgBXFiEE8tIU/DBpi6ZahGsr8upI1MMWQG0FAmrGeYUbFIAA
AAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsDBQsJCAcCAiICBhUKCQgLAgQWAgMB
Ah4HAheAAAoJEPLqSNTDFkBtLBkA/A5wIEVYO/mpakMV+T4I9Y8Z+HCuLh2Oy722
eEn6W9QCAQDrk2JF5dRGbMiDaTquHYTWUVg+NEmt0bxxbksz3ZtdC7gzBGrGeYUW
CSsGAQQB2kcPAQEHQDnRh6fddjdslf4QMGGuGp5hzJGESXTxg5cW60CLXfrbiJQE
GBYKADwWIQTy0hT8MGmLplqEayvy6kjUwxZAbQUCasZ5hRsUgAAAAAAEAA5tYW51
MiwyLjUrMS4xMiwwLDMCGyAACgkQ8upI1MMWQG2eaQEAhw+0ttO0kaJPlY9pz0Ho
V/WyYaRSFYOCKmVsZOd1Jv4A/00ciZ6Oj9a+jxA2MLlfxZjKkT1K8OSySOTtfYIL
2UkDuDgEasZ5hRIKKwYBBAGXVQEFAQEHQNoqEMbmvYG0yb2oS4V9IUbBFztH3rKV
OePtWxlmbc0gAwEIB4iUBBgWCgA8FiEE8tIU/DBpi6ZahGsr8upI1MMWQG0FAmrG
eYUbFIAAAAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsMAAoJEPLqSNTDFkBtVVMA
/3BFxap7S+j/RlaVqjE98hWf1dbFrqa3ZOGodNlrAwy+AP9CwsXZL5qxnh5lbLZ1
R4hbkCkhO1gJ8YPQI35crFB2Cw==
=vqzS
-----END PGP PUBLIC KEY BLOCK-----
",
    r"-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEasZ7VRYJKwYBBAHaRw8BAQdArAN41i8xSz4cfF5QQTnwUGJUMdF8VKvpNu+E
0/A/zoG0MEZlcnJvSEVBTFRIIExpY2Vuc2luZyA8bGljZW5zaW5nQEZlcnJvSEVB
TFRILmV1PoivBBMWCgBXFiEEC86EgiXvrsqRIXUpLb+wFMpQYxEFAmrGe1UbFIAA
AAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsDBQsJCAcCAiICBhUKCQgLAgQWAgMB
Ah4HAheAAAoJEC2/sBTKUGMRW34A/3rzy1in1tBtLC39RgAISkPWbx4FEa3xnd5N
HadtQqGWAP41liDIRpy4w2Yni8FcXzH5akOFpsfc2Su7ARzlmUCECbgzBGrGe1UW
CSsGAQQB2kcPAQEHQMmBwa3DybzH3oCsrojGCwZfSlfqw8t92oEfDJl0zYBniJQE
GBYKADwWIQQLzoSCJe+uypEhdSktv7AUylBjEQUCasZ7VRsUgAAAAAAEAA5tYW51
MiwyLjUrMS4xMiwwLDMCGyAACgkQLb+wFMpQYxEjzAD+IsHmliSr3LS5HDTmOpSy
Iq36OPD8NKe3XOh1vrH2AhwBAPeompaj41Q8hOtWqpX084QnKwfUtUprHfOykx+l
QywMuDgEasZ7VRIKKwYBBAGXVQEFAQEHQJC93RnYJieoJANYMb196K6/mDKfHnXf
u/eyWMZ4qewSAwEIB4iUBBgWCgA8FiEEC86EgiXvrsqRIXUpLb+wFMpQYxEFAmrG
e1UbFIAAAAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsMAAoJEC2/sBTKUGMRYA4A
/3li9dausUjrS+FlgczSE1cQz6KNVXxQi8Q3khpY8bieAQCGbFemHxT4tlB/8XtR
WfHi1z2bYxIUhRN3SML4TVuTDA==
=xipr
-----END PGP PUBLIC KEY BLOCK-----
",
];

/// The licence token every build carries: the licensor's `non-commercial` grant.
///
/// A deployment without its own token runs under this explicit, signed
/// licence (id `01a1175c-cd67-777d-bdc0-c95bf1c900d3`, valid to 2099-12-31)
/// and stamps identifiers with its key; a configured `[licence] file` takes
/// precedence. A unit test proves it verifies against
/// [`ANCHOR_CERTIFICATES`], so a release can never ship a token its own
/// verifier refuses.
pub const EMBEDDED_TOKEN: &str = r#"-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

{
  "licence_id": "01a1175c-cd67-777d-bdc0-c95bf1c900d3",
  "licensee": "Everyone, under the Business Source License 1.1",
  "issued": "2026-10-07",
  "not_before": "2026-10-07",
  "not_after": "2099-12-31",
  "use": "non-commercial"
}
-----BEGIN PGP SIGNATURE-----

iJEEARYKADkWIQTy0hT8MGmLplqEayvy6kjUwxZAbQUCasZ+KBsUgAAAAAAEAA5t
YW51MiwyLjUrMS4xMiwwLDMACgkQ8upI1MMWQG3/1AEAl6V4VNwHhlilgj+v6m3e
Y7mPlIbO2WZBB1NDzo8Dvk8A+wW7wRPBsZZah7vAws8SxRQXBNke5HvppmFsdQc0
P+wF
=QA+/
-----END PGP SIGNATURE-----
-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEasZ5hRYJKwYBBAHaRw8BAQdAuu7e0mjOwr0pPtHaTts30zbi0WvJ/JPXSqvn
ggn41nm0MEZlcnJvSEVBTFRIIExpY2Vuc2luZyA8bGljZW5zaW5nQEZlcnJvSEVB
TFRILmV1PoivBBMWCgBXFiEE8tIU/DBpi6ZahGsr8upI1MMWQG0FAmrGeYUbFIAA
AAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsDBQsJCAcCAiICBhUKCQgLAgQWAgMB
Ah4HAheAAAoJEPLqSNTDFkBtLBkA/A5wIEVYO/mpakMV+T4I9Y8Z+HCuLh2Oy722
eEn6W9QCAQDrk2JF5dRGbMiDaTquHYTWUVg+NEmt0bxxbksz3ZtdC7gzBGrGeYUW
CSsGAQQB2kcPAQEHQDnRh6fddjdslf4QMGGuGp5hzJGESXTxg5cW60CLXfrbiJQE
GBYKADwWIQTy0hT8MGmLplqEayvy6kjUwxZAbQUCasZ5hRsUgAAAAAAEAA5tYW51
MiwyLjUrMS4xMiwwLDMCGyAACgkQ8upI1MMWQG2eaQEAhw+0ttO0kaJPlY9pz0Ho
V/WyYaRSFYOCKmVsZOd1Jv4A/00ciZ6Oj9a+jxA2MLlfxZjKkT1K8OSySOTtfYIL
2UkDuDgEasZ5hRIKKwYBBAGXVQEFAQEHQNoqEMbmvYG0yb2oS4V9IUbBFztH3rKV
OePtWxlmbc0gAwEIB4iUBBgWCgA8FiEE8tIU/DBpi6ZahGsr8upI1MMWQG0FAmrG
eYUbFIAAAAAABAAObWFudTIsMi41KzEuMTIsMCwzAhsMAAoJEPLqSNTDFkBtVVMA
/3BFxap7S+j/RlaVqjE98hWf1dbFrqa3ZOGodNlrAwy+AP9CwsXZL5qxnh5lbLZ1
R4hbkCkhO1gJ8YPQI35crFB2Cw==
=vqzS
-----END PGP PUBLIC KEY BLOCK-----
"#;

/// An embedded anchor certificate did not parse.
#[derive(Debug, thiserror::Error)]
#[error("embedded licence anchor {index} is not an armored public certificate")]
pub struct AnchorError {
    /// Position in [`ANCHOR_CERTIFICATES`].
    pub index: usize,
    /// The parse failure.
    #[source]
    pub source: pgp::errors::Error,
}

/// Every embedded anchor certificate, parsed: the trust anchors the verifier
/// takes.
///
/// # Errors
/// [`AnchorError`] naming the first certificate that does not parse; a
/// build-time constant, so a unit test keeps this unreachable in a release.
pub fn anchors() -> Result<Vec<SignedPublicKey>, AnchorError> {
    ANCHOR_CERTIFICATES
        .iter()
        .enumerate()
        .map(|(index, armored)| {
            SignedPublicKey::from_string(armored)
                .map(|(certificate, _headers)| certificate)
                .map_err(|source| AnchorError { index, source })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_anchor_parses() {
        let anchors = anchors().expect("embedded anchors parse");
        assert_eq!(anchors.len(), ANCHOR_CERTIFICATES.len());
    }

    /// A build never ships an embedded token its own verifier refuses, and the
    /// embedded grant is the non-commercial one.
    #[test]
    fn the_embedded_token_verifies_as_non_commercial() {
        let anchors = anchors().expect("embedded anchors parse");
        assert!(!anchors.is_empty(), "a release embeds at least one anchor");
        let token = token::Token::parse(EMBEDDED_TOKEN).expect("embedded token parses");
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date();
        let verified = verify::verify(&token, &anchors, today).expect("embedded token verifies");
        assert_eq!(verified.licence.permitted_use, document::Use::NonCommercial);
    }
}
