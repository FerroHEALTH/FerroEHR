// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR integration core: the mapping model and FLAT builder, the outbound
//! reverse-map, the feeder-audit probe, the ATNA `AuditEvent` renderer, and the
//! terminology-response decoder.
//!
//! No openEHR spec governs FHIR resource representation — our own
//! design/extension. This module owns only the pure conversion logic and the
//! typed `fhir-model` resource surface, which no other crate names; the
//! service and REST glue lives in the platform crate behind its `fhir` feature.
//!
//! The connector speaks FHIR R4, the release its wire advertises (`/fhir/r4`).
//! Every resource it builds and every data type they carry is outside R4B's
//! changed set, so the documents are equally valid under either release:
//! "implementers that do not use the specific portions where changes have been
//! made can continue to use either R4 or R4B without any functional difference"
//! (<https://hl7.org/fhir/R4B/r4b-explanation.html>). [`mapping`] and
//! [`reverse`] fix no resource at all — the resource type is mapping data.
//! [`terminology`] stays R4B by its own design, decoding responses from external
//! servers whose release is the remote's property.

pub mod audit;
pub mod feeder_audit;
pub mod mapping;
pub mod reverse;
pub mod terminology;

/// The EHR subject a mapped inbound resource addresses.
///
/// The mapping's output identity (id + namespace, a person by definition of
/// the inbound connector), which the platform resolves through its EHR index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSubject {
    /// The subject identifier value.
    pub id: String,
    /// The identifier namespace.
    pub namespace: String,
}
