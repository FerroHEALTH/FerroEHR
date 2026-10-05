// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ITEM_TAG` CRUD on the `item_tag` table — a RELEASED ITS-REST 1.1.0 surface
//! (23 dedicated operations plus the two wrapper headers, added by SPECITS-77;
//! ITS-REST overview `Amendment_record.md` §Release-1.1.0). Server support for
//! it is optional — "If the server does not support `ITEM_TAGs`, these headers
//! will also be unsupported" (overview `Requests_and_responses.md` §item-tag
//! headers) — and this server supports it.
//!
//! Spec: RM `ITEM_TAG`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`
//! — `target: UID_BASED_ID` "may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`",
//! `owner_id: OBJECT_REF`, the invariants `Inv_key_valid` / `Inv_value_valid`;
//! RM ehr `ehr.adoc` `EHR.tags`: "Tag target values can only be within the
//! same EHR") and ITS-REST overview `Requests_and_responses.md` §item-tag
//! headers (identity: "uniquely identified by their `key` and `target_path`
//! pair"; container vs specific-VERSION targets are distinct). `PUT …/tags`
//! is a full-collection replace of the ADDRESSED collection — the container's
//! or one VERSION's, never both. Not an SM-EHR interface. The `item_tag`
//! table SQL is spec-silent (storage seam — our own design).

use openehr_base::prelude::{
    HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId, UidBasedId,
};
use openehr_its::rest::generated::common::UpdateItemTag;
use openehr_rm::prelude::ItemTag;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::{ServiceError, Violation};
use crate::service::status::{CallStatusType, SmError};
use crate::versioning::object_version_id::{VersionIdError, parse_uid_based_id};

impl FerroEhrService {
    /// All tags in an EHR, optionally filtered by key / value / target path.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the tag listing fails.
    pub(in crate::service) async fn ehr_tags(
        &self,
        ehr_id: EhrId,
        key: Option<&str>,
        value: Option<&str>,
        target_path: Option<&str>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        // The released 404 trigger ("when an EHR with `ehr_id` does not
        // exist", 404_unknown_ehr_id) — an unknown EHR is 404, never an
        // empty 200 list; an EXISTING EHR with no matching tags is [].
        self.ensure_ehr_exists(ehr_id).await?;
        let rows = crate::storage::tag_repo::list_tags(
            &self.pool,
            Some(ehr_id),
            None,
            key,
            value,
            target_path,
        )
        .await?;
        rows.iter()
            .map(|r| Self::stored_item_tag(ehr_id, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// Tags on one target object (a COMPOSITION or `EHR_STATUS`). The caller
    /// runs [`Self::ensure_tag_target`] first — this reads the collection of
    /// an already-verified target (an existing target with no tags is an
    /// empty list, never an error).
    ///
    /// NOTE (supersession): a CONTAINER-addressed tag survives every new version
    /// of its target and a VERSION-addressed tag stays pinned to the version it
    /// names, migrated by nothing — the two arities are the point of
    /// `ITEM_TAG.target`, a `UID_BASED_ID` that "may be a `VERSIONED_OBJECT<T>`
    /// or a `VERSION<T>`" (RM common
    /// `UML/classes/org.openehr.rm.common.item_tag.adoc` §Attributes), and RM ehr
    /// `master04-ehr_package.adoc` §Tags forbids a commit from touching tags at
    /// all ("they do not cause re-versioning of the content"). Our own design.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the tag listing fails.
    pub(in crate::service) async fn target_tags(
        &self,
        ehr_id: EhrId,
        target_vo_id: VoId,
        target_version: Option<&str>,
    ) -> Result<Vec<ItemTag>, ServiceError> {
        let rows = crate::storage::tag_repo::list_tags(
            &self.pool,
            Some(ehr_id),
            Some((target_vo_id, target_version)),
            None,
            None,
            None,
        )
        .await?;
        rows.iter()
            .map(|r| Self::stored_item_tag(ehr_id, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// Replace the **whole** tag collection of a target with the posted set,
    /// returning the target's tags after — `PUT` full-collection semantics
    ///: tags omitted from the body are removed, and an empty list
    /// clears all tags on the target.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist or the target
    /// versioned object is not in this EHR; [`ServiceError::Unprocessable`]
    /// when a tag violates `Inv_key_valid` / `Inv_value_valid`;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn replace_tags(
        &self,
        ehr_id: EhrId,
        target_vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
        target_type: &str,
        tags: &[UpdateItemTag],
    ) -> Result<Vec<ItemTag>, ServiceError> {
        self.ensure_ehr_exists(ehr_id).await?;
        self.ensure_tag_target(ehr_id, target_vo_id, target_version, target_type)
            .await?;
        // The storage replace dedupes the posted set on the ITEM_TAG identity —
        // the (key, target_path) PAIR (ITS-REST Requests_and_responses.md
        // §item-tag headers), last-wins. Each posted UPDATE_ITEM_TAG is first
        // turned into the ITEM_TAG the write would store, with the server's own
        // `target`/`owner_id`, and judged by `ItemTag`'s own `Validate` impl.
        let target = match target_version {
            Some(version) => UidBasedId::ObjectVersionId(version.clone()),
            None => UidBasedId::HierObjectId(HierObjectId::from(target_vo_id.0)),
        };
        let owner_id = ehr_owner_ref(ehr_id);
        let mut new_tags: Vec<crate::storage::tag_repo::NewTag<'_>> =
            Vec::with_capacity(tags.len());
        for tag in tags {
            let target_path = normalized_target_path(tag.target_path.as_deref());
            // Construction IS the invariant check: a violating tag cannot exist
            // as a typed ItemTag.
            ItemTag::new(
                tag.key.clone(),
                tag.value.clone(),
                target.clone(),
                target_path.map(str::to_owned),
                owner_id.clone(),
            )
            .map_err(|e| item_tag_refusal(&e))?;
            new_tags.push(crate::storage::tag_repo::NewTag {
                target_type,
                key: &tag.key,
                value: tag.value.as_deref(),
                target_path,
            });
        }
        let mut tx = self.pool.begin().await?;
        // The replace RETURNS the stored collection (list order), so the
        // response never re-reads the rows the transaction just wrote.
        let stored = crate::storage::tag_repo::replace_tags(
            &mut tx,
            Some(ehr_id),
            target_vo_id,
            tag_target_tail(target_version),
            &new_tags,
        )
        .await?;
        tx.commit().await?;
        stored
            .iter()
            .map(|r| Self::stored_item_tag(ehr_id, r))
            .collect::<Result<Vec<_>, _>>()
    }

    /// The tag-target guard: the addressed versioned object must exist in
    /// THIS EHR ("Tag target values can only be within the same EHR" — RM ehr
    /// `ehr.adoc` `EHR.tags`), its stored kind must match the route family
    /// (a COMPOSITION route must not tag an `EHR_STATUS` container), and a
    /// VERSION-addressed target must name an existing version. The `item_tag`
    /// table is deliberately FK-less, so these checks live here.
    async fn ensure_tag_target(
        &self,
        ehr_id: EhrId,
        target_vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
        target_type: &str,
    ) -> Result<(), ServiceError> {
        let not_found = || {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!(
                    "tag target {target_vo_id} does not exist in EHR {ehr_id} \
                     (tag targets can only be within the same EHR)"
                ),
            )
        };
        let Some((owner, kind)) =
            crate::storage::version_repo::meta::vo_owner_kind(&self.pool, target_vo_id).await?
        else {
            return Err(not_found());
        };
        if owner != Some(ehr_id) || kind != target_type {
            return Err(not_found());
        }
        if let Some(version) = target_version {
            let (_, tree) = crate::versioning::object_version_id::components(version)?;
            let (trunk, branch_number, branch_version) = tree.columns();
            if !crate::storage::version_repo::meta::version_exists(
                &self.pool,
                target_vo_id,
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

    /// Delete a target collection's tags by key. The wire addresses tags by
    /// `key` alone while the `ITEM_TAG` identity is the (`key`, `target_path`)
    /// pair — so this is a SET delete: every tag under the key in the
    /// addressed collection goes (ITS-REST `Requests_and_responses.md`
    /// §item-tag headers; the Release-1.1.0 tag routes carry no path
    /// selector).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when no such tag exists on the target;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn delete_tag(
        &self,
        ehr_id: EhrId,
        target_vo_id: VoId,
        target_version: Option<&ObjectVersionId>,
        target_type: &str,
        key: &str,
    ) -> Result<(), ServiceError> {
        // The target guard runs on the DELETE too: the released 404 trigger
        // covers "when the `uid_based_id` does not exist", the collection is
        // EHR-scoped ("owned by EHR identified by `ehr_id`"), and the route
        // family is kind-checked — a composition-route DELETE must not touch
        // an EHR_STATUS container's tags (adjudicated).
        self.ensure_tag_target(ehr_id, target_vo_id, target_version, target_type)
            .await?;
        if !crate::storage::tag_repo::delete_tag(
            &self.pool,
            Some(ehr_id),
            target_vo_id,
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

    /// One stored tag row as the RM `ITEM_TAG` it is (`item_tag.adoc`): `key`,
    /// optional `value`/`target_path`, `target` as a bare `UID_BASED_ID` — a
    /// `HIER_OBJECT_ID` for a container target, an `OBJECT_VERSION_ID` for a
    /// VERSION target ("may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`") —
    /// and `owner_id` as the RM's `OBJECT_REF` to the owning EHR. The served
    /// bytes are then the generated canonical-JSON encoding of this instance,
    /// produced at the protocol edge.
    ///
    /// NOTE: `_type: "ITEM_TAG"` IS on the wire — the released `ItemTag.yaml` is
    /// `additionalProperties: false` without declaring `_type` while the same
    /// group's `discriminator.propertyName` names that member, and that
    /// contradiction resolves in favour of the discriminator, since an `ITEM_TAG`
    /// carries no `uid` and no archetype id to identify itself by. `target` is a
    /// BARE `UID_BASED_ID`, the RM's shape, not the OAS's `OBJECT_REF` wrapper:
    /// where a wire projection disagrees with the released model it projects
    /// about what an attribute IS, the model decides.
    fn stored_item_tag(
        ehr_id: EhrId,
        row: &crate::storage::tag_repo::TagRow,
    ) -> Result<ItemTag, ServiceError> {
        // A stored row that no longer constructs is storage corruption, not a
        // client fault — fail loud as the 500 class.
        ItemTag::new(
            row.key.clone(),
            row.value.clone(),
            tag_target(row)?,
            row.target_path.clone(),
            ehr_owner_ref(ehr_id),
        )
        .map_err(|e| ServiceError::internal("stored ITEM_TAG row", e))
    }
}

/// The `ITEM_TAG.target` identifier of a stored tag row: "Identifier of target,
/// which may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`" (RM common
/// `UML/classes/org.openehr.rm.common.item_tag.adoc` §Attributes) — a
/// version-scoped tag names the `OBJECT_VERSION_ID`, an object-scoped one the
/// container's `HIER_OBJECT_ID`. Shared with the demographic tag surface
/// (`crate::service::demographic::tags`), which tags the same rows under a
/// different owner.
///
/// # Errors
/// [`VersionIdError`] when the stored `target_version` tail does not compose
/// with the target key into a well-formed `OBJECT_VERSION_ID` (BASE
/// `master05-identification_package.adoc` §Syntaxes) — the identifier is
/// refused rather than served malformed.
pub(in crate::service) fn tag_target(
    row: &crate::storage::tag_repo::TagRow,
) -> Result<UidBasedId, VersionIdError> {
    let target_vo_id = row.target_vo_id;
    Ok(match &row.target_version {
        Some(tail) => {
            let raw = format!("{target_vo_id}::{tail}");
            UidBasedId::ObjectVersionId(
                ObjectVersionId::new(raw.clone())
                    .map_err(|source| VersionIdError::Malformed { raw, source })?,
            )
        }
        // A bare container key is a UUID by type, so the conversion is total
        // (BASE §Syntaxes: `uid = iso_oid | uuid | internet_id`).
        None => UidBasedId::HierObjectId(HierObjectId::from(target_vo_id.0)),
    })
}

/// The `ITEM_TAG.owner_id` of an EHR-scoped tag: the RM's `OBJECT_REF` to the
/// owning EHR ("Identifier of owner object, such as EHR", RM common
/// `UML/classes/org.openehr.rm.common.item_tag.adoc` §Attributes; RM ehr
/// `ehr.adoc` `EHR.tags` scopes tag targets to that same EHR). One function so
/// the shape a tag is VALIDATED under and the shape it is SERVED under can
/// never drift apart.
fn ehr_owner_ref(ehr_id: EhrId) -> ObjectRef {
    ObjectRef::ObjectRef(ObjectRefData {
        namespace: "local".to_owned(),
        r#type: "EHR".to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::from(ehr_id.0)),
    })
}

/// `target_path: ""` normalizes to ABSENT — one identity, not two.
///
/// RM models `target_path` 0..1 (absent = no path) with no non-empty
/// invariant, while six of the seven released `ItemTagOf<T>` examples write
/// `target_path: ""`; under the (`key`, `target_path`) identity those would be
/// two distinct tags, so the normalization is our own — and this is the ONE
/// function that applies it: the EHR and demographic families both call it, so
/// applying it identically across the two families is a structural fact rather
/// than a claim.
pub(in crate::service) fn normalized_target_path(raw: Option<&str>) -> Option<&str> {
    raw.filter(|p| !p.is_empty())
}
/// Refuse an [`ItemTagError`](openehr_rm::v1_2::common::tags::item_tag_impl::ItemTagError)
/// as the ITS-REST 422 shape (the same
/// `Violation` the pre-constructor `validate_item_tag` produced): the typed
/// constructor IS the invariant check now (construction =
/// validation), so the service seam only maps the refusal onto the wire.
pub(in crate::service) fn item_tag_refusal(
    err: &openehr_rm::v1_2::common::tags::item_tag_impl::ItemTagError,
) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new(format!("item tag violates its RM invariants: {err}"))
            .with_path("ITEM_TAG")
            .with_source(err.clone()),
    )
}

// ── The ITS-REST tags call surface ────────────────────────────────────────────

impl FerroEhrService {
    /// `GET /ehr/{ehr_id}/tags` — all tags in an EHR, optionally filtered.
    ///
    /// # Errors
    /// [`SmError`] if the tag listing fails.
    pub async fn ehr_tags_get(
        &self,
        an_ehr_id: EhrId,
        key: Option<String>,
        value: Option<String>,
        target_path: Option<String>,
    ) -> Result<Vec<ItemTag>, SmError> {
        Ok(self
            .ehr_tags(
                an_ehr_id,
                key.as_deref(),
                value.as_deref(),
                target_path.as_deref(),
            )
            .await?)
    }

    /// `GET …/{uid_based_id}/tags` — the tags on one target object.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `uid_based_id` (400-equivalent) or a
    /// failing tag listing.
    pub async fn target_tags_get(
        &self,
        an_ehr_id: EhrId,
        uid_based_id: String,
        target_type: &str,
    ) -> Result<Vec<ItemTag>, SmError> {
        let (vo_id, version) = parse_tag_target(&uid_based_id)?;
        // The released 404 trigger — "when the `uid_based_id` does not
        // exist" — plus the EHR scope and the route-kind discipline
        // (adjudicated): the guard runs on the GET too; an existing
        // target with no tags stays an empty 200 list.
        self.ensure_tag_target(an_ehr_id, vo_id, version.as_ref(), target_type)
            .await?;
        Ok(self
            .target_tags(an_ehr_id, vo_id, tag_target_tail(version.as_ref()))
            .await?)
    }

    /// `PUT …/{uid_based_id}/tags` — full-collection replace of a target's
    /// tags, returning the collection after the write.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `uid_based_id`, a missing EHR or target
    /// (404-equivalent), an invalid tag (422-equivalent), or a storage
    /// failure.
    pub async fn target_tags_replace(
        &self,
        an_ehr_id: EhrId,
        uid_based_id: String,
        target_type: &str,
        tags: Vec<UpdateItemTag>,
    ) -> Result<Vec<ItemTag>, SmError> {
        let (vo_id, version) = parse_tag_target(&uid_based_id)?;
        Ok(self
            .replace_tags(an_ehr_id, vo_id, version.as_ref(), target_type, &tags)
            .await?)
    }

    /// `DELETE …/{uid_based_id}/tags/{key}` — delete one tag by key.
    ///
    /// # Errors
    /// [`SmError`] for a malformed `uid_based_id`, a missing tag
    /// (404-equivalent), or a storage failure.
    pub async fn target_tag_delete(
        &self,
        an_ehr_id: EhrId,
        uid_based_id: String,
        target_type: &str,
        key: String,
    ) -> Result<(), SmError> {
        let (vo_id, version) = parse_tag_target(&uid_based_id)?;
        self.delete_tag(an_ehr_id, vo_id, version.as_ref(), target_type, &key)
            .await?;
        Ok(())
    }
}

/// Decode a tag route's `uid_based_id` into the versioned-object id and — for a
/// VERSION-addressed target — the full `OBJECT_VERSION_ID` it named (RM
/// `item_tag.adoc`: `target` "may be a `VERSIONED_OBJECT<T>` or a
/// `VERSION<T>`"), through the one platform decoder
/// ([`parse_uid_based_id`]).
///
/// The stored tail (`creating_system_id::version_tree_id`) is read back off the
/// decoded id as [`ObjectVersionId::extension`] — BASE
/// `master05-identification_package.adoc` `UID_BASED_ID.extension` is exactly
/// "the part right of the first `::`" — so the verbatim, case-preserving bytes
/// come from the typed structure rather than a second hand-rolled split
/// ([`tag_target_tail`]).
///
/// # Errors
/// [`SmError`] (precondition, → `400`) when the value is neither a UUID nor a
/// well-formed `OBJECT_VERSION_ID`.
pub(in crate::service) fn parse_tag_target(
    uid_based_id: &str,
) -> Result<(VoId, Option<ObjectVersionId>), SmError> {
    let decoded = parse_uid_based_id(uid_based_id)?;
    Ok((decoded.vo_id, decoded.version))
}

/// The stored tag-target version tail — `creating_system_id::version_tree_id`,
/// verbatim (`UID_BASED_ID.extension`, BASE master05) — for a
/// VERSION-addressed target, or `None` for a container-addressed one.
pub(in crate::service) fn tag_target_tail(version: Option<&ObjectVersionId>) -> Option<&str> {
    version.map(ObjectVersionId::extension)
}
