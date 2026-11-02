// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The optional-integration extensions of FerroEHR.
//!
//! Multimedia is carved out of the platform library behind the additive
//! `multimedia` cargo feature (tracker #1890; no openEHR spec governs this
//! surface — our own design/extension).

#[cfg(feature = "multimedia")]
pub mod multimedia;
