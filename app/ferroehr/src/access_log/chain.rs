// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a checkpoint or an exported bundle of the access log fails, and the
//! digests a bundle is verified by (#5129, #6242).
//!
//! A bundle is one EHR's records signed whole by the exporting system: its
//! digest is the SHA-256 of its RFC 8785 canonical form without its proof,
//! and its records digest the SHA-256 of each record's uid and content
//! digest, in uid order, each field as its length in bytes, a colon, the
//! field and a line feed, after the version tag
//! `ferroehr.access_log.bundle.v1`. A tombstone keeps the content digest of
//! the entry it replaced, so a bundle verifies after retention removed
//! content inside it. No openEHR spec governs the transfer: our own design.

use crate::digest::sha256_hex;

/// The one digest algorithm of the seals and bundles, as a checkpoint names
/// it.
pub const ALGORITHM: &str = "SHA-256";

/// The version tag, the first field of a records digest's text.
pub const BUNDLE_TAG: &str = "ferroehr.access_log.bundle.v1";

/// One thing a checkpoint or a bundle fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainFinding {
    /// The seal at the checkpoint's sequence is not the one it signs.
    #[error("the seal at {sequence} is not the seal the checkpoint signs")]
    CheckpointDigest {
        /// The checkpoint's sequence.
        sequence: i64,
    },
    /// The checkpoint's proof does not verify against a trusted key.
    #[error("the checkpoint at {sequence} is not signed by a trusted key")]
    CheckpointProof {
        /// The checkpoint's sequence.
        sequence: i64,
    },
    /// The bundle's proof does not verify against a trusted key.
    #[error("the bundle is not signed by a trusted key")]
    BundleProof,
    /// A record of the bundle belongs to another system than the bundle.
    #[error("the record {uid} belongs to the system {system_id:?}")]
    OtherSystem {
        /// The record's uid.
        uid: String,
        /// The system it names.
        system_id: String,
    },
    /// A bundle names a different number of content digests than records.
    #[error("the bundle names {digests} content digests for {records} records")]
    Digests {
        /// The records it holds.
        records: usize,
        /// The digests it names.
        digests: usize,
    },
    /// An entry's content is not the content its digest names.
    #[error("the content of the entry {uid} was modified after it was written")]
    ContentDigest {
        /// The entry's uid.
        uid: String,
    },
}

fn field(text: &mut String, value: &str) {
    use std::fmt::Write as _;
    // NOTE: `impl fmt::Write for String` never fails
    // (<https://doc.rust-lang.org/std/fmt/trait.Write.html#impl-Write-for-String>).
    let _written = writeln!(text, "{}:{value}", value.len());
}

/// The records digest of `records`, pairs of uid and content digest.
#[must_use]
pub fn records_digest(records: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = records.iter().collect();
    sorted.sort();
    let mut text = String::with_capacity(sorted.len().saturating_mul(110).saturating_add(32));
    field(&mut text, BUNDLE_TAG);
    for (uid, digest) in sorted {
        field(&mut text, uid);
        field(&mut text, digest);
    }
    sha256_hex(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_records_digest_does_not_follow_the_order_given() {
        let a = ("b".to_owned(), "1".repeat(64));
        let b = ("a".to_owned(), "2".repeat(64));
        assert_eq!(
            records_digest(&[a.clone(), b.clone()]),
            records_digest(&[b, a])
        );
    }
}
