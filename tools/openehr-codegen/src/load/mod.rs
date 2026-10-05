// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 1 — LOAD. Parse the vendored inputs into verbatim in-memory models,
//! with no analysis or decisions: the BMM meta-model ([`bmm`]), the XSD reader
//! ([`xsd`]) and the OAS reader ([`oas`]). Every later stage consumes these
//! loaded models; none of them reach back to the raw files. The generated
//! crates' hand-written `*_impl.rs` siblings ([`impls`]) are loaded the same
//! way — they are an input the emitter reads, never something it decides.

pub(crate) mod bmm;
pub(crate) mod impls;
pub(crate) mod oas;
pub(crate) mod xsd;
