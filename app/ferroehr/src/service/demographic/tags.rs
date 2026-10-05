// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic `ITEM_TAG` surface — the RM `common.item_tag` class applied
//! to parties (ehr-less: `ehr_id IS NULL`). The wire contract is RELEASED:
//! ITS-REST 1.1.0 publishes `demographic_tags_get` plus per-kind
//! `{person,agent,group,organisation,role}_tags_{get,update,delete}`
//! (SPECITS-77; the Demographic API's own lifecycle within the release is
//! DEVELOPMENT). The tag store is backed by the `item_tag` table via
//! `crate::storage::tag_repo` (storage owns the SQL — no openEHR spec governs
//! the storage, our own design); the RM `ITEM_TAG` invariants
//! (`Inv_key_valid`/`Inv_value_valid`) are judged through the one shared seam
//! `crate::service::ehr::tags::validate_item_tag`, so this family and the EHR
//! family cannot drift apart.

use std::collections::BTreeMap;

use openehr_base::prelude::{
    HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId, UidBasedId,
};
use openehr_its::rest::generated::common::UpdateItemTag;
use openehr_rm::prelude::ItemTag;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::ehr::tags::{item_tag_refusal, normalized_target_path, tag_target_tail};
use crate::service::error::ServiceError;
use crate::service::status::CallStatusType;
use crate::storage::tag_repo;
use crate::versioning::object_version_id::{VersionIdError, hier_object_id};

impl FerroEhrService {
    /// All demographic tags (ehr-less), optionally filtered by key/value/path.
    pub(super) async fn demographic_tags(
        &self,
        key: Option<&str>,
        value: Option<&str>,
        target_path: Option<&str>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let rows = tag_repo::list_tags(&self.demographic_pool, None, None, key, value, target_path)
            .await?;
        let sid = self.effective_system_id();
        rows.iter()
            .map(|r| party_item_tag(&sid, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// The tags on one party target — the `VERSIONED_PARTY` container
    /// (`target_version: None`) or ONE of its VERSIONs (`Some(tail)`). The two
    /// are DISJOINT collections of the same `target_vo_id` (RM
    /// `common.item_tag` `ITEM_TAG.target`: "may be a `VERSIONED_OBJECT<T>` or
    /// a `VERSION<T>`"; the released dual-form `uid_based_id` on every
    /// demographic tag operation).
    pub(super) async fn party_tags(
        &self,
        vo_id: VoId,
        target_version: Option<&str>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let rows = tag_repo::list_tags(
            &self.demographic_pool,
            None,
            Some((vo_id, target_version)),
            None,
            None,
            None,
        )
        .await?;
        let sid = self.effective_system_id();
        rows.iter()
            .map(|r| party_item_tag(&sid, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// The demographic tag-target guard, mirroring the EHR side
    /// (`service::ehr::tags::ensure_tag_target`): the addressed versioned
    /// object must exist AND be a party of the routed kind (a person route
    /// must not reach an agent's tags — the kind-checked-routes law), and a
    /// VERSION-addressed target must name an existing version. The released
    /// 404 trigger: `404_unknown_uid_based_id.yaml` — "returned when the
    /// `uid_based_id` does not exist".
    pub(super) async fn ensure_party_tag_target(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
    ) -> Result<(), ServiceError> {
        let stored = crate::versioning::read::object_kind(&self.demographic_pool, vo_id).await?;
        if stored != Some(crate::service::demographic::support::kind_of(kind)) {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("tag target {} {vo_id}", kind.rm_type()),
            ));
        }
        if let Some(version) = target_version {
            let (_, tree) = crate::versioning::object_version_id::components(version)?;
            let (trunk, branch_number, branch_version) = tree.columns();
            if !crate::storage::version_repo::meta::version_exists(
                &self.demographic_pool,
                vo_id,
                trunk,
                branch_number,
                branch_version,
            )
            .await?
            {
                return Err(ServiceError::sm(
                    CallStatusType::ObjectVersionDoesNotExist,
                    format!("tag target version {}", version.value()),
                ));
            }
        }
        Ok(())
    }

    /// Replace the whole tag collection of a party with the posted set (PUT
    /// full-collection semantics; an empty list clears all). Duplicates in the
    /// body are last-wins on the `ITEM_TAG` identity — the `(key, target_path)`
    /// PAIR (ITS-REST overview `Requests_and_responses.md` §openehr-item-tag and
    /// openehr-version-item-tag: "uniquely identified by their `key` and
    /// `target_path` pair attributes"; RM common master07-tags). The RM
    /// `ITEM_TAG` invariants are enforced before any write: `Inv_key_valid`
    /// (non-empty, no surrounding whitespace) and `Inv_value_valid`
    /// (`value /= Void implies not value.is_empty`).
    pub(super) async fn replace_party_tags(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
        tags: &[UpdateItemTag],
    ) -> Result<Vec<ItemTag>, ServiceError> {
        self.ensure_party_tag_target(kind, vo_id, target_version)
            .await?;
        // Validate + dedup (last wins) before touching the DB, keyed on the
        // ITEM_TAG identity — the (key, target_path) PAIR, never the key alone.
        // The invariants and the `target_path: ""` normalization are the EHR
        // family's, called rather than restated.
        let target = match target_version {
            Some(version) => UidBasedId::ObjectVersionId(version.clone()),
            // A bare container key is a UUID by type, so the conversion is
            // total (BASE `master05-identification_package.adoc` §Syntaxes:
            // `uid = iso_oid | uuid | internet_id`).
            None => UidBasedId::HierObjectId(HierObjectId::from(vo_id.0)),
        };
        let owner_id = party_owner_ref(&self.effective_system_id())?;
        let mut deduped: BTreeMap<(String, Option<String>), Option<String>> = BTreeMap::new();
        for tag in tags {
            let target_path = normalized_target_path(tag.target_path.as_deref());
            // Construction IS the invariant check.
            ItemTag::new(
                tag.key.clone(),
                tag.value.clone(),
                target.clone(),
                target_path.map(str::to_owned),
                owner_id.clone(),
            )
            .map_err(|e| item_tag_refusal(&e))?;
            deduped.insert(
                (tag.key.clone(), target_path.map(str::to_owned)),
                tag.value.clone(),
            );
        }

        // A NULL (demographic) scope never collides on the unique index (NULLs
        // are distinct), so the pre-dedup above is what enforces last-wins.
        let new_tags: Vec<tag_repo::NewTag<'_>> = deduped
            .iter()
            .map(|((key, target_path), value)| tag_repo::NewTag {
                target_type: kind.rm_type(),
                key: key.as_str(),
                value: value.as_deref(),
                target_path: target_path.as_deref(),
            })
            .collect();
        let mut tx = self.demographic_pool.begin().await?;
        // The replace RETURNS the stored collection (list order), so the
        // response never re-reads the rows the transaction just wrote.
        let stored = tag_repo::replace_tags(
            &mut tx,
            None,
            vo_id,
            tag_target_tail(target_version),
            &new_tags,
        )
        .await?;
        tx.commit().await?;
        let sid = self.effective_system_id();
        stored
            .iter()
            .map(|r| party_item_tag(&sid, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// Delete a target's tags by key (a SET delete over the `(key,
    /// target_path)` identities sharing the key, addressed to the container or
    /// ONE VERSION per the parsed `uid_based_id`). The target itself is gated
    /// first (`404_unknown_uid_based_id_or_key.yaml`: "the `uid_based_id` does
    /// not exist, or … the `ITEM_TAG` identified by the `key` does not exist");
    /// an unknown key on an existing target is the same `404`.
    pub(super) async fn delete_party_tag(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
        key: &str,
    ) -> Result<(), ServiceError> {
        self.ensure_party_tag_target(kind, vo_id, target_version)
            .await?;
        if !tag_repo::delete_tag(
            &self.demographic_pool,
            None,
            vo_id,
            tag_target_tail(target_version),
            key,
        )
        .await?
        {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("item tag {key:?}"),
            ));
        }
        Ok(())
    }
}

/// One stored demographic tag row as the RM `ITEM_TAG` it is
/// (`item_tag.adoc`): `key`, optional `value` and `target_path`, and `target` as
/// a bare `UID_BASED_ID`, a `HIER_OBJECT_ID` for a container target and an
/// `OBJECT_VERSION_ID` for a VERSION target ("may be a `VERSIONED_OBJECT<T>` or
/// a `VERSION<T>`"). This is the EHR sibling's shape; the released OAS
/// `ItemTag` schema's `OBJECT_REF` wrapper loses the conflict to the RM.
///
/// NOTE: `owner_id` follows the released examples' shape, an `OBJECT_REF`
/// `{namespace: local, type: SYSTEM}` whose `id` carries the server's configured
/// system identifier (every `schemas/demographic/ItemTagOf<T>.yaml` example); no
/// demographic class declares a `tags` containment, so the EHR side's `EHR.tags`
/// anchor has no analogue here.
///
/// # Errors
/// [`VersionIdError`] when the configured `system_id` or the stored tag target
/// is not a well-formed BASE identifier.
fn party_item_tag(system_id: &str, row: &tag_repo::TagRow) -> Result<ItemTag, ServiceError> {
    // A stored row that no longer constructs is storage corruption, not a
    // client fault — fail loud.
    ItemTag::new(
        row.key.clone(),
        row.value.clone(),
        crate::service::ehr::tags::tag_target(row)?,
        row.target_path.clone(),
        party_owner_ref(system_id)?,
    )
    .map_err(|e| ServiceError::internal("stored ITEM_TAG row", e))
}

/// The `ITEM_TAG.owner_id` of a demographic (ehr-less) tag — the `OBJECT_REF`
/// `{namespace: local, type: SYSTEM}` of every released
/// `schemas/demographic/ItemTagOf<T>.yaml` example, whose `id` carries the
/// server's configured system identifier. One function so
/// the shape a tag is VALIDATED under is the shape it is SERVED under.
///
/// # Errors
/// [`VersionIdError`] when the configured `system_id` is not a well-formed
/// BASE identifier.
fn party_owner_ref(system_id: &str) -> Result<ObjectRef, VersionIdError> {
    Ok(ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: "SYSTEM".to_owned(),
        id: ObjectId::HierObjectId(hier_object_id(system_id)?),
    }))
}
