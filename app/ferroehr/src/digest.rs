// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SHA-256 of a text in lower-case hexadecimal, the one spelling of a
//! digest the access-log chain, the party binding and the cohort keys share.
//!
//! No openEHR spec governs these digests: our own design.

use sha2::Digest as _;

/// The SHA-256 of `text`'s UTF-8 octets, in lower-case hexadecimal.
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            // NOTE: `impl fmt::Write for String` never fails
            // (<https://doc.rust-lang.org/std/fmt/trait.Write.html#impl-Write-for-String>).
            let _written = write!(hex, "{byte:02x}");
            hex
        })
}
