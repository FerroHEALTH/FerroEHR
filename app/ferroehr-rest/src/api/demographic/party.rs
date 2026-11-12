// SPDX-FileCopyrightText: Cadasto B.V.
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

use ferroehr::versioning::object_version_id::parse_uid_based_id;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::negotiate::resource::Tree;
use crate::state::AppState;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::response::ServiceResponse;

/// The `create` operation.
pub(crate) async fn create(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let base = state.config().api.base_path.clone();
    // Demographic PARTY types are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;
    // All per-kind `*CreateParams` are field-identical; reuse one.
    let _p = parts.decode(AgentCreateParams::from_request, AgentCreateParams::PARAMS)?;
    let body = decode_party_body(kind, h, &parts.body, state.config().spec_profile)?;
    let pending_tags = item_tags::pending(h)?;
    let resp = state
        .backend()
        .party_create(
            kind,
            crate::overview::committal::write_request(
                h,
                ferroehr::versioning::committer::request_committer(),
                body,
                None,
            )?,
        )
        .await?;
    // The party must exist first (`item_tag.target_vo_id` FK), so the
    // request-header tags are persisted after the create.
    let statuses = (StatusCode::CREATED, StatusCode::CREATED);
    respond_written(&state, kind, h, &base, statuses, pending_tags, resp).await
}

/// Persists the request-header tags of a committed create or update, adds the
/// stable-profile `reverse_relationships` to its representation and renders
/// the write response with the minimal and representation `statuses`.
///
/// # Errors
/// The tag write and the index read failures.
async fn respond_written(
    state: &AppState,
    kind: PartyKind,
    h: &HeaderMap,
    base: &str,
    statuses: (StatusCode, StatusCode),
    pending_tags: item_tags::PendingItemTags,
    mut resp: ServiceResponse,
) -> Result<Response, RestError> {
    persist_request_tags(state, kind, pending_tags, &mut resp).await?;
    let served = written_reverse_relationships(state, h, &mut resp).await?;
    let mut out = write_party(state, kind, h, base, statuses, &resp);
    super::set_item_tag_headers(&mut out, &resp);
    audit_served(&mut out, served);
    Ok(out)
}

/// The `update` operation.
pub(crate) async fn update(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let base = state.config().api.base_path.clone();
    // Demographic PARTY types are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;
    let p = parts.decode(AgentUpdateParams::from_request, AgentUpdateParams::PARAMS)?;
    let body = decode_party_body(kind, h, &parts.body, state.config().spec_profile)?;
    // Judged before the commit, so a defective tag refuses the request
    // while nothing is durable.
    let pending_tags = item_tags::pending(h)?;
    let request = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        body,
        None,
    )?;
    let (vo_id, precondition) = super::update_target(&p.uid_based_id, &p.if_match)?;
    match state
        .backend()
        .party_update(kind, vo_id, precondition, request)
        .await
    {
        Ok(resp) => {
            let statuses = (StatusCode::NO_CONTENT, StatusCode::OK);
            respond_written(&state, kind, h, &base, statuses, pending_tags, resp).await
        }
        // person_update.yaml: `If-Match` mismatch → 412 + latest version_uid.
        Err(e) if super::is_precondition(&e) => {
            let meta = state
                .backend()
                .demographic_latest_meta(kind, vo_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::resource::error_with_meta(
                crate::overview::service_error::api_error(e),
                meta.as_ref(),
                Tree::Demographic,
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// Reads a party (`person_get.yaml`): a deleted current version is `204`, and
/// a stable-profile read carries `reverse_relationships` in canonical JSON and
/// XML alike ([`serves_reverse_relationships`]), as they stand now, or, for a
/// read of a version by its id or at a time, as they stood at that version's
/// commit ([`pinned_instant`]).
pub(crate) async fn get(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // Demographic PARTY types are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let h = &parts.headers;
    // A deleted current version → Null body → 204 (like composition_get).
    let Some(Addressed {
        vo_id,
        pinned,
        mut resp,
    }) = read_addressed(&state, Some(kind), &parts).await?
    else {
        return Ok(negotiate::body::empty(StatusCode::NO_CONTENT));
    };
    let profile = state.config().spec_profile;
    let mut served = Vec::new();
    if serves_reverse_relationships(profile, h) {
        let at = pinned_instant(pinned, &resp);
        served = add_reverse_relationships(&state, vo_id, at, &mut resp.body).await?;
    }
    let mut out = read_party(kind, h, &resp, profile);
    audit_served(&mut out, served);
    Ok(out)
}

/// One party version a read addressed.
pub(super) struct Addressed {
    /// The party's version container.
    pub(super) vo_id: VoId,
    /// Whether the read named one version, by its id or at a time.
    pub(super) pinned: bool,
    /// The version read.
    pub(super) resp: ServiceResponse,
}

/// Reads the party version the `uid_based_id` and `version_at_time` of
/// `parts` address, of `kind` or, when `None`, of the kind it is stored as;
/// `None` when that version is a deletion marker.
///
/// # Errors
/// The `400` of a malformed address and the read's own refusals.
pub(super) async fn read_addressed(
    state: &AppState,
    kind: Option<PartyKind>,
    parts: &RequestParts,
) -> Result<Option<Addressed>, RestError> {
    let p = parts.decode(AgentGetParams::from_request, AgentGetParams::PARAMS)?;
    let address = parse_uid_based_id(&p.uid_based_id)?;
    let vo_id = address.vo_id;
    let pinned = address.tree().is_some() || p.version_at_time.is_some();
    let kind = match kind {
        Some(kind) => kind,
        None => state.backend().party_kind(vo_id).await?,
    };
    let resp = state
        .backend()
        .party_get(kind, address, p.version_at_time)
        .await?;
    Ok((!resp.is_empty()).then_some(Addressed {
        vo_id,
        pinned,
        resp,
    }))
}

/// The instant a party read serves `reverse_relationships` at: `None`, now,
/// for a read of the current version by its container id, else the commit
/// instant of the version a read by version id or at a time serves.
///
/// No openEHR spec says how the derived list behaves on an old version (RM
/// 1.1.0 `PARTY` §Attributes; SPECRM-124 makes it a function in RM 1.2.0).
/// Pinning a version-addressed read to its version's commit gives every
/// representation of one version, whose `ETag` names that version, one list:
/// our own design (#5473).
pub(super) fn pinned_instant(pinned: bool, resp: &ServiceResponse) -> Option<jiff::Timestamp> {
    if !pinned {
        return None;
    }
    resp.meta.as_ref().and_then(|meta| meta.last_modified)
}

/// Adds the `reverse_relationships` of the party `vo_id`, as they stood at
/// `at` (now when `None`), to the canonical `body` of one of its versions, and
/// returns the source party containers the references disclose; a party no
/// relationship targets gets no member, the RM 1.1.0 attribute being `0..1`
/// and never empty (`Reverse_relationships_validity`).
///
/// # Errors
/// The storage failure of the index read.
pub(super) async fn add_reverse_relationships(
    state: &AppState,
    vo_id: VoId,
    at: Option<jiff::Timestamp>,
    body: &mut serde_json::Value,
) -> Result<Vec<String>, RestError> {
    let reverse = state
        .backend()
        .party_reverse_relationships(vo_id, at)
        .await?;
    if reverse.references.is_empty() {
        return Ok(Vec::new());
    }
    let serde_json::Value::Object(map) = body else {
        return Ok(Vec::new());
    };
    map.insert(
        "reverse_relationships".to_owned(),
        serde_json::Value::Array(reverse.references),
    );
    Ok(reverse.sources)
}

/// Records the source parties `served` disclosed on the response `out`: the
/// references name each source's container and relationship, so each source
/// gets its own access record beside the addressed party's.
pub(super) fn audit_served(out: &mut Response, served: Vec<String>) {
    if served.is_empty() {
        return;
    }
    out.extensions_mut()
        .insert(crate::system_log::served::AuditServedParties { parties: served });
}

/// Adds `reverse_relationships` to the representation a stable-profile
/// create or update returns, as they stood at the commit it made, so the
/// written document is the one a read of that version serves.
///
/// Only a write that returns its representation reads the index.
///
/// # Errors
/// The storage failure of the index read.
async fn written_reverse_relationships(
    state: &AppState,
    h: &HeaderMap,
    resp: &mut ServiceResponse,
) -> Result<Vec<String>, RestError> {
    let representation =
        negotiate::prefer::prefers(h, negotiate::prefer::Preference::Representation);
    if !representation || !serves_reverse_relationships(state.config().spec_profile, h) {
        return Ok(Vec::new());
    }
    let Some(meta) = resp.meta.as_ref() else {
        return Ok(Vec::new());
    };
    let vo_id = parse_uid_based_id(&meta.uid)?.vo_id;
    let at = pinned_instant(true, resp);
    add_reverse_relationships(state, vo_id, at, &mut resp.body).await
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
pub(crate) async fn delete(
    state: AppState,
    kind: PartyKind,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // Demographic PARTY types are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let h = &parts.headers;
    // All per-kind delete params are field-identical; reuse one.
    let p = parts.decode(AgentDeleteParams::from_request, AgentDeleteParams::PARAMS)?;
    let request = crate::overview::committal::delete_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        None,
    )?;
    // NOTE: the preceding version comes from the path `uid_based_id`, so
    // `If-Match` is accepted but never required — overview §"If-Match and
    // accidental overwrites" requires it only when the path lacks it.
    let (vo_id, precondition) = super::delete_target(
        &p.uid_based_id,
        kind.rm_type(),
        crate::overview::version_id::if_match(h).as_deref(),
    )?;
    match state
        .backend()
        .party_delete(kind, vo_id, precondition, request)
        .await
    {
        Ok(resp) => {
            let mut out = negotiate::body::empty(StatusCode::NO_CONTENT);
            if let Some(meta) = resp.meta.as_ref() {
                negotiate::resource::set_versioning_headers(&mut out, meta, Tree::Demographic);
            }
            Ok(out)
        }
        // 409_PERSON_with_uid_based_id: the supplied uid is not the latest.
        Err(e @ ServiceError::VersionConflict(_)) => {
            let meta = state
                .backend()
                .demographic_latest_meta(kind, vo_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::resource::error_with_meta(
                ApiError::Conflict(e.message()),
                meta.as_ref(),
                Tree::Demographic,
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
        return negotiate::decode::rm_value::<Current>(h, body);
    }
    let json = std::str::from_utf8(body)
        .map_err(|e| ApiError::BadRequest(format!("body is not UTF-8: {e}")))?;
    let released: Stable = openehr_its::json::from_canonical_json(json)
        .map_err(|e| negotiate::decode::canonical_json_refusal(json, &e))?;
    let mut value = openehr_its::json::to_canonical_value(&released);
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("reverse_relationships");
    }
    openehr_its::json::from_canonical_value::<Current>(&value)
        .map_err(|e| ApiError::BadRequest(format!("invalid canonical JSON body: {e}")))
}

/// Whether a party read carries `PARTY.reverse_relationships`: under the
/// stable profile, whose RM 1.1.0 declares the attribute (`…demographic.party.adoc`
/// 1.1.0 §Attributes), in either canonical representation.
///
/// RM 1.2.0 turns the attribute into a function (SPECRM-124), so a
/// development-profile read leaves it out.
fn serves_reverse_relationships(
    profile: ferroehr::config::profile::SpecProfile,
    h: &HeaderMap,
) -> bool {
    profile == ferroehr::config::profile::SpecProfile::Stable
        && negotiate::resolve_accept(
            h,
            negotiate::CANONICAL,
            negotiate::WireFormat::CanonicalJson,
        )
        .is_some()
}

/// Renders a party body in the negotiated representation, re-typed for XML
/// through the development RM generation.
fn respond_party(
    kind: PartyKind,
    h: &HeaderMap,
    status: StatusCode,
    body: &serde_json::Value,
) -> Response {
    let root = openehr_its::xml::LOCATABLE_ROOT;
    match kind {
        PartyKind::Agent => negotiate::body::respond_rm::<Agent>(h, status, body, root),
        PartyKind::Group => negotiate::body::respond_rm::<Group>(h, status, body, root),
        PartyKind::Organisation => {
            negotiate::body::respond_rm::<Organisation>(h, status, body, root)
        }
        PartyKind::Person => negotiate::body::respond_rm::<Person>(h, status, body, root),
        PartyKind::Role => negotiate::body::respond_rm::<Role>(h, status, body, root),
    }
}

/// Renders a stable-profile party read, re-typed for XML through the RM 1.1.0
/// generation, whose `PARTY` declares the `reverse_relationships` element the
/// ITS-XML RM Release-1.1.0 `Demographic.xsd` carries.
fn respond_released_party(
    kind: PartyKind,
    h: &HeaderMap,
    ok: StatusCode,
    body: &serde_json::Value,
) -> Response {
    use openehr_rm::v1_1::demographic::{agent, group, organisation, person, role};
    let root = openehr_its::xml::LOCATABLE_ROOT;
    match kind {
        PartyKind::Agent => negotiate::body::respond_rm::<agent::Agent>(h, ok, body, root),
        PartyKind::Group => negotiate::body::respond_rm::<group::Group>(h, ok, body, root),
        PartyKind::Organisation => {
            negotiate::body::respond_rm::<organisation::Organisation>(h, ok, body, root)
        }
        PartyKind::Person => negotiate::body::respond_rm::<person::Person>(h, ok, body, root),
        PartyKind::Role => negotiate::body::respond_rm::<role::Role>(h, ok, body, root),
    }
}

/// Builds a create/update response honouring `Prefer` and setting the
/// demographic `ETag`/`Last-Modified` plus the `Location` of the committed
/// version (overview §Location, §"Prefer minimal…").
///
/// The body and `Preference-Applied` go through the shared
/// [`negotiate::prefer::write_negotiated`] seam, so a demographic write honours the full
/// `Prefer` triad and declares the preference it applied. `statuses` are the
/// minimal and the representation status; a representation is rendered in the
/// RM generation the active profile serves, as a read of it is.
fn write_party(
    state: &AppState,
    kind: PartyKind,
    h: &HeaderMap,
    base: &str,
    (minimal_status, repr_status): (StatusCode, StatusCode),
    resp: &ServiceResponse,
) -> Response {
    let stable = state.config().spec_profile == ferroehr::config::profile::SpecProfile::Stable;
    let mut out = negotiate::prefer::write_negotiated(
        h,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| {
            if stable {
                respond_released_party(kind, h, status, &resp.body)
            } else {
                respond_party(kind, h, status, &resp.body)
            }
        },
    );
    if let Some(meta) = resp.meta.as_ref() {
        negotiate::resource::set_resource_headers(
            &mut out,
            base,
            Tree::Demographic,
            Some(kind.segment()),
            meta,
        );
    }
    out
}

/// Serves a `200 OK` read of a party with the demographic `ETag`/`Last-Modified`
/// and `ITEM_TAG` response headers (`person_get.yaml`), in the RM generation
/// the active profile serves.
///
/// No `Location`: it "MUST NOT be used to indicate an alternate representation
/// of an existing resource" (overview §Location).
fn read_party(
    kind: PartyKind,
    h: &HeaderMap,
    resp: &ServiceResponse,
    profile: ferroehr::config::profile::SpecProfile,
) -> Response {
    let mut out = if profile == ferroehr::config::profile::SpecProfile::Stable {
        respond_released_party(kind, h, StatusCode::OK, &resp.body)
    } else {
        respond_party(kind, h, StatusCode::OK, &resp.body)
    };
    if let Some(meta) = resp.meta.as_ref() {
        negotiate::resource::set_versioning_headers(&mut out, meta, Tree::Demographic);
    }
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
