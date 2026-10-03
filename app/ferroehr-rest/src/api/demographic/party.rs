// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The per-kind party CRUD operations (`{kind}_{create,get,update,delete}`).
//!
//! Spec: `operations/person_{create,get,update,delete}.yaml` and the
//! field-identical `agent_*`/`group_*`/`organisation_*`/`role_*`. Party bodies
//! use the LOCATABLE content negotiation: canonical JSON and XML.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::{HeaderMap, StatusCode};

use openehr_its::rest::generated::demographic::{
    AgentCreateParams, AgentDeleteParams, AgentGetParams, AgentUpdateParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::{Agent, Group, Organisation, Party, Person, Role};

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::{RestError, sm_api_error};
use crate::state::AppState;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::response::ServiceResponse;
use ferroehr::service::status::CallStatusType;

/// The per-kind CRUD operations (`create`/`get`/`update`/`delete`).
pub(super) async fn run(
    state: AppState,
    kind: PartyKind,
    action: &str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let base = state.config().server.base_path.clone();
    let seg = kind.segment();

    // Demographic PARTY types are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;

    match action {
        "create" => {
            // All per-kind `*CreateParams` are field-identical; reuse one.
            let _p =
                AgentCreateParams::from_request(&parts.route_for(AgentCreateParams::PARAMS), q, h)?;
            let body = decode_party_body(kind, h, &parts.body, state.config().spec_profile)?;
            let pending_tags = item_tags::pending(h)?;
            let mut resp = state
                .backend()
                .party_create(
                    kind,
                    body,
                    crate::overview::committal::committal_commit(
                        h,
                        crate::api::ehr::committer_proxy(),
                    )?,
                )
                .await?;
            // The party must exist first (`item_tag.target_vo_id` FK), so the
            // request-header tags are persisted after the create.
            persist_request_tags(&state, kind, pending_tags, &mut resp).await?;
            let mut out = write_party(
                kind,
                h,
                &base,
                StatusCode::CREATED,
                StatusCode::CREATED,
                &resp,
            );
            super::set_item_tag_headers(&mut out, &resp);
            Ok(out)
        }
        "get" => {
            let p = AgentGetParams::from_request(&parts.route_for(AgentGetParams::PARAMS), q, h)?;
            let resp = state
                .backend()
                .party_get(kind, p.uid_based_id, p.version_at_time)
                .await?;
            // A deleted current version → Null body → 204 (like composition_get).
            if resp.is_empty() {
                return Ok(negotiate::empty(StatusCode::NO_CONTENT));
            }
            Ok(read_party(kind, h, &resp))
        }
        "update" => {
            let p =
                AgentUpdateParams::from_request(&parts.route_for(AgentUpdateParams::PARAMS), q, h)?;
            let uid = p.uid_based_id.clone();
            let body = decode_party_body(kind, h, &parts.body, state.config().spec_profile)?;
            // Judged before the commit, so a defective tag refuses the request
            // while nothing is durable.
            let pending_tags = item_tags::pending(h)?;
            match state
                .backend()
                .party_update(
                    kind,
                    p.uid_based_id,
                    // The `W/"…"`/quoted ETag syntax is decoded here, at the
                    // adapter seam, so the service compares a bare
                    // OBJECT_VERSION_ID (overview §"`ETag` and Last-Modified").
                    super::if_match_token(&p.if_match),
                    body,
                    crate::overview::committal::committal_commit(
                        h,
                        crate::api::ehr::committer_proxy(),
                    )?,
                )
                .await
            {
                Ok(mut resp) => {
                    persist_request_tags(&state, kind, pending_tags, &mut resp).await?;
                    let mut out = write_party(
                        kind,
                        h,
                        &base,
                        StatusCode::NO_CONTENT,
                        StatusCode::OK,
                        &resp,
                    );
                    super::set_item_tag_headers(&mut out, &resp);
                    Ok(out)
                }
                // person_update.yaml: `If-Match` mismatch → 412 + latest version_uid.
                Err(e) if super::is_precondition(&e) => {
                    let meta = state
                        .backend()
                        .demographic_latest_meta(kind, uid)
                        .await
                        .ok()
                        .flatten();
                    Ok(super::error_with_meta(sm_api_error(e), meta.as_ref()))
                }
                Err(e) => Err(RestError::from(e)),
            }
        }
        "delete" => run_delete(&state, kind, &parts).await,
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted demographic party operation: {seg}_{other}"
        )))),
    }
}

/// Stores the `ITEM_TAG` lists the wrapper headers asked for on the just-written
/// party, and hands the stored lists to the response metadata seam.
///
/// The write itself is [`item_tags::persist`], the one wrapper
/// implementation this group shares with the EHR group; what is party-specific
/// is only the response plumbing — a demographic write echoes its tags off
/// [`ServiceResponse::meta`], not off the stored lists directly. A party that
/// minted no version metadata has no target to tag, so nothing is written.
///
/// Both wrapper headers are accepted on create and update alike, per the
/// released update's own prose ("`openehr-item-tag` or
/// `openehr-version-item-tag`").
///
/// # Errors
/// Whatever the shared write refuses: a server-fault version uid, or a storage
/// failure.
async fn persist_request_tags(
    state: &AppState,
    kind: PartyKind,
    pending: item_tags::PendingItemTags,
    resp: &mut ServiceResponse,
) -> Result<(), RestError> {
    if pending.is_empty() {
        return Ok(());
    }
    let Some(version_uid) = resp.meta.as_ref().map(|m| m.uid.clone()) else {
        return Ok(());
    };
    let stored = item_tags::persist(
        state,
        item_tags::TagTarget::Party(kind),
        &version_uid,
        pending,
    )
    .await?;
    if let Some(meta) = resp.meta.as_mut() {
        if let Some(tags) = stored.object {
            meta.item_tags = Some(tags);
        }
        if let Some(tags) = stored.version {
            meta.version_item_tags = Some(tags);
        }
    }
    Ok(())
}

/// Logically deletes a party: `204` plus the deleted version's `ETag`, and no
/// `Location` (overview §"Deprecated headers" deprecates it on `DELETE`).
///
/// `person_delete.yaml` places the `preceding_version_uid` in the path, not in
/// an `If-Match` header. Responses: `204_version_deleted`,
/// `400_already_deleted`, `404`, and `409_PERSON_with_uid_based_id` when the
/// supplied uid is not the latest version.
async fn run_delete(
    state: &AppState,
    kind: PartyKind,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    // All per-kind delete params are field-identical; reuse one.
    let p = AgentDeleteParams::from_request(
        &parts.route_for(AgentDeleteParams::PARAMS),
        parts.query.as_deref(),
        h,
    )?;
    let preceding = p.uid_based_id.clone();
    // NOTE: the preceding version comes from the path `uid_based_id`, so
    // `If-Match` is accepted but never required — overview §"If-Match and
    // accidental overwrites" requires it only when the path lacks it.
    match state
        .backend()
        .party_delete(
            kind,
            preceding.clone(),
            super::if_match_of(h),
            crate::overview::committal::committal_audit_for_delete(
                h,
                crate::api::ehr::committer_proxy(),
            )?,
        )
        .await
    {
        Ok(resp) => {
            let mut out = negotiate::empty(StatusCode::NO_CONTENT);
            super::set_versioning_headers(&mut out, resp.meta.as_ref());
            Ok(out)
        }
        // 409_PERSON_with_uid_based_id: the supplied uid is not the latest.
        Err(e) if e.status == CallStatusType::VersionMismatch => {
            let meta = state
                .backend()
                .demographic_latest_meta(kind, preceding)
                .await
                .ok()
                .flatten();
            Ok(super::error_with_meta(
                ApiError::Conflict(e.message),
                meta.as_ref(),
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// Decodes a party request body into the concrete `openehr-rm` party type of the
/// routed kind, carried as the RM `PARTY` the service seam takes.
///
/// The routed kind picks the type, so a body whose `_type` names a different
/// party class is refused by the strict reader itself — the parse class, `400`
/// (overview `Requests_and_responses.md` §HTTP status codes).
fn decode_party_body(
    kind: PartyKind,
    h: &HeaderMap,
    body: &bytes::Bytes,
    profile: ferroehr::config::profile::SpecProfile,
) -> Result<Party, ApiError> {
    match kind {
        PartyKind::Agent => {
            rm_party::<openehr_rm::v1_1::demographic::agent::Agent, Agent>(h, body, profile)
                .map(Party::Agent)
        }
        PartyKind::Group => {
            rm_party::<openehr_rm::v1_1::demographic::group::Group, Group>(h, body, profile)
                .map(Party::Group)
        }
        PartyKind::Organisation => rm_party::<
            openehr_rm::v1_1::demographic::organisation::Organisation,
            Organisation,
        >(h, body, profile)
        .map(Party::Organisation),
        PartyKind::Person => {
            rm_party::<openehr_rm::v1_1::demographic::person::Person, Person>(h, body, profile)
                .map(Party::Person)
        }
        PartyKind::Role => {
            rm_party::<openehr_rm::v1_1::demographic::role::Role, Role>(h, body, profile)
                .map(Party::Role)
        }
    }
}

/// Decodes one concrete party kind through the active profile's acceptance
/// boundary.
///
/// Under the stable profile a canonical-JSON body is read by the RM 1.1.0
/// generation's own strict reader first, because that generation's surface
/// admits `PARTY.reverse_relationships`
/// (`RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc` 1.1.0, removed
/// in the development line by SPECRM-124) and the development reader would
/// refuse it as an undeclared key. The validated value then enters the typed
/// core with that attribute dropped: SPECRM-124 records it as the computed
/// inverse of `relationships`, so the server re-derives it.
///
/// The XML branch needs no profile split: the XSD-grounded reader skips
/// undeclared elements in every profile.
fn rm_party<Stable, Current>(
    h: &HeaderMap,
    body: &bytes::Bytes,
    profile: ferroehr::config::profile::SpecProfile,
) -> Result<Current, ApiError>
where
    Stable: serde::de::DeserializeOwned + serde::Serialize,
    Current: openehr_its::xml::runtime::FromXml + serde::de::DeserializeOwned,
{
    let stable_json = profile == ferroehr::config::profile::SpecProfile::Stable
        && matches!(
            negotiate::content_type_format(h),
            Some(negotiate::WireFormat::CanonicalJson)
        );
    if !stable_json {
        return negotiate::rm_value::<Current>(h, body);
    }
    let json = std::str::from_utf8(body)
        .map_err(|e| ApiError::BadRequest(format!("body is not UTF-8: {e}")))?;
    let released: Stable = openehr_its::json::from_canonical_json(json)
        .map_err(|e| ApiError::BadRequest(format!("invalid canonical JSON body: {e}")))?;
    let mut value = openehr_its::json::to_canonical_value(&released);
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("reverse_relationships");
    }
    openehr_its::json::from_canonical_value::<Current>(&value)
        .map_err(|e| ApiError::BadRequest(format!("invalid canonical JSON body: {e}")))
}

/// Renders a party body as JSON or canonical XML, monomorphized per kind.
fn respond_party(
    kind: PartyKind,
    h: &HeaderMap,
    status: StatusCode,
    body: &serde_json::Value,
) -> Response {
    match kind {
        PartyKind::Agent => negotiate::respond_rm::<Agent>(h, status, body, "agent"),
        PartyKind::Group => negotiate::respond_rm::<Group>(h, status, body, "group"),
        PartyKind::Organisation => {
            negotiate::respond_rm::<Organisation>(h, status, body, "organisation")
        }
        PartyKind::Person => negotiate::respond_rm::<Person>(h, status, body, "person"),
        PartyKind::Role => negotiate::respond_rm::<Role>(h, status, body, "role"),
    }
}

/// Builds a create/update response honouring `Prefer` and setting the
/// demographic `ETag`/`Last-Modified` plus the `Location` of the committed
/// version (overview §Location, §"Prefer minimal…").
///
/// The body and `Preference-Applied` go through the shared
/// [`negotiate::write_negotiated`] seam, so a demographic write honours the full
/// `Prefer` triad and declares the preference it applied.
fn write_party(
    kind: PartyKind,
    h: &HeaderMap,
    base: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    resp: &ServiceResponse,
) -> Response {
    let mut out = negotiate::write_negotiated(
        h,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| respond_party(kind, h, status, &resp.body),
    );
    super::set_write_headers(&mut out, base, kind.segment(), resp.meta.as_ref());
    out
}

/// Serves a `200 OK` read of a party with the demographic `ETag`/`Last-Modified`
/// and `ITEM_TAG` response headers (`person_get.yaml`).
///
/// No `Location`: it "MUST NOT be used to indicate an alternate representation
/// of an existing resource" (overview §Location).
fn read_party(kind: PartyKind, h: &HeaderMap, resp: &ServiceResponse) -> Response {
    let mut out = respond_party(kind, h, StatusCode::OK, &resp.body);
    super::set_versioning_headers(&mut out, resp.meta.as_ref());
    super::set_item_tag_headers(&mut out, resp);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroehr::config::profile::SpecProfile;
    use serde_json::json;

    fn json_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        h
    }

    /// A minimal RM-valid PERSON body (raw JSON: client-simulation input).
    fn person_body() -> serde_json::Value {
        json!({
            "_type": "PERSON",
            "name": { "_type": "DV_TEXT", "value": "PERSON" },
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
            "identities": [{
                "_type": "PARTY_IDENTITY",
                "name": { "_type": "DV_TEXT", "value": "legal identity" },
                "archetype_node_id": "at0002",
                "details": {
                    "_type": "ITEM_TREE",
                    "name": { "_type": "DV_TEXT", "value": "tree" },
                    "archetype_node_id": "at0003"
                }
            }]
        })
    }

    /// The same body carrying the RM 1.1.0 `reverse_relationships` surface.
    fn person_body_with_reverse_relationships() -> serde_json::Value {
        let mut body = person_body();
        body["reverse_relationships"] = json!([{
            "_type": "LOCATABLE_REF",
            "namespace": "local",
            "type": "PARTY_RELATIONSHIP",
            "id": {
                "_type": "HIER_OBJECT_ID",
                "value": "11111111-1111-4111-8111-111111111111"
            }
        }]);
        body
    }

    fn decode(profile: SpecProfile, body: &serde_json::Value) -> Result<Party, ApiError> {
        decode_party_body(
            PartyKind::Person,
            &json_headers(),
            &bytes::Bytes::from(body.to_string()),
            profile,
        )
    }

    /// The stable profile accepts `PARTY.reverse_relationships` — the RM
    /// 1.1.0 released surface (`party.adoc` 1.1.0; removed by SPECRM-124 in
    /// the development line) — and the value enters the typed core with the
    /// derived attribute dropped.
    #[test]
    fn stable_profile_accepts_reverse_relationships() {
        let decoded = decode(
            SpecProfile::Stable,
            &person_body_with_reverse_relationships(),
        )
        .unwrap();
        let Party::Person(person) = decoded else {
            panic!("expected a PERSON");
        };
        assert_eq!(person.identities.len(), 1);
    }

    /// The development profile keeps refusing the removed attribute — the
    /// refusal twin pinning that the boundary widens ONLY the stable
    /// profile.
    #[test]
    fn development_profile_still_refuses_reverse_relationships() {
        let err = decode(
            SpecProfile::Development,
            &person_body_with_reverse_relationships(),
        )
        .expect_err("the development reader refuses the undeclared key");
        assert!(
            format!("{err:?}").contains("reverse_relationships"),
            "{err:?}"
        );
    }

    /// The stable boundary VALIDATES the attribute against RM 1.1.0 — a
    /// defective `reverse_relationships` value is a 400, never blind-stripped.
    #[test]
    fn stable_profile_validates_the_attribute_it_accepts() {
        let mut body = person_body();
        body["reverse_relationships"] =
            json!([{ "_type": "DV_TEXT", "value": "not a LOCATABLE_REF" }]);
        assert!(decode(SpecProfile::Stable, &body).is_err());
        // An empty list violates the container bound just the same.
        let mut body = person_body();
        body["reverse_relationships"] = json!([]);
        assert!(decode(SpecProfile::Stable, &body).is_err());
    }

    /// A body without the removed attribute decodes identically under both
    /// profiles — the boundary is transparent for shared surface.
    #[test]
    fn plain_bodies_decode_under_both_profiles() {
        assert!(decode(SpecProfile::Stable, &person_body()).is_ok());
        assert!(decode(SpecProfile::Development, &person_body()).is_ok());
    }
}
