// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic chapter's shared seam onto the [`crate::versioning`]
//! change-control machinery and the version spine in
//! `crate::storage::version_repo`: [`PartyKind`]↔[`Kind`] mapping, the
//! ehr-less version loads, commit-audit construction, and the canonical wire
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

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData};
use openehr_rm::prelude::{
    RevisionHistory, RevisionHistoryItem, VersionedObject, VersionedObjectData, VersionedParty,
};
use serde_json::Value;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::error::ServiceError;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::CallStatusType;
use crate::storage::version_repo;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::Committed;
use crate::versioning::object_version_id::{
    TreeId, VersionIdError, hier_object_id, object_version_id, version_id,
};
use crate::versioning::read::{VersionRead, read_current, read_version, version_at};
use crate::versioning::wire::version_envelope;
use crate::versioning::{CommitEnv, Kind};
use openehr_its::rest::generated::common::UpdateAudit;

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

/// The REST [`PartyKind`] for a versioned-object [`Kind`], or `None` for a
/// non-party kind (COMPOSITION / `EHR_STATUS` / … / `PARTY_RELATIONSHIP`).
pub(super) fn party_kind_of(kind: Kind) -> Option<PartyKind> {
    match kind {
        Kind::Agent => Some(PartyKind::Agent),
        Kind::Group => Some(PartyKind::Group),
        Kind::Organisation => Some(PartyKind::Organisation),
        Kind::Person => Some(PartyKind::Person),
        Kind::Role => Some(PartyKind::Role),
        _ => None,
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
        (Some(v), _) => read_version(pool, profile, vo_id, v).await?,
        (None, Some(at)) => version_at(pool, profile, vo_id, at).await?,
        (None, None) => read_current(pool, profile, vo_id).await?,
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

/// The create/update representation built **from the commit result**, never a
/// post-commit re-read: the served body is the just-written `canonical` with
/// the `uid` injected, and the identity + commit instant come straight from
/// [`Committed`] (RM common master06 §Committal and Audits — the
/// written version identity). Byte-identical to a fresh read: the served form is
/// `inject_uid(reassemble(decompose(canonical)))`, and the node codec
/// round-trips `canonical` losslessly (pinned by a test). The caller passes
/// the pre-write `canonical`; the multimedia-externalization fallback (where
/// the stored form is offloaded and the in-memory body would diverge) stays in
/// the party write paths.
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

/// Count one committed write transaction — the `db_transactions` outcome
/// metric every service write path records (no openEHR spec governs this —
/// our own telemetry).
pub(super) fn record_commit() {
    crate::telemetry::metrics::metrics()
        .db_transactions
        .add(1, &[opentelemetry::KeyValue::new("outcome", "commit")]);
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
    /// The version `commit_audit` for a demographic write (RM common master04
    /// §Audit Details): when the request carried committal headers,
    /// `update_audit`'s `UPDATE_VERSION` audit attributes merge with the
    /// server rules (ITS-REST overview §"openehr-version and
    /// openehr-audit-details", a MUST — including a caller-supplied
    /// `change_type`, validated against the `audit_change_type` group and the
    /// operation); otherwise the server defaults apply — the effective
    /// `system_id`, the numeric `audit_change_type` group code, and the
    /// request's default committer (the authenticated principal's
    /// `PARTY_PROXY`, from [`CommitEnv::default_committer`]).
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] / [`ServiceError::BadRequest`] — the
    /// caller's `change_type` is out-of-group / contradicts the operation
    /// ([`AuditInput::from_update`]).
    pub(super) fn demographic_audit(
        &self,
        update_audit: Option<&UpdateAudit>,
        change_type: &str,
        description: &str,
    ) -> Result<AuditInput, ServiceError> {
        match update_audit {
            Some(u) => {
                AuditInput::from_update(u, change_type, description, &self.effective_system_id())
            }
            None => Ok(AuditInput {
                system_id: self.effective_system_id(),
                change_type: change_type.to_owned(),
                description: Some(crate::versioning::audit::dv_text(description)),
                committer: CommitEnv::default_committer(self),
                attestation: None,
            }),
        }
    }

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
                ServiceError::sm(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("{label} {vo_id}"),
                )
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
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
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
            ServiceError::sm(
                CallStatusType::ObjectVersionDoesNotExist,
                format!("{label} {vo_id} v{version}"),
            )
        })?;
        let signer = CommitEnv::signing_ctx(self).signer;
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
                ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
                    format!("{label} {vo_id} version at time"),
                )
            })?;
        let meta = ResourceMeta::new(
            String::new(),
            object_version_id(vo_id, &read.creating_system_id, read.tree),
        )
        .with_last_modified(read.time_committed);
        let signer = CommitEnv::signing_ctx(self).signer;
        let ov = version_envelope(&read, signer)?;
        Ok(ServiceResponse::new(ov, meta))
    }
}
