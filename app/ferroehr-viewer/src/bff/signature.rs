// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `VERSION.signature` as the viewer names it, and a version's
//! attestations.
//!
//! The stored value tells its own kind (book `signing/digest.md`, What the
//! digest covers): FerroEHR's SHA-256 digest is `sha256:` and base64, and a
//! signature is an ASCII-armored `OpenPGP` detached signature (RFC 9580 §6),
//! whose public-key algorithm is the third octet of a version 4 or 6
//! signature packet (RFC 9580 §5.2.3) and 30 for ML-DSA-65+Ed25519 (RFC
//! 9980). The viewer verifies nothing: the CDR does on read. An attestation
//! is RM `ATTESTATION` (`org.openehr.rm.common.attestation.adoc`).

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use base64::Engine as _;
use serde_json::Value;

/// The kind of a stored signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureKind {
    /// FerroEHR's SHA-256 integrity digest.
    Digest,
    /// An `OpenPGP` signature, with the public-key algorithm id its packet
    /// names when it reads.
    OpenPgp(Option<u8>),
    /// A value of neither form, such as a client's own.
    Other,
}

/// A stored `VERSION.signature`: its kind and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Its kind.
    pub kind: SignatureKind,
    /// The value as stored.
    pub text: String,
}

/// The armor line that opens an `OpenPGP` signature.
const ARMOR_BEGIN: &str = "-----BEGIN PGP SIGNATURE-----";

impl Signature {
    /// The signature `text` names, `None` for an empty one.
    #[must_use]
    pub fn of(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let kind = if text.starts_with("sha256:") {
            SignatureKind::Digest
        } else if text.starts_with(ARMOR_BEGIN) {
            SignatureKind::OpenPgp(packet_algorithm(&dearmor(text)?))
        } else {
            SignatureKind::Other
        };
        Some(Self {
            kind,
            text: text.to_owned(),
        })
    }
}

/// The octets an armored message carries: the base64 lines after the
/// headers, up to the checksum or the closing line (RFC 9580 §6.2).
fn dearmor(text: &str) -> Option<Vec<u8>> {
    let mut lines = text.lines().map(str::trim).skip(1);
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if !line.contains(':') {
            return base64_body(std::iter::once(line).chain(lines));
        }
    }
    base64_body(lines)
}

/// The octets of the base64 lines `lines`, which end at the checksum or the
/// closing line; `None` when they are no base64.
fn base64_body<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Vec<u8>> {
    let body: String = lines
        .take_while(|line| !line.starts_with('=') && !line.starts_with("-----"))
        .collect();
    // NOTE: RFC 9580 §6.2; an armor that does not decode is a signature of unknown algorithm.
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

/// The public-key algorithm of the signature packet `octets` opens with,
/// `None` when it is no version 4 or 6 signature packet.
fn packet_algorithm(octets: &[u8]) -> Option<u8> {
    let first = *octets.first()?;
    if first & 0x80 == 0 {
        return None;
    }
    let (tag, skip) = if first & 0x40 == 0 {
        let skip = match first & 0x03 {
            0 => 2,
            1 => 3,
            2 => 5,
            _ => 1,
        };
        ((first >> 2) & 0x0F, skip)
    } else {
        let length = *octets.get(1)?;
        let skip = match length {
            0..=191 => 2,
            192..=223 => 3,
            255 => 6,
            _ => return None,
        };
        (first & 0x3F, skip)
    };
    if tag != 2 {
        return None;
    }
    let body = octets.get(skip..)?;
    match body.first() {
        Some(4 | 6) => body.get(2).copied(),
        _ => None,
    }
}

/// One `ATTESTATION` of a version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attestation {
    /// Who attested: the committer of its audit.
    pub attester: String,
    /// When.
    pub time: String,
    /// Why, in the attester's words.
    pub reason: String,
    /// Whether it is still pending.
    pub pending: bool,
    /// The paths of the items it attests, empty for the whole version.
    pub items: Vec<String>,
}

/// The text at `pointer` under `node`, empty when it holds none.
fn text_at(node: &Value, pointer: &str) -> String {
    node.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The attestations of the version `version`.
#[must_use]
pub fn attestations_of(version: &Value) -> Vec<Attestation> {
    version
        .get("attestations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|attestation| Attestation {
            attester: [
                text_at(attestation, "/committer/name"),
                text_at(attestation, "/committer/external_ref/id/value"),
            ]
            .into_iter()
            .find(|words| !words.is_empty())
            .unwrap_or_default(),
            time: text_at(attestation, "/time_committed/value"),
            reason: text_at(attestation, "/reason/value"),
            pending: attestation
                .get("is_pending")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            items: attestation
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|item| text_at(item, "/value"))
                .filter(|path| !path.is_empty())
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Signature, SignatureKind, attestations_of, packet_algorithm};
    use base64::Engine as _;

    /// An armored message carrying `octets`.
    fn armored(octets: &[u8]) -> String {
        format!(
            "-----BEGIN PGP SIGNATURE-----\n\n{}\n=abcd\n-----END PGP SIGNATURE-----",
            base64::engine::general_purpose::STANDARD.encode(octets)
        )
    }

    #[test]
    fn a_digest_and_a_composite_signature_name_their_kind() {
        let digest = Signature::of("sha256:jtWX/CULavvzX0ehjowv2XZPICTQhN1t0+AXHfbEaNc=");
        assert_eq!(digest.map(|s| s.kind), Some(SignatureKind::Digest));
        let v6 = armored(&[0xC2, 0x04, 0x06, 0x00, 30, 0x0A]);
        assert_eq!(
            Signature::of(&v6).map(|s| s.kind),
            Some(SignatureKind::OpenPgp(Some(30)))
        );
        let headed = v6.replacen("\n\n", "\nComment: signed\n\n", 1);
        assert_eq!(
            Signature::of(&headed).map(|s| s.kind),
            Some(SignatureKind::OpenPgp(Some(30)))
        );
        assert_eq!(Signature::of("  "), None);
        assert_eq!(
            Signature::of("a client's own").map(|s| s.kind),
            Some(SignatureKind::Other)
        );
    }

    #[test]
    fn only_a_v4_or_v6_signature_packet_names_an_algorithm() {
        assert_eq!(packet_algorithm(&[0x88, 0x04, 0x04, 0x00, 27]), Some(27));
        assert_eq!(packet_algorithm(&[0xC2, 0x02, 0x03, 0x05]), None);
        assert_eq!(packet_algorithm(&[0xC6, 0x02, 0x04, 0x00]), None);
        assert_eq!(packet_algorithm(&[0x02]), None);
        assert_eq!(packet_algorithm(&[]), None);
    }

    #[test]
    fn an_attestation_names_its_attester_reason_and_items() {
        let version = serde_json::json!({"attestations": [{
            "_type": "ATTESTATION",
            "committer": {"_type": "PARTY_IDENTIFIED", "name": "Dr Example"},
            "time_committed": {"value": "2026-11-26T12:00:00Z"},
            "reason": {"value": "Reviewed"},
            "is_pending": false,
            "items": [{"_type": "DV_EHR_URI", "value": "ehr:/compositions/1"}]
        }]});
        let found = attestations_of(&version);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].attester, "Dr Example");
        assert_eq!(found[0].reason, "Reviewed");
        assert_eq!(found[0].items, vec!["ehr:/compositions/1".to_owned()]);
        assert_eq!(attestations_of(&serde_json::json!({})), Vec::new());
    }
}
