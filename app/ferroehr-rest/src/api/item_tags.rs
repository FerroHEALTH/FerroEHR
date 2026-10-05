// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ITEM_TAG` write-wrapper seam, shared by every API group that accepts it.
//!
//! Governing spec: `Requests_and_responses.md` §"openehr-item-tag and
//! openehr-version-item-tag". The two wrapper headers are "convenient wrappers
//! around the dedicated `ITEM_TAG` operations", so a group that offers them offers
//! exactly the same judgement, the same target resolution and the same echo as
//! every other group: one implementation lives here, and the EHR and demographic
//! dispatchers consume it.
//!
//! The request half is [`pending`] (parse + RM-invariant judgement, run BEFORE
//! any content is committed), the write half is [`persist`] (run AFTER the
//! commit that mints the target version), and the response half is [`echo`].
//! [`write_body`] serves the dedicated `PUT` operations the headers wrap.

use axum::response::Response;
use bytes::Bytes;
use http::{HeaderMap, HeaderName};

use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::generated::common::UpdateItemTag;
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::ItemTag;

use ferroehr::ids::EhrId;
use ferroehr::service::demographic::types::PartyKind;

use crate::overview::error::RestError;
use crate::overview::negotiate;
use crate::overview::params::{
    H_ITEM_TAG, H_VERSION_ITEM_TAG, ItemTagHeaderEntry, emit_item_tag_header,
    item_tag_to_header_entry, parse_item_tag_header, validate_item_tag_entries,
};
use crate::state::AppState;

/// The resource family a wrapper-header write addresses, and with it the
/// service seam that stores the tags.
///
/// The spec names the wrapper's targets as "change-controlled resources
/// (e.g. `COMPOSITION`, `EHR_STATUS`, `FOLDER`, etc.)"
/// (`Requests_and_responses.md` §"openehr-item-tag and
/// openehr-version-item-tag"); a demographic PARTY is change-controlled the same
/// way and reaches its tags through the demographic service seam.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TagTarget {
    /// Change-controlled EHR content, named by its RM type
    /// (`COMPOSITION`/`EHR_STATUS`/`FOLDER`) within one EHR.
    EhrContent {
        /// The EHR that owns the tagged content.
        ehr_id: EhrId,
        /// The RM type name of the tagged content.
        target_type: &'static str,
    },
    /// A demographic PARTY of one kind.
    Party(PartyKind),
}

/// The `ITEM_TAG` lists a request's wrapper headers ask for, parsed and
/// invariant-checked **before** any content is committed.
///
/// `None` per field = the request carried no such header, so that collection is
/// left untouched; `Some(list)` = the list to write (empty = the release's
/// clear-all form).
#[derive(Debug, Default)]
pub(crate) struct PendingItemTags {
    /// What `openehr-item-tag` asks to store on the `VERSIONED_OBJECT`.
    object: Option<Vec<UpdateItemTag>>,
    /// What `openehr-version-item-tag` asks to store on the committed VERSION.
    version: Option<Vec<UpdateItemTag>>,
}

impl PendingItemTags {
    /// Returns `true` when the request carried neither wrapper header, so no
    /// collection is written and no target has to be resolved.
    pub(crate) fn is_empty(&self) -> bool {
        self.object.is_none() && self.version.is_none()
    }
}

/// The `ITEM_TAG` collections a wrapper-header write stored, kept one per header
/// because `openehr-item-tag` and `openehr-version-item-tag` "apply to"
/// different targets — a `VERSIONED_OBJECT` and a specific VERSION within it
/// (`Requests_and_responses.md` §"openehr-item-tag and
/// openehr-version-item-tag"). `None` = the request carried no such header, so
/// that target was not written and nothing is echoed for it; `Some(list)` = the
/// list now stored on that target (empty confirms a clear).
#[derive(Debug, Default)]
pub(crate) struct StoredItemTags {
    /// The `VERSIONED_OBJECT` container's stored tags (`openehr-item-tag`).
    pub(crate) object: Option<Vec<ItemTag>>,
    /// The committed VERSION's own stored tags (`openehr-version-item-tag`).
    pub(crate) version: Option<Vec<ItemTag>>,
}

impl StoredItemTags {
    /// Echoes both stored collections onto a create/update response.
    pub(crate) fn echo(&self, resp: &mut Response) {
        echo(resp, self.object.as_deref(), self.version.as_deref());
    }
}

/// Parses and validates both wrapper headers before the content write.
///
/// The release gives the wrapper headers no atomicity semantics, so the ordering
/// is ours: a header defect the server can detect without touching storage
/// rejects the whole request while nothing has been committed, because failing
/// after the commit would answer 4xx for a request whose VERSION is already
/// durable and whose response carries no `ETag`/`Location`.
///
/// The tag write itself stays after the commit and must: the tags target the
/// version the commit mints, and RM common `master07-tags.adoc` forbids a tag
/// from participating in the content's change control — "they do not cause
/// re-versioning of the content".
///
/// # Errors
/// [`ApiError::BadRequest`] for a malformed header entry;
/// [`ApiError::Unprocessable`] for an entry that breaks an RM `ITEM_TAG`
/// invariant.
pub(crate) fn pending(headers: &HeaderMap) -> Result<PendingItemTags, RestError> {
    // `None` = header absent (leave tags intact); `Some(empty)` = empty header
    // value (clear all tags) — the parse helper draws the distinction.
    Ok(PendingItemTags {
        object: validated_entries(parse_item_tag_header(headers, H_ITEM_TAG)?, H_ITEM_TAG)?,
        version: validated_entries(
            parse_item_tag_header(headers, H_VERSION_ITEM_TAG)?,
            H_VERSION_ITEM_TAG,
        )?,
    })
}

/// Turns one header's parsed entries into the write DTOs, refusing any entry the
/// RM `ITEM_TAG` invariants reject
/// ([`crate::overview::params::validate_item_tag_entries`] — the one judgement
/// every tag family shares).
fn validated_entries(
    entries: Option<Vec<ItemTagHeaderEntry>>,
    name: &str,
) -> Result<Option<Vec<UpdateItemTag>>, RestError> {
    let Some(entries) = entries else {
        return Ok(None);
    };
    validate_item_tag_entries(&entries, name)?;
    Ok(Some(
        entries
            .into_iter()
            .map(|entry| UpdateItemTag {
                key: entry.key,
                value: entry.value,
                target_path: entry.target_path,
            })
            .collect(),
    ))
}

/// Applies the `openehr-item-tag` / `openehr-version-item-tag` write-wrapper
/// request headers on a change-controlled write
/// (`Requests_and_responses.md` §"openehr-item-tag and openehr-version-item-tag"
/// §Usage in Requests): the provided tag list **replaces** the target's
/// `ITEM_TAG` list, and "providing an empty value for this header will
/// effectively remove all `ITEM_TAGs` associated with the given target".
///
/// This is the WRITE half only: the headers were already parsed and
/// invariant-checked by [`pending`], BEFORE the content commit, so by the time
/// this runs the only failure left is a storage one. The write stays after the
/// commit because the tags target the version the commit mints, and a tag must
/// not re-version the content it annotates. The entries are folded onto the
/// existing uid-keyed replace seam the dedicated `*_tags_*` operations use, as
/// the release's own "convenient wrappers around the dedicated `ITEM_TAG`
/// operations" requires.
///
/// Returns the stored list **per header**, because the two headers address
/// distinct targets ([`StoredItemTags`]); a header the request did not carry
/// leaves its target's tags untouched and echoes nothing. This server supports
/// `ITEM_TAGs`; a server that did not would ignore the headers (spec: "these
/// headers will also be unsupported").
///
/// # Errors
/// [`ApiError::Internal`] if the just-committed version uid does not parse as an
/// `OBJECT_VERSION_ID`, plus whatever the storage seam refuses.
pub(crate) async fn persist(
    state: &AppState,
    target: TagTarget,
    version_uid: &str,
    pending: PendingItemTags,
) -> Result<StoredItemTags, RestError> {
    let PendingItemTags {
        object: object_tags,
        version: version_tags,
    } = pending;
    // The two wrappers address DISTINCT collections: `openehr-item-tag`
    // replaces the VERSIONED_OBJECT container's tags, `openehr-version-item-tag`
    // the just-committed VERSION's own. An absent header leaves its collection
    // untouched and the two lists are never merged.
    let container_uid = container_uid(version_uid)?;
    let mut stored = StoredItemTags::default();
    if let Some(tags) = object_tags {
        stored.object = Some(write(state, target, container_uid, tags).await?);
    }
    if let Some(tags) = version_tags {
        stored.version = Some(write(state, target, version_uid.to_owned(), tags).await?);
    }
    Ok(stored)
}

/// The `VERSIONED_OBJECT` id the `openehr-item-tag` header addresses: the
/// `object_id` of the committed `OBJECT_VERSION_ID`, read through the BASE
/// accessor (`base_types` §Functions `object_id`), never a local `::` split.
///
/// # Errors
/// [`ApiError::Internal`]: the just-committed version uid is server-minted, so a
/// parse failure here is a server fault, never a client one.
fn container_uid(version_uid: &str) -> Result<String, RestError> {
    Ok(ObjectVersionId::new(version_uid)
        .map_err(|e| {
            crate::overview::error::internal_fault(
                "read the committed version uid",
                &format!("{version_uid:?}: {e}"),
            )
        })?
        .object_id()
        .value()
        .into_owned())
}

/// Replaces one target's `ITEM_TAG` list through the service seam its resource
/// family owns.
async fn write(
    state: &AppState,
    target: TagTarget,
    uid: String,
    tags: Vec<UpdateItemTag>,
) -> Result<Vec<ItemTag>, RestError> {
    Ok(match target {
        TagTarget::EhrContent {
            ehr_id,
            target_type,
        } => {
            state
                .backend()
                .target_tags_replace(ehr_id, uid, target_type, tags)
                .await?
        }
        TagTarget::Party(kind) => state.backend().party_tags_update(kind, uid, tags).await?,
    })
}

/// Echoes stored `ITEM_TAG` lists onto a response: "Servers MAY include the
/// `openehr-item-tag` or `openehr-version-item-tag` header in responses to
/// confirm the actual list of `ITEM_TAGs` stored on the server side"
/// (`Requests_and_responses.md` §Usage in Responses), which the same section
/// extends to `GET` ("the server MAY also add these headers to the response").
///
/// Each header carries its own target's collection and nothing else, so a
/// response never repeats one target's tags under the other target's name; a
/// collection the caller passes as `None` is not echoed at all, and an empty
/// list confirms a clear.
///
/// A list that cannot be rendered as an HTTP field value — a tag carrying a
/// control character, which nothing in the RM bars from a key — omits the header
/// entirely rather than emitting an empty one: an empty `openehr-item-tag` is
/// the release's "remove all `ITEM_TAGs`" instruction (§Usage in Requests), so
/// echoing one would tell a mirroring client to wipe the collection this
/// response just confirmed. The echo is a MAY, so declining is always available.
pub(crate) fn echo(resp: &mut Response, object: Option<&[ItemTag]>, version: Option<&[ItemTag]>) {
    for (name, tags) in [(H_ITEM_TAG, object), (H_VERSION_ITEM_TAG, version)] {
        let Some(tags) = tags else {
            continue;
        };
        let entries: Vec<ItemTagHeaderEntry> = tags.iter().map(item_tag_to_header_entry).collect();
        // The empty-collection guard lives in `emit_item_tag_header` itself —
        // one rule for every echo path.
        if let Some(value) = emit_item_tag_header(&entries) {
            resp.headers_mut()
                .insert(HeaderName::from_static(name), value);
        }
    }
}

/// Decodes the body of a dedicated `ITEM_TAG` write operation
/// (`{resource}_tags_update`), the operation family the wrapper headers wrap.
///
/// The decode is strict against `schemas/common/UpdateItemTag.yaml`
/// (`additionalProperties: false`, `key` required): an undeclared member or a
/// non-string `value`/`target_path` is a `400` naming the member by its JSON
/// path, never a silent drop.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not canonical
/// JSON; [`ApiError::BadRequest`] if the bytes are not a JSON array, or if any
/// element violates the declared schema.
pub(crate) fn write_body(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Vec<UpdateItemTag>, ApiError> {
    negotiate::typed_json_vec::<UpdateItemTag>(headers, body)
}
