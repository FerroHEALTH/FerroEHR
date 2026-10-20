// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed FHIR surface the platform keeps: the terminology-response
//! decoder. The ATNA `AuditEvent` is written by the shared audit crates.
//!
//! No openEHR spec governs FHIR resource representation — our own
//! design/extension. This module owns only the pure conversion logic and the
//! typed `fhir-model` resource surface, which no other crate names; the
//! service and REST glue lives in the platform crate behind its `fhir` feature.
//! FHIR R4 resource mapping is not part of this crate.
//!
//! [`terminology`] stays R4B by its own design, decoding responses from
//! external servers whose release is the remote's property.

pub mod terminology;
