// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care relationship resources: **an extension of ITS-REST**.
//!
//! ITS-REST 1.1.0 has no resource for a relationship. These follow the draft
//! access control package (ITS-REST Resources, Care Relationships): the care
//! relationships of a subject addressed by its subject reference
//! (`subject_id` and `subject_namespace`, as `GET /ehr` addresses an EHR) or
//! by its `PERSON`, one relationship by its uid, and the reverse reading by
//! carer. A care relationship is the standalone `under_care_of`
//! `PARTY_RELATIONSHIP` of `/demographic/party_relationship`, so each change
//! commits a version with its `AUDIT_DETAILS` in a `CONTRIBUTION`; `PUT` and
//! `DELETE` take `If-Match` with the relationship's version uid and every
//! answer carries its `ETag` (overview `Requests_and_responses.md` §"If-Match
//! and accidental overwrites", §"`ETag` and Last-Modified"). Every read is
//! decided per subject it reveals and writes one access record, and so one
//! access-log entry, per subject it reveals
//! (`ferroehr::service::demographic::care_relationship`). No ITS-REST
//! operation governs these routes: our own extension, excluded from any
//! conformance-profile claim.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragments \
              the service read (stored-content serving / commit interior)"
)]

use std::collections::BTreeMap;

use axum::response::Response;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::service::demographic::care_relationship::{CareRead, Revealed};
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::response::{ResourceMeta, ServiceResponse};
use ferroehr::versioning::object_version_id::parse_uid_based_id;
use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::LOCATABLE_ROOT;
use openehr_rm::prelude::PartyRelationship;
use uuid::Uuid;

use super::care_query::CareQuery;
use crate::api::{RequestParts, guarded_handler};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::negotiate::resource::Tree;
use crate::state::AppState;
use crate::system_log::served::{AuditServedEhrs, AuditServedParties};

/// The path of the relationship resources below the extensions root.
const RESOURCE: &str = "care_relationship";

/// The care relationship routes as a native `utoipa-axum` router, paths
/// relative to the extensions root.
pub(crate) fn care_relationship_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(care_relationship_list, care_relationship_create))
        .routes(routes!(
            care_relationship_get,
            care_relationship_update,
            care_relationship_delete
        ))
        .routes(routes!(
            person_care_relationship_list,
            person_care_relationship_create
        ))
}

guarded_handler! {
    /// List care relationships (`GET /care_relationship`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** With
    /// `subject_id` and `subject_namespace`, the care relationships of that
    /// subject, live ones by default; with `carer`, the reverse reading, the
    /// relationships that reach that carer, one page. A relationship of a
    /// subject whose record the caller may not read is left out, unless the
    /// caller writes care relationships for its provider; each subject a list
    /// reveals writes one access-log entry.
    #[utoipa::path(
        get, path = "/care_relationship", tag = "demographic-relationship",
        params(
            ("subject_id" = Option<String>, Query, description = "The subject's id, with subject_namespace."),
            ("subject_namespace" = Option<String>, Query, description = "The namespace of subject_id."),
            ("at" = Option<String>, Query, description = "The day `YYYY-MM-DD` the relationships are live on, with the policy's grace; today when absent."),
            ("all" = Option<bool>, Query, description = "With the subject: `true` lists ended relationships too."),
            ("carer" = Option<String>, Query, description = "The reverse reading: the carer's party container id."),
            ("role" = Option<String>, Query, description = "With carer: the care_role as the token `terminology_id|code`."),
            ("path" = Option<String>, Query, description = "With carer: `direct` or `team`."),
            ("offset" = Option<u64>, Query, description = "With carer: the first relationship of the page."),
            ("fetch" = Option<u64>, Query, description = "With carer: the largest page, 1 to 1000, 100 when absent.")
        ),
        responses(
            (status = 200, description = "A JSON array of PARTY_RELATIONSHIPs, each with its uid.", body = serde_json::Value),
            (status = 400, description = "A parameter outside its form, or both forms.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 406, description = "The Accept header admits no JSON.", body = serde_json::Value)
        )
    )]
    care_relationship_list => list
}

guarded_handler! {
    /// Create a care relationship of a subject by its reference
    /// (`POST /care_relationship?subject_id=&subject_namespace=`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** The
    /// body is an `under_care_of` `PARTY_RELATIONSHIP` whose source is the
    /// `PERSON` the subject reference names; the commit is that of
    /// `/demographic/party_relationship`.
    #[utoipa::path(
        post, path = "/care_relationship", tag = "demographic-relationship",
        params(
            ("subject_id" = String, Query, description = "The subject's id."),
            ("subject_namespace" = String, Query, description = "The namespace of subject_id."),
            ("Prefer" = Option<String>, Header, description = "`return=representation`, `return=identifier` or `return=minimal` (the default)."),
            ("openehr-audit-details" = Option<String>, Header, description = "Committal AUDIT_DETAILS of the CONTRIBUTION."),
            ("openehr-version" = Option<String>, Header, description = "Committal metadata of the VERSION.")
        ),
        request_body(content = serde_json::Value, description = "An under_care_of PARTY_RELATIONSHIP as canonical JSON or XML."),
        responses(
            (status = 201, description = "Created; ETag and Location name the new version.", body = serde_json::Value),
            (status = 400, description = "A malformed body or query.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "The writer's authority does not cover the provider or the target kind.", body = serde_json::Value),
            (status = 422, description = "No care relationship of this subject, or a target, provider or bound it breaks.", body = serde_json::Value)
        )
    )]
    care_relationship_create => create
}

guarded_handler! {
    /// Retrieve one care relationship (`GET /care_relationship/{relationship_uid}`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** A
    /// relationship the caller may not read answers `404`, as one that does
    /// not exist; a read writes one access-log entry for its subject.
    #[utoipa::path(
        get, path = "/care_relationship/{relationship_uid}", tag = "demographic-relationship",
        params(("relationship_uid" = String, Path, description = "The relationship's container id for its current version, or one version's uid.")),
        responses(
            (status = 200, description = "The PARTY_RELATIONSHIP, with its ETag.", body = serde_json::Value),
            (status = 204, description = "The version read is a deletion marker."),
            (status = 400, description = "A malformed uid.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 404, description = "No care relationship the caller may read has this uid.", body = serde_json::Value)
        )
    )]
    care_relationship_get => get
}

guarded_handler! {
    /// Change a care relationship (`PUT /care_relationship/{relationship_uid}`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** It
    /// changes the role or the period; closing `time_validity` ends it. The
    /// source stays the same subject.
    #[utoipa::path(
        put, path = "/care_relationship/{relationship_uid}", tag = "demographic-relationship",
        params(
            ("relationship_uid" = String, Path, description = "The relationship's container id."),
            ("If-Match" = String, Header, description = "REQUIRED: the current version uid."),
            ("Prefer" = Option<String>, Header, description = "`return=representation`, `return=identifier` or `return=minimal` (the default).")
        ),
        request_body(content = serde_json::Value, description = "The new version, an under_care_of PARTY_RELATIONSHIP of the same subject."),
        responses(
            (status = 200, description = "Updated, with the representation or identifier asked for.", body = serde_json::Value),
            (status = 204, description = "Updated."),
            (status = 400, description = "A malformed body or uid, or no If-Match.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "The writer's authority does not cover the change.", body = serde_json::Value),
            (status = 404, description = "No care relationship has this uid.", body = serde_json::Value),
            (status = 412, description = "If-Match names no current version; ETag names the current one.", body = serde_json::Value),
            (status = 422, description = "No care relationship of the same subject, or a bound it breaks.", body = serde_json::Value)
        )
    )]
    care_relationship_update => update
}

guarded_handler! {
    /// Remove a care relationship written in error
    /// (`DELETE /care_relationship/{relationship_uid}`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** Its
    /// earlier versions keep it.
    #[utoipa::path(
        delete, path = "/care_relationship/{relationship_uid}", tag = "demographic-relationship",
        params(
            ("relationship_uid" = String, Path, description = "The relationship's container id."),
            ("If-Match" = String, Header, description = "REQUIRED: the current version uid.")
        ),
        responses(
            (status = 204, description = "Deleted; ETag names the deletion's version."),
            (status = 400, description = "A malformed uid, no If-Match, or an already deleted relationship.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "The writer's authority does not cover it.", body = serde_json::Value),
            (status = 404, description = "No care relationship has this uid.", body = serde_json::Value),
            (status = 412, description = "If-Match names no current version; ETag names the current one.", body = serde_json::Value)
        )
    )]
    care_relationship_delete => delete
}

guarded_handler! {
    /// List the care relationships of a `PERSON`
    /// (`GET /demographic/person/{uid_based_id}/care_relationship`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** As
    /// the list by subject reference, for the subject this `PERSON` is.
    #[utoipa::path(
        get, path = "/demographic/person/{uid_based_id}/care_relationship", tag = "demographic-relationship",
        params(
            ("uid_based_id" = String, Path, description = "The subject's PERSON, by its container id or a version uid."),
            ("at" = Option<String>, Query, description = "The day `YYYY-MM-DD` the relationships are live on; today when absent."),
            ("all" = Option<bool>, Query, description = "`true` lists ended relationships too.")
        ),
        responses(
            (status = 200, description = "A JSON array of PARTY_RELATIONSHIPs, each with its uid.", body = serde_json::Value),
            (status = 400, description = "A malformed uid or parameter.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 404, description = "No PERSON has this id.", body = serde_json::Value)
        )
    )]
    person_care_relationship_list => person_list
}

guarded_handler! {
    /// Create a care relationship of a `PERSON`
    /// (`POST /demographic/person/{uid_based_id}/care_relationship`).
    ///
    /// **An extension of ITS-REST: no ITS-REST operation governs this.** As
    /// the create by subject reference, for the subject this `PERSON` is.
    #[utoipa::path(
        post, path = "/demographic/person/{uid_based_id}/care_relationship", tag = "demographic-relationship",
        params(
            ("uid_based_id" = String, Path, description = "The subject's PERSON, by its container id."),
            ("Prefer" = Option<String>, Header, description = "`return=representation`, `return=identifier` or `return=minimal` (the default).")
        ),
        request_body(content = serde_json::Value, description = "An under_care_of PARTY_RELATIONSHIP whose source is this PERSON."),
        responses(
            (status = 201, description = "Created; ETag and Location name the new version.", body = serde_json::Value),
            (status = 400, description = "A malformed body or uid.", body = serde_json::Value),
            (status = 401, description = "Unauthenticated.", body = serde_json::Value),
            (status = 403, description = "The writer's authority does not cover the provider or the target kind.", body = serde_json::Value),
            (status = 404, description = "No PERSON has this id.", body = serde_json::Value),
            (status = 422, description = "No care relationship of this PERSON, or a target, provider or bound it breaks.", body = serde_json::Value)
        )
    )]
    person_care_relationship_create => person_create
}

/// The `care_relationship_list` operation.
pub(crate) async fn list(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let backend = state.backend();
    let read = match CareQuery::parse(parts.query.as_deref())? {
        CareQuery::Subject(subject, span) => match backend.care_subject_person(&subject).await? {
            Some(person) => backend.care_relationships_of(person, span).await?,
            None => CareRead {
                body: Vec::new(),
                revealed: Revealed::default(),
            },
        },
        CareQuery::Carer(query) => backend.care_relationships_reaching(&query).await?,
    };
    Ok(listed(&parts.headers, read))
}

/// The `person_care_relationship_list` operation.
pub(crate) async fn person_list(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let span = super::care_query::span(parts.query.as_deref())?;
    let person = person_of(&state, &parts).await?;
    let read = state.backend().care_relationships_of(person, span).await?;
    Ok(listed(&parts.headers, read))
}

/// The `care_relationship_create` operation.
pub(crate) async fn create(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let subject = match CareQuery::parse(parts.query.as_deref())? {
        CareQuery::Subject(subject, span) if !span.all && span.at.is_none() => subject,
        _ => {
            return Err(malformed(
                "a care relationship is created for the subject subject_id and \
                 subject_namespace name, and nothing else",
            ));
        }
    };
    let Some(person) = state.backend().care_subject_person(&subject).await? else {
        return Err(RestError::from(ServiceError::ValidationFailed(vec![
            openehr_base::validate::InvariantViolation::at(
                "/source",
                format!(
                    "the subject reference in namespace {} names no PERSON of this store",
                    subject.namespace
                ),
            ),
        ])));
    };
    created(&state, &parts, person).await
}

/// The `person_care_relationship_create` operation.
pub(crate) async fn person_create(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let person = person_of(&state, &parts).await?;
    created(&state, &parts, person).await
}

/// The `care_relationship_get` operation.
pub(crate) async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    crate::formats::dispatch::guard_non_templated(h)?;
    let address = parse_uid_based_id(relationship_uid(&parts)?)?;
    let read = state.backend().care_relationship_get(address).await?;
    let mut out = if read.body.body.is_null() {
        negotiate::body::empty(StatusCode::NO_CONTENT)
    } else {
        negotiate::resource::read_raw_rm::<PartyRelationship>(
            h,
            read.body,
            LOCATABLE_ROOT,
            Tree::Demographic,
        )
    };
    audit_revealed(&mut out, read.revealed);
    Ok(out)
}

/// The `care_relationship_update` operation.
pub(crate) async fn update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    crate::formats::dispatch::guard_non_templated(h)?;
    let (vo_id, precondition) = target(&parts)?;
    let body = negotiate::decode::rm_value::<PartyRelationship>(h, &parts.body)?;
    let request = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(state.config().api.system_id()),
        body,
        None,
    )?;
    let written = state
        .backend()
        .care_relationship_update(vo_id, precondition, request)
        .await;
    match written {
        Ok(resp) => Ok(write_answer(
            &state,
            h,
            (StatusCode::NO_CONTENT, StatusCode::OK),
            &resp,
        )),
        Err(e) => Ok(stale(&state, vo_id, e).await?),
    }
}

/// The `care_relationship_delete` operation.
pub(crate) async fn delete(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    crate::formats::dispatch::guard_non_templated(h)?;
    let (vo_id, precondition) = target(&parts)?;
    let request = crate::overview::committal::delete_request(
        h,
        ferroehr::versioning::committer::request_committer(state.config().api.system_id()),
        None,
    )?;
    match state
        .backend()
        .care_relationship_delete(vo_id, precondition, request)
        .await
    {
        Ok(resp) => {
            let mut out = negotiate::body::empty(StatusCode::NO_CONTENT);
            if let Some(meta) = resp.meta.as_ref() {
                negotiate::resource::set_versioning_headers(&mut out, meta, Tree::Demographic);
            }
            Ok(out)
        }
        Err(e) => Ok(stale(&state, vo_id, e).await?),
    }
}

/// Commits the care relationship of the request body for the subject
/// `person`, answered as a creation.
async fn created(
    state: &AppState,
    parts: &RequestParts,
    person: Uuid,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    crate::formats::dispatch::guard_non_templated(h)?;
    let body = negotiate::decode::rm_value::<PartyRelationship>(h, &parts.body)?;
    let request = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(state.config().api.system_id()),
        body,
        None,
    )?;
    let resp = state
        .backend()
        .care_relationship_create(person, request)
        .await?;
    Ok(write_answer(
        state,
        h,
        (StatusCode::CREATED, StatusCode::CREATED),
        &resp,
    ))
}

/// The `PERSON` the `uid_based_id` of `parts` names.
///
/// # Errors
/// A `400` when malformed; a `404` when it names no `PERSON`.
async fn person_of(state: &AppState, parts: &RequestParts) -> Result<Uuid, RestError> {
    let raw = parts
        .path
        .get("uid_based_id")
        .ok_or_else(|| malformed("the PERSON's uid_based_id is missing"))?;
    let vo_id = parse_uid_based_id(raw)?.vo_id;
    match state.backend().party_kind(vo_id).await? {
        PartyKind::Person => Ok(vo_id.0),
        _ => Err(RestError::from(ApiError::NotFound(format!(
            "no PERSON has the id {vo_id}"
        )))),
    }
}

/// The `relationship_uid` of `parts`.
fn relationship_uid(parts: &RequestParts) -> Result<&str, RestError> {
    parts
        .path
        .get("relationship_uid")
        .map(String::as_str)
        .ok_or_else(|| malformed("the relationship_uid is missing"))
}

/// The relationship a `PUT` or `DELETE` changes, by its container, and the
/// precondition its mandatory `If-Match` names.
///
/// # Errors
/// A `400` when the uid is malformed or names a version, or `If-Match` is
/// absent or malformed.
fn target(parts: &RequestParts) -> Result<(VoId, Precondition), RestError> {
    let address = parse_uid_based_id(relationship_uid(parts)?)?;
    if address.version.is_some() {
        return Err(malformed(
            "a care relationship is changed by its container id, its current version named in \
             If-Match",
        ));
    }
    let Some(if_match) = crate::overview::version_id::if_match(&parts.headers) else {
        return Err(malformed(
            "If-Match is required: it names the relationship's current version uid",
        ));
    };
    Ok((address.vo_id, Precondition::parse(Some(&if_match), None)?))
}

/// The answer to a refused change of `vo_id`: a precondition failure echoes
/// the current version in `ETag` (overview §"If-Match and accidental
/// overwrites"); any other refusal is the error.
async fn stale(state: &AppState, vo_id: VoId, e: ServiceError) -> Result<Response, RestError> {
    if !matches!(e, ServiceError::VersionConflict(_)) {
        return Err(RestError::from(e));
    }
    // NOTE: our own design, a current version that cannot be read leaves the refusal without
    // its ETag, which is a SHOULD of the overview.
    let meta: Option<ResourceMeta> = state
        .backend()
        .party_relationship_latest_meta(vo_id)
        .await
        .ok()
        .flatten();
    Ok(negotiate::resource::error_with_meta(
        crate::overview::service_error::api_error(e),
        meta.as_ref(),
        Tree::Demographic,
    ))
}

/// A write's answer honouring `Prefer`, with the `ETag` of the version it
/// wrote and the `Location` of the relationship resource at that version.
fn write_answer(
    state: &AppState,
    h: &HeaderMap,
    (minimal, representation): (StatusCode, StatusCode),
    resp: &ServiceResponse,
) -> Response {
    let uid = resp.meta.as_ref().map(|meta| meta.uid.as_str());
    let mut out = negotiate::prefer::write_negotiated(h, minimal, representation, uid, |status| {
        negotiate::body::respond_rm::<PartyRelationship>(h, status, &resp.body, LOCATABLE_ROOT)
    });
    if let Some(meta) = resp.meta.as_ref() {
        negotiate::resource::set_versioning_headers(&mut out, meta, Tree::Demographic);
        let location = format!(
            "{}/{RESOURCE}/{}",
            state.config().api.extensions_path(),
            meta.uid
        );
        if let Ok(value) = HeaderValue::from_str(&location) {
            out.headers_mut().insert(header::LOCATION, value);
        }
    }
    out
}

/// A list's answer: the JSON array of `read`, with its access records.
fn listed(h: &HeaderMap, read: CareRead<Vec<Value>>) -> Response {
    let mut out = negotiate::body::respond(h, StatusCode::OK, &Value::Array(read.body));
    audit_revealed(&mut out, read.revealed);
    out
}

/// Marks `out` with each subject a read revealed, so the audit layer writes
/// one access record for each: by its EHR, an access-log entry, or by its
/// `PERSON` when it has none.
pub(super) fn audit_revealed(out: &mut Response, revealed: Revealed) {
    if !revealed.ehrs.is_empty() {
        out.extensions_mut().insert(AuditServedEhrs {
            served: revealed
                .ehrs
                .into_iter()
                .map(|(ehr, count)| (ehr.0.to_string(), count))
                .collect(),
            content: BTreeMap::new(),
            query: None,
        });
    }
    if !revealed.persons.is_empty() {
        out.extensions_mut().insert(AuditServedParties {
            parties: revealed
                .persons
                .into_iter()
                .map(|person| person.to_string())
                .collect(),
        });
    }
}

/// The `400` of a malformed request.
fn malformed(message: impl Into<String>) -> RestError {
    RestError::from(ApiError::BadRequest(message.into()))
}
