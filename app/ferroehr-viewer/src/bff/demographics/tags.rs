// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic `ITEM_TAG` surface: the space-wide tag index the browser
//! screen lists, and the per-party tag editor on the party detail's Tags tab.
//!
//! Both are released operations. The index is
//! `GET /demographic/tags`, whose list "can be filtered by the given one or
//! more `tag_key`, `tag_value`, `tag_target_path` query parameters" and where
//! "in case no such parameter is provided then all `ITEM_TAG` resources will be
//! retrieved" (`operations/demographic_tags_get.yaml`). The editor is the
//! per-kind trio `{kind}_tags_get` / `{kind}_tags_update` /
//! `{kind}_tags_delete` (`operations/person_tags_*.yaml`).
//!
//! The row type, the wire codec, the merge and the editor panel are the shared
//! kit (`crate::components::item_tags`), which carries the tag model's own
//! rules — whole-collection replace, `(key, target_path)` identity, no
//! conditional header — so this module is only the demographic ROUTES.
//!
//! The addressed collection here is the version CONTAINER's: the released
//! operation makes the container and a single VERSION two DISJOINT collections
//! of the same object (`operations/person_tags_get.yaml`), and a container tag
//! survives the party's next version, which is what an operator marking a party
//! means.

use crate::bff::demographics::PartyKind;
use crate::bff::item_tags::ItemTagRow;
use crate::error::ViewerError;
use crate::locale::Msg;
use crate::locale::records::RecordsMsg;
use crate::locale::tr_served;

/// The demographic tag index (`GET /demographic/tags`), optionally filtered.
///
/// Each filter is sent only when non-empty
/// ([`tag_filter_query`](crate::bff::item_tags::tag_filter_query)),
/// which is exactly the released "omitted parameter constrains nothing"
/// behaviour (`operations/demographic_tags_get.yaml`). The list spans every
/// party kind and both target forms; a no-match answer is `200 []`, never a
/// `404`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Internal`] when the list is not valid JSON.
pub async fn list_demographic_tags(
    // Exact `ITEM_TAG.key` filter; empty constrains nothing.
    key: String,
    // Exact `ITEM_TAG.value` filter; empty constrains nothing.
    value: String,
    // Exact `ITEM_TAG.target_path` filter; empty constrains nothing.
    target_path: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let query = crate::bff::item_tags::tag_filter_query(&key, &value, &target_path);
    let url = cdr.rest_v1(&format!("demographic/tags{query}"));
    let response = cdr.get(&url).ok().await?;
    crate::bff::item_tags::parse_item_tags(&response.body)
}

/// One party's `ITEM_TAG`s
/// (`GET /demographic/{kind}/{uid_based_id}/tags`).
///
/// The addressed id is the version CONTAINER, so these are the
/// `VERSIONED_PARTY`'s own tags (module docs).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] on an unknown kind segment; a non-2xx CDR answer
/// (the `404` for an unknown id included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok); [`ViewerError::Internal`]
/// when the list is not valid JSON.
pub async fn fetch_party_tags(
    // The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    // The party's version container id.
    uid: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(RecordsMsg::PartyKind)))?;
    read_party_tags(&cdr, kind, &uid).await
}

/// Read one party's container tag collection.
///
/// The plain function behind [`fetch_party_tags`], so [`set_party_tag`]'s
/// read-modify-write calls THIS rather than another public endpoint (a server
/// fn is thin and the logic it shares lives in an ordinary function).
///
/// # Errors
/// CDR transport errors pass through; a non-2xx answer normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok);
/// [`ViewerError::Internal`] when the list is not valid JSON.
async fn read_party_tags(
    cdr: &crate::bff::Cdr,
    kind: PartyKind,
    uid: &str,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let url = tags_url(cdr, kind, uid);
    crate::bff::item_tags::parse_item_tags(&cdr.get(&url).ok().await?.body)
}

/// Set one tag on a party, keeping every other tag it already carries
/// (`PUT /demographic/{kind}/{uid_based_id}/tags`).
///
/// Read-modify-write, because the operation replaces the whole collection: the
/// current tags are read and
/// [`merged_tag_body`](crate::bff::item_tags::merged_tag_body) inserts
/// or replaces the requested `(key, target_path)` entry.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] on an unknown kind segment or a blank key; any
/// non-2xx CDR answer (the `422` for a tag that breaks an `ITEM_TAG` invariant
/// included) normalizes via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok);
/// [`ViewerError::Internal`] when the current list is not valid JSON.
pub async fn set_party_tag(
    // The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    // The party's version container id.
    uid: String,
    // The tag key to set.
    key: String,
    // The tag value; blank stores a tag with no value.
    value: String,
    // The tag's `target_path`; blank tags the whole object.
    target_path: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(RecordsMsg::PartyKind)))?;
    if key.trim().is_empty() {
        return Err(ViewerError::Invalid(
            { tr_served(Msg::TagKeyRequiredItemTagInvariant) }.clone(),
        ));
    }
    let existing = read_party_tags(&cdr, kind, &uid).await?;
    let body = crate::bff::item_tags::merged_tag_body(
        &existing,
        key.trim(),
        value.trim(),
        target_path.trim(),
    );
    let url = tags_url(&cdr, kind, &uid);
    // NOTE: clients are "strongly encouraged to always include the `Prefer`
    // request header explicitly" — ITS-REST
    // `specifications/docs/overview/Requests_and_responses.md` §Deprecated headers.
    cdr.put(&url)
        .body("application/json", body)
        .headers(&[("Prefer", "return=minimal")])
        .ok()
        .await?;
    Ok(())
}

/// Delete a party's tags carrying `key`
/// (`DELETE /demographic/{kind}/{uid_based_id}/tags/{key}`).
///
/// The released operation addresses a key, not a `(key, target_path)` pair, so
/// it removes every tag on the target that shares the key
/// (`operations/person_tags_delete.yaml`); an unknown key on an existing target
/// is its `404`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] on an unknown kind segment or a blank key.
pub async fn delete_party_tag(
    // The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    // The party's version container id.
    uid: String,
    // The tag key to remove.
    key: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(RecordsMsg::PartyKind)))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::TagKeyRequired)));
    }
    let url = format!(
        "{}/{}",
        tags_url(&cdr, kind, &uid),
        urlencoding::encode(key)
    );
    cdr.delete(&url).ok().await?;
    Ok(())
}

/// The `/demographic/{kind}/{uid_based_id}/tags` URL of a party's container tag
/// collection.
fn tags_url(cdr: &crate::cdr::CdrClient, kind: PartyKind, uid: &str) -> String {
    cdr.rest_v1(&format!(
        "demographic/{}/{}/tags",
        kind.segment(),
        urlencoding::encode(&crate::uid::container_uid_of(uid))
    ))
}
