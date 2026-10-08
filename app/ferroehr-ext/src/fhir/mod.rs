// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed FHIR surface the platform keeps: the ATNA `AuditEvent` renderer
//! and the terminology-response decoder.
//!
//! No openEHR spec governs FHIR resource representation — our own
//! design/extension. This module owns only the pure conversion logic and the
//! typed `fhir-model` resource surface, which no other crate names; the
//! service and REST glue lives in the platform crate behind its `fhir` feature.
//! FHIR R4 resource mapping is not part of this crate.
//!
//! The two submodules carry two release identities. [`audit`] renders the
//! `AuditEvent` the ITI-81 retrieval serves under `/fhir/r4`; the resource is
//! unchanged between R4 and R4B (<https://hl7.org/fhir/R4B/auditevent.html>).
//! [`terminology`] stays R4B by its own design, decoding responses from
//! external servers whose release is the remote's property.

pub mod audit;
pub mod terminology;
