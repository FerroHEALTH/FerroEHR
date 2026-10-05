// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DEMOGRAPHIC (PARTY + `PARTY_RELATIONSHIP`) service module.
//!
//! The platform-crate realization of the SM DEMOGRAPHIC group over the shared
//! [`crate::versioning`] change-control machinery, with no EHR scope
//! (`ehr_id = None`; a party has no owning EHR — our own design). Parties
//! (PERSON / ORGANISATION / GROUP / AGENT / ROLE) and `PARTY_RELATIONSHIP`s are
//! versioned objects in the demographics repository.
//!
//! Every storage call on this path goes through
//! `FerroEhrService::demographic_pool`, never `pool`: the two pools differ only
//! in `search_path`, so the same storage functions read and write the
//! `demographic` schema here and the clinical one everywhere else. Parties are
//! physically separated from clinical content and, when the deployment
//! configures `[db].demographic_url`, reachable only by their own database role
//! (GDPR Art. 4(5) and Art. 32(1)(a)). No openEHR spec governs storage layout
//! or database roles — our own design/extension; the wire contract and the
//! change-control semantics are unchanged.
//!
//! One file per concern, the domain files mirroring the SM interface boundaries
//! (SM `master06`):
//! - `types` — the public [`types::PartyKind`] resource-family key,
//! - `validate` — the inbound-body RM/BASE invariant checks and the
//!   CONTRIBUTION-path validator entry points,
//! - `support` — the shared seam onto [`crate::versioning`] (kind mapping,
//!   ehr-less version loads, commit audits, canonical wire assembly),
//! - `party` — `I_PARTY` CRUD plus `I_DEMOGRAPHIC_SERVICE.create_party`,
//! - `relationship` — `I_PARTY_RELATIONSHIP` plus `create_party_relationship`,
//!   the one surface absent from the released Demographic API (our extension),
//! - `versioned` — the `VERSIONED_PARTY` read surface,
//! - `contribution` — the demographic (ehr-less) CONTRIBUTION,
//! - `tags` — the demographic `ITEM_TAG` surface,
//! - `api` — the public `FerroEhrService` seam the ITS-REST adapter calls.
//!
//! The wire contract is the Demographic API of ITS-REST Release-1.1.0
//! (DEVELOPMENT lifecycle within the released spec), whose
//! status/`ETag`/`Location`/`Prefer`/`If-Match`/deleted-read semantics are
//! identical to the EHR group by the spec's own design.
//!
//! Three deliberate divergences:
//! - The `UV_PARTY`/`UV_PARTY_RELATIONSHIP` envelope (`uv_party.adoc`) is
//!   realized server-side; the wire seam carries a bare RM party and
//!   `lifecycle_state` defaults to `532|complete|`.
//! - The `definitions_valid` precondition and `definition_unknown` error
//!   (`i_demographic_service.adoc` §`create_party`) are unimplemented, there
//!   being no demographic archetype or OPT store; only `valid_content` → `422`
//!   is enforced.
//! - `PARTY.reverse_relationships` (`party.adoc`
//!   §`Reverse_relationships_validity`) is a derived `0..1` attribute the server
//!   leaves unpopulated.
//!
//! Spec oracles for the RM-level rules enforced here:
//! - `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`
//!   (PARTY invariants `Identities_valid`, `Contacts_valid`,
//!   `Relationships_validity`, `Uid_mandatory`),
//!   `…demographic.actor.adoc` (`Roles_valid`), `…demographic.role.adoc`
//!   (`Capabilities_valid`, `performer`),
//!   `…demographic.party_relationship.adoc` (`source`/`target`),
//! - `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.party_ref.adoc`
//!   (`PARTY_REF.Type_validity`) + `…object_ref.adoc` (`OBJECT_REF.namespace`),
//! - `docs/specs/openehr/SM/docs/UML/classes/i_party.adoc` /
//!   `i_demographic_service.adoc` / `i_party_relationship.adoc`.

pub mod identifier;
pub mod types;

mod api;
mod contribution;
mod party;
mod relationship;
mod support;
mod tags;
pub(crate) mod validate;
mod versioned;

// The commit-path validators the CONTRIBUTION engine (`validate_for_commit`,
// `service/ehr/composition_validate.rs`) dispatches to once the
// versioned-object kind is known from the payload `_type`.
