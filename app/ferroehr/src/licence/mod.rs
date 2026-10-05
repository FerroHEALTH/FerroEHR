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
//! - [`verify`]: the chain check against the embedded primary key.
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
use pgp::packet::PublicKey;

/// The licensor's public certificates the server trusts, armored, embedded at
/// build time. Only their primary key packets are trust anchors; signing
/// subkeys arrive with each token.
///
/// Primary `5834C101A3F48578FD3CD1796C65DDBC5F712E51` (certify-only, no
/// expiry). A rotation of the primary appends the new certificate here and
/// keeps the old one for one release so existing tokens keep verifying.
pub const ANCHOR_CERTIFICATES: &[&str] = &[r"-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEaqQCCBYJKwYBBAHaRw8BAQdACGyBINh2SN3BQoB2FcKYnkpTw05o7uieeRgD
4AmdldG0KkZlcnJvRUhSIExpY2Vuc2luZyA8bGljZW5zaW5nQGZlcnJvZWhyLmV1
PoivBBMWCgBXFiEEWDTBAaP0hXj9PNF5bGXdvF9xLlEFAmqkAggbFIAAAAAABAAO
bWFudTIsMi41KzEuMTIsMCwzAhsBBQsJCAcCAiICBhUKCQgLAgQWAgMBAh4HAheA
AAoJEGxl3bxfcS5RnOABAMpA6e8fTgzV5Wzxy9kejqkejpvrSiyneQiH4E/iTwJ/
AQCokShD2yZbOJhzuEykQ43Qb8/xXmOC+fMvzY0wvtutDbgzBGqkAg4WCSsGAQQB
2kcPAQEHQEmYgjq48nNIaU0qpxg40dluLnejqQegSibTJhGZkbnIiQERBBgWCgBC
FiEEWDTBAaP0hXj9PNF5bGXdvF9xLlEFAmqkAg4bFIAAAAAABAAObWFudTIsMi41
KzEuMTIsMCwzAhsCBQkDwmcAAIEJEGxl3bxfcS5RdiAEGRYKAB0WIQRcZw0ojRZl
LZpxk6nybicDBUbrXwUCaqQCDgAKCRDybicDBUbrX1BnAP4pO+NQiBH7EhD2PDOf
8b34kvR1NFTtwN40yS0NKP1tuwEA87l2eDYZ/wRerPOjf6LEqVAHayIo1VtgVRjO
Eyil1QfI3AEAl5ANcHcjtUcbbCJHffy5ca2zSmmb0GgHRu1bx3Zaj70A/joWZTkq
cnH8F49CyICuGOx8b6GIx4aHSI0GzqWa7b0O
=TSvN
-----END PGP PUBLIC KEY BLOCK-----
"];

/// The licence token every build carries: the licensor's `non-commercial` grant.
///
/// A deployment without its own token runs under this explicit, signed
/// licence (id `01a090a8-59e4-76da-92f5-3f7e4ea9d7c0`, valid to 2099-12-31)
/// and stamps identifiers with its key; a configured `[licence] file` takes
/// precedence. A unit test proves it verifies against
/// [`ANCHOR_CERTIFICATES`], so a release can never ship a token its own
/// verifier refuses.
pub const EMBEDDED_TOKEN: &str = r#"-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

{
  "licence_id": "01a090a8-59e4-76da-92f5-3f7e4ea9d7c0",
  "licensee": "Everyone, under the Business Source License 1.1",
  "issued": "2026-09-11",
  "not_before": "2026-09-11",
  "not_after": "2099-12-31",
  "use": "non-commercial"
}
-----BEGIN PGP SIGNATURE-----

iJEEARYKADkWIQRcZw0ojRZlLZpxk6nybicDBUbrXwUCaqQCIxsUgAAAAAAEAA5t
YW51MiwyLjUrMS4xMiwwLDMACgkQ8m4nAwVG619P+QEAgvra2wbffXTbaqdnyZje
xRk4esh7OIYbQAtJ0RNxO2gBAIwGXPvsJ8Zb0M+dQ3DRy5rDka2rH4vELrrrLbM1
H/sB
=ogu+
-----END PGP SIGNATURE-----
-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEaqQCCBYJKwYBBAHaRw8BAQdACGyBINh2SN3BQoB2FcKYnkpTw05o7uieeRgD
4AmdldG0KkZlcnJvRUhSIExpY2Vuc2luZyA8bGljZW5zaW5nQGZlcnJvZWhyLmV1
PoivBBMWCgBXFiEEWDTBAaP0hXj9PNF5bGXdvF9xLlEFAmqkAggbFIAAAAAABAAO
bWFudTIsMi41KzEuMTIsMCwzAhsBBQsJCAcCAiICBhUKCQgLAgQWAgMBAh4HAheA
AAoJEGxl3bxfcS5RnOABAMpA6e8fTgzV5Wzxy9kejqkejpvrSiyneQiH4E/iTwJ/
AQCokShD2yZbOJhzuEykQ43Qb8/xXmOC+fMvzY0wvtutDbgzBGqkAg4WCSsGAQQB
2kcPAQEHQEmYgjq48nNIaU0qpxg40dluLnejqQegSibTJhGZkbnIiQERBBgWCgBC
FiEEWDTBAaP0hXj9PNF5bGXdvF9xLlEFAmqkAg4bFIAAAAAABAAObWFudTIsMi41
KzEuMTIsMCwzAhsCBQkDwmcAAIEJEGxl3bxfcS5RdiAEGRYKAB0WIQRcZw0ojRZl
LZpxk6nybicDBUbrXwUCaqQCDgAKCRDybicDBUbrX1BnAP4pO+NQiBH7EhD2PDOf
8b34kvR1NFTtwN40yS0NKP1tuwEA87l2eDYZ/wRerPOjf6LEqVAHayIo1VtgVRjO
Eyil1QfI3AEAl5ANcHcjtUcbbCJHffy5ca2zSmmb0GgHRu1bx3Zaj70A/joWZTkq
cnH8F49CyICuGOx8b6GIx4aHSI0GzqWa7b0O
=TSvN
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

/// The primary key packets of every embedded anchor certificate.
///
/// # Errors
/// [`AnchorError`] naming the first certificate that does not parse; a
/// build-time constant, so a unit test keeps this unreachable in a release.
pub fn anchors() -> Result<Vec<PublicKey>, AnchorError> {
    ANCHOR_CERTIFICATES
        .iter()
        .enumerate()
        .map(|(index, armored)| {
            SignedPublicKey::from_string(armored)
                .map(|(certificate, _headers)| certificate.primary_key)
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
