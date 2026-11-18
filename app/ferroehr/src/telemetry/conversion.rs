// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The representation changes the server makes outside the codec entry points.
//!
//! Each is reported under the codec's own `trace` target
//! ([`openehr_its::conversion::TARGET`]), so one counter sees every change a
//! request makes and the conversion budget holds it (#6007).
//!
//! No openEHR specification governs this: our own design.

/// Records one representation change `op` the server makes itself.
#[inline]
pub fn record(op: &'static str) {
    tracing::trace!(target: openehr_its::conversion::TARGET, op);
}
