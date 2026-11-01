// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deterministic UUID the example generator writes for an output-form
//! uid.
//!
//! No openEHR specification governs this: our own design.

/// A deterministic, RFC-4122-shaped UUID derived from `seed` (FNV-1a; no
/// randomness). Not a cryptographic v5, but syntactically a valid UUID and stable
/// for a given seed — sufficient for a pedagogical example's `OBJECT_VERSION_ID`.
pub(crate) fn deterministic_uuid(seed: &str) -> String {
    let h1 = fnv1a64(seed.as_bytes(), 0xcbf2_9ce4_8422_2325);
    let h2 = fnv1a64(seed.as_bytes(), 0x8422_2325_cbf2_9ce4);
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&h1.to_be_bytes());
    b[8..].copy_from_slice(&h2.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x50; // version 5 nibble
    b[8] = (b[8] & 0x3f) | 0x80; // RFC-4122 variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15],
    )
}

fn fnv1a64(data: &[u8], basis: u64) -> u64 {
    let mut hash = basis;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
