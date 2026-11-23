// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The optional-integration extensions of FerroEHR.
//!
//! Multimedia sits behind the `multimedia` cargo feature. No openEHR spec
//! governs this surface: our own design.

#[cfg(feature = "multimedia")]
pub mod multimedia;
