// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the ITS-REST **demographic API** (Release-1.1.0,
//! DEVELOPMENT status).
//!
//! The wire contract is the vendored ITS-REST Demographic API
//! (`specifications/demographic.openapi.yaml` and its per-operation contracts
//! under `.../operations/`), from which the route table and `*Params` structs
//! are generated into `openehr_its::rest::generated::demographic`. This module
//! implements that contract over the native API: a spec-defined,
//! development-maturity wire rather than an extension by analogy with the EHR
//! group, so where the two response envelopes coincide the shape is justified
//! from the demographic contract.
//!
//! The only genuinely spec-absent surface is `PARTY_RELATIONSHIP` (see
//! `relationship`), which the vendored API does not define: those routes are our
//! own extension and are excluded from
//! conformance-profile claims.
//!
//! The five per-kind operation families collapse into one handler per operation
//! that takes its `PartyKind` from the route, the generated per-kind
//! `*Params` structs being field-identical. The file layout mirrors the spec
//! resources: `party`, `tags`, `versioned_party`, `contribution` and
//! `relationship`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::HeaderMap;

use openehr_its::rest::runtime::ApiError;

use crate::overview::error::RestError;
use crate::overview::negotiate::resource::{Tree, set_versioning_headers};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::response::{ResourceMeta, ServiceResponse};
use ferroehr::versioning::object_version_id::parse_uid_based_id;

mod contribution;
pub(crate) mod openapi_routes;
mod party;
pub(crate) mod relationship;
mod tags;
mod versioned_party;
pub(crate) mod versioned_relationship;

// `app/ferroehr-rest/src/api/mod.rs` mounts the generated-contract routes of
// `openapi_routes` and the native `utoipa-axum` router for the own-design
// PARTY_RELATIONSHIP extension (`demographic::relationship_routes`); each route
// calls its operation's handler directly.

/// The versioned object a demographic `uid_based_id` or
/// `versioned_object_uid` names, a `400` when malformed.
///
/// # Errors
/// The `400` of [`parse_uid_based_id`].
fn object_of(raw: &str) -> Result<VoId, RestError> {
    Ok(parse_uid_based_id(raw)?.vo_id)
}

/// The target of a demographic update and its mandatory `If-Match`, whose
/// `W/"…"` or quoted entity-tag syntax is decoded here so the precondition
/// holds a bare `OBJECT_VERSION_ID` (overview §"`ETag` and Last-Modified").
///
/// # Errors
/// A `400` when `raw` or the `If-Match` value is malformed.
fn update_target(raw: &str, if_match: &str) -> Result<(VoId, Precondition), RestError> {
    let vo_id = object_of(raw)?;
    let token = crate::overview::version_id::strip_etag(if_match);
    Ok((vo_id, Precondition::parse(Some(token), None)?))
}

/// The precondition of a demographic delete: the path names the version it
/// deletes, and `If-Match`, when sent, names the version expected current.
///
/// # Errors
/// A `400` when `raw` is malformed, names the versioned object rather than
/// one of its versions, or a sent `If-Match` is malformed.
fn delete_target(
    raw: &str,
    rm_type: &str,
    if_match: Option<&str>,
) -> Result<(VoId, Precondition), RestError> {
    let address = parse_uid_based_id(raw)?;
    // NOTE: ITS-REST `parameters/path/uid_based_id_as_version_uid.yaml` types the
    // delete path as a `version_uid`, so a bare container id is an invalid parameter (400).
    let Some(path) = address.tree() else {
        return Err(RestError::from(ApiError::BadRequest(format!(
            "uid_based_id {raw} must be the OBJECT_VERSION_ID of the latest {rm_type} \
             version, not its versioned object id"
        ))));
    };
    Ok((address.vo_id, Precondition::parse(if_match, Some(path))?))
}

/// Whether an error is the optimistic-concurrency precondition failure
/// (`If-Match` mismatch → `412` on update; a stale `uid_based_id` → `409` on
/// delete, per `person_delete.yaml` `409_PERSON_with_uid_based_id`).
fn is_precondition(e: &ServiceError) -> bool {
    matches!(e, ServiceError::VersionConflict(_))
}

/// The versioning metadata of a demographic versioned-object read whose service
/// response carries none (the `VERSIONED_PARTY` container, its
/// `REVISION_HISTORY`, and a version-addressed `ORIGINAL_VERSION`), derived from
/// the served body.
///
/// `ETag` source — overview §"`ETag` and Last-Modified": the value "is usually
/// taken from e.g. `VERSIONED_OBJECT.uid.value`, `VERSION.uid.value`". The
/// body's own `uid.value` is exactly one of those two for the container and the
/// version reads; a `REVISION_HISTORY` carries no `uid` of its own, so the
/// addressed `versioned_object_uid` (the container the history belongs to) is
/// the fallback.
///
/// `Last-Modified` source — same section: "derived from
/// `VERSION.commit_audit.time_committed.value`". That is the `ORIGINAL_VERSION`'s
/// own `commit_audit` for a version read, and the most recent revision-history
/// item's first audit for a history read (`REVISION_HISTORY.items` is
/// most-recent-last, RM common `master04-generic_package.adoc` §Revision
/// History + `UML/classes/org.openehr.rm.common.revision_history.adoc` §items
/// — "the items in this history in most-recent-last order", with
/// `most_recent_version_time_committed`'s postcondition
/// `items.last.audits.first.time_committed.value`). A `VERSIONED_OBJECT` container body exposes no
/// commit audit — which is why the container READ does not use this seam:
/// its `Last-Modified` rides the service metadata (the version spine's
/// newest commit instant, overview §"`ETag` and Last-Modified").
fn read_meta(versioned_object_uid: &str, body: &serde_json::Value) -> ResourceMeta {
    let uid = body
        .get("uid")
        .and_then(|u| u.get("value"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(versioned_object_uid);
    // Parties are not EHR-scoped, so the demographic `ehr_id` is empty.
    let meta = ResourceMeta::new(String::new(), uid.to_owned());
    match commit_time(body) {
        Some(at) => meta.with_last_modified(at),
        None => meta,
    }
}

/// The commit instant a served demographic body exposes: an `ORIGINAL_VERSION`'s
/// `commit_audit.time_committed`, else a `REVISION_HISTORY`'s most recent item's
/// first audit (see [`read_meta`] for the citations).
///
/// NOTE: the history branch mirrors the RM spec function
/// `REVISION_HISTORY.most_recent_version_time_committed` (`RM/docs/UML/classes/org.openehr.rm.common.revision_history.adoc`
/// §Functions — `Post: Result.is_equal (items.last.audits.first.time_committed.value)`),
/// read off the already-serialized body for that one leaf rather than through
/// its realization
/// [`openehr_rm::v1_2::common::generic::revision_history::RevisionHistory::most_recent_version_time_committed`],
/// which stays the authority for the value.
fn commit_time(body: &serde_json::Value) -> Option<jiff::Timestamp> {
    let raw = body
        .get("commit_audit")
        .and_then(|a| a.get("time_committed"))
        .and_then(|t| t.get("value"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            body.get("items")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.last())
                .and_then(|item| item.get("audits"))
                .and_then(|audits| audits.get(0))
                .and_then(|audit| audit.get("time_committed"))
                .and_then(|t| t.get("value"))
                .and_then(serde_json::Value::as_str)
        })?;
    raw.parse::<jiff::Timestamp>().ok()
}

/// Renders a demographic read whose versioning headers ride the service's
/// response metadata (overview §"`ETag` and Last-Modified"), with no
/// `Location` (§Location restricts it to creation and redirect responses).
fn respond_versioned(h: &HeaderMap, resp: &ServiceResponse) -> Response {
    let mut out = crate::overview::negotiate::body::respond(h, http::StatusCode::OK, &resp.body);
    if let Some(meta) = resp.meta.as_ref() {
        set_versioning_headers(&mut out, meta, Tree::Demographic);
    }
    out
}

/// Render a demographic read that the service returns without metadata, with
/// the versioning headers derived from the served body ([`read_meta`]).
fn read_versioned(h: &HeaderMap, versioned_object_uid: &str, body: &serde_json::Value) -> Response {
    let mut out = crate::overview::negotiate::body::respond(h, http::StatusCode::OK, body);
    set_versioning_headers(
        &mut out,
        &read_meta(versioned_object_uid, body),
        Tree::Demographic,
    );
    out
}

/// Emits the `openehr-item-tag` / `openehr-version-item-tag` **response**
/// headers mandated by `responses/201_PERSON.yaml` (create) and `person_get.yaml`
/// (get) when a party carries `ITEM_TAGs`.
///
/// A party's stored tags ride the response metadata seam
/// ([`ResourceMeta::item_tags`] for the `VERSIONED_OBJECT` container,
/// [`ResourceMeta::version_item_tags`] for the served VERSION), so this is the
/// group's adapter onto [`crate::api::item_tags::echo`], where the rendering and
/// its spec grounds live once for every group.
fn set_item_tag_headers(resp_out: &mut Response, resp: &ServiceResponse) {
    let Some(meta) = resp.meta.as_ref() else {
        return;
    };
    crate::api::item_tags::echo(
        resp_out,
        meta.item_tags.as_deref(),
        meta.version_item_tags.as_deref(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::negotiate::resource::{error_with_meta, set_resource_headers};
    use http::{HeaderValue, header};

    const BASE: &str = "/ferroehr/rest/openehr/v1";
    const VO: &str = "8849182c-82ad-4088-a07f-48ead4180515";
    const UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";

    fn meta() -> ResourceMeta {
        ResourceMeta::new(String::new(), UID.to_owned())
    }

    fn etag(resp: &Response) -> Option<&str> {
        resp.headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
    }

    fn location(resp: &Response) -> Option<&str> {
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
    }

    fn last_modified(resp: &Response) -> Option<&str> {
        resp.headers()
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
    }

    fn if_match_headers(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("if-match", HeaderValue::from_str(value).unwrap());
        h
    }

    /// Every non-creation demographic response carries the weak `ETag` and NO
    /// `Location` — overview `Requests_and_responses.md` §Location: the header
    /// "MUST NOT be used to indicate an alternate representation of an existing
    /// resource (e.g. via `GET` method)" and "MUST ONLY be used for resource
    /// creation … or redirect responses".
    #[test]
    fn versioning_headers_carry_no_location() {
        let mut resp = crate::overview::negotiate::body::empty(http::StatusCode::OK);
        set_versioning_headers(&mut resp, &meta(), Tree::Demographic);
        assert_eq!(etag(&resp), Some(format!("W/\"{UID}\"").as_str()));
        assert!(
            location(&resp).is_none(),
            "ITS-REST overview §Location: no Location on a non-creation response"
        );
    }

    /// The commit instant rides `Last-Modified` when the metadata carries one
    /// (overview §"`ETag` and Last-Modified": "derived from
    /// `VERSION.commit_audit.time_committed.value`").
    #[test]
    fn versioning_headers_emit_last_modified_when_known() {
        let at: jiff::Timestamp = "2009-07-22T19:15:56Z".parse().unwrap();
        let mut resp = crate::overview::negotiate::body::empty(http::StatusCode::OK);
        set_versioning_headers(&mut resp, &meta().with_last_modified(at), Tree::Demographic);
        assert_eq!(last_modified(&resp), Some("Wed, 22 Jul 2009 19:15:56 GMT"));

        let mut bare = crate::overview::negotiate::body::empty(http::StatusCode::OK);
        set_versioning_headers(&mut bare, &meta(), Tree::Demographic);
        assert!(last_modified(&bare).is_none());
    }

    /// A create/update write DOES carry `Location` (overview §Location:
    /// creation responses; §"Prefer minimal…": "the newly created or updated
    /// resource").
    #[test]
    fn write_headers_add_the_creation_location() {
        let mut resp = crate::overview::negotiate::body::empty(http::StatusCode::CREATED);
        set_resource_headers(&mut resp, BASE, Tree::Demographic, Some("person"), &meta());
        assert_eq!(etag(&resp), Some(format!("W/\"{UID}\"").as_str()));
        assert_eq!(
            location(&resp),
            Some(format!("{BASE}/demographic/person/{UID}").as_str())
        );
    }

    /// A `412`/`409` echoes the latest version in `ETag` only — overview
    /// §"If-Match and accidental overwrites" asks for the latest `version_uid`
    /// in `ETag`; §Location forbids `Location` on a non-creation response.
    #[test]
    fn error_response_echoes_etag_without_location() {
        let resp = error_with_meta(
            ApiError::PreconditionFailed("stale".to_owned()),
            Some(&meta()),
            Tree::Demographic,
        );
        assert_eq!(resp.status(), http::StatusCode::PRECONDITION_FAILED);
        assert_eq!(etag(&resp), Some(format!("W/\"{UID}\"").as_str()));
        assert!(
            location(&resp).is_none(),
            "ITS-REST overview §Location: no Location on an error response"
        );
    }

    /// `If-Match` is accepted in the weak `W/"…"` form the server itself emits,
    /// in the deprecated bare quoted form, and unquoted — all naming the same
    /// `OBJECT_VERSION_ID` (overview §"`ETag` and Last-Modified" +
    /// §"Deprecated headers").
    #[test]
    fn if_match_normalizes_weak_quoted_and_bare_forms() {
        for raw in [
            format!("W/\"{UID}\""),
            format!("w/\"{UID}\""),
            format!("\"{UID}\""),
            UID.to_owned(),
            format!("  W/\"{UID}\"  "),
        ] {
            assert_eq!(
                crate::overview::version_id::if_match(&if_match_headers(&raw)).as_deref(),
                Some(UID),
                "If-Match {raw:?} must reduce to the bare OBJECT_VERSION_ID"
            );
        }
        assert_eq!(
            crate::overview::version_id::if_match(&HeaderMap::new()),
            None
        );
        // A present-but-empty precondition is NOT "absent": it reaches the
        // service, which rejects it (overview §If-Match: a received header MUST
        // be honoured).
        assert_eq!(
            crate::overview::version_id::if_match(&if_match_headers("\"\"")).as_deref(),
            Some(""),
            "an empty If-Match must not read as an absent precondition"
        );
    }

    /// A version read's `ETag` is `VERSION.uid.value` and its `Last-Modified`
    /// the version's own `commit_audit.time_committed` (overview §"`ETag` and
    /// Last-Modified").
    #[test]
    fn read_meta_of_an_original_version() {
        let body = serde_json::json!({
            "_type": "ORIGINAL_VERSION",
            "uid": { "_type": "OBJECT_VERSION_ID", "value": UID },
            "commit_audit": {
                "_type": "AUDIT_DETAILS",
                "time_committed": { "_type": "DV_DATE_TIME", "value": "2024-03-04T05:06:07Z" }
            }
        });
        let m = read_meta(VO, &body);
        assert_eq!(m.uid, UID);
        assert_eq!(
            m.last_modified,
            Some("2024-03-04T05:06:07Z".parse::<jiff::Timestamp>().unwrap())
        );
    }

    /// A `REVISION_HISTORY` carries no `uid`, so the `ETag` falls back to the
    /// addressed `VERSIONED_OBJECT.uid.value`; `Last-Modified` comes from the
    /// most recent item (`REVISION_HISTORY.items` is most-recent-last, RM common
    /// `master04-generic_package.adoc` §Revision History +
    /// `UML/classes/org.openehr.rm.common.revision_history.adoc` §items).
    #[test]
    fn read_meta_of_a_revision_history() {
        let body = serde_json::json!({
            "_type": "REVISION_HISTORY",
            "items": [
                { "version_id": { "value": format!("{VO}::s::1") },
                  "audits": [{ "time_committed": { "value": "2024-01-01T00:00:00Z" } }] },
                { "version_id": { "value": format!("{VO}::s::2") },
                  "audits": [{ "time_committed": { "value": "2024-03-04T05:06:07Z" } }] }
            ]
        });
        let m = read_meta(VO, &body);
        assert_eq!(m.uid, VO);
        assert_eq!(
            m.last_modified,
            Some("2024-03-04T05:06:07Z".parse::<jiff::Timestamp>().unwrap())
        );
    }

    /// A `VERSIONED_OBJECT` container read takes its `ETag` from
    /// `VERSIONED_OBJECT.uid.value` and exposes no commit audit, so it carries
    /// no `Last-Modified` (the header is a SHOULD).
    #[test]
    fn read_meta_of_a_versioned_object_container() {
        // The body-scrape fallback finds no commit audit in a container body
        // — which is exactly why the container READ no longer uses this
        // seam: versioned_party_get carries Last-Modified via the service
        // metadata (the version spine's newest commit instant, §"ETag and
        // Last-Modified"). This pins the fallback's honest behaviour for
        // the remaining body-derived reads.
        let body = serde_json::json!({
            "_type": "VERSIONED_PARTY",
            "uid": { "_type": "HIER_OBJECT_ID", "value": VO }
        });
        let m = read_meta(VO, &body);
        assert_eq!(m.uid, VO);
        assert_eq!(m.last_modified, None);
    }

    /// A versioned read emits the derived versioning headers and no `Location`.
    #[test]
    fn read_versioned_emits_etag_without_location() {
        let body = serde_json::json!({
            "_type": "VERSIONED_PARTY",
            "uid": { "_type": "HIER_OBJECT_ID", "value": VO }
        });
        let resp = read_versioned(&HeaderMap::new(), VO, &body);
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(etag(&resp), Some(format!("W/\"{VO}\"").as_str()));
        assert!(
            location(&resp).is_none(),
            "ITS-REST overview §Location: no Location on a GET"
        );
    }
}
