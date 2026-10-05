// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Versioning and integrity: the openEHR change-control model realized over the
//! greenfield PG18 storage.
//!
//! Spec oracles, in precedence order:
//! - RM common `master06-change_control_package.adoc` — the change-control law
//!   (`VERSIONED_OBJECT`, VERSION, ORIGINAL/IMPORTED, CONTRIBUTION, committal
//!   and audits, digital signature, attestation, version lifecycle, logical
//!   deletion, version identification, copying and merging).
//! - RM common `master04-generic_package.adoc` — `AUDIT_DETAILS`, ATTESTATION,
//!   `REVISION_HISTORY`(_ITEM), `PARTY_PROXY`.
//! - BASE `base_types` `master05-identification_package.adoc` —
//!   `OBJECT_VERSION_ID` / `VERSION_TREE_ID` lexical forms and case rules.
//! - BASE arch-overview `master07-security.adoc` §Integrity,
//!   `master08-versioning.adoc`, `master09-identification.adoc`.
//!
//! The layout derives from the spec's own decomposition: the digital signature
//! is a section of master06, so the signer and verifier live inside this module
//! ([`signature`]) rather than as a standalone sibling.
//!
//! # Module tree
//!
//! | module | concern |
//! |---|---|
//! | `object_version_id` | `OBJECT_VERSION_ID` / `VERSION_TREE_ID` decoding (BASE master05) |
//! | `lifecycle` | `version_lifecycle_state` codes + the transition state machine |
//! | `audit` | `AUDIT_DETAILS` values, the `audit_change_type` group, committer invariants |
//! | [`change`] | the change-set unit, version-tree placement, the shared commit engine |
//! | `contribution` | CONTRIBUTION classify + commit orchestration + retrieval |
//! | `attestation` | attaching `ATTESTATION`s at or after committal |
//! | `read` | loading stored versions (`read::VersionRead` and friends) |
//! | `wire` | the served canonical-JSON builders (`ORIGINAL_VERSION`, `VERSIONED_*`, `REVISION_HISTORY`) |
//! | `integrity` | signing policy at commit + verification policy at read |
//! | `import` | replaying received originals as `IMPORTED_VERSION`s |
//! | [`signature`] | the digest / `OpenPGP` signature primitives + configuration |
//!
//! # Seam with storage (`crate::storage`)
//!
//! This module owns the decisions (classify, tree placement, lifecycle
//! transition, sign, attest, import policy) and the builders
//! (`ORIGINAL_VERSION` / `VERSIONED_OBJECT` / `REVISION_HISTORY` value
//! construction). All `sqlx` execution is delegated to a storage-owned
//! repository: [`crate::storage::version_repo`] for the `version` / `audit` /
//! `contribution` / `vo_attestation` spine plus the folder-membership and
//! event-outbox writes, and [`crate::storage::node_repo`] for the `decompose` /
//! `reassemble` codec and the `node` writes. The per-function docs there are the
//! authority for each call this module makes.
//!
//! NOTE: RM common `master06-change_control_package.adoc` §Overview explicitly
//! sanctions relational decomposition of a `VERSIONED_OBJECT` ("Other
//! implementations (e.g. using orthodox relational structures) might use
//! references"); the row shapes are our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use serde_json::Value;
use sqlx::PgPool;

use crate::ids::{EhrId, VoId};
use crate::service::error::ServiceError;
use crate::versioning::signature::signer::Signer;

pub(crate) mod attestation;
pub(crate) mod audit;
pub mod change;
pub(crate) mod contribution;
pub(crate) mod import;
pub(crate) mod integrity;
pub(crate) mod lifecycle;
pub mod object_version_id;
pub(crate) mod origins;
pub(crate) mod profile;
pub(crate) mod read;
pub mod signature;
pub(crate) mod wire;

// Re-exports: the versioning API the service layer and SM adapters consume.

/// The kind of versioned object (discriminates `version.kind`).
///
/// RM common master06 keeps one change-control model for all versioned content;
/// this CDR realizes it with one unified `version`/`node` machinery, so a
/// single [`Kind`] discriminates COMPOSITION / `EHR_STATUS` / `EHR_ACCESS` / FOLDER
/// (EHR-scoped) and the demographic party roots + `PARTY_RELATIONSHIP` (no EHR
/// scope, RM demographic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A COMPOSITION — the clinical content of an EHR.
    Composition,
    /// The EHR's `EHR_STATUS` (subject, queryable/modifiable flags).
    EhrStatus,
    /// The EHR-wide access-control object created with the EHR (RM ehr §"EHR
    /// Creation") and versioned "via the normal mechanism" (RM ehr §"EHR
    /// Access").
    EhrAccess,
    /// A FOLDER hierarchy — a member of `EHR.folders` (the lowest-ranked one
    /// being `EHR.directory`).
    Folder,
    // Demographic party roots: versioned objects with no EHR scope; the same
    // machinery with a NULL `ehr_id`.
    /// A demographic AGENT.
    Agent,
    /// A demographic GROUP.
    Group,
    /// A demographic ORGANISATION.
    Organisation,
    /// A demographic PERSON.
    Person,
    /// A demographic ROLE.
    Role,
    /// A demographic `PARTY_RELATIONSHIP` (RM demographic): a versioned object
    /// with no EHR scope, like the party roots, but *not* a PARTY.
    PartyRelationship,
}

impl Kind {
    /// The stored `version.kind` discriminator — the full RM type name.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Composition => "COMPOSITION",
            Kind::EhrStatus => "EHR_STATUS",
            Kind::EhrAccess => "EHR_ACCESS",
            Kind::Folder => "FOLDER",
            Kind::Agent => "AGENT",
            Kind::Group => "GROUP",
            Kind::Organisation => "ORGANISATION",
            Kind::Person => "PERSON",
            Kind::Role => "ROLE",
            Kind::PartyRelationship => "PARTY_RELATIONSHIP",
        }
    }

    /// Whether this kind is a demographic party root (no EHR scope). This is the
    /// `/versioned_party` read scope — a `PARTY_RELATIONSHIP` is *not* a party.
    pub(crate) fn is_party(self) -> bool {
        matches!(
            self,
            Kind::Agent | Kind::Group | Kind::Organisation | Kind::Person | Kind::Role
        )
    }

    /// Whether this kind is a demographic versioned object (no EHR scope): the
    /// five party roots plus `PARTY_RELATIONSHIP`.
    pub(crate) fn is_demographic(self) -> bool {
        self.is_party() || self == Kind::PartyRelationship
    }

    /// Every versioned-object kind, in declaration order — the domain of
    /// [`Kind`], for callers that need to derive a subset of the versioned-root
    /// RM types rather than restate one.
    pub(crate) const ALL: [Kind; 10] = [
        Kind::Composition,
        Kind::EhrStatus,
        Kind::EhrAccess,
        Kind::Folder,
        Kind::Agent,
        Kind::Group,
        Kind::Organisation,
        Kind::Person,
        Kind::Role,
        Kind::PartyRelationship,
    ];

    /// The versioned-object kind for an RM `_type`, if it is a versioned root —
    /// the inverse of [`Kind::as_str`] over [`Kind::ALL`], so the RM type names
    /// are written once and the two directions cannot drift apart.
    pub(crate) fn from_type(rm_type: &str) -> Option<Self> {
        Kind::ALL.into_iter().find(|kind| kind.as_str() == rm_type)
    }
}

/// The signing context threaded into every versioned-object write so the
/// assembled `ORIGINAL_VERSION` is signed (RM common master06 §Digital
/// Signature). Borrows the effective system id and the configured [`Signer`].
pub(crate) struct SigningCtx<'a> {
    /// The effective openEHR `system_id` for this write: the one logical EHR
    /// system this instance is (BASE `architecture_overview` master06 §System
    /// Identity).
    pub(crate) system_id: String,
    pub(crate) signer: &'a Signer,
    /// The licence stamp key every server-minted identifier on this write is
    /// stamped with ([`crate::licence::stamp`]). No openEHR spec governs
    /// identifier entropy — our own design.
    pub(crate) stamp: &'a crate::licence::stamp::StampKey,
    /// The ACTIVE openEHR specification generation set. The commit path asks
    /// the RELEASED generation's reader whether it could express the accepted
    /// body and stores the answer (`version.stable_compatible`), so a
    /// deployment later configured to the `stable` profile can refuse rather
    /// than silently serve or down-convert
    /// ([`crate::versioning::profile`]).
    pub(crate) spec_profile: crate::config::profile::SpecProfile,
    /// The optional `DV_MULTIMEDIA` externalization engine (no openEHR spec
    /// governs media externalization — our own extension). When set, the commit
    /// path offloads large inline `DV_MULTIMEDIA.data` before the canonical body
    /// is decomposed and signed.
    #[cfg(feature = "multimedia")]
    pub(crate) multimedia: Option<&'a ferroehr_ext::multimedia::MultimediaEngine>,
    /// The optional national-identifier protection engine (no openEHR spec
    /// governs identifier protection — our own extension). When set, the
    /// demographic commit path seals every configured-scheme identifier out of
    /// the body before it is decomposed and signed, so the stored, signed and
    /// served form is the same one — the invariant the multimedia offload
    /// above already keeps.
    pub(crate) identifiers:
        Option<&'a crate::service::demographic::identifier::engine::IdentifierProtection>,
    /// Whether to write the transactional event outbox on this commit. `false`
    /// when no eventing consumer is configured, so the per-commit `event_outbox`
    /// INSERT + envelope serialization is skipped entirely. No openEHR spec
    /// governs eventing — our own extension.
    pub(crate) outbox_enabled: bool,
}

/// The cross-area hooks the CONTRIBUTION commit orchestration
/// ([`contribution::commit_version_set`]) needs from the service layer. Each
/// hook is owned by another register: content validation (validation register),
/// EHR existence + `current_vo` (EHR register), `EHR_ACCESS` cache
/// invalidation (EHR register), committer default (EHR register).
/// `crate::service::FerroEhrService` implements it; versioning owns only the
/// change-set decision logic. `default_committer` is the EHR worker's
/// `committer()`; `ensure_ehr_exists` closes the `Pre_has_ehr` check (SM
/// `i_ehr_contribution.adoc` §`commit_contribution` `Pre_has_ehr`). The
/// `is_modifiable` content-write guard is NOT a hook: it runs inside the
/// commit transaction ([`change::ensure_content_writable_tx`]).
///
/// The two in-transaction hooks ([`Self::pre_composition_modify`] and
/// [`Self::post_status_commit`]) are the cross-version invariant / promoted-
/// subject-column glue the direct write paths run inline; they are wired here so
/// the CONTRIBUTION path runs them too, in the same commit transaction.
#[async_trait::async_trait]
pub(crate) trait CommitEnv {
    /// The connection pool for the commit transaction — the clinical domain.
    fn pool(&self) -> &PgPool;
    /// The connection pool serving the demographic pseudonymisation domain.
    ///
    /// A CONTRIBUTION is committed entirely in one domain: an EHR-scoped set
    /// through [`Self::pool`], a party-only set through this one. No openEHR
    /// spec governs storage layout — our own design/extension.
    fn demographic_pool(&self) -> &PgPool;
    /// The effective openEHR `system_id` for this request.
    fn effective_system_id(&self) -> String;
    /// The default committer `PARTY_PROXY` (the authenticated principal).
    fn default_committer(&self) -> openehr_rm::prelude::PartyProxy;
    /// The write-time signing context.
    fn signing_ctx(&self) -> SigningCtx<'_>;
    /// Validate a version's content for commit (relaxed when `incomplete`).
    async fn validate_for_commit(
        &self,
        kind: Kind,
        data: &Value,
        incomplete: bool,
    ) -> Result<(), ServiceError>;
    /// Check every FOLDER `items` reference that claims this system resolves
    /// in `ehr_id` — run AFTER the change set's inserts, inside the commit
    /// transaction, so the verdict never depends on member order within the
    /// set (RM ehr master04 §Folders groups contemporaneous folder and
    /// content commits in one CONTRIBUTION; version identity is
    /// repository-allocated here, so a same-set reference is only expressible
    /// once an identity-echo mechanism exists — this placement is already
    /// correct for it).
    async fn check_folder_item_refs(
        &self,
        tx: &mut sqlx::PgConnection,
        ehr_id: EhrId,
        folder: &Value,
    ) -> Result<(), ServiceError>;
    /// the target EHR must exist before a CONTRIBUTION is committed to it.
    async fn ensure_ehr_exists(&self, ehr_id: EhrId) -> Result<(), ServiceError>;
    /// The current versioned object of `kind` in `ehr_id`, if any (for the
    /// EHR-singleton create guard).
    async fn current_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, i32)>, ServiceError>;
    /// Drop the cached `EHR_ACCESS` settings after an `EHR_ACCESS` commit.
    async fn invalidate_ehr_access(&self, ehr_id: EhrId);
    /// Whether the EHR already holds a LIVE folder hierarchy whose root
    /// carries the `(archetype_node_id, name)` LOCATABLE identity pair — the
    /// CONTRIBUTION-route duplicate-directory rejection (CNF schedule master08
    /// §`commit_contribution` E.2: creating the root FOLDER again is negative; a
    /// DISTINCT hierarchy is a new `EHR.folders` member — RM ehr master04
    /// §Folders; same-archetype siblings are distinguished by name, RM common
    /// paths semantics).
    async fn folder_root_exists(
        &self,
        ehr_id: EhrId,
        archetype_node_id: &str,
        name: &str,
    ) -> Result<bool, ServiceError>;
    /// Enforce the `VERSIONED_COMPOSITION` cross-version invariants
    /// (`Archetype_node_id_valid` / `Persistent_validity`, RM ehr
    /// `versioned_composition.adoc`) of a COMPOSITION *modify* against the
    /// stored first version, **before** the new version is written. Runs inside
    /// the commit transaction (`tx`) so the check and the write are atomic — the
    /// same hook the direct update path runs inline.
    async fn pre_composition_modify(
        &self,
        tx: &mut sqlx::PgConnection,
        vo_id: VoId,
        canonical: &Value,
    ) -> Result<(), ServiceError>;
    /// Keep the EHR's promoted subject columns (`ehr.subject_id` /
    /// `subject_namespace`, spec-silent index plumbing — our own design) in sync
    /// with a committed `EHR_STATUS` (`subject.external_ref`, RM ehr master04
    /// §EHR Status). Runs inside the commit transaction (`tx`) so a subject
    /// uniqueness conflict rolls the whole CONTRIBUTION back — the same hook the
    /// direct EHR-create / EHR_STATUS-update paths run inline.
    async fn post_status_commit(
        &self,
        tx: &mut sqlx::PgConnection,
        ehr_id: EhrId,
        status: &Value,
    ) -> Result<(), ServiceError>;
}
