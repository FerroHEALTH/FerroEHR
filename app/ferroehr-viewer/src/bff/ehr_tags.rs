// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-side `ITEM_TAG` surfaces: the per-target tag panels the composition
//! viewer and the EHR-status tab mount, and the EHR-wide tag browser.
//!
//! Seven released operations, all in the EHR API
//! (`docs/specs/openehr/ITS-REST/specifications/ehr.openapi.yaml`): the
//! aggregate `ehr_tags_get`, and the `composition_tags_*` /
//! `ehr_status_tags_*` get-update-delete trios. There is no directory tag
//! route — a FOLDER's tags are written with the `openehr-item-tag` commit
//! headers instead (`specifications/docs/overview/Requests_and_responses.md`
//! §`openehr-item-tag and openehr-version-item-tag`) — but the aggregate list
//! still reports them, so the browser resolves and links a directory target
//! without offering to edit it.
//!
//! Four wire facts this module is built around, each verified against the
//! released operation files:
//!
//! 1. **No tag route is conditional.** The three write operations declare
//!    `Prefer` and the two content headers and NOTHING else — no `If-Match` —
//!    and a tag collection carries neither `ETag` nor `Last-Modified`, which
//!    `Requests_and_responses.md` §"`ETag` and `Last-Modified`" reserves for
//!    resources "that have versioning or unique state identifiers". A tag write
//!    is last-writer-wins and commits no CONTRIBUTION; every panel says so.
//! 2. **The container form and the VERSION form address DISJOINT collections.**
//!    A `uid_based_id` is "an `OBJECT_VERSION_ID` … used to get the tags of a
//!    particular (target) version … whereas the latter … is be used to get the
//!    tags of the target `VERSIONED_COMPOSITION` container"
//!    (`operations/composition_tags_get.yaml`), and an `ITEM_TAG` has exactly
//!    one `target` (RM
//!    `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`),
//!    so neither collection is a view of the other.
//! 3. **An aggregate tag row names its target but not its target's KIND.** The
//!    RM types `target` as a bare `UID_BASED_ID`, which carries no `type`
//!    member, so opening a row asks the CDR which object holds that id
//!    ([`resolve_ehr_target`]) exactly as the demographic tag index does.
//! 4. **`GET /ehr/{ehr_id}/tags` filters, and an empty answer is `200 []`** —
//!    "This will return an empty list when there is no matching `ITEM_TAG`"
//!    (`operations/ehr_tags_get.yaml`); a `404` means the EHR itself is unknown.
//!
//! The row type, the wire codec, the merge, the editor panel and the filter
//! form are the shared kit (`crate::components::item_tags`) — the same one
//! the demographic tag surfaces use.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! Every `#[server]` fn below guards with
//! [`require_session`](crate::session::require_session) first, the CDR
//! credential never reaches client-visible state, and every path segment is
//! percent-encoded server-side.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::bff::item_tags::ItemTagRow;
use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
use crate::locale::{Msg, tr, tr_with};
use crate::uid::container_uid_of;

/// Which EHR-side object an `ITEM_TAG` target turned out to be.
///
/// The three taggable kinds inside an EHR. Two of them —
/// [`Composition`](Self::Composition) and [`EhrStatus`](Self::EhrStatus) — have
/// the released tag route trios; [`Directory`](Self::Directory) has none (fact
/// 1 in the module docs), so it is a navigation answer only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EhrTargetKind {
    /// A COMPOSITION or its `VERSIONED_COMPOSITION`.
    Composition,
    /// An `EHR_STATUS` or its `VERSIONED_EHR_STATUS`.
    EhrStatus,
    /// The EHR's FOLDER directory.
    Directory,
}

impl EhrTargetKind {
    /// This kind's own route segment (`composition`, `ehr_status`,
    /// `directory`) — the viewer's wire key for the kind.
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            Self::Composition => "composition",
            Self::EhrStatus => "ehr_status",
            Self::Directory => "directory",
        }
    }

    /// The kind a segment names, or `None` for anything else.
    ///
    /// Server functions are a public HTTP API, so a segment a caller hands one
    /// is validated back into this closed set rather than interpolated into a
    /// CDR path.
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        match segment {
            "composition" => Some(Self::Composition),
            "ehr_status" => Some(Self::EhrStatus),
            "directory" => Some(Self::Directory),
            _ => None,
        }
    }

    /// The path segment this kind's tag routes live under, or `None` when the
    /// release publishes none for it (the directory — module docs).
    #[must_use]
    pub fn tags_route(self) -> Option<&'static str> {
        match self {
            Self::Composition => Some("composition"),
            Self::EhrStatus => Some("ehr_status"),
            Self::Directory => None,
        }
    }

    /// The viewer screen that owns an object of this kind.
    ///
    /// A composition has its own route; the status and the directory are tabs
    /// of the EHR detail screen. Every id is percent-encoded.
    #[must_use]
    pub fn href(self, ehr_id: &str, container: &str) -> String {
        let ehr = urlencoding::encode(ehr_id);
        match self {
            Self::Composition => format!(
                "/ehrs/{ehr}/compositions/{}",
                urlencoding::encode(container)
            ),
            Self::EhrStatus => format!("/ehrs/{ehr}/status"),
            Self::Directory => format!("/ehrs/{ehr}/directory"),
        }
    }
}

/// The tag collection of one COMPOSITION or `EHR_STATUS` target
/// (`GET /ehr/{ehr_id}/{kind}/{uid_based_id}/tags`).
///
/// `uid` addresses either the version container or one VERSION, and the two are
/// disjoint collections (module docs fact 2) — the caller decides which by what
/// it passes. An existing, untagged target answers `200 []`; a target that does
/// not exist is a `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a kind that has no tag routes; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown
/// `ehr_id`/`uid_based_id` included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_ehr_tags(
    /// The EHR that owns the tagged object.
    ehr_id: String,
    /// The target kind, as its segment ([`EhrTargetKind::segment`]).
    kind: String,
    /// The addressed `uid_based_id` — a container id or one `OBJECT_VERSION_ID`.
    uid: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = tags_url(&state, &ehr_id, &kind, &uid)?;
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::bff::item_tags::parse_item_tags(&response.body)
}

/// Set one tag on a COMPOSITION or `EHR_STATUS` target, keeping every other tag
/// that target already carries
/// (`PUT /ehr/{ehr_id}/{kind}/{uid_based_id}/tags`).
///
/// Read-modify-write, because the operation replaces the whole collection —
/// "Providing an empty list will effectively remove all `ITEM_TAG` associated
/// with the given target" (`operations/composition_tags_update.yaml`) — so
/// [`merged_tag_body`](crate::bff::item_tags::merged_tag_body) inserts
/// or replaces the requested `(key, target_path)` entry into the current list.
/// No `If-Match` is sent because the operation declares none (module docs fact
/// 1).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a kind that has no tag routes or a blank key;
/// CDR transport errors pass through; any non-2xx CDR answer (the `422` for a
/// tag that breaks an `ITEM_TAG` invariant included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the current list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn set_ehr_tag(
    /// The EHR that owns the tagged object.
    ehr_id: String,
    /// The target kind, as its segment ([`EhrTargetKind::segment`]).
    kind: String,
    /// The addressed `uid_based_id`.
    uid: String,
    /// The tag key to set.
    key: String,
    /// The tag value; blank stores a tag with no value.
    value: String,
    /// The tag's `target_path`; blank tags the whole object.
    target_path: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = tags_url(&state, &ehr_id, &kind, &uid)?;
    set_tag_at(
        &state,
        &session.credential,
        &url,
        &key,
        &value,
        &target_path,
    )
    .await
}

#[cfg(not(target_arch = "wasm32"))]
/// Insert-or-replace one tag in the collection at `url`.
///
/// The plain function behind [`set_ehr_tag`] and [`set_status_tag`], so neither
/// server function calls the other (a server fn is thin and the logic it shares
/// lives in an ordinary function).
///
/// # Errors
/// [`ViewerError::Invalid`] on a blank key; CDR transport errors pass through;
/// any non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the current list is not valid JSON.
async fn set_tag_at(
    state: &crate::state::AppState,
    credential: &crate::session::Credential,
    url: &str,
    key: &str,
    value: &str,
    target_path: &str,
) -> Result<(), ViewerError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ViewerError::Invalid(
            { tr(Msg::TagKeyRequiredItemTagInvariant) }.clone(),
        ));
    }
    let response = state.cdr.get(credential, url, "application/json").await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let existing = crate::bff::item_tags::parse_item_tags(&response.body)?;
    let body =
        crate::bff::item_tags::merged_tag_body(&existing, key, value.trim(), target_path.trim());
    // NOTE: clients are "strongly encouraged to always include the `Prefer`
    // request header explicitly" — ITS-REST
    // `specifications/docs/overview/Requests_and_responses.md` §Deprecated headers.
    let response = state
        .cdr
        .put(
            credential,
            url,
            "application/json",
            "application/json",
            &[("Prefer", "return=minimal")],
            body,
        )
        .await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

/// Delete a target's tags carrying `key`
/// (`DELETE /ehr/{ehr_id}/{kind}/{uid_based_id}/tags/{key}`).
///
/// A SET delete: identity is the `(key, target_path)` pair and the route has no
/// `target_path` selector, so every tag under the key on the addressed
/// collection goes (`operations/composition_tags_delete.yaml`, which deletes
/// "the `ITEM_TAG` resource(s) identified by `tag_key`"). A key that is not on
/// the addressed collection is its `404` — including a key that exists only on
/// the OTHER collection of the same versioned object.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a kind that has no tag routes or a blank key;
/// CDR transport errors pass through; any non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_ehr_tag(
    /// The EHR that owns the tagged object.
    ehr_id: String,
    /// The target kind, as its segment ([`EhrTargetKind::segment`]).
    kind: String,
    /// The addressed `uid_based_id`.
    uid: String,
    /// The tag key to remove.
    key: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = tags_url(&state, &ehr_id, &kind, &uid)?;
    delete_tag_at(&state, &session.credential, &url, &key).await
}

#[cfg(not(target_arch = "wasm32"))]
/// Delete every tag under `key` from the collection at `url`.
///
/// The plain function behind [`delete_ehr_tag`] and
/// [`delete_status_tag`].
///
/// # Errors
/// [`ViewerError::Invalid`] on a blank key; CDR transport errors pass through;
/// any non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
async fn delete_tag_at(
    state: &crate::state::AppState,
    credential: &crate::session::Credential,
    url: &str,
    key: &str,
) -> Result<(), ViewerError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ViewerError::Invalid(tr(Msg::TagKeyRequired)));
    }
    let url = format!("{url}/{}", urlencoding::encode(key));
    let response = state.cdr.delete(credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

/// Every `ITEM_TAG` in one EHR (`GET /ehr/{ehr_id}/tags`), optionally filtered.
///
/// The aggregate read: it "retrieves the list of `ITEM_TAG` resources
/// associated with any target VERSION or `VERSIONED_OBJECT` within the EHR",
/// so one list spans both target forms and every taggable kind. Each filter is
/// sent only when non-empty
/// ([`tag_filter_query`](crate::bff::item_tags::tag_filter_query)) —
/// the released "omitted parameter constrains nothing" behaviour.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown
/// `ehr_id`) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_ehr_tags(
    /// The EHR whose tags to list.
    ehr_id: String,
    /// Exact `ITEM_TAG.key` filter; empty constrains nothing.
    key: String,
    /// Exact `ITEM_TAG.value` filter; empty constrains nothing.
    value: String,
    /// Exact `ITEM_TAG.target_path` filter; empty constrains nothing.
    target_path: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let query = crate::bff::item_tags::tag_filter_query(&key, &value, &target_path);
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/tags{query}", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::bff::item_tags::parse_item_tags(&response.body)
}

/// Which object inside `ehr_id` holds `uid`, or `None` when none does.
///
/// An aggregate tag row reports its target as a bare `UID_BASED_ID` with no RM
/// type attached (module docs fact 3), so the only honest way to open a row is
/// to ask the CDR.
///
/// Three steps, and the ORDER is load-bearing: the two IDENTITY comparisons run
/// first — the EHR resource's own `ehr_status` reference, then the directory's
/// `uid` — because each names its object exactly. Only then is the COMPOSITION
/// resource read, the one step that infers a kind from a route answering at
/// all, since inference would otherwise claim every id that route happens to
/// serve. Any answer other than `404` from that read is raised rather than read
/// as "not a composition".
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when `uid` is empty; CDR transport errors pass
/// through; a refusal or any non-`404` failure normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn resolve_ehr_target(
    /// The EHR that owns the tagged object.
    ehr_id: String,
    /// The tag's target, in either `uid_based_id` form.
    uid: String,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let container = container_uid_of(&uid);
    if container.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::TagTargetIdRequired)));
    }
    if ehr_status_container(&state, &session.credential, &ehr_id).await? == container {
        return Ok(Some(EhrTargetKind::EhrStatus.segment().to_owned()));
    }
    let ehr = urlencoding::encode(&ehr_id).into_owned();
    let directory = state.cdr.rest_v1(&format!("ehr/{ehr}/directory"));
    let response = state
        .cdr
        .get(&session.credential, &directory, "application/json")
        .await?;
    if response.is(http::StatusCode::OK)
        && container_uid_of(&crate::uid::uid_value_of(&response.body)) == container
    {
        return Ok(Some(EhrTargetKind::Directory.segment().to_owned()));
    }
    let composition = state.cdr.rest_v1(&format!(
        "ehr/{ehr}/composition/{}",
        urlencoding::encode(&container)
    ));
    let response = state
        .cdr
        .get(&session.credential, &composition, "application/json")
        .await?;
    if response.is(http::StatusCode::OK) || response.is(http::StatusCode::NO_CONTENT) {
        return Ok(Some(EhrTargetKind::Composition.segment().to_owned()));
    }
    if !response.is(http::StatusCode::NOT_FOUND) {
        drop(crate::cdr::CdrClient::expect_success(response)?);
    }
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
/// The `/ehr/{ehr_id}/{kind}/{uid_based_id}/tags` URL of one target's tag
/// collection.
///
/// # Errors
/// [`ViewerError::Invalid`] when the segment is outside the closed kind set or
/// names a kind the release publishes no tag route for (the directory).
fn tags_url(
    state: &crate::state::AppState,
    ehr_id: &str,
    kind: &str,
    uid: &str,
) -> Result<String, ViewerError> {
    let route = EhrTargetKind::from_segment(kind)
        .and_then(EhrTargetKind::tags_route)
        .ok_or_else(|| {
            ViewerError::Invalid(format!(
                "{kind:?} is not an EHR object the openEHR tag operations address"
            ))
        })?;
    Ok(state.cdr.rest_v1(&format!(
        "ehr/{}/{route}/{}/tags",
        urlencoding::encode(ehr_id),
        urlencoding::encode(uid)
    )))
}

#[cfg(not(target_arch = "wasm32"))]
/// The `VERSIONED_EHR_STATUS` container of an EHR, read from the EHR resource's
/// own `ehr_status` reference (`GET /ehr/{ehr_id}`).
///
/// The `EHR_STATUS` tag routes take a `uid_based_id` and there is no
/// status-tags route keyed on the `ehr_id` alone, so the container has to be
/// resolved before the collection can be addressed. `EHR.ehr_status` is an
/// `OBJECT_REF` whose `id` addresses the CURRENT `EHR_STATUS` version (RM
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr.adoc`), so
/// its container part is the versioned object the panel edits.
///
/// This is a second WINDOW of the endpoint the EHR-detail header already reads,
/// for an identifier rather than for a fact the screen shows.
///
/// # Errors
/// CDR transport errors pass through; a non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
async fn ehr_status_container(
    state: &crate::state::AppState,
    credential: &crate::session::Credential,
    ehr_id: &str,
) -> Result<String, ViewerError> {
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}", urlencoding::encode(ehr_id)));
    let response = state.cdr.get(credential, &url, "application/json").await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let doc: serde_json::Value = serde_json::from_str(&response.body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::EhrJson2, &[("e", (e).to_string().into())]))
    })?;
    Ok(container_uid_of(
        doc.get("ehr_status")
            .and_then(|status| status.get("id"))
            .and_then(|id| id.get("value"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    ))
}

/// The `EHR_STATUS`'s own tag collection
/// (`GET /ehr/{ehr_id}/ehr_status/{versioned_object_uid}/tags`), addressed by
/// the container the EHR itself names.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when a body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_status_tags(
    /// The EHR whose status tags to read.
    ehr_id: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = status_tags_url(&state, &session.credential, &ehr_id).await?;
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::bff::item_tags::parse_item_tags(&response.body)
}

#[cfg(not(target_arch = "wasm32"))]
/// The tag-collection URL of an EHR's `VERSIONED_EHR_STATUS` container.
///
/// # Errors
/// As [`ehr_status_container`]; the kind is this module's own constant, so the
/// [`tags_url`] refusal is unreachable here.
async fn status_tags_url(
    state: &crate::state::AppState,
    credential: &crate::session::Credential,
    ehr_id: &str,
) -> Result<String, ViewerError> {
    let container = ehr_status_container(state, credential, ehr_id).await?;
    tags_url(
        state,
        ehr_id,
        EhrTargetKind::EhrStatus.segment(),
        &container,
    )
}

/// Set one tag on the EHR's `VERSIONED_EHR_STATUS` container, resolving the
/// container from the EHR itself first.
///
/// # Errors
/// As [`set_ehr_tag`], plus the EHR read's own failures.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn set_status_tag(
    /// The EHR whose status to tag.
    ehr_id: String,
    /// The tag key to set.
    key: String,
    /// The tag value; blank stores a tag with no value.
    value: String,
    /// The tag's `target_path`; blank tags the whole object.
    target_path: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = status_tags_url(&state, &session.credential, &ehr_id).await?;
    set_tag_at(
        &state,
        &session.credential,
        &url,
        &key,
        &value,
        &target_path,
    )
    .await
}

/// Delete the EHR status container's tags carrying `key`, resolving the
/// container from the EHR itself first.
///
/// # Errors
/// As [`delete_ehr_tag`], plus the EHR read's own failures.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_status_tag(
    /// The EHR whose status tag to remove.
    ehr_id: String,
    /// The tag key to remove.
    key: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = status_tags_url(&state, &session.credential, &ehr_id).await?;
    delete_tag_at(&state, &session.credential, &url, &key).await
}

#[cfg(test)]
mod tests {
    use super::EhrTargetKind;

    #[test]
    fn every_kind_round_trips_through_its_segment() {
        for kind in [
            EhrTargetKind::Composition,
            EhrTargetKind::EhrStatus,
            EhrTargetKind::Directory,
        ] {
            assert_eq!(EhrTargetKind::from_segment(kind.segment()), Some(kind));
        }
        // …and nothing else does, so no caller-supplied string can steer a CDR
        // path (a server function is a public HTTP endpoint).
        for hostile in ["", "..", "ehr", "versioned_composition", "tags", "/"] {
            assert_eq!(EhrTargetKind::from_segment(hostile), None, "{hostile}");
        }
    }

    #[test]
    fn only_the_two_kinds_with_released_tag_routes_are_addressable() {
        // The release publishes composition_tags_* and ehr_status_tags_* and no
        // directory tag route at all (ehr.openapi.yaml).
        assert_eq!(EhrTargetKind::Composition.tags_route(), Some("composition"));
        assert_eq!(EhrTargetKind::EhrStatus.tags_route(), Some("ehr_status"));
        assert_eq!(EhrTargetKind::Directory.tags_route(), None);
    }

    #[test]
    fn a_targets_href_opens_the_screen_that_owns_it_and_encodes_every_id() {
        assert_eq!(
            EhrTargetKind::Composition.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/compositions/8849182c"
        );
        assert_eq!(
            EhrTargetKind::EhrStatus.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/status"
        );
        assert_eq!(
            EhrTargetKind::Directory.href("7d44b88c", "8849182c"),
            "/ehrs/7d44b88c/directory"
        );
        // A hostile id can never break out of its path segment.
        assert_eq!(
            EhrTargetKind::Composition.href("a/b", "c?d#e"),
            "/ehrs/a%2Fb/compositions/c%3Fd%23e"
        );
    }
}
