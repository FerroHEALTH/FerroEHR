// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Terminology component of the platform crate.
//!
//! The concrete realization of the SM `I_TERMINOLOGY_SERVICE` interface on
//! [`FerroEhrService`](crate::service::FerroEhrService), plus the AQL
//! terminology seam.
//!
//! Spec: `docs/specs/openehr/SM/docs/openehr_platform/
//! master12-terminology_service.adoc` + `UML/classes/i_terminology_service.adoc`
//! (the 9 calls and their preconditions) and the extract model
//! (`terminology_extract.adoc`). BASE
//! `docs/architecture_overview/master12-terminology.adoc` models the concrete
//! backend as an external terminology query server while the SM defines a single
//! interface, so the interface/provider split is logical: one service routing
//! among providers.
//!
//! Module tree, one file per concern:
//!
//! - [`types`] — the SM extract data model (`Terminology_description`,
//!   `Terminology_extract`, `Term_code`/`Defined_term`, relationships).
//! - `bundle` — the in-process `openehr-term` bundle provider (TERM 3.1.0), the
//!   enumerable local default.
//! - [`fhir`] — [`FhirTerminologyProvider`](fhir::FhirTerminologyProvider), a
//!   remote FHIR R4B TS client (opt-in via
//!   [`ExternalTerminologyConfig`](config::ExternalTerminologyConfig)).
//! - [`oauth2`] — [`TokenSource`](oauth2::TokenSource), the `OAuth2`
//!   client-credentials authentication a provider may carry.
//! - [`tls`] — the per-provider outbound TLS material (no openEHR spec governs
//!   transport security — our own design).
//! - [`router`] — [`TerminologyRouter`](router::TerminologyRouter): every
//!   configured server, materialised, with the terminology → provider routing.
//! - `routing` — the 9 SM calls on `FerroEhrService`.
//! - `binding` — commit-time resolution of archetype constraint bindings
//!   (ac-code value sets) against the routed terminology service.
//! - `expander` — the AQL `TERMINOLOGY()` seam
//!   ([`crate::aql::terminology::TerminologyExpander`] on `FerroEhrService`).
//! - [`config`] — the `[terminology]` config section (no openEHR spec governs
//!   configuration — our own design).
//!
//! # Provider routing
//!
//! Enumeration (`get_terminology_ids`, `has_terminology`,
//! `get_terminology_description`) is answered only by the bundle, a FHIR TS
//! being a validation and expansion backend rather than an enumerable openEHR
//! terminology, so a FHIR-only deployment still answers these. Lookup and
//! validation (`has_term`, `get_term`, `subsumes`, `value_set_validate`,
//! `has_value_set`, `get_value_set`) go to the bundle when it knows the
//! terminology, else route by terminology to one of the configured FHIR
//! providers ([`router::TerminologyRouter`]), else fall through to the bundle's
//! `Pre_has_terminology` and `NotFound`. With no FHIR provider configured, the
//! default, this is byte-identical to a bundle-only service. Several servers
//! answer simultaneously — SNOMED CT on one, LOINC on another in the same
//! instance — which is the deployment BASE
//! `docs/architecture_overview/master12-terminology.adoc` §Overview describes.

mod binding;
mod bundle;
pub mod config;
mod expander;
pub mod fhir;
pub mod oauth2;
pub mod router;
mod routing;
pub mod tls;

pub mod types;
