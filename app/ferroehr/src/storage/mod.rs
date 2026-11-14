// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Greenfield decomposed node storage.
//!
//! The codec between canonical openEHR JSON and the stored `node` rows, plus the
//! row I/O for the versioned-object spine
//! (`version`/`audit`/`contribution`/`vo_attestation`).
//!
//! No openEHR spec governs the physical storage; this is our own PG18-native
//! design, grounded on docs-verified `PostgreSQL` physics (no partial jsonb
//! detoast, GIN serves no ordering). The change-control law the row I/O upholds
//! is RM common master06; the identifier forms preserved verbatim in each
//! fragment are BASE `base_types` master05 and `foundation_types`
//! master03/05/06.
//!
//! One file per concern; consumers import each item from its defining submodule
//! (no re-exports):
//! - [`codec::decompose`] / [`codec::reassemble`] — the pure content transform.
//! - [`row::NodeRow`] / [`row::ReadRow`] / [`row::NodeContent`] — the write and
//!   lean read row shapes.
//! - [`structure::is_structure_type`] / [`structure::is_versioned_root_type`] /
//!   [`structure::archetype_parts`] — the decomposition granularity, delegated
//!   to the BMM-generated RM model plus the demographic delta (the party roots
//!   and the containers nested in them).
//! - [`error::StorageError`] plus the crate-internal `error::classify_sqlx` —
//!   the error surface and the SQLSTATE to status bridge.
//! - [`promoted::PROMOTED_LEAVES`] — the `(rm_type, path)` to node-column
//!   mapping the write codec and the AQL read lowering both consult, so a hot
//!   leaf reads an indexed column.
//! - [`slim::slim`] — what a `node` row stores of its fragment once its
//!   columns hold the rest, and the typed `ELEMENT` value columns.
//! - [`header`] — the archetype header, language and encoding a node below
//!   the root keeps in columns.
//! - [`node_repo`] — `node`-table writes and the node-to-canonical reloads.
//! - [`blob_ref`] — the multimedia blob reference index the node write path
//!   maintains and the blob collector reads.
//! - [`version_repo`] — the versioned-object spine (row I/O, folder-membership
//!   and event-outbox writes, the [`version_repo::read::StoredVersion`] read
//!   shape), itself one file per concern.
//! - [`ehr_repo`] — `ehr`-table and `ehr_folder`-membership reads and writes.
//! - [`extract_repo`] — the reads behind an EHR Extract.
//! - [`tag_repo`] — the `item_tag` store (EHR-scoped and demographic).
//! - [`marks`] — the restriction register, the research objection and the
//!   retention register, and the marks on `ehr` / `vo_head` they drive.
//! - [`access_projection`] — the `EHR_ACCESS` settings, directives and labels
//!   each EHR's current version projects.
//! - [`access_delay`] — the EHDS Art. 3(3) delay of the person's own access
//!   and its mark on `vo_head`.
//! - [`access_marks`] — the marks of the EHDS Art. 8 restriction and Art. 10
//!   opt-out directives of `EHR_ACCESS` on `ehr` / `vo_head`.
//! - [`policy_register`] — the versions of the operational-policy admin
//!   registers.
//! - [`access_policy`] — the versions of the controller's
//!   `VERSIONED_ACCESS_POLICY`.
//! - [`statistics`] — the counts and listings behind the admin statistics.
//! - [`integrity`] — the reads of the storage-integrity sweep and the node rebuild.
//! - [`template_repo`] — the OPT 1.4 `template_store` and the `template_ref`
//!   registry of every template address a version may name.
//! - [`archetype_repo`] — the ADL 1.4 `archetype_store`.
//! - [`adl2_repo`] — the ADL2 `adl2_artefact` store and its specialisation
//!   edges.
//! - [`stored_query_repo`] — the registered queries of `stored_query`.
//! - [`dump_repo`] — the rows the admin dump writes to an archive and the load
//!   writes back.
//! - [`dpop_replay`] — the `DPoP` proof replay store every replica shares.
//! - [`erasure`] — the physical erasure of an EHR or a party: what the cascade
//!   leaves behind, what it reads first, and the erasure tombstone.
//!
//! The seam with the versioning layer is a value contract rather than shared
//! SQL: versioning owns the semantics (classify, tree placement, lifecycle,
//! sign, attest, import policy) and calls these functions with plain inputs,
//! consuming [`version_repo::read::StoredVersion`] on read.

/// The columns of [`access_facts::HeadFacts`], read off the `vo_head` row
/// aliased `h`: what the access evaluator judges an object by.
macro_rules! head_facts {
    () => {
        concat!(
            "h.ehr_id AS access_ehr_id, h.template_id AS access_template, ",
            "h.committed_at AS access_recorded, ",
            "(SELECT t.origins FROM version t WHERE t.tier = h.tier AND t.vo_id = h.vo_id ",
            "AND t.sys_version = h.trunk_head_sys_version) AS access_origins, ",
            "(SELECT COALESCE(n.data->>'archetype_node_id', ",
            "ext.node_archetype_node_id(n.archetype, n.rm_version)) FROM node n ",
            "WHERE n.tier = h.tier AND n.vo_id = h.vo_id ",
            "AND n.sys_version = h.trunk_head_content_key AND n.num = 0) AS access_archetype"
        )
    };
}

pub mod access_delay;
pub mod access_facts;
pub mod access_marks;
pub mod access_policy;
pub mod access_projection;
pub mod adl2_repo;
pub mod archetype_repo;
pub mod blob_ref;
pub mod codec;
pub mod cohort_predicate;
pub mod compact;
pub mod ddl_template;
pub mod dpop_replay;
pub mod dump_repo;
pub mod ehr_repo;
pub mod erasure;
pub mod error;
pub mod extract_repo;
pub mod header;
pub mod identifier_repo;
pub mod integrity;
pub mod marks;
pub mod node_repo;
pub mod node_write;
pub mod party_heads;
pub mod party_reverse;
pub mod policy_register;
pub mod promoted;
pub mod row;
pub mod slim;
pub mod statistics;
pub mod stored_query_repo;
pub mod structure;
pub mod tag_repo;
pub mod template_repo;
pub mod version_repo;
