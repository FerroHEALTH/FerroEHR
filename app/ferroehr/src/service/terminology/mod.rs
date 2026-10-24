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
//! `docs/architecture_overview/master12-terminology.adoc` keeps the openEHR
//! support terminology inside the platform (§Terminology to Support the
//! Reference Model) and places the resolver of an archetype constraint binding
//! in an external "terminology query server" (§Binding Terminology Value-sets
//! to Archetypes), so the service answers from the in-process bundle and from
//! one FHIR terminology server.
//!
//! Module tree, one file per concern:
//!
//! - [`types`] — the SM extract data model (`Terminology_description`,
//!   `Terminology_extract`, `Term_code`/`Defined_term`, relationships).
//! - `bundle` — the in-process `openehr-term` bundle provider (TERM 3.1.0), the
//!   openEHR support terminology.
//! - [`fhir`] — [`FhirTerminologyProvider`](fhir::FhirTerminologyProvider), the
//!   client of the one FHIR R4B terminology server (opt-in via
//!   [`TerminologyServerConfig`](config::TerminologyServerConfig)).
//! - `routing` — the 9 SM calls on `FerroEhrService`.
//! - `binding` — commit-time resolution of archetype constraint bindings
//!   (ac-code value sets) against the terminology server.
//! - `expander` — the AQL `TERMINOLOGY()` seam
//!   ([`crate::aql::terminology::TerminologyExpander`] on `FerroEhrService`).
//! - [`config`] — the `[terminology]` config section (no openEHR spec governs
//!   configuration — our own design).
//!
//! # Which source answers
//!
//! Enumeration (`get_terminology_ids`, `has_terminology`,
//! `get_terminology_description`) is answered only by the bundle, a FHIR TS
//! being a validation and expansion backend rather than an enumerable openEHR
//! terminology, so a deployment with a server still answers these. Lookup and
//! validation (`has_term`, `get_term`, `subsumes`, `value_set_validate`,
//! `has_value_set`, `get_value_set`) go to the bundle when it knows the
//! terminology, else to the configured terminology server, else fall through
//! to the bundle's `Pre_has_terminology` and `NotFound`. With no server
//! configured, the default, this is byte-identical to a bundle-only service.

mod binding;
mod bundle;
pub mod config;
mod expander;
pub mod fhir;
mod routing;

pub mod types;
