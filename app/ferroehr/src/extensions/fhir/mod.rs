// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR connector: mapping store + inbound ingest + read façade + outbound
//! reverse-map.
//!
//! **No openEHR spec governs this — our own design/extension.** master14's
//! integration model is archetype-to-archetype data conversion
//! (`GENERIC_ENTRY` with `FEEDER_AUDIT`), not FHIR resources; this connector
//! maps directly to
//! *designed* templates (mapping-as-data), a different, spec-silent mechanism.
//! Gate: the `/fhir/r4/*` + `/admin/fhir_mapping` routes are config-gated in
//! `ferroehr-rest`; the outbound emitter behind [`config::FhirOutboundConfig`].
//!
//! Concerns, all on `FerroEhrService`:
//! * the **mapping store** — CRUD over `fhir_mapping` (the deployable
//!   "mapping-as-data" artefacts), mirroring the event-subscription store;
//! * the **inbound ingest** — [`FerroEhrService::fhir_ingest`](crate::service::FerroEhrService::fhir_ingest): resolve a
//!   mapping by resource type + profile, build a COMPOSITION from it (the pure
//!   transform in `mapping`), stamp `FEEDER_AUDIT` provenance
//!   (`feeder_audit`), and commit it through the NORMAL validated create path;
//! * the **read façade** ([`FerroEhrService::fhir_search`](crate::service::FerroEhrService::fhir_search)) and the **outbound
//!   reverse-map** (`FerroEhrService::fhir_outbound_messages`) — the inverse
//!   transform (`reverse`).

#![cfg_attr(
    feature = "fhir",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external \
                  standard with no RM type; the Value sites are fhir-gated, so the expectation \
                  exists only where it is fulfilled"
    )
)]

//
// Cross-area seams: the `pub(crate)` `FerroEhrService.pool` field,
// `service::query::execute_aql` and `service::ehr::create_composition`
// (`pub(crate)`), and the storage `version_repo` reads.

pub mod config;
#[cfg(feature = "fhir")]
pub mod outbound;

#[cfg(feature = "fhir")]
mod ingest;
