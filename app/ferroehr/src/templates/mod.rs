// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operational-template ingestion, storage, and derived-runtime resolution.
//!
//! The module owns the platform crate's template surface, layered the way the
//! spec layers the material: resource identity, operational form, derived
//! runtime artefact.
//!
//! - `identity` — the `ARCHETYPE_ID` / `TEMPLATE_ID` identity law
//!   (`BASE/docs/base_types/master05-identification_package.adoc` §Composite
//!   Identifiers and Case: case-preserving and case-insensitive), applied at
//!   every lookup and cache boundary.
//! - `ingest` — OPT 1.4 canonical XML parsed to `OperationalTemplate`; the
//!   artefact is an `AUTHORED_RESOURCE`
//!   (`BASE/docs/resource/master02-resource_package.adoc` §Meta-data).
//! - `store` — the `template_store` repository: insert-only upload (`409` on a
//!   duplicate id) and the `template_id`-keyed retrieval and listing behind the
//!   ITS-REST `/definition/template/adl1.4` surface
//!   (`ITS-REST/specifications/definition.openapi.yaml`).
//! - `runtime` — the derived near-runtime form
//!   (`BASE/docs/architecture_overview/master10-archetypes.adoc` §Archetypes and
//!   Templates at Runtime, §Deploying Archetypes and Templates), `moka`-cached;
//!   the `WebTemplate` format is spec-silent and stays in `openehr_sdt::flat`.
//!
//! The service methods live on [`crate::service::FerroEhrService`], defined
//! across `store` (`store_template`, `get_template_xml`, `template_summaries`)
//! and `runtime` (`web_template_for`, `template_example`). The UUID-keyed SM
//! `I_DEFINITION_ADL14` OPT operations
//! (`SM/docs/openehr_platform/master04-definition_package.adoc`) belong to the
//! definition register (`service/definition/adl14.rs`), which reads the same
//! `template_store` table directly; this module does not duplicate them.

pub(crate) mod identity;
mod ingest;
mod runtime;
mod store;
