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
//! own extension realizing SM `I_PARTY_RELATIONSHIP` and are excluded from
//! conformance-profile claims.
//!
//! The five per-kind operation families collapse by mapping the operation-id
//! prefix to a [`PartyKind`] (`parse_party_op`), the generated per-kind
//! `*Params` structs being field-identical. The file layout mirrors the spec
//! resources: `party`, `tags`, `versioned_party`, `contribution`,
//! `relationship`, and `dispatch` as the operation-id match.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, header};

use openehr_its::rest::runtime::ApiError;

use crate::overview::error::RestError;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::response::{ResourceMeta, ServiceResponse};
use ferroehr::service::status::{CallStatusType, SmError};

mod contribution;
pub(crate) mod dispatch;
pub(crate) mod openapi_routes;
mod party;
pub(crate) mod relationship;
mod tags;
mod versioned_party;

// The two symbols `app/ferroehr-rest/src/api/mod.rs` mounts: the group
// dispatcher (`demographic::dispatch`) and the native `utoipa-axum` router for
// the own-design PARTY_RELATIONSHIP extension (`demographic::relationship_routes`,
// which single-sources those routes + their OpenAPI paths and dispatches back
// through the group dispatcher).

/// Map an operation id to `(PartyKind, action)` where `action` is the suffix
/// after the kind prefix (e.g. `person_tags_update` → `(Person, "tags_update")`).
/// `None` for the kind-agnostic operations (`versioned_party_*`,
/// `contribution_*`, `demographic_tags_get`).
fn parse_party_op(op: &str) -> Option<(PartyKind, &str)> {
    const KINDS: &[(&str, PartyKind)] = &[
        ("agent", PartyKind::Agent),
        ("group", PartyKind::Group),
        ("organisation", PartyKind::Organisation),
        ("person", PartyKind::Person),
        ("role", PartyKind::Role),
    ];
    for (prefix, kind) in KINDS {
        if let Some(rest) = op.strip_prefix(prefix).and_then(|r| r.strip_prefix('_')) {
            return Some((*kind, rest));
        }
    }
    None
}

/// Whether an error is the optimistic-concurrency precondition failure
/// (`If-Match` mismatch → `412` on update; a stale `uid_based_id` → `409` on
/// delete, per `person_delete.yaml` `409_PERSON_with_uid_based_id`).
fn is_precondition(e: &SmError) -> bool {
    e.status == CallStatusType::VersionMismatch
}

/// Normalize an `If-Match` wire value to the bare `OBJECT_VERSION_ID` the
/// service seam compares.
///
/// The overview §"`ETag` and Last-Modified" makes every resource-identifier
/// `ETag` weak-type (`W/"…"`), and §"Deprecated headers" keeps the pre-1.1.0
/// bare quoted form supported ("implementations MAY still support it") — so a
/// client echoing either shape of the `ETag` this server emitted names the same
/// version. [`crate::overview::version_id::strip_etag`] is the one place this
/// server decodes that syntax; the demographic seam now goes through it instead
/// of stripping quotes only (which turned the server's OWN weak `ETag` into a
/// malformed precondition).
fn if_match_token(raw: &str) -> String {
    crate::overview::version_id::strip_etag(raw).to_owned()
}

/// The `If-Match` request header normalized by [`if_match_token`], or `None`
/// when the header is absent (or not valid header text). A present-but-empty
/// value yields `Some("")`, which the service rejects as a malformed
/// precondition — never silently as "no precondition" (overview §"If-Match and
/// accidental overwrites": a received `If-Match` MUST be honoured).
fn if_match_of(h: &HeaderMap) -> Option<String> {
    h.get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(if_match_token)
}

/// Set the versioning response headers from a demographic [`ResourceMeta`]
/// (whose `ehr_id` is empty — parties are not EHR-scoped): the weak `ETag`
/// (`W/"{uid}"` — the ITS-REST overview §"`ETag` and Last-Modified" makes
/// resource-identifier `ETag`s weak-type; the bare quoted form is deprecated)
/// and, when the metadata carries a commit time, `Last-Modified` (same section:
/// "derived from `VERSION.commit_audit.time_committed.value`"; both SHOULD
/// accompany `VERSION`/`VERSIONED_OBJECT` responses). Also stamps the ATNA
/// audit object.
///
/// **No `Location`** — overview §Location: it "MUST NOT be used to indicate an
/// alternate representation of an existing resource (e.g. via `GET` method)"
/// and "MUST ONLY be used for resource creation (e.g., `201 Created`) or
/// redirect responses"; §"Deprecated headers" deprecates it on `GET` and
/// `DELETE` responses alike. Writes add it through [`set_write_headers`].
fn set_versioning_headers(resp: &mut Response, meta: Option<&ResourceMeta>) {
    let Some(meta) = meta else { return };
    // The weak-form construction is shared with the EHR/definition surfaces so
    // there is one implementation of the `W/"…"` shape in the adapter.
    crate::overview::negotiate::set_etag(resp, &meta.uid);
    if let Some(at) = meta.last_modified
        && let Ok(lm) = HeaderValue::from_str(&crate::overview::negotiate::http_date(at))
    {
        resp.headers_mut().insert(header::LAST_MODIFIED, lm);
    }
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditObject {
            ehr_id: None,
            uid: Some(meta.uid.clone()),
            result_count: None,
            domain: Some(ferroehr::system_log::event::AccessDomain::Demographic),
            origins: Vec::new(),
            origin_count: None,
        });
}

/// The write-response headers: the versioning headers plus the
/// `/demographic/{segment}/{uid}` `Location` of the version this request
/// created.
///
/// `Location` rides ONLY the create/update writes: overview §Location scopes
/// the header to "resource creation … or redirect responses", and §"Prefer
/// minimal, identifier or full representation response" names the target as
/// "the newly created or updated resource" — an openEHR update commits a new
/// VERSION, which IS a newly created resource at the emitted URL. Reads,
/// deletes, and error responses use [`set_versioning_headers`] instead.
fn set_write_headers(resp: &mut Response, base: &str, segment: &str, meta: Option<&ResourceMeta>) {
    set_versioning_headers(resp, meta);
    let Some(meta) = meta else { return };
    let location = format!("{base}/demographic/{segment}/{}", meta.uid);
    if let Ok(loc) = HeaderValue::from_str(&location) {
        resp.headers_mut().insert(header::LOCATION, loc);
    }
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

/// Render a demographic read that the service returns without metadata, with
/// the versioning headers derived from the served body ([`read_meta`]).
fn read_versioned(h: &HeaderMap, versioned_object_uid: &str, body: &serde_json::Value) -> Response {
    let mut out = crate::overview::negotiate::respond(h, http::StatusCode::OK, body);
    set_versioning_headers(&mut out, Some(&read_meta(versioned_object_uid, body)));
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

/// Render an error, additionally echoing the latest version in the `ETag` the
/// `412` (update) / `409` (delete) responses require — overview §"If-Match and
/// accidental overwrites": on a false precondition the service "SHOULD return
/// also latest `version_uid` in the `ETag` response headers".
///
/// No `Location`: an error response creates nothing, and overview §Location
/// scopes the header to creation/redirect responses only.
fn error_with_meta(error: ApiError, meta: Option<&ResourceMeta>) -> Response {
    let mut out = RestError(error).into_response();
    set_versioning_headers(&mut out, meta);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut resp = crate::overview::negotiate::empty(http::StatusCode::OK);
        set_versioning_headers(&mut resp, Some(&meta()));
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
        let mut resp = crate::overview::negotiate::empty(http::StatusCode::OK);
        set_versioning_headers(&mut resp, Some(&meta().with_last_modified(at)));
        assert_eq!(last_modified(&resp), Some("Wed, 22 Jul 2009 19:15:56 GMT"));

        let mut bare = crate::overview::negotiate::empty(http::StatusCode::OK);
        set_versioning_headers(&mut bare, Some(&meta()));
        assert!(last_modified(&bare).is_none());
    }

    /// A create/update write DOES carry `Location` (overview §Location:
    /// creation responses; §"Prefer minimal…": "the newly created or updated
    /// resource").
    #[test]
    fn write_headers_add_the_creation_location() {
        let mut resp = crate::overview::negotiate::empty(http::StatusCode::CREATED);
        set_write_headers(&mut resp, BASE, "person", Some(&meta()));
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
                if_match_of(&if_match_headers(&raw)).as_deref(),
                Some(UID),
                "If-Match {raw:?} must reduce to the bare OBJECT_VERSION_ID"
            );
        }
        assert_eq!(if_match_of(&HeaderMap::new()), None);
        // A present-but-empty precondition is NOT "absent": it reaches the
        // service, which rejects it (overview §If-Match: a received header MUST
        // be honoured).
        assert_eq!(
            if_match_of(&if_match_headers("\"\"")).as_deref(),
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
