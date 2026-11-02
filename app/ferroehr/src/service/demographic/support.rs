// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic chapter's shared seam onto the [`crate::versioning`]
//! change-control machinery and the version spine in
//! `crate::storage::version_repo`: [`PartyKind`]↔[`Kind`] mapping, the
//! ehr-less version loads, the write precondition, and the canonical wire
//! assembly (uid injection, `REVISION_HISTORY`, versioned-object wrappers,
//! `ORIGINAL_VERSION` reads) used by both the party and relationship surfaces.
//!
//! The version-spine reads go through `crate::storage::version_repo` (storage
//! owns the SQL — no openEHR spec governs the storage read, our own design);
//! RM common master04 §Revision History / master06 §Versioned Objects govern
//! the assembled wire shapes. Versioning's `revision_history` /
//! `versioned_object` builders are EHR-scoped, so this chapter maps the
//! ehr-less `VersionMeta` rows into the wire shape itself.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::str::FromStr as _;

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId};
use openehr_rm::prelude::{
    RevisionHistory, RevisionHistoryItem, VersionedObject, VersionedObjectData, VersionedParty,
};
use serde_json::Value;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::error::ServiceError;
use crate::service::refusal::Missing;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::storage::version_repo;
use crate::storage::version_repo::placement::TipRow;
use crate::storage::version_repo::read::VersionAt;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::Committed;
use crate::versioning::change::tip_uid;
use crate::versioning::object_version_id::{
    TreeId, VersionIdError, expected_from_if_match, hier_object_id, if_match_token,
    object_version_id, version_id,
};
use crate::versioning::read::VersionRead;
use crate::versioning::wire::version_envelope;

/// The versioned-object [`Kind`] for a REST [`PartyKind`].
pub(super) fn kind_of(kind: PartyKind) -> Kind {
    match kind {
        PartyKind::Agent => Kind::Agent,
        PartyKind::Group => Kind::Group,
        PartyKind::Organisation => Kind::Organisation,
        PartyKind::Person => Kind::Person,
        PartyKind::Role => Kind::Role,
    }
}

/// Inject the `uid` (`OBJECT_VERSION_ID`) into a versioned object's JSON on read
/// — PARTY `Uid_mandatory` (`demographic.party.adoc`), the party's identity
/// copied from its enclosing VERSION.
///
/// # Errors
/// [`VersionIdError`] when the stored `creating_system_id` is not a legal BASE
/// `uid`, so the three parts do not compose into a well-formed
/// `OBJECT_VERSION_ID` (`crate::versioning::object_version_id::version_id`).
pub(super) fn inject_uid(
    mut canonical: Value,
    vo_id: VoId,
    creating_system_id: &str,
    tree: TreeId,
) -> Result<Value, VersionIdError> {
    if let Value::Object(map) = &mut canonical {
        map.insert(
            "uid".to_owned(),
            openehr_its::json::to_canonical_value(&version_id(vo_id, creating_system_id, tree)?),
        );
    }
    Ok(canonical)
}

/// Load one **ehr-less** version of a versioned object: a specific `version`,
/// the version extant `at` an instant, or the current one when both are `None`.
/// `None` when the version does not exist or the object is EHR-scoped — a
/// demographic surface never serves an EHR-owned object (`ehr_id = None` is
/// the demographics repository's scope, module NOTE).
///
/// # Errors
/// [`ServiceError`] on a storage/database fault during the version read, or
/// the `409`-class `spec_profile` refusal of a stored body the active
/// generation set cannot express
/// ([`crate::versioning::profile::gate`]).
pub(super) async fn load_ehrless(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
    version: Option<TreeId>,
    at: Option<jiff::Timestamp>,
) -> Result<Option<VersionRead>, ServiceError> {
    let read = match (version, at) {
        (Some(v), _) => {
            crate::versioning::read::read(pool, profile, VersionAt::Tree(vo_id, v)).await?
        }
        (None, Some(at)) => {
            crate::versioning::read::read(pool, profile, VersionAt::Instant(vo_id, at)).await?
        }
        (None, None) => {
            crate::versioning::read::read(pool, profile, VersionAt::Current(vo_id)).await?
        }
    };
    Ok(read.filter(|r| r.ehr_id.is_none()))
}

/// A [`ServiceResponse`] for a loaded party / relationship version: its
/// canonical body with the `uid` injected (PARTY `Uid_mandatory`) plus the
/// resource metadata (an empty `ehr_id` — demographic objects are not
/// EHR-scoped).
///
/// # Errors
/// [`VersionIdError`] when the stored `creating_system_id` is not a legal BASE
/// `uid` (see [`inject_uid`]).
pub(super) fn version_response(
    vo_id: VoId,
    read: VersionRead,
) -> Result<ServiceResponse, VersionIdError> {
    let meta = ResourceMeta::new(
        String::new(),
        object_version_id(vo_id, &read.creating_system_id, read.tree),
    )
    .with_last_modified(read.time_committed);
    Ok(ServiceResponse::new(
        inject_uid(read.canonical, vo_id, &read.creating_system_id, read.tree)?,
        meta,
    ))
}

/// Whether a demographic commit stores a different body than the one submitted.
///
/// Two collaborators rewrite the body before it is decomposed and signed: the
/// `DV_MULTIMEDIA` offload replaces inline media with a reference, and
/// national-identifier protection replaces each protected value with a
/// reference to its sealed row. A write response built from the submitted
/// body would then serve bytes the store does not hold — for identifier
/// protection, the very value the store exists to keep out of the record — so
/// the write paths re-read the stored form instead. No openEHR spec governs
/// either rewrite — our own design/extension.
pub(super) fn commit_rewrites_body(service: &FerroEhrService) -> bool {
    #[cfg(feature = "multimedia")]
    let offloads = service
        .multimedia
        .as_ref()
        .is_some_and(|engine| engine.offload_enabled());
    #[cfg(not(feature = "multimedia"))]
    let offloads = false;
    offloads || service.identifiers.is_some()
}

/// The create/update representation built **from the commit result**, never a
/// post-commit re-read: the served body is the just-written `canonical` with
/// the `uid` injected, and the identity + commit instant come straight from
/// [`Committed`] (RM common master06 §Committal and Audits — the
/// written version identity). Byte-identical to a fresh read: the served form is
/// `inject_uid(reassemble(decompose(canonical)))`, and the node codec
/// round-trips `canonical` losslessly (pinned by a test). The caller passes
/// the pre-write `canonical`, and only when [`commit_rewrites_body`] says the
/// commit stores that body unchanged; otherwise it re-reads the stored form.
///
/// # Errors
/// [`VersionIdError`] when the committed `creating_system_id` is not a legal
/// BASE `uid` (see [`inject_uid`]).
pub(super) fn committed_response(
    canonical: Value,
    committed: &Committed,
) -> Result<ServiceResponse, VersionIdError> {
    let vo_id = committed.vo_id;
    let meta = ResourceMeta::new(
        String::new(),
        object_version_id(vo_id, &committed.creating_system_id, committed.tree),
    )
    .with_last_modified(committed.time_committed);
    Ok(ServiceResponse::new(
        inject_uid(
            canonical,
            vo_id,
            &committed.creating_system_id,
            committed.tree,
        )?,
        meta,
    ))
}

/// One `REVISION_HISTORY_ITEM` for a stored version row: its
/// `OBJECT_VERSION_ID` and the change's `AUDIT_DETAILS` (RM common master04
/// §Revision History). Built as the generated RM type so the serialized item
/// carries `_type` first and the BMM's own attribute order.
///
/// # Errors
/// [`ServiceError::Unprocessable`] when the stored audit's committer is not a
/// canonical `PARTY_PROXY`.
fn revision_history_item(
    vo_id: VoId,
    meta: &version_repo::meta::VersionMeta,
) -> Result<RevisionHistoryItem, ServiceError> {
    let tree = TreeId::from_columns(meta.trunk_version, meta.branch_number, meta.branch_version);
    // NOTE: `REVISION_HISTORY_ITEM.audits` holds the commit audit plus any
    // attestations (RM `revision_history_item.adoc` §Attributes); the
    // demographic API exposes no attestation route, so the vector is singular.
    Ok(RevisionHistoryItem {
        version_id: version_id(vo_id, &meta.creating_system_id, tree)?,
        // `REVISION_HISTORY_ITEM.audits` is `1..*`
        // (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.revision_history_item.adoc`
        // §Attributes); this item always carries its commit audit.
        audits: openehr_base::containers::NonEmptyVec::of(
            AuditInput::from_meta(meta)?.typed(&meta.time_committed),
        ),
    })
}

impl FerroEhrService {
    /// The versioned-object wrapper (`VERSIONED_PARTY` / `VERSIONED_OBJECT`)
    /// for an ehr-less demographic object. `type_name` is the wire `_type` and
    /// `label` names the family for the `404`.
    /// `VERSIONED_OBJECT.time_created` is the commit time of the earliest held
    /// version (for a locally-created object, v1).
    ///
    /// NOTE: `VERSIONED_OBJECT.owner_id` (1..1) has no EHR owner for a
    /// demographic versioned object, so it names the serving system, the "other
    /// relevant owning entity" limb of RM common
    /// `UML/classes/org.openehr.rm.common.versioned_object.adoc` §Attributes, in
    /// the shape the released `VersionedParty` example fixes
    /// (`{namespace: local, type: SYSTEM, id: HIER_OBJECT_ID}`).
    ///
    /// The body is constructed as the generated [`VersionedObject`] subtype and
    /// serialized through the native codec, so it carries `_type` first and the
    /// BMM's own attribute order.
    ///
    /// # Errors
    /// - [`ServiceError::NotFound`] — the object holds no versions (`label`
    ///   names the family in the message).
    /// - [`ServiceError`] on a storage/database fault reading `time_created`.
    pub(super) async fn versioned_wrapper(
        &self,
        vo_id: VoId,
        type_name: &str,
        label: &str,
    ) -> Result<Value, ServiceError> {
        let time_created = version_repo::meta::time_created(&self.demographic_pool, vo_id)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(Missing::VersionedObject, format!("{label} {vo_id}"))
            })?;
        let uid = HierObjectId::from(vo_id.0);
        let owner_id = ObjectRef::ObjectRef(ObjectRefData {
            namespace: "local".to_owned(),
            r#type: "SYSTEM".to_owned(),
            id: ObjectId::HierObjectId(hier_object_id(&self.effective_system_id())?),
        });
        let time_created = crate::versioning::audit::dv_date_time(&time_created);
        let container = match type_name {
            "VERSIONED_PARTY" => VersionedObject::VersionedParty(VersionedParty {
                uid,
                owner_id,
                time_created,
            }),
            // The generic container of RM common master06 §Versioned Objects,
            // for a demographic kind with no dedicated `VERSIONED_*` binding
            // (`PARTY_RELATIONSHIP`).
            _ => VersionedObject::VersionedObject(VersionedObjectData {
                uid,
                owner_id,
                time_created,
            }),
        };
        Ok(openehr_its::json::to_canonical_value(&container))
    }

    /// The `REVISION_HISTORY` of an ehr-less demographic object: one item per
    /// version with its `OBJECT_VERSION_ID` and commit `AUDIT_DETAILS` (RM
    /// common master04 §Revision History). The caller has already confirmed
    /// the object belongs to its family (party / relationship).
    ///
    /// # Errors
    /// [`ServiceError`] on a storage/database fault reading the version spine,
    /// or the [`revision_history_item`] rejection of an uninterpretable stored
    /// audit.
    pub(super) async fn demographic_revision_history(
        &self,
        vo_id: VoId,
    ) -> Result<Value, ServiceError> {
        let metas = version_repo::meta::all_version_meta(&self.demographic_pool, vo_id).await?;
        let items = metas
            .iter()
            .map(|meta| revision_history_item(vo_id, meta))
            .collect::<Result<Vec<_>, _>>()?;
        // `REVISION_HISTORY.items` is `1..*`
        // (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.revision_history.adoc`
        // §Attributes): a versioned object with no versions has no revision
        // history resource to return.
        let items = openehr_base::containers::NonEmptyVec::new(items).map_err(|empty| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("versioned object {vo_id}: {empty}"),
            )
        })?;
        Ok(openehr_its::json::to_canonical_value(&RevisionHistory {
            items,
        }))
    }

    /// The VERSION envelope of an ehr-less demographic object at a specific
    /// `version`, signed per the server signing context (`label` names the
    /// family for the `404`).
    ///
    /// # Errors
    /// - [`ServiceError::NotFound`] — no ehr-less version `version` of `vo_id`
    ///   exists.
    /// - [`ServiceError`] on a storage/database fault, or when the
    ///   VERSION envelope assembly/signing fails.
    pub(super) async fn demographic_version_envelope(
        &self,
        vo_id: VoId,
        version: TreeId,
        label: &str,
    ) -> Result<Value, ServiceError> {
        let read = load_ehrless(
            &self.demographic_pool,
            self.spec_profile,
            vo_id,
            Some(version),
            None,
        )
        .await?
        .ok_or_else(|| {
            ServiceError::not_found(
                Missing::ObjectVersion,
                format!("{label} {vo_id} v{version}"),
            )
        })?;
        let signer = self.signing_ctx().signer;
        version_envelope(&read, signer)
    }

    /// The VERSION envelope of an ehr-less demographic object extant at `at`
    /// (or the latest when `None`), with `ETag`/`Location` metadata for the
    /// VERSION resource (`label` names the family for the `404`).
    ///
    /// # Errors
    /// - [`ServiceError::NotFound`] — no ehr-less version of `vo_id` existed
    ///   at `at` (or no current version when `at` is `None`).
    /// - [`ServiceError`] on a storage/database fault, or when the
    ///   VERSION envelope assembly/signing fails.
    pub(super) async fn demographic_version_envelope_at(
        &self,
        vo_id: VoId,
        at: Option<jiff::Timestamp>,
        label: &str,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = load_ehrless(&self.demographic_pool, self.spec_profile, vo_id, None, at)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(
                    Missing::ObjectVersion,
                    format!("{label} {vo_id} version at time"),
                )
            })?;
        let meta = ResourceMeta::new(
            String::new(),
            object_version_id(vo_id, &read.creating_system_id, read.tree),
        )
        .with_last_modified(read.time_committed);
        let signer = self.signing_ctx().signer;
        let ov = version_envelope(&read, signer)?;
        Ok(ServiceResponse::new(ov, meta))
    }
}

/// A demographic write's version precondition, judged once against the tip
/// the write locks ([`crate::versioning::change::lock_current`]).
///
/// The `If-Match` value arrives as a token: a full `OBJECT_VERSION_ID`, a
/// bare `VERSION_TREE_ID` trunk number, or the RFC 9110 `*` wildcard
/// ([`expected_from_if_match`]). A delete also names its preceding version in
/// the path, which stands in when no `If-Match` names one.
pub(super) struct Precondition {
    /// A full `OBJECT_VERSION_ID` the tip must be (ITS-REST overview
    /// `Requests_and_responses.md` §"If-Match and accidental overwrites": a
    /// false condition is `412`).
    full: Option<ObjectVersionId>,
    /// The `VERSION_TREE_ID` the tip must be at.
    tree: Option<TreeId>,
}

impl Precondition {
    /// The precondition of an `If-Match` value, with `path` as the version a
    /// delete names when the header names none.
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] for a malformed `If-Match`, rejected rather
    /// than run as if no precondition was sent.
    pub(super) fn parse(
        if_match: Option<&str>,
        path: Option<TreeId>,
    ) -> Result<Self, ServiceError> {
        let Some(raw) = if_match else {
            return Ok(Self {
                full: None,
                tree: path,
            });
        };
        let token = if_match_token(raw);
        // NOTE: a token that is not a full OBJECT_VERSION_ID is legitimately the
        // `*` or trunk form; `expected_from_if_match` below refuses every other shape.
        let full = ObjectVersionId::from_str(token).ok();
        let tree = expected_from_if_match(raw).map_err(VersionIdError::into_argument)?;
        Ok(Self {
            full,
            tree: tree.or(path),
        })
    }

    /// The full-identity half: `412` when the `If-Match` names another
    /// version than the locked tip, compared case-insensitively (BASE
    /// `base_types` master05 §"Composite Identifiers and Case").
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] on a mismatch.
    pub(super) fn check_full(&self, vo_id: VoId, tip: &TipRow) -> Result<(), ServiceError> {
        crate::versioning::change::ensure_preceding(&tip_uid(vo_id, tip), self.full.as_ref())
    }

    /// The version-tree half: a stale preceding version is a version
    /// mismatch, which the wire answers with the latest `version_uid` in
    /// `ETag` (`responses/409_PERSON_with_uid_based_id.yaml`).
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the locked tip is at another
    /// version.
    pub(super) fn check_tree(&self, tip: &TipRow) -> Result<(), ServiceError> {
        let current =
            TreeId::from_columns(tip.trunk_version, tip.branch_number, tip.branch_version);
        match self.tree {
            Some(expected) if expected != current => Err(ServiceError::version_conflict(format!(
                "preceding_version_uid names version {expected}, latest is {current}"
            ))),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VO: &str = "8849182c-82ad-4088-a07f-48ead4180515";

    fn vo() -> VoId {
        VoId(VO.parse().expect("uuid"))
    }

    /// The locked tip `{VO}::openEHRSys.example.com::{trunk}`.
    fn tip(trunk: i32) -> TipRow {
        TipRow {
            ehr_id: None,
            kind: "PERSON".to_owned(),
            sys_version: trunk,
            trunk_version: trunk,
            branch_number: 0,
            branch_version: 0,
            creating_system_id: "openEHRSys.example.com".to_owned(),
            lifecycle_state: "532".to_owned(),
            open: true,
        }
    }

    fn full(if_match: &str) -> Result<(), ServiceError> {
        Precondition::parse(Some(if_match), None)?.check_full(vo(), &tip(2))
    }

    /// An `If-Match` naming the locked tip in full satisfies the precondition
    /// (ITS-REST overview §"If-Match and accidental overwrites").
    #[test]
    fn matching_full_ovid_passes() {
        let uid = format!("{VO}::openEHRSys.example.com::2");
        assert!(full(&uid).is_ok());
        // The quote tolerance of the library boundary.
        assert!(full(&format!("\"{uid}\"")).is_ok());
    }

    /// `creating_system_id` is a composite identifier, so a case variant names
    /// the SAME version and must NOT raise a spurious `412` (BASE `base_types`
    /// master05 §"Composite Identifiers and Case").
    #[test]
    fn case_variant_creating_system_id_matches() {
        for variant in [
            format!("{VO}::OPENEHRSYS.EXAMPLE.COM::2"),
            format!("{VO}::openehrsys.example.com::2"),
            format!("{}::openEHRSys.example.com::2", VO.to_uppercase()),
        ] {
            assert!(
                full(&variant).is_ok(),
                "BASE master05 §Composite Identifiers and Case: {variant:?} names the \
                 same version as the locked tip"
            );
        }
    }

    /// A stale full `OBJECT_VERSION_ID` — a mismatch in ANY segment — is the
    /// `412` branch.
    #[test]
    fn stale_or_foreign_ovid_is_a_version_mismatch() {
        for stale in [
            // wrong version_tree_id
            format!("{VO}::openEHRSys.example.com::1"),
            // wrong creating_system_id (not a mere case variant)
            format!("{VO}::other.system::2"),
            // wrong object_id
            "00000000-0000-4000-8000-0000000000ff::openEHRSys.example.com::2".to_owned(),
        ] {
            let err = full(&stale).expect_err("stale precondition");
            assert!(
                matches!(err, ServiceError::VersionConflict(_)),
                "{stale:?} must fail the precondition, got {err:?}"
            );
        }
    }

    /// An absent header states no precondition.
    #[test]
    fn absent_header_states_no_precondition() {
        let precondition = Precondition::parse(None, None).expect("no header");
        assert!(precondition.check_full(vo(), &tip(2)).is_ok());
        assert!(precondition.check_tree(&tip(2)).is_ok());
    }

    /// A non-OVID token carries no full identity to compare: the RFC 9110 `*`
    /// wildcard passes, the lenient trunk number is the tree precondition, and
    /// any other shape is rejected as malformed (`400`); nothing is silently
    /// skipped.
    #[test]
    fn non_ovid_tokens_are_the_tree_precondition() {
        let wildcard = Precondition::parse(Some("*"), None).expect("wildcard");
        assert!(wildcard.check_full(vo(), &tip(2)).is_ok());
        assert!(wildcard.check_tree(&tip(2)).is_ok());
        let trunk = Precondition::parse(Some("2"), None).expect("trunk number");
        assert!(trunk.check_full(vo(), &tip(2)).is_ok());
        assert!(trunk.check_tree(&tip(2)).is_ok());
        assert!(matches!(
            trunk.check_tree(&tip(3)),
            Err(ServiceError::VersionConflict(_))
        ));
        assert!(Precondition::parse(Some("garbage"), None).is_err());
    }

    /// A delete's path version stands in when no `If-Match` names one, and an
    /// `If-Match` naming one wins over it.
    #[test]
    fn path_version_stands_in_for_an_absent_if_match() {
        let path = Some(TreeId::trunk(1));
        let from_path = Precondition::parse(None, path).expect("no header");
        assert!(matches!(
            from_path.check_tree(&tip(2)),
            Err(ServiceError::VersionConflict(_))
        ));
        let from_header = Precondition::parse(Some("2"), path).expect("trunk number");
        assert!(from_header.check_tree(&tip(2)).is_ok());
    }
}
