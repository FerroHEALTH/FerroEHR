// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The digest a search token is stored and found by, computed by the server
//! with the same bytes as `audit.search_token_digest(parameter, system,
//! code)` (`migrations/audit/0017`, `0043`), so the intake sends each token's
//! digest and the database computes none per token (#5889).
//!
//! The database hashes the text `PostgreSQL` writes for
//! `jsonb_build_array(parameter, system, code)`: the three elements between
//! brackets, separated by a comma and a space, a NULL as `null`, and each
//! text as a JSON string escaped as `PostgreSQL`'s `escape_json` escapes it
//! (`"`, `\`, backspace, form feed, newline, carriage return and tab by their
//! short escapes, every other character below U+0020 as `\u00xx` in
//! lower-case hexadecimal, and every other character as itself). The
//! `CHECK` constraint of `audit_search_token` compares the two whenever a
//! token is stored, and the verifier does again. No openEHR spec governs the
//! search tokens: our own design.

use sha2::Digest as _;

/// The first 16 octets of the SHA-256 over the token's JSON array text.
#[must_use]
pub(crate) fn token_digest(parameter: &str, system: Option<&str>, code: Option<&str>) -> [u8; 16] {
    let mut text = String::with_capacity(
        8 + parameter.len() + system.map_or(4, str::len) + code.map_or(4, str::len),
    );
    text.push('[');
    push_json(&mut text, Some(parameter));
    text.push_str(", ");
    push_json(&mut text, system);
    text.push_str(", ");
    push_json(&mut text, code);
    text.push(']');
    let full = sha2::Sha256::digest(text.as_bytes());
    let mut digest = [0_u8; 16];
    for (out, byte) in digest.iter_mut().zip(full.iter()) {
        *out = *byte;
    }
    digest
}

/// `value` as `PostgreSQL` writes a JSON string or a JSON null.
fn push_json(out: &mut String, value: Option<&str>) {
    let Some(value) = value else {
        out.push_str("null");
        return;
    };
    crate::aql::fragment::write_string(value, out);
}

#[cfg(test)]
mod tests {
    use super::token_digest;

    fn hex(digest: [u8; 16]) -> String {
        digest.iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            write!(hex, "{byte:02x}").expect("a String takes the digits");
            hex
        })
    }

    /// Each expected value is what `audit.search_token_digest` answers on
    /// `PostgreSQL` 18.6 for the same three values.
    #[test]
    fn the_digest_is_the_databases_for_every_escape() {
        let cases: [(&str, Option<&str>, Option<&str>, &str); 6] = [
            (
                "type",
                None,
                Some("rest"),
                "4e187e9d036ca202950c91c87b3dc503",
            ),
            (
                "entity.identifier",
                Some("urn:ietf:rfc:3986"),
                Some("urn:uuid:01a1e66c-79b1-703e-b1e3-d9766867bbe5"),
                "66bc229fa8f3050dd1c2ba94b979e1a3",
            ),
            ("address", None, None, "cafceb6b5c6f37a12c2e08238f93de53"),
            (
                "agent.identifier",
                Some(""),
                Some(""),
                "296d4a180536db479a6c84080ede4d4e",
            ),
            (
                "subtype",
                Some("q\"uote\\\\back"),
                Some("tab\there\nnl\rcr\u{8}bs\u{c}ff"),
                "93cdad2e4d58ea3040676a2e484c3e58",
            ),
            (
                "patient.identifier",
                Some("ctl\u{1}\u{1f}\u{7f}"),
                Some("Zoë 日本 😀 /slash"),
                "55689b7090e017ec5759f4d25c8989ad",
            ),
        ];
        for (parameter, system, code, expected) in cases {
            assert_eq!(
                hex(token_digest(parameter, system, code)),
                expected,
                "{parameter} {system:?} {code:?}"
            );
        }
    }
}
