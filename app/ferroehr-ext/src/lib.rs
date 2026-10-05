// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The optional-integration extensions of FerroEHR.
//!
//! FHIR, events, and multimedia are carved out of the platform library
//! behind one additive cargo feature each — `fhir`, `events`, `multimedia`
//! (tracker #1890; no openEHR spec governs these surfaces — our own
//! design/extensions).

#[cfg(feature = "multimedia")]
pub mod multimedia;

#[cfg(feature = "events")]
pub mod events;

#[cfg(feature = "fhir")]
pub mod fhir;
