// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The representation changes the codec entry points make, reported one
//! `trace` event each under [`TARGET`] when the `tracing` feature is on.
//!
//! A server counts these per request to hold a conversion budget: a request
//! that parses its body once and renders its answer once records two. No
//! openEHR specification governs this: our own design.

/// The `tracing` target every conversion event carries.
pub const TARGET: &str = "openehr::conversion";

/// Records one conversion `op` of a value of type `T`; a no-op without the
/// `tracing` feature.
#[inline]
#[cfg_attr(
    not(feature = "tracing"),
    expect(
        clippy::extra_unused_type_parameters,
        reason = "the type is read only by the `tracing` event"
    )
)]
pub(crate) fn record<T: ?Sized>(op: &'static str) {
    #[cfg(feature = "tracing")]
    tracing::trace!(target: TARGET, op, of = std::any::type_name::<T>());
    #[cfg(not(feature = "tracing"))]
    let _ = op;
}
