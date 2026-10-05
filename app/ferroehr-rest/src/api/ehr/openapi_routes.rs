// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native utoipa-axum routing for the EHR API group.
//!
//! No openEHR spec governs an OAS layout; the operation semantics are the
//! ITS-REST EHR API (`docs/specs/openehr/ITS-REST`). Each handler forwards to
//! the group dispatcher through `guarded_dispatch`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api::guarded_dispatch;
use crate::state::AppState;

/// The EHR-group routes as a native `utoipa-axum` router, each
/// `#[utoipa::path]` handler single-sourcing its route and its `OpenAPI` path.
///
/// Paths are group-relative, nested under the configured `base_path`, and every
/// operation is served through [`guarded_dispatch`] onto
/// [`crate::api::ehr::dispatch::dispatch`].
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ehr_get_by_subject, ehr_create))
        .routes(routes!(ehr_get_by_id, ehr_create_with_id))
        .routes(routes!(ehr_status_get_by_version_id))
        .routes(routes!(ehr_status_get_at_time, ehr_status_update))
        .routes(routes!(versioned_ehr_status_get))
        .routes(routes!(versioned_ehr_status_revision_history))
        .routes(routes!(versioned_ehr_status_version_get_at_time))
        .routes(routes!(versioned_ehr_status_version_get_by_id))
        .routes(routes!(composition_create))
        .routes(routes!(
            composition_get,
            composition_update,
            composition_delete
        ))
        .routes(routes!(versioned_composition_get))
        .routes(routes!(versioned_composition_revision_history))
        .routes(routes!(versioned_composition_version_get_at_time))
        .routes(routes!(versioned_composition_version_get_by_id))
        .routes(routes!(
            directory_get_at_time,
            directory_update,
            directory_create,
            directory_delete
        ))
        .routes(routes!(directory_get_by_version_id))
        // POST + GET share `/ehr/{ehr_id}/contribution`, so they are one route.
        .routes(routes!(contribution_create, contribution_list))
        .routes(routes!(contribution_get))
        .routes(routes!(ehr_tags_get))
        .routes(routes!(composition_tags_get, composition_tags_update))
        .routes(routes!(composition_tags_delete))
        .routes(routes!(ehr_status_tags_get, ehr_status_tags_update))
        .routes(routes!(ehr_status_tags_delete))
}

// Every handler snapshots the request into `RequestParts` and runs it through
// the shared guarded dispatch onto the EHR-group dispatcher, so the EHR_ACCESS
// gate, the ABAC PEP and the ATNA audit tagging apply uniformly.

// ── EHR ───────────────────────────────────────────────────────────────────────

/// Retrieve an EHR by subject id (`GET /ehr`).
///
/// Matches `subject_id`/`subject_namespace` against the EHR's
/// `EHR_STATUS.subject.external_ref.id.value` and `.namespace`.
#[utoipa::path(
    get, path = "/ehr", tag = "EHR",
    params(
        ("subject_id" = String, Query,
         description = "The EHR subject id (matched against \
                        EHR_STATUS.subject.external_ref.id.value). Required.",
         example = "ins01"),
        ("subject_namespace" = String, Query,
         description = "The EHR subject id namespace (matched against \
                        EHR_STATUS.subject.external_ref.namespace). Required.",
         example = "demographic")
    ),
    responses(
        (status = 200, description = "The EHR (canonical JSON/XML per `Accept`). \
                                      `ETag` (weak `W/` form) carries \
                                      `EHR.ehr_id.value`. No `Location` and no \
                                      `Last-Modified`: `Requests_and_responses.md` \
                                      §Location forbids `Location` on a `GET` \
                                      (\"It MUST NOT be used to indicate an \
                                      alternate representation of an existing \
                                      resource\"), and the RM `EHR` root is not a \
                                      VERSION, so the §\"ETag and Last-Modified\" \
                                      source `VERSION.commit_audit.time_committed` \
                                      does not exist for it. When the EHR has a \
                                      directory, the body additionally carries \
                                      the RM-grounded `directory` OBJECT_REF and \
                                      `folders` list (RM ehr `EHR` class: \
                                      `directory` 0..1 refers to `folders`' \
                                      first member — `Directory_in_folders`); \
                                      the ITS-REST `Ehr` schema omits both, but \
                                      the RM model is the body authority.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<ehr_id>\"` \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\": the value \"is usually taken from \
                             e.g. … EHR.ehr_id.value\" and the `W/` weakness \
                             indicator is required since Release 1.1.0)."),
         ),
         example = json!({
             "_type": "EHR",
             "system_id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" },
             "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
             "ehr_status": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "VERSIONED_EHR_STATUS",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
             },
             "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
             "ehr_access": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "VERSIONED_EHR_ACCESS",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8" }
             }
         })),
        (status = 400, description = "A required subject query parameter \
                                      (`subject_id`, `subject_namespace`) is \
                                      missing, or a supplied one is malformed \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "No EHR exists with the supplied subject \
                                      id and namespace.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR resource has only the canonical \
                                      `application/json` / `application/xml` \
                                      representations (`Resources.md` §\"XML \
                                      Format\"/§\"JSON Format\": \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\"; the Simplified Formats \
                                      are not defined for a non-templated \
                                      resource).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_get_by_subject(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_get_by_subject",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Create a new EHR with an auto-generated id (`POST /ehr`).
///
/// The committal headers `openehr-version` / `openehr-audit-details` are
/// accepted and merged into the creating CONTRIBUTION and its `EHR_STATUS`
/// version (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    post, path = "/ehr", tag = "EHR",
    params(
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — empty body; `return=identifier` — the \
                        body is only `{ \"uid\": \"<ehr_id>\" }`; \
                        `return=representation` — the full RM `EHR`. An absent \
                        header means `return=minimal` (\"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\"); the token actually applied is echoed \
                        in the `Preference-Applied` response header.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the EHR_STATUS VERSION the \
                        creation commits, as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. Merged with the \
                        server defaults.\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the creating CONTRIBUTION, \
                        as an attribute-path list; the header MAY repeat — e.g. \
                        `description.value=\"EHR opened at triage\"`, \
                        `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` is constrained to `249|creation|` (a \
                        create commits a first version); `time_committed` is \
                        always server-set, and an omitted `system_id` defaults \
                        to the server's configured identifier.")
    ),
    request_body(content = serde_json::Value,
                 description = "Optional EHR_STATUS for the new EHR; when \
                                omitted a default (is_queryable=true, \
                                is_modifiable=true, PARTY_SELF subject) is used.",
                 example = json!({
                     "_type": "EHR_STATUS",
                     "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                         "rm_version": "1.2.0"
                     },
                     "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                     "subject": {
                         "_type": "PARTY_SELF",
                         "external_ref": {
                             "_type": "PARTY_REF",
                             "namespace": "demographic",
                             "type": "PERSON",
                             "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                         }
                     },
                     "is_queryable": true,
                     "is_modifiable": true
                 })),
    responses(
        (status = 201, description = "Created. `ETag` (weak `W/` form) carries \
                                      the new `ehr_id`, `Last-Modified` the \
                                      creation instant, `Location` the EHR URL, \
                                      and `Preference-Applied` the `Prefer` token \
                                      actually honoured. The body is \
                                      `Prefer`-conditional \
                                      (`Requests_and_responses.md` §\"Prefer \
                                      minimal, identifier or full representation \
                                      response\"): the full RM `EHR` for \
                                      `return=representation` (the \
                                      `representation` example), the single-`uid` \
                                      object for `return=identifier` (the \
                                      `identifier` example), and no body at all \
                                      for the default `return=minimal`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<ehr_id>\"` of the created \
                             EHR (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "The URL of the newly created EHR, \
                             `<base_path>/ehr/<ehr_id>` (§Location: used \"in `201 \
                             Created` responses when a new resource is \
                             successfully created\")."),
             ("Last-Modified" = String,
              description = "The creating CONTRIBUTION's commit instant as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured (§\"Representation details negotiation\")."),
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the full RM EHR",
              value = json!({
                  "_type": "EHR",
                  "system_id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" },
                  "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
                  "ehr_status": {
                      "_type": "OBJECT_REF",
                      "namespace": "local",
                      "type": "VERSIONED_EHR_STATUS",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
                  },
                  "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                  "ehr_access": {
                      "_type": "OBJECT_REF",
                      "namespace": "local",
                      "type": "VERSIONED_EHR_ACCESS",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8" }
                  }
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new ehr_id",
              value = json!({ "uid": "7d44b88c-4199-4bad-97dc-d78268e01398" })))
         )),
        (status = 400, description = "The request could not be parsed, or a \
                                      committal `change_type` names a legal \
                                      audit_change_type code that contradicts a \
                                      creation (§\"HTTP status codes\", the `400` \
                                      row: \"malformed request syntax, \
                                      syntactically invalid content\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR/EHR_STATUS resources have only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST be \
                                      `406`).",
         body = serde_json::Value),
        (status = 409, description = "An EHR already exists for the subject \
                                      id/namespace of the supplied EHR_STATUS \
                                      (§\"HTTP status codes\", the `409` row: the \
                                      request \"might generate a duplicate or a \
                                      conflict\").",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not a format \
                                      this resource can process — notably a \
                                      Simplified Format, which is defined only for \
                                      templated COMPOSITION content \
                                      (`Resources.md` §\"Simplified Formats\": \"If \
                                      the service cannot process the request \
                                      payload as the simplified format is not \
                                      supported, it MUST respond with HTTP status \
                                      code `415 Unsupported Media Type`\"; \
                                      §\"XML Format\"/§\"JSON Format\" carry the \
                                      same MUST for the canonical types).",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed but cannot be \
                                      followed: the supplied EHR_STATUS is \
                                      semantically invalid, or a committal \
                                      `change_type`/`lifecycle_state` is not a \
                                      member of its openEHR terminology group \
                                      (§\"HTTP status codes\", the `422` row: \
                                      \"The request was well-formed but was unable \
                                      to be followed due to semantic errors\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_create",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve an EHR by id (`GET /ehr/{ehr_id}`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}", tag = "EHR",
    params(("ehr_id" = String, Path,
            description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                           (`Resources.md` §\"Identifier types\": \"the EHR \
                           identifier is `7d44b88c-4199-4bad-97dc-d78268e01398`, \
                           taken from EHR.ehr_id.value\"; the SM types the \
                           argument `UUID` — `master02-overview.adoc` §\"Functional \
                           Style\", `interface I_EHR_SERVICE { Boolean \
                           has_ehr(UUID an_ehr_id); … }`).",
            example = "7d44b88c-4199-4bad-97dc-d78268e01398")),
    responses(
        (status = 200, description = "The EHR (canonical JSON/XML per `Accept`). \
                                      `ETag` (weak `W/` form) carries \
                                      `EHR.ehr_id.value`. No `Location` and no \
                                      `Last-Modified`: `Requests_and_responses.md` \
                                      §Location forbids `Location` on a `GET`, and \
                                      the RM `EHR` root is not a VERSION, so it has \
                                      no `commit_audit.time_committed` to report.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<ehr_id>\"` \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "EHR",
             "system_id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" },
             "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
             "ehr_status": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "VERSIONED_EHR_STATUS",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
             },
             "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
             "ehr_access": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "VERSIONED_EHR_ACCESS",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8" }
             }
         })),
        (status = 400, description = "`ehr_id` is malformed — it is not a UUID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\"). \
                                      A syntactically valid but unknown id is \
                                      `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "No EHR exists with `ehr_id`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR resource has only the canonical \
                                      `application/json` / `application/xml` \
                                      representations (`Resources.md` §\"XML \
                                      Format\"/§\"JSON Format\": an unfulfillable \
                                      `Accept` MUST be `406`).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_get_by_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_get_by_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Create an EHR with a client-supplied id (`PUT /ehr/{ehr_id}`).
///
/// `ehr_id` is a UUID: the SM types the argument `UUID`
/// (`SM/docs/openehr_platform/master02-overview.adoc` §"Functional Style":
/// `UUID create_ehr_with_id(UUID an_ehr_id)`), and ITS-REST identifies an EHR
/// by `EHR.ehr_id.value` in UUID form (`Resources.md` §"Identifier types").
/// Every UUID is a valid `HIER_OBJECT_ID` root, so no released sentence is
/// narrowed by accepting only UUIDs.
/// The committal headers `openehr-version` / `openehr-audit-details` are
/// accepted and merged into the creating CONTRIBUTION and its `EHR_STATUS`
/// version (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}", tag = "EHR",
    params(
        ("ehr_id" = String, Path,
         description = "The client-supplied EHR id — a UUID, which becomes \
                        `EHR.ehr_id.value` (`Resources.md` §\"Identifier types\"; \
                        the SM argument is typed `UUID` — \
                        `master02-overview.adoc` §\"Functional Style\", `UUID \
                        create_ehr_with_id(UUID an_ehr_id)`). Every UUID is a \
                        valid `HIER_OBJECT_ID` root, so this accepts every id the \
                        released text requires.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — empty body; `return=identifier` — the \
                        body is only `{ \"uid\": \"<ehr_id>\" }`; \
                        `return=representation` — the full RM `EHR`. An absent \
                        header means `return=minimal`; the token actually applied \
                        is echoed in the `Preference-Applied` response header.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the EHR_STATUS VERSION the \
                        creation commits, as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. Merged with the \
                        server defaults.\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the creating CONTRIBUTION, \
                        as an attribute-path list; the header MAY repeat — e.g. \
                        `description.value=\"EHR opened at triage\"`, \
                        `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` is constrained to `249|creation|` (a \
                        create commits a first version); `time_committed` is \
                        always server-set, and an omitted `system_id` defaults \
                        to the server's configured identifier.")
    ),
    request_body(content = serde_json::Value,
                 description = "Optional EHR_STATUS for the new EHR; when \
                                omitted a default (is_queryable=true, \
                                is_modifiable=true, PARTY_SELF subject) is used.",
                 example = json!({
                     "_type": "EHR_STATUS",
                     "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                         "rm_version": "1.2.0"
                     },
                     "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                     "subject": {
                         "_type": "PARTY_SELF",
                         "external_ref": {
                             "_type": "PARTY_REF",
                             "namespace": "demographic",
                             "type": "PERSON",
                             "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                         }
                     },
                     "is_queryable": true,
                     "is_modifiable": true
                 })),
    responses(
        (status = 201, description = "Created. `ETag` (weak `W/` form) carries \
                                      the `ehr_id`, `Last-Modified` the creation \
                                      instant, `Location` the EHR URL, and \
                                      `Preference-Applied` the `Prefer` token \
                                      actually honoured. The body is \
                                      `Prefer`-conditional \
                                      (`Requests_and_responses.md` §\"Prefer \
                                      minimal, identifier or full representation \
                                      response\"): the full RM `EHR` for \
                                      `return=representation` (the \
                                      `representation` example), the single-`uid` \
                                      object for `return=identifier` (the \
                                      `identifier` example), and no body at all \
                                      for the default `return=minimal`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<ehr_id>\"` of the created \
                             EHR (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "The URL of the newly created EHR, \
                             `<base_path>/ehr/<ehr_id>` (§Location: used \"in `201 \
                             Created` responses when a new resource is \
                             successfully created\")."),
             ("Last-Modified" = String,
              description = "The creating CONTRIBUTION's commit instant as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured (§\"Representation details negotiation\")."),
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the full RM EHR",
              value = json!({
                  "_type": "EHR",
                  "system_id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" },
                  "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
                  "ehr_status": {
                      "_type": "OBJECT_REF",
                      "namespace": "local",
                      "type": "VERSIONED_EHR_STATUS",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
                  },
                  "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                  "ehr_access": {
                      "_type": "OBJECT_REF",
                      "namespace": "local",
                      "type": "VERSIONED_EHR_ACCESS",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8" }
                  }
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the ehr_id",
              value = json!({ "uid": "7d44b88c-4199-4bad-97dc-d78268e01398" })))
         )),
        (status = 400, description = "`ehr_id` is not a UUID, the request could \
                                      not be parsed, or a committal `change_type` \
                                      names a legal audit_change_type code that \
                                      contradicts a creation (§\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR/EHR_STATUS resources have only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST be \
                                      `406`).",
         body = serde_json::Value),
        (status = 409, description = "An EHR already exists with this `ehr_id` \
                                      (§\"HTTP status codes\", the `409` row: the \
                                      request \"might generate a duplicate or a \
                                      conflict\"). Also returned when the supplied \
                                      EHR_STATUS names a subject that already owns \
                                      an EHR.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not a format \
                                      this resource can process — notably a \
                                      Simplified Format, which is defined only for \
                                      templated COMPOSITION content \
                                      (`Resources.md` §\"Simplified Formats\": \"If \
                                      the service cannot process the request \
                                      payload as the simplified format is not \
                                      supported, it MUST respond with HTTP status \
                                      code `415 Unsupported Media Type`\"; \
                                      §\"XML Format\"/§\"JSON Format\" carry the \
                                      same MUST for the canonical types).",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed but cannot be \
                                      followed: the supplied EHR_STATUS is \
                                      semantically invalid, or a committal \
                                      `change_type`/`lifecycle_state` is not a \
                                      member of its openEHR terminology group \
                                      (§\"HTTP status codes\", the `422` row: \
                                      \"The request was well-formed but was unable \
                                      to be followed due to semantic errors\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_create_with_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_create_with_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── EHR_STATUS ────────────────────────────────────────────────────────────────

/// Retrieve an `EHR_STATUS` at a version id
/// (`GET /ehr/{ehr_id}/ehr_status/{version_uid}`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/ehr_status/{version_uid}", tag = "EHR_STATUS",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_uid" = String, Path,
         description = "VERSION identifier, taken from VERSION.uid.value — an \
                        OBJECT_VERSION_ID \
                        `{object_id}::{creating_system_id}::{version_tree_id}` \
                        (`Resources.md` §\"Identifier types\"). The addressed \
                        uid must name the served version's full three-part \
                        identity; a fabricated `creating_system_id` names no \
                        VERSION here and is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The bare EHR_STATUS at that version \
                                      (canonical JSON/XML per `Accept`; the \
                                      VERSION envelope is served by the \
                                      `versioned_ehr_status` operations). No \
                                      `Location`: `Requests_and_responses.md` \
                                      §Location forbids it on a `GET` (\"It MUST \
                                      NOT be used to indicate an alternate \
                                      representation of an existing resource\"). \
                                      The `openehr-item-tag`/\
                                      `openehr-version-item-tag` response headers \
                                      are a MAY on reads (§\"openehr-item-tag and \
                                      openehr-version-item-tag\", \"Usage in \
                                      Responses\"): this server emits them only \
                                      on the `PUT` write wrapper, never on a \
                                      `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\"; the `W/` \
                             weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "That version's commit instant as an HTTP-date — \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\"). The bare EHR_STATUS \
                             body carries no commit audit, so the instant comes \
                             from the version metadata."),
         ),
         example = json!({
             "_type": "EHR_STATUS",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                 "rm_version": "1.2.0"
             },
             "name": { "_type": "DV_TEXT", "value": "EHR Status" },
             "subject": {
                 "_type": "PARTY_SELF",
                 "external_ref": {
                     "_type": "PARTY_REF",
                     "namespace": "demographic",
                     "type": "PERSON",
                     "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                 }
             },
             "is_queryable": true,
             "is_modifiable": true
         })),
        (status = 400, description = "`ehr_id` is not a UUID, or `version_uid` is \
                                      not a well-formed OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\"). \
                                      A syntactically valid but unknown id is \
                                      `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no EHR_STATUS version \
                                      with `version_uid`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR_STATUS resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION content \
                                      only, and EHR_STATUS is not templated).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_get_by_version_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_get_by_version_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve the `EHR_STATUS` at a point in time
/// (`GET /ehr/{ehr_id}/ehr_status`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/ehr_status", tag = "EHR_STATUS",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_at_time" = Option<String>, Query,
         description = "A time in the extended ISO 8601 format; the version \
                        extant at that time is returned. Absent means the \
                        latest version. The timezone is optional — server-local \
                        when omitted (`Resources.md` §\"Datetime format\": \
                        query parameters \"MUST always use the _extended_ ISO \
                        8601 format\" and \"Timezone SHOULD be only supplied \
                        when needed, otherwise the local timezone is \
                        assumed\").",
         example = "2026-07-26T09:12:44.512Z"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The bare EHR_STATUS extant at that time \
                                      (canonical JSON/XML per `Accept`). No \
                                      `Location`: `Requests_and_responses.md` \
                                      §Location forbids it on a `GET`. The \
                                      `openehr-item-tag`/\
                                      `openehr-version-item-tag` response headers \
                                      are a MAY on reads (§\"openehr-item-tag and \
                                      openehr-version-item-tag\", \"Usage in \
                                      Responses\"): this server emits them only \
                                      on the `PUT` write wrapper, never on a \
                                      `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             version served (§\"ETag and Last-Modified\"; the \
                             `W/` weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "That version's commit instant as an HTTP-date — \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\"). The bare EHR_STATUS \
                             body carries no commit audit, so the instant comes \
                             from the version metadata."),
         ),
         example = json!({
             "_type": "EHR_STATUS",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                 "rm_version": "1.2.0"
             },
             "name": { "_type": "DV_TEXT", "value": "EHR Status" },
             "subject": {
                 "_type": "PARTY_SELF",
                 "external_ref": {
                     "_type": "PARTY_REF",
                     "namespace": "demographic",
                     "type": "PERSON",
                     "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                 }
             },
             "is_queryable": true,
             "is_modifiable": true
         })),
        (status = 400, description = "`ehr_id` is not a UUID, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 datetime (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no EHR_STATUS version \
                                      at the specified time.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR_STATUS resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION content \
                                      only, and EHR_STATUS is not templated).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_get_at_time",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Update the `EHR_STATUS` (`PUT /ehr/{ehr_id}/ehr_status`).
///
/// The committal headers `openehr-version` / `openehr-audit-details` are
/// accepted and merged into the commit
/// (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}/ehr_status", tag = "EHR_STATUS",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("If-Match" = String, Header,
         description = "The latest EHR_STATUS version uid (which becomes the new \
                        version's `preceding_version_uid`), double-quoted; the \
                        weak `W/\"…\"` form the server emits is also accepted. \
                        Required — `Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\": \"When the service expects \
                        `If-Match` for an operation, but the client does not \
                        provide it, the service SHOULD respond with `400 Bad \
                        Request`\"; a non-matching value MUST be `412`.",
         example = "W/\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — no body, `204 No Content`; \
                        `return=identifier` — the body is only \
                        `{ \"uid\": \"<new version uid>\" }` at `200 OK`, never \
                        `204` (§\"Prefer only identifier\": \"a variant of \
                        preference that implies minimal response semantics, but \
                        with a non-empty response body\"); \
                        `return=representation` — the full RM `EHR_STATUS` at \
                        `200 OK`. An absent header means `return=minimal` (\"If \
                        no `Prefer` header is provided, the default behavior is \
                        assumed to be `return=minimal`\"); the token actually \
                        applied is echoed in the `Preference-Applied` response \
                        header.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the new EHR_STATUS VERSION, as an \
                        attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. Merged with the \
                        server defaults (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\": whatever \
                        is provided \"MUST be merged with the default VERSION and \
                        VERSION.audit_details attributes on commit runtime\").\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY repeat \
                        — e.g. `description.value=\"Status corrected at \
                        triage\"`, `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` defaults to `251|modification|` and is \
                        client-overridable to any audit_change_type code \
                        consistent with an update (e.g. `250|amendment|`); \
                        `time_committed` \"is always set by the server\", and an \
                        omitted `system_id` defaults to the server's configured \
                        identifier (\"when `system_id` is not provided by the \
                        client, the server MUST set it to its own configured \
                        system identifier\")."),
        ("openehr-item-tag" = Option<String>, Header,
         description = "Item tags to set on the VERSIONED_EHR_STATUS \
                        (VERSIONED_OBJECT-level target); an empty value removes \
                        all (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", \"Usage in Requests\"). MAY \
                        be echoed back in the response header of the same name."),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "Item tags to set on the new EHR_STATUS VERSION; an empty \
                        value removes all (same section, \"Usage in Requests\"). \
                        MAY be echoed back in the response header of the same \
                        name.")
    ),
    request_body(content = serde_json::Value,
                 description = "The new EHR_STATUS, canonical JSON or XML per \
                                `Content-Type`. The Simplified Formats do not \
                                apply — EHR_STATUS is not templated.",
                 example = json!({
                     "_type": "EHR_STATUS",
                     "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                         "rm_version": "1.2.0"
                     },
                     "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                     "subject": {
                         "_type": "PARTY_SELF",
                         "external_ref": {
                             "_type": "PARTY_REF",
                             "namespace": "demographic",
                             "type": "PERSON",
                             "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                         }
                     },
                     "is_queryable": true,
                     "is_modifiable": false
                 })),
    responses(
        (status = 200, description = "Updated, with a body: the full RM \
                                      `EHR_STATUS` for \
                                      `Prefer: return=representation` (the \
                                      `representation` example) or the \
                                      single-`uid` object for \
                                      `return=identifier` (the `identifier` \
                                      example) — `Requests_and_responses.md` \
                                      §\"Prefer minimal, identifier or full \
                                      representation response\". `ETag`, \
                                      `Last-Modified` and `Location` describe the \
                                      newly committed version, and \
                                      `Preference-Applied` declares the token \
                                      honoured.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new version uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\" and \"changes \
                             as soon as the resource changes\")."),
             ("Location" = String,
              description = "The URL of the newly committed version, \
                             `<base_path>/ehr/<ehr_id>/ehr_status/<version_uid>` \
                             (§\"Prefer minimal, identifier or full \
                             representation response\": the response \"SHOULD \
                             include a `Location` header pointing to the newly \
                             created or updated resource\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the new version as an \
                             HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured (§\"Representation details \
                             negotiation\")."),
             ("openehr-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the \
                             VERSIONED_EHR_STATUS, present only when the request \
                             carried the header (§\"openehr-item-tag and \
                             openehr-version-item-tag\", \"Usage in Responses\": \
                             servers \"MAY include\" it \"to confirm the actual \
                             list of ITEM_TAGs stored on the server side\")."),
             ("openehr-version-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the new \
                             EHR_STATUS VERSION, present only when the request \
                             carried the header (same section, \"Usage in \
                             Responses\")."),
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the full RM EHR_STATUS",
              value = json!({
                  "_type": "EHR_STATUS",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                      "rm_version": "1.2.0"
                  },
                  "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                  "subject": {
                      "_type": "PARTY_SELF",
                      "external_ref": {
                          "_type": "PARTY_REF",
                          "namespace": "demographic",
                          "type": "PERSON",
                          "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                      }
                  },
                  "is_queryable": true,
                  "is_modifiable": false
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "Updated with no body — the default \
                                      `Prefer: return=minimal` \
                                      (`Requests_and_responses.md` §\"Prefer \
                                      minimal, identifier or full representation \
                                      response\": \"If no response body is \
                                      returned, the service SHOULD use `204 No \
                                      Content`\"). The version headers are \
                                      carried exactly as on the `200`.",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new version uid>\"` \
                             (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "The URL of the newly committed version, \
                             `<base_path>/ehr/<ehr_id>/ehr_status/<version_uid>` \
                             (§\"Prefer minimal, identifier or full \
                             representation response\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the new version as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (§\"Representation details \
                             negotiation\")."),
             ("openehr-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the \
                             VERSIONED_EHR_STATUS, present only when the request \
                             carried the header (§\"openehr-item-tag and \
                             openehr-version-item-tag\", \"Usage in \
                             Responses\")."),
             ("openehr-version-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the new \
                             EHR_STATUS VERSION, present only when the request \
                             carried the header (same section, \"Usage in \
                             Responses\")."),
         )),
        (status = 400, description = "`ehr_id` is not a UUID, the EHR_STATUS \
                                      payload could not be parsed, or `If-Match` \
                                      is missing/empty/not a well-formed \
                                      OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\"; \
                                      §\"If-Match and accidental overwrites\" for \
                                      the missing-`If-Match` case).",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the EHR_STATUS resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION content \
                                      only, and EHR_STATUS is not templated).",
         body = serde_json::Value),
        (status = 412, description = "`If-Match` does not name the latest \
                                      EHR_STATUS version, so the update was not \
                                      performed (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\": \
                                      the service \"MUST NOT perform the \
                                      requested method\" and \"MUST respond with \
                                      HTTP status code `412 Precondition \
                                      Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<current latest version \
                             uid>\"` — the same section's SHOULD: the service \
                             \"SHOULD return also latest `version_uid` in the \
                             `ETag` response headers\"."),
             ("Last-Modified" = String,
              description = "The commit instant of that current latest version \
                             as an HTTP-date, carried alongside the `ETag` \
                             (§\"ETag and Last-Modified\")."),
         )),
        (status = 415, description = "The request `Content-Type` is not a format \
                                      this resource can process — notably a \
                                      Simplified Format, which is defined only \
                                      for templated COMPOSITION content \
                                      (`Resources.md` §\"JSON Format\": \"If the \
                                      service cannot process the request payload \
                                      as JSON format, it MUST respond with HTTP \
                                      status code `415 Unsupported Media Type`\"; \
                                      §\"Simplified Formats\" carries the same \
                                      MUST for the simplified types).",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_update",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── VERSIONED_EHR_STATUS ──────────────────────────────────────────────────────

/// Retrieve the `VERSIONED_EHR_STATUS` container
/// (`GET /ehr/{ehr_id}/versioned_ehr_status`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/versioned_ehr_status", tag = "EHR_STATUS",
    params(("ehr_id" = String, Path,
            description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                           (`Resources.md` §\"Identifier types\").",
            example = "7d44b88c-4199-4bad-97dc-d78268e01398")),
    responses(
        (status = 200, description = "The VERSIONED_EHR_STATUS container \
                                      (canonical JSON/XML per `Accept`): the \
                                      versioned-object identity and its \
                                      `owner_id`/`time_created`, not the version \
                                      content. No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag \
                             `W/\"<versioned_object_uid>\"` (§\"ETag and \
                             Last-Modified\": the value \"is usually taken from \
                             e.g. VERSIONED_OBJECT.uid.value\"; the `W/` weakness \
                             indicator is required since Release 1.1.0)."),
             ("Last-Modified" = String,
              description = "The commit instant of the container's most recent \
                             version as an HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\": both headers \"SHOULD \
                             be included in responses for VERSION, \
                             VERSIONED_OBJECT, or other resources that have \
                             versioning or unique state identifiers\")."),
         ),
         example = json!({
             "_type": "VERSIONED_EHR_STATUS",
             "uid": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
             "owner_id": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "EHR",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
             },
             "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" }
         })),
        (status = 400, description = "`ehr_id` is not a UUID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the VERSIONED_EHR_STATUS container has only \
                                      the canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      not defined for this resource).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_ehr_status_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_ehr_status_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve the `EHR_STATUS` revision history
/// (`GET /ehr/{ehr_id}/versioned_ehr_status/revision_history`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/versioned_ehr_status/revision_history", tag = "EHR_STATUS",
    params(("ehr_id" = String, Path,
            description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                           (`Resources.md` §\"Identifier types\").",
            example = "7d44b88c-4199-4bad-97dc-d78268e01398")),
    responses(
        (status = 200, description = "The REVISION_HISTORY of the \
                                      VERSIONED_EHR_STATUS (canonical JSON/XML \
                                      per `Accept`): one REVISION_HISTORY_ITEM \
                                      per committed version, most recent LAST \
                                      (RM common \
                                      `org.openehr.rm.common.revision_history.adoc`, \
                                      `REVISION_HISTORY.items`: \"The items in \
                                      this history in most-recent-last \
                                      order\"). No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag \
                             `W/\"<versioned_object_uid>\"` of the container the \
                             history belongs to (§\"ETag and Last-Modified\": the \
                             value \"is usually taken from e.g. \
                             VERSIONED_OBJECT.uid.value\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the most recent revision as \
                             an HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "REVISION_HISTORY",
             "items": [
                 {
                     "_type": "REVISION_HISTORY_ITEM",
                     "version_id": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                     "audits": [ {
                         "_type": "AUDIT_DETAILS",
                         "system_id": "openEHRSys.example.com",
                         "committer": { "_type": "PARTY_IDENTIFIED", "name": "John Doe" },
                         "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                         "change_type": {
                             "_type": "DV_CODED_TEXT",
                             "value": "creation",
                             "defining_code": {
                                 "_type": "CODE_PHRASE",
                                 "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                                 "code_string": "249"
                             }
                         }
                     } ]
                 },
                 {
                     "_type": "REVISION_HISTORY_ITEM",
                     "version_id": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                     "audits": [ {
                         "_type": "AUDIT_DETAILS",
                         "system_id": "openEHRSys.example.com",
                         "committer": { "_type": "PARTY_IDENTIFIED", "name": "John Doe" },
                         "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                         "change_type": {
                             "_type": "DV_CODED_TEXT",
                             "value": "modification",
                             "defining_code": {
                                 "_type": "CODE_PHRASE",
                                 "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                                 "code_string": "251"
                             }
                         }
                     } ]
                 }
             ]
         })),
        (status = 400, description = "`ehr_id` is not a UUID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the REVISION_HISTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      not defined for this resource).",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_ehr_status_revision_history(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_ehr_status_revision_history",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve an `EHR_STATUS` version at a point in time
/// (`GET /ehr/{ehr_id}/versioned_ehr_status/version`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/versioned_ehr_status/version", tag = "EHR_STATUS",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_at_time" = Option<String>, Query,
         description = "A time in the extended ISO 8601 format; the VERSION \
                        extant at that time is returned. Absent means the \
                        latest VERSION. The timezone is optional — server-local \
                        when omitted (`Resources.md` §\"Datetime format\": query \
                        parameters \"MUST always use the _extended_ ISO 8601 \
                        format\" and \"Timezone SHOULD be only supplied when \
                        needed, otherwise the local timezone is assumed\").",
         example = "2026-07-26T09:12:44.512Z"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION envelope of the \
                                      EHR_STATUS extant at that time (canonical \
                                      JSON/XML per `Accept`): the version \
                                      identity, its CONTRIBUTION reference, the \
                                      commit audit, the lifecycle state, and the \
                                      EHR_STATUS itself under `data`. A version this server received from another system is served as its \
                                      IMPORTED_VERSION wrapper instead — the \
                                      local contribution and commit audit, with \
                                      the received ORIGINAL_VERSION under `item` \
                                      (RM common master06 §Committal and Audits). No \
                                      `Location`: `Requests_and_responses.md` \
                                      §Location forbids it on a `GET` (\"It MUST \
                                      NOT be used to indicate an alternate \
                                      representation of an existing \
                                      resource\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\"; the `W/` \
                             weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "The version's own \
                             `commit_audit.time_committed` as an HTTP-date — \
                             \"For openEHR resources, this value should be \
                             derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "ORIGINAL_VERSION",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "contribution": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "CONTRIBUTION",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" }
             },
             "commit_audit": {
                 "_type": "AUDIT_DETAILS",
                 "system_id": "openEHRSys.example.com",
                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "John Doe" },
                 "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                 "change_type": {
                     "_type": "DV_CODED_TEXT",
                     "value": "modification",
                     "defining_code": {
                         "_type": "CODE_PHRASE",
                         "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                         "code_string": "251"
                     }
                 }
             },
             "lifecycle_state": {
                 "_type": "DV_CODED_TEXT",
                 "value": "complete",
                 "defining_code": {
                     "_type": "CODE_PHRASE",
                     "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                     "code_string": "532"
                 }
             },
             "data": {
                 "_type": "EHR_STATUS",
                 "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                 "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                 "archetype_details": {
                     "_type": "ARCHETYPED",
                     "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                     "rm_version": "1.2.0"
                 },
                 "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                 "subject": {
                     "_type": "PARTY_SELF",
                     "external_ref": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "PERSON",
                         "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                     }
                 },
                 "is_queryable": true,
                 "is_modifiable": true
             }
         })),
        (status = 400, description = "`ehr_id` is not a UUID, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 datetime (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no VERSION at the \
                                      specified time.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the ORIGINAL_VERSION resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      not defined for this resource).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_ehr_status_version_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_ehr_status_version_get_at_time",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve an `EHR_STATUS` version by id
/// (`GET /ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}", tag = "EHR_STATUS",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_uid" = String, Path,
         description = "VERSION identifier, taken from VERSION.uid.value — an \
                        OBJECT_VERSION_ID \
                        `{object_id}::{creating_system_id}::{version_tree_id}` \
                        (`Resources.md` §\"Identifier types\"). The addressed \
                        uid must name the served version's full three-part \
                        identity; a fabricated `creating_system_id` names no \
                        VERSION here and is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION envelope of the \
                                      EHR_STATUS identified by `version_uid` \
                                      (canonical JSON/XML per `Accept`): the \
                                      version identity, its CONTRIBUTION \
                                      reference, the commit audit, the lifecycle \
                                      state, and the EHR_STATUS itself under \
                                      `data`. A version this server received from another system is served as its \
                                      IMPORTED_VERSION wrapper instead — the \
                                      local contribution and commit audit, with \
                                      the received ORIGINAL_VERSION under `item` \
                                      (RM common master06 §Committal and Audits). No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\"; the `W/` \
                             weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "The version's own \
                             `commit_audit.time_committed` as an HTTP-date — \
                             \"For openEHR resources, this value should be \
                             derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "ORIGINAL_VERSION",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "contribution": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "CONTRIBUTION",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" }
             },
             "commit_audit": {
                 "_type": "AUDIT_DETAILS",
                 "system_id": "openEHRSys.example.com",
                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "John Doe" },
                 "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                 "change_type": {
                     "_type": "DV_CODED_TEXT",
                     "value": "modification",
                     "defining_code": {
                         "_type": "CODE_PHRASE",
                         "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                         "code_string": "251"
                     }
                 }
             },
             "lifecycle_state": {
                 "_type": "DV_CODED_TEXT",
                 "value": "complete",
                 "defining_code": {
                     "_type": "CODE_PHRASE",
                     "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                     "code_string": "532"
                 }
             },
             "data": {
                 "_type": "EHR_STATUS",
                 "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                 "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                 "archetype_details": {
                     "_type": "ARCHETYPED",
                     "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                     "rm_version": "1.2.0"
                 },
                 "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                 "subject": {
                     "_type": "PARTY_SELF",
                     "external_ref": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "PERSON",
                         "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                     }
                 },
                 "is_queryable": true,
                 "is_modifiable": true
             }
         })),
        (status = 400, description = "`ehr_id` is not a UUID, or `version_uid` is \
                                      not a well-formed OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\"). \
                                      A syntactically valid but unknown id is \
                                      `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no VERSION with \
                                      `version_uid`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the ORIGINAL_VERSION resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      not defined for this resource).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_ehr_status_version_get_by_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_ehr_status_version_get_by_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── COMPOSITION ───────────────────────────────────────────────────────────────

/// Create the first version of a COMPOSITION
/// (`POST /ehr/{ehr_id}/composition`).
///
/// The committal headers `openehr-version` / `openehr-audit-details` are
/// accepted and merged into the creating CONTRIBUTION and its COMPOSITION
/// version (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    post, path = "/ehr/{ehr_id}/composition", tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — empty body; `return=identifier` — the \
                        body is only `{ \"uid\": \"<new version uid>\" }`; \
                        `return=representation` — the committed COMPOSITION. An \
                        absent header means `return=minimal` (\"If no `Prefer` \
                        header is provided, the default behavior is assumed to be \
                        `return=minimal`\"); the token actually applied is echoed \
                        in the `Preference-Applied` response header. A \
                        Simplified-Format `Accept` always answers with the \
                        committed COMPOSITION in that form — the `Accept` decides \
                        the body there, not `Prefer` — so the applied preference \
                        is then `return=representation` whatever was asked for.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the COMPOSITION VERSION this \
                        creation commits, as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"553\"`, which commits the \
                        version as `553|incomplete|` and relaxes the template's \
                        lower-cardinality limits (RM common master06 §\"Version \
                        Lifecycle\"). The default is `532|complete|`. Merged with \
                        the server defaults (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\": whatever \
                        is provided \"MUST be merged with the default VERSION and \
                        VERSION.audit_details attributes on commit runtime\").\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this \
                        creation commits, as an attribute-path list; the header \
                        MAY repeat — e.g. `description.value=\"Encounter \
                        recorded at triage\"`, `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` is constrained to `249|creation|` (a \
                        create commits a first version — RM common master06 \
                        §Contributions); `time_committed` \"is always set by the \
                        server\", and an omitted `system_id` defaults to the \
                        server's configured identifier (\"when `system_id` is not \
                        provided by the client, the server MUST set it to its own \
                        configured system identifier\")."),
        ("openehr-template-id" = Option<String>, Header,
         description = "The operational-template id the committed content is \
                        validated against. `Requests_and_responses.md` \
                        §openehr-template-id: the header \"MUST be used whenever \
                        committing COMPOSITION (via `PUT` or `POST` methods) \
                        using a Simplified Format which does not support \
                        TEMPLATE_ID value under an equivalent \
                        `LOCATABLE.archetype_details.template_id` attribute of \
                        contained data\". A canonical JSON/XML body carries its \
                        own `archetype_details.template_id`, so the header is not \
                        needed there; a `application/openehr.wt.flat+json` / \
                        `…structured+json` commit without it cannot be resolved \
                        to a template and is `422` (`Requests_and_responses.md` \
                        §\"HTTP status codes\", the `422` row — the released text \
                        assigns no status to the missing header, so the code \
                        choice is ours).",
         example = "problem_list.v1"),
        ("openehr-item-tag" = Option<String>, Header,
         description = "Item tags to set on the VERSIONED_COMPOSITION \
                        (VERSIONED_OBJECT-level target); an empty value removes \
                        all (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", \"Usage in Requests\"). MAY \
                        be echoed back in the response header of the same name."),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "Item tags to set on the new COMPOSITION VERSION; an empty \
                        value removes all (same section, \"Usage in Requests\"). \
                        MAY be echoed back in the response header of the same \
                        name.")
    ),
    request_body(
        // COMPOSITION content negotiates canonical JSON/XML + the two Simplified
        // Formats (`Resources.md` §"Data representation": a service "MUST
        // support at least one of the openEHR XML or JSON canonical formats" and
        // the Simplified Formats "SHOULD be supported"; §"Simplified Formats"
        // fixes the two media types below).
        content(
            (serde_json::Value = "application/json", example = json!({
                "_type": "COMPOSITION",
                "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                    "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                    "rm_version": "1.2.0"
                },
                "name": { "_type": "DV_TEXT", "value": "Encounter" },
                "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                "category": {
                    "_type": "DV_CODED_TEXT", "value": "event",
                    "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                },
                "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                "context": {
                    "_type": "EVENT_CONTEXT",
                    "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                    "setting": {
                        "_type": "DV_CODED_TEXT", "value": "other care",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                    }
                },
                "content": [ {
                    "_type": "EVALUATION",
                    "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                    "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                    "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                    "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                    "subject": { "_type": "PARTY_SELF" },
                    "data": {
                        "_type": "ITEM_TREE",
                        "archetype_node_id": "at0001",
                        "name": { "_type": "DV_TEXT", "value": "Tree" },
                        "items": [ {
                            "_type": "ELEMENT",
                            "archetype_node_id": "at0002",
                            "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                            "value": { "_type": "DV_TEXT", "value": "Asthma" }
                        } ]
                    }
                } ]
            })),
            (serde_json::Value = "application/xml"),
            (serde_json::Value = "application/openehr.wt.flat+json"),
            (serde_json::Value = "application/openehr.wt.structured+json")
        ),
        description = "The COMPOSITION, in canonical JSON/XML or one of the two \
                       Simplified Formats per `Content-Type` (`Resources.md` \
                       §\"Simplified Formats\": \
                       `application/openehr.wt.flat+json` for Simplified Flat \
                       JSON, `application/openehr.wt.structured+json` for \
                       Simplified Structured JSON). The example is canonical \
                       JSON; a simplified body additionally requires the \
                       `openehr-template-id` request header."
    ),
    responses(
        (
            status = 201, description = "Created. `ETag` (weak `W/` form) carries \
                                        the new version uid, `Last-Modified` its \
                                        commit instant, `Location` the COMPOSITION \
                                        version URL, and `Preference-Applied` the \
                                        `Prefer` token actually honoured. The body \
                                        is `Prefer`-conditional \
                                        (`Requests_and_responses.md` §\"Prefer \
                                        minimal, identifier or full representation \
                                        response\"): the committed COMPOSITION for \
                                        `return=representation` (the \
                                        `representation` example), the \
                                        single-`uid` object for \
                                        `return=identifier` (the `identifier` \
                                        example), and no body at all for the \
                                        default `return=minimal`. The response \
                                        representation is negotiated across the \
                                        same four media types as the request.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<new version uid>\"` \
                                (§\"ETag and Last-Modified\": the value \"is \
                                usually taken from e.g. … VERSION.uid.value\"; the \
                                `W/` weakness indicator is required since Release \
                                1.1.0)."),
                ("Location" = String,
                 description = "The URL of the newly created COMPOSITION version, \
                                `<base_path>/ehr/<ehr_id>/composition/<version_uid>` \
                                (§Location: used \"in `201 Created` responses when \
                                a new resource is successfully created\")."),
                ("Last-Modified" = String,
                 description = "The creating CONTRIBUTION's commit instant as an \
                                HTTP-date — \"derived from \
                                VERSION.commit_audit.time_committed.value\" \
                                (§\"ETag and Last-Modified\")."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured (§\"Representation details \
                                negotiation\")."),
                ("openehr-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the \
                                VERSIONED_COMPOSITION, present only when the \
                                request carried the header (§\"openehr-item-tag \
                                and openehr-version-item-tag\", \"Usage in \
                                Responses\": servers \"MAY include\" it \"to \
                                confirm the actual list of ITEM_TAGs stored on the \
                                server side\")."),
                ("openehr-version-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the new \
                                COMPOSITION VERSION, present only when the request \
                                carried the header (same section, \"Usage in \
                                Responses\")."),
            ),
            content(
                (serde_json::Value = "application/json", examples(
                    ("representation" = (summary = "Prefer: return=representation — the committed COMPOSITION",
                     value = json!({
                        "_type": "COMPOSITION",
                        "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" },
                        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                        "archetype_details": {
                            "_type": "ARCHETYPED",
                            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                            "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                            "rm_version": "1.2.0"
                        },
                        "name": { "_type": "DV_TEXT", "value": "Encounter" },
                        "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                        "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                        "category": {
                            "_type": "DV_CODED_TEXT", "value": "event",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                        },
                        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                        "context": {
                            "_type": "EVENT_CONTEXT",
                            "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                            "setting": {
                                "_type": "DV_CODED_TEXT", "value": "other care",
                                "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                            }
                        },
                        "content": [ {
                            "_type": "EVALUATION",
                            "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                            "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                            "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                            "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                            "subject": { "_type": "PARTY_SELF" },
                            "data": {
                                "_type": "ITEM_TREE",
                                "archetype_node_id": "at0001",
                                "name": { "_type": "DV_TEXT", "value": "Tree" },
                                "items": [ {
                                    "_type": "ELEMENT",
                                    "archetype_node_id": "at0002",
                                    "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                                    "value": { "_type": "DV_TEXT", "value": "Asthma" }
                                } ]
                            }
                        } ]
                     }))),
                    ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
                     value = json!({ "uid": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" })))
                )),
                (serde_json::Value = "application/xml"),
                (serde_json::Value = "application/openehr.wt.flat+json"),
                (serde_json::Value = "application/openehr.wt.structured+json")
            )
        ),
        (status = 400, description = "`ehr_id` is not a UUID, the COMPOSITION \
                                      payload could not be parsed, or a committal \
                                      `change_type` names a legal \
                                      audit_change_type code that contradicts a \
                                      creation (only `249|creation|` is compatible \
                                      — RM common master06 §Contributions) \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      COMPOSITION is served as canonical \
                                      `application/json` / `application/xml` or as \
                                      `application/openehr.wt.flat+json` / \
                                      `application/openehr.wt.structured+json` \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\"/§\"Simplified Formats\": \"If the \
                                      service cannot fulfill this aspect of the \
                                      request, it MUST respond with HTTP status \
                                      code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 409, description = "The creation conflicts with the EHR's \
                                      current state (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `409` row: the \
                                      request \"might generate a duplicate or a \
                                      conflict\"). Two triggers, both of which are \
                                      OUR OWN DESIGN — the released text assigns \
                                      neither a status code: (1) the EHR already \
                                      holds a live persistent COMPOSITION for the \
                                      same template (no released text defines a \
                                      uniqueness rule for persistent \
                                      COMPOSITIONs — RM ehr master05 §\"Persistent \
                                      Compositions\" and the COMPOSITION \
                                      invariants are silent; the conformance \
                                      catalogue carries it as an ambiguity-register \
                                      entry, reported and excluded from profile \
                                      computation); (2) the EHR is not modifiable \
                                      (`EHR_STATUS.is_modifiable = false`, RM ehr \
                                      master04 §\"EHR Active Status\": the flag \
                                      \"is used to indicate whether the contents \
                                      of an EHR are modifiable\" — the refusal is \
                                      spec-required, but the status choice is \
                                      ours).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one this \
                                      resource can process — a media type outside \
                                      canonical JSON/XML and the two Simplified \
                                      Formats, or a deprecated \
                                      `…schema+json` variant (`Resources.md` \
                                      §\"Simplified Formats\": \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\"; §\"XML \
                                      Format\"/§\"JSON Format\" carry the same \
                                      MUST for the canonical types).",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed and converted to \
                                      a COMPOSITION, but cannot be followed: the \
                                      named operational template is unknown or \
                                      does not validate the content, an RM \
                                      class-invariant or terminology-binding check \
                                      failed, a committal \
                                      `change_type`/`lifecycle_state` is not a \
                                      member of its openEHR terminology group, or \
                                      a Simplified-Format body arrived without the \
                                      `openehr-template-id` header \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `422` row: \"The request was \
                                      well-formed but was unable to be followed \
                                      due to semantic errors\"; ITS-REST \
                                      `specifications/responses/422.yaml`: \"the \
                                      underlying template is not known or is not \
                                      validating the supplied resource\").",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_create",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve a COMPOSITION
/// (`GET /ehr/{ehr_id}/composition/{uid_based_id}`).
///
/// The one route that accepts BOTH identifier forms (`Resources.md`
/// §"Multiple identifiers for the same resource"): an `OBJECT_VERSION_ID`
/// serves that exact version, a `HIER_OBJECT_ID` container id serves the latest
/// version or the one extant at `version_at_time`. The served body is the
/// BARE COMPOSITION; the `ORIGINAL_VERSION` envelope is served by the
/// `versioned_composition` operations below.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/composition/{uid_based_id}", tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The COMPOSITION address, in either of the two forms \
                        `Resources.md` §\"Multiple identifiers for the same \
                        resource\" defines for this resource. As an \
                        **explicit version reference** it is an \
                        OBJECT_VERSION_ID taken from VERSION.uid.value \
                        (`{object_id}::{creating_system_id}::{version_tree_id}`) \
                        and serves THAT version — including a superseded, \
                        non-current one; the addressed uid must name the served \
                        version's full three-part identity, so a fabricated \
                        `creating_system_id` names no VERSION here and is `404`. \
                        As an **implicit latest version reference** it is a \
                        HIER_OBJECT_ID taken from VERSIONED_OBJECT.uid.value \
                        (the version container) and serves the version extant at \
                        `version_at_time`, or the latest when that parameter is \
                        absent. `version_at_time` applies only to the container \
                        form; combined with an explicit version reference the \
                        version addressing wins and the parameter is ignored (the \
                        released text defines no combination).",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2"),
        ("version_at_time" = Option<String>, Query,
         description = "A time in the extended ISO 8601 format; the version \
                        extant at that time is returned. Applies to the \
                        container (HIER_OBJECT_ID) form of `uid_based_id`; absent \
                        means the latest version. The timezone is optional — \
                        server-local when omitted (`Resources.md` §\"Datetime \
                        format\": query parameters \"MUST always use the \
                        _extended_ ISO 8601 format\" and \"Timezone SHOULD be \
                        only supplied when needed, otherwise the local timezone \
                        is assumed\").",
         example = "2026-07-26T09:12:44.512Z"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (
            status = 200, description = "The COMPOSITION, in the representation \
                                        negotiated from `Accept` (canonical \
                                        JSON/XML or one of the two Simplified \
                                        Formats — `Resources.md` §\"Simplified \
                                        Formats\"). The version-identity headers \
                                        are representation-independent: \"the \
                                        `ETag` value is independent of its \
                                        resource serialization format \
                                        (JSON/XML)\" (§\"ETag and \
                                        Last-Modified\"). No `Location`: \
                                        `Requests_and_responses.md` §Location \
                                        forbids it on a `GET` (\"It MUST NOT be \
                                        used to indicate an alternate \
                                        representation of an existing \
                                        resource\"). The `openehr-item-tag`/\
                                        `openehr-version-item-tag` response \
                                        headers are a MAY on reads (§\"openehr-\
                                        item-tag and openehr-version-item-tag\", \
                                        \"Usage in Responses\"): this server \
                                        emits them only on the `POST`/`PUT` write \
                                        wrappers, never on a `GET`.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<version uid>\"` of the \
                                version actually served — the addressed one for \
                                an explicit version reference, the resolved \
                                latest/extant-at one for the container form \
                                (§\"ETag and Last-Modified\"; the `W/` weakness \
                                indicator is required since Release 1.1.0)."),
                ("Last-Modified" = String,
                 description = "That version's commit instant as an HTTP-date — \
                                \"derived from \
                                VERSION.commit_audit.time_committed.value\" \
                                (§\"ETag and Last-Modified\"). The bare \
                                COMPOSITION body carries no commit audit, so the \
                                instant comes from the version metadata."),
            ),
            content(
                (serde_json::Value = "application/json", example = json!({
                    "_type": "COMPOSITION",
                    "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
                    "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                    "archetype_details": {
                        "_type": "ARCHETYPED",
                        "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                        "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                        "rm_version": "1.2.0"
                    },
                    "name": { "_type": "DV_TEXT", "value": "Encounter" },
                    "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                    "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                    "category": {
                        "_type": "DV_CODED_TEXT", "value": "event",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                    },
                    "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                    "context": {
                        "_type": "EVENT_CONTEXT",
                        "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                        "setting": {
                            "_type": "DV_CODED_TEXT", "value": "other care",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                        }
                    },
                    "content": [ {
                        "_type": "EVALUATION",
                        "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                        "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                        "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                        "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                        "subject": { "_type": "PARTY_SELF" },
                        "data": {
                            "_type": "ITEM_TREE",
                            "archetype_node_id": "at0001",
                            "name": { "_type": "DV_TEXT", "value": "Tree" },
                            "items": [ {
                                "_type": "ELEMENT",
                                "archetype_node_id": "at0002",
                                "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                                "value": { "_type": "DV_TEXT", "value": "Asthma" }
                            } ]
                        }
                    } ]
                })),
                (serde_json::Value = "application/xml"),
                (serde_json::Value = "application/openehr.wt.flat+json"),
                (serde_json::Value = "application/openehr.wt.structured+json")
            )
        ),
        (status = 204, description = "The addressed COMPOSITION version is \
                                      logically deleted, so there is no \
                                      representation to serve: a logical delete \
                                      commits a new version whose `data` is \
                                      removed and whose `lifecycle_state` is \
                                      `523|deleted|` (RM common master06 \
                                      §\"Logical Deletion\": \"create a new \
                                      Version …; delete its `data` …; set the \
                                      `lifecycle_state` value to the code for \
                                      `deleted`\"), and RM common \
                                      `org.openehr.rm.common.version` types \
                                      `ORIGINAL_VERSION.data` `0..1`. This server \
                                      answers `204` for ALL THREE addressing \
                                      forms — the latest version, the version \
                                      extant at `version_at_time`, and an \
                                      explicit `version_uid` naming a deleted \
                                      version. Only the middle one is covered by \
                                      released text (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`: \
                                      \"`204 No Content` is returned when the \
                                      resource identified by the request \
                                      parameters (at specified `version_at_time`) \
                                      time has been deleted\"); the explicit-\
                                      version and implicit-latest branches have no \
                                      released branch assignment at all, so \
                                      extending the same outcome to them is OUR \
                                      OWN reading, carried as an \
                                      ambiguity-register entry in the conformance \
                                      catalogue. The VERSION envelope routes below \
                                      answer differently — `200` with a data-less \
                                      `523|deleted|` ORIGINAL_VERSION."),
        (status = 400, description = "`ehr_id` is not a UUID, `uid_based_id` is \
                                      neither a well-formed OBJECT_VERSION_ID nor \
                                      a UUID version-container id, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 datetime (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\"). A syntactically valid \
                                      but unknown id is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`; no COMPOSITION with \
                                      `uid_based_id` in this EHR; no version at \
                                      the requested `version_at_time`; or an \
                                      explicit `version_uid` whose full \
                                      three-part identity names no stored VERSION \
                                      (a fabricated `creating_system_id`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      COMPOSITION is served as canonical \
                                      `application/json` / `application/xml` or as \
                                      `application/openehr.wt.flat+json` / \
                                      `application/openehr.wt.structured+json` \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\"/§\"Simplified Formats\": \"If the \
                                      service cannot fulfill this aspect of the \
                                      request, it MUST respond with HTTP status \
                                      code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Update a COMPOSITION
/// (`PUT /ehr/{ehr_id}/composition/{uid_based_id}`).
///
/// Addresses the version CONTAINER and takes the superseded version in a
/// mandatory `If-Match`, so — unlike the `DELETE` — the precondition is not
/// in the path (`Requests_and_responses.md` §"If-Match and accidental
/// overwrites"). The committal headers `openehr-version` /
/// `openehr-audit-details` are accepted and merged into the commit
/// (§openehr-version-and-audit-details).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}/composition/{uid_based_id}", tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The version container to commit the new version into — a \
                        HIER_OBJECT_ID taken from VERSIONED_OBJECT.uid.value \
                        (`Resources.md` §\"Identifier types\"). Unlike the `GET`, \
                        the update takes ONLY this form; the version being \
                        superseded travels in `If-Match`. A body \
                        `COMPOSITION.uid`, if present, must identify the same \
                        versioned object — its `object_id` part must equal this \
                        segment (RM common master06 §\"Version Identification\": \
                        the `object_id()` part \"is a copy of the `uid` of the \
                        owning VERSIONED_OBJECT\"); a mismatch is `400`, never a \
                        silent write to the path's object.",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21"),
        ("If-Match" = String, Header,
         description = "The latest COMPOSITION version uid (which becomes the new \
                        version's `preceding_version_uid`), double-quoted; the \
                        weak `W/\"…\"` form the server emits is also accepted. \
                        Required — `Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\": the header is required \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\", which is exactly this operation; \"When the \
                        service expects `If-Match` for an operation, but the \
                        client does not provide it, the service SHOULD respond \
                        with `400 Bad Request`\", and a non-matching value MUST be \
                        `412`.",
         example = "W/\"df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — no body, `204 No Content`; \
                        `return=identifier` — the body is only \
                        `{ \"uid\": \"<new version uid>\" }` at `200 OK`, never \
                        `204` (§\"Prefer only identifier\": \"a variant of \
                        preference that implies minimal response semantics, but \
                        with a non-empty response body\"); `return=representation` \
                        — the committed COMPOSITION at `200 OK`. An absent header \
                        means `return=minimal`; the token actually applied is \
                        echoed in `Preference-Applied`. A Simplified-Format \
                        `Accept` always answers with the committed COMPOSITION in \
                        that form, so the applied preference is then \
                        `return=representation` whatever was asked for.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the new COMPOSITION VERSION, as an \
                        attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"553\"` (`553|incomplete|` \
                        relaxes the template's lower-cardinality limits, RM common \
                        master06 §\"Version Lifecycle\"); the default is \
                        `532|complete|`. Merged with the server defaults \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\": whatever is provided \"MUST be \
                        merged with the default VERSION and \
                        VERSION.audit_details attributes on commit runtime\").\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY repeat \
                        — e.g. `description.value=\"Diagnosis corrected\"`, \
                        `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` defaults to `251|modification|` and is \
                        client-overridable to any audit_change_type code \
                        consistent with a content-carrying new version of an \
                        existing object — `250|amendment|`, `252|synthesis|`, \
                        `253|unknown|`, `816|restoration|`, \
                        `817|format conversion|` — but never `249|creation|`, \
                        `523|deleted|` or `666|attestation|`, which belong to \
                        other operations (RM common master06 §Contributions); an \
                        incompatible legal code is `400`. `time_committed` \"is always set by the \
                        server\", and an omitted `system_id` defaults to the \
                        server's configured identifier."),
        ("openehr-template-id" = Option<String>, Header,
         description = "The operational-template id the committed content is \
                        validated against. `Requests_and_responses.md` \
                        §openehr-template-id: the header \"MUST be used whenever \
                        committing COMPOSITION (via `PUT` or `POST` methods) \
                        using a Simplified Format which does not support \
                        TEMPLATE_ID value under an equivalent \
                        `LOCATABLE.archetype_details.template_id` attribute of \
                        contained data\". A canonical JSON/XML body carries its \
                        own `archetype_details.template_id`; a simplified commit \
                        without the header is `422`.",
         example = "problem_list.v1"),
        ("openehr-item-tag" = Option<String>, Header,
         description = "Item tags to set on the VERSIONED_COMPOSITION \
                        (VERSIONED_OBJECT-level target); an empty value removes \
                        all (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", \"Usage in Requests\"). MAY \
                        be echoed back in the response header of the same name."),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "Item tags to set on the new COMPOSITION VERSION; an empty \
                        value removes all (same section, \"Usage in Requests\"). \
                        MAY be echoed back in the response header of the same \
                        name.")
    ),
    request_body(
        content(
            (serde_json::Value = "application/json", example = json!({
                "_type": "COMPOSITION",
                "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" },
                "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                    "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                    "rm_version": "1.2.0"
                },
                "name": { "_type": "DV_TEXT", "value": "Encounter" },
                "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                "category": {
                    "_type": "DV_CODED_TEXT", "value": "event",
                    "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                },
                "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                "context": {
                    "_type": "EVENT_CONTEXT",
                    "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                    "setting": {
                        "_type": "DV_CODED_TEXT", "value": "other care",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                    }
                },
                "content": [ {
                    "_type": "EVALUATION",
                    "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                    "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                    "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                    "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                    "subject": { "_type": "PARTY_SELF" },
                    "data": {
                        "_type": "ITEM_TREE",
                        "archetype_node_id": "at0001",
                        "name": { "_type": "DV_TEXT", "value": "Tree" },
                        "items": [ {
                            "_type": "ELEMENT",
                            "archetype_node_id": "at0002",
                            "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                            "value": { "_type": "DV_TEXT", "value": "Asthma — well controlled" }
                        } ]
                    }
                } ]
            })),
            (serde_json::Value = "application/xml"),
            (serde_json::Value = "application/openehr.wt.flat+json"),
            (serde_json::Value = "application/openehr.wt.structured+json")
        ),
        description = "The new COMPOSITION, in canonical JSON/XML or one of the \
                       two Simplified Formats per `Content-Type` (`Resources.md` \
                       §\"Simplified Formats\"). A body `uid` is optional; when \
                       present its `object_id` part must equal the path \
                       `uid_based_id`. A simplified body additionally requires the \
                       `openehr-template-id` request header."
    ),
    responses(
        (
            status = 200, description = "Updated, with a body: the committed \
                                        COMPOSITION for \
                                        `Prefer: return=representation` (the \
                                        `representation` example) or the \
                                        single-`uid` object for \
                                        `return=identifier` (the `identifier` \
                                        example) — `Requests_and_responses.md` \
                                        §\"Prefer minimal, identifier or full \
                                        representation response\". `ETag`, \
                                        `Last-Modified` and `Location` describe \
                                        the newly committed version, and \
                                        `Preference-Applied` declares the token \
                                        honoured.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<new version uid>\"` \
                                (§\"ETag and Last-Modified\": the value \"is \
                                usually taken from e.g. … VERSION.uid.value\" and \
                                \"changes as soon as the resource changes\")."),
                ("Location" = String,
                 description = "The URL of the newly committed version, \
                                `<base_path>/ehr/<ehr_id>/composition/<version_uid>` \
                                (§\"Prefer minimal, identifier or full \
                                representation response\": the response \"SHOULD \
                                include a `Location` header pointing to the newly \
                                created or updated resource\")."),
                ("Last-Modified" = String,
                 description = "The commit instant of the new version as an \
                                HTTP-date — \"derived from \
                                VERSION.commit_audit.time_committed.value\" \
                                (§\"ETag and Last-Modified\")."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured (§\"Representation details \
                                negotiation\")."),
                ("openehr-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the \
                                VERSIONED_COMPOSITION, present only when the \
                                request carried the header (§\"openehr-item-tag \
                                and openehr-version-item-tag\", \"Usage in \
                                Responses\")."),
                ("openehr-version-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the new \
                                COMPOSITION VERSION, present only when the request \
                                carried the header (same section, \"Usage in \
                                Responses\")."),
            ),
            content(
                (serde_json::Value = "application/json", examples(
                    ("representation" = (summary = "Prefer: return=representation — the committed COMPOSITION",
                     value = json!({
                        "_type": "COMPOSITION",
                        "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
                        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                        "archetype_details": {
                            "_type": "ARCHETYPED",
                            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                            "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                            "rm_version": "1.2.0"
                        },
                        "name": { "_type": "DV_TEXT", "value": "Encounter" },
                        "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                        "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                        "category": {
                            "_type": "DV_CODED_TEXT", "value": "event",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                        },
                        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                        "context": {
                            "_type": "EVENT_CONTEXT",
                            "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                            "setting": {
                                "_type": "DV_CODED_TEXT", "value": "other care",
                                "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                            }
                        },
                        "content": [ {
                            "_type": "EVALUATION",
                            "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                            "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                            "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                            "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                            "subject": { "_type": "PARTY_SELF" },
                            "data": {
                                "_type": "ITEM_TREE",
                                "archetype_node_id": "at0001",
                                "name": { "_type": "DV_TEXT", "value": "Tree" },
                                "items": [ {
                                    "_type": "ELEMENT",
                                    "archetype_node_id": "at0002",
                                    "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                                    "value": { "_type": "DV_TEXT", "value": "Asthma — well controlled" }
                                } ]
                            }
                        } ]
                     }))),
                    ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
                     value = json!({ "uid": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" })))
                )),
                (serde_json::Value = "application/xml"),
                (serde_json::Value = "application/openehr.wt.flat+json"),
                (serde_json::Value = "application/openehr.wt.structured+json")
            )
        ),
        (status = 204, description = "Updated with no body — the default \
                                      `Prefer: return=minimal` \
                                      (`Requests_and_responses.md` §\"Prefer \
                                      minimal, identifier or full representation \
                                      response\": \"If no response body is \
                                      returned, the service SHOULD use `204 No \
                                      Content`\"). The version headers are carried \
                                      exactly as on the `200`.",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new version uid>\"` \
                             (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "The URL of the newly committed version, \
                             `<base_path>/ehr/<ehr_id>/composition/<version_uid>` \
                             (§\"Prefer minimal, identifier or full \
                             representation response\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the new version as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (§\"Representation details \
                             negotiation\")."),
             ("openehr-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the \
                             VERSIONED_COMPOSITION, present only when the request \
                             carried the header (§\"openehr-item-tag and \
                             openehr-version-item-tag\", \"Usage in \
                             Responses\")."),
             ("openehr-version-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the new \
                             COMPOSITION VERSION, present only when the request \
                             carried the header (same section, \"Usage in \
                             Responses\")."),
         )),
        (status = 400, description = "`ehr_id` is not a UUID, `uid_based_id` is \
                                      not a well-formed version-container id, the \
                                      COMPOSITION payload could not be parsed, \
                                      `If-Match` is missing/empty/not a \
                                      well-formed OBJECT_VERSION_ID, or a \
                                      committal `change_type` names a legal \
                                      audit_change_type code that contradicts an \
                                      update (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\"; §\"If-Match and accidental \
                                      overwrites\" for the missing-`If-Match` \
                                      case). A body `COMPOSITION.uid` naming a \
                                      different versioned object than the path is \
                                      a semantic 422, not a 400 (no released \
                                      sentence assigns the rejection — our \
                                      adjudicated handling).",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`; no COMPOSITION with \
                                      `uid_based_id` in this EHR; or the \
                                      COMPOSITION's latest version is already \
                                      logically deleted, so there is nothing to \
                                      supersede (RM common master06 §\"Logical \
                                      Deletion\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      COMPOSITION is served as canonical \
                                      `application/json` / `application/xml` or as \
                                      `application/openehr.wt.flat+json` / \
                                      `application/openehr.wt.structured+json` \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\"/§\"Simplified Formats\": an \
                                      unfulfillable `Accept` MUST be `406`).",
         body = serde_json::Value),
        (status = 409, description = "The EHR is not modifiable \
                                      (`EHR_STATUS.is_modifiable = false`), so its \
                                      contents cannot be updated (RM ehr master04 \
                                      §\"EHR Active Status\": the flag \"is used to \
                                      indicate whether the contents of an EHR are \
                                      modifiable\"). The refusal is spec-required; \
                                      the status code is OUR OWN DESIGN — no \
                                      released ITS-REST text assigns a branch to \
                                      it — chosen for the `409` row's \"conflict\" \
                                      meaning (§\"HTTP status codes\").",
         body = serde_json::Value),
        (status = 412, description = "`If-Match` does not name the latest \
                                      COMPOSITION version, so the update was not \
                                      performed (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\": the \
                                      service \"MUST NOT perform the requested \
                                      method\" and \"MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<current latest version \
                             uid>\"` — the same section's SHOULD: the service \
                             \"SHOULD return also latest `version_uid` in the \
                             `ETag` response headers\"."),
             ("Last-Modified" = String,
              description = "The commit instant of that current latest version as \
                             an HTTP-date, carried alongside the `ETag` \
                             (§\"ETag and Last-Modified\")."),
         )),
        (status = 415, description = "The request `Content-Type` is not one this \
                                      resource can process — a media type outside \
                                      canonical JSON/XML and the two Simplified \
                                      Formats (`Resources.md` §\"Simplified \
                                      Formats\": \"If the service cannot process \
                                      the request payload as the simplified format \
                                      is not supported, it MUST respond with HTTP \
                                      status code `415 Unsupported Media Type`\"; \
                                      §\"XML Format\"/§\"JSON Format\" carry the \
                                      same MUST for the canonical types).",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed and converted to \
                                      a COMPOSITION, but cannot be followed: the \
                                      named operational template is unknown or \
                                      does not validate the content, an RM \
                                      class-invariant or terminology-binding check \
                                      failed, a committal `lifecycle_state` is not \
                                      a member of its openEHR terminology group, a \
                                      Simplified-Format body arrived without the \
                                      `openehr-template-id` header, the body's \
                                      `COMPOSITION.uid` names a different \
                                      versioned object than the request path (a \
                                      well-formed body whose contradiction with \
                                      the URL cannot be followed — no released \
                                      sentence assigns this rejection; our \
                                      adjudicated handling), or the body \
                                      declares a DIFFERENT `template_id` than the \
                                      stored composition it supersedes \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `422` row; ITS-REST \
                                      `specifications/responses/422.yaml`: \"the \
                                      underlying template is not known or is not \
                                      validating the supplied resource\"). The \
                                      template-change rejection specifically is \
                                      OUR OWN CONVENTION: no released text forbids \
                                      changing `archetype_details.template_id` \
                                      across the versions of one \
                                      VERSIONED_COMPOSITION, and the conformance \
                                      catalogue carries the rule as a \
                                      register-backed, reported-only expectation.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_update",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Delete a COMPOSITION
/// (`DELETE /ehr/{ehr_id}/composition/{uid_based_id}`).
///
/// Deletion is LOGICAL: a new `523|deleted|` version with no `data` is
/// committed (RM common master06 §"Logical Deletion"), and the `204` reports
/// THAT version in `ETag`/`Last-Modified`. The path segment carries the
/// superseded version's uid, which is why no `If-Match` is taken
/// (`Requests_and_responses.md` §"If-Match and accidental overwrites" — the
/// header is required only when the `preceding_version_uid` is not a path
/// segment).
/// The committal headers `openehr-version` / `openehr-audit-details` are
/// accepted and merged into the deletion commit
/// (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    delete, path = "/ehr/{ehr_id}/composition/{uid_based_id}", tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The LATEST version's uid — an OBJECT_VERSION_ID taken \
                        from VERSION.uid.value, the `preceding_version_uid` the \
                        deletion supersedes (`Resources.md` §\"Identifier \
                        types\"). A bare HIER_OBJECT_ID container id is `400`. \
                        Because the preceding version travels IN THE PATH, this \
                        operation takes no `If-Match`: \
                        `Requests_and_responses.md` §\"If-Match and accidental \
                        overwrites\" requires the header only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" — the path IS the precondition here, and a \
                        non-latest uid is `409`, not `412`.",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2"),
        ("openehr-version" = Option<String>, Header,
         description = "Accepted for uniformity — `Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\" requires \
                        the committal headers on `PUT`, `POST` **and** `DELETE`. \
                        The deleting version's `lifecycle_state` is not client-\
                        selectable, though: a logical deletion sets it to \
                        `523|deleted|` by definition (RM common master06 \
                        §\"Logical Deletion\"). A supplied `lifecycle_state` \
                        naming any OTHER state contradicts the operation and is \
                        refused `400` rather than silently discarded — the same \
                        section makes the merge a MUST, and a value that cannot \
                        be merged is reported, not dropped; `523|deleted|` \
                        itself is accepted as the redundant statement it is."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this \
                        deletion commits, as an attribute-path list; the header \
                        MAY repeat — e.g. `description.value=\"Committed to the \
                        wrong record\"`, `committer.name=\"John Doe\"`, \
                        `system_id=\"example.openehr.systemid\"`. `change_type` \
                        is constrained to `523|deleted|` (RM common master06 \
                        §Contributions: a deletion commits a deleted version); \
                        `time_committed` is always server-set, and an omitted \
                        `system_id` defaults to the server's configured \
                        identifier.")
    ),
    responses(
        (status = 204, description = "Logically deleted. Deletion is never \
                                      physical: a NEW version is committed whose \
                                      `data` is removed and whose \
                                      `lifecycle_state` — and whose \
                                      `commit_audit.change_type` — is \
                                      `523|deleted|` (RM common master06 \
                                      §\"Logical Deletion\": \"create a new \
                                      Version in the normal way; delete its \
                                      `data`…; set the `lifecycle_state` value to \
                                      the code for `deleted`; commit in the normal \
                                      way\"), inside a new CONTRIBUTION. No \
                                      `Location`: `Requests_and_responses.md` \
                                      §\"HTTP headers\" records that \"the \
                                      `Location` response header was deprecated \
                                      from responses of `DELETE` methods\".",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new deleted version \
                             uid>\"` — the identity of the version the deletion \
                             just committed, not the superseded one named in the \
                             path. The released text never says which of the two \
                             the header carries, so this choice is OURS, \
                             adjudicated; it \
                             follows from §\"ETag and Last-Modified\" (the tag \
                             \"changes as soon as the resource changes\", and the \
                             resource's current version is now the deleted one)."),
             ("Last-Modified" = String,
              description = "The deleting version's commit instant as an \
                             HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\"); a logical delete is a \
                             commit, so it IS the resource's last modification."),
         )),
        (status = 400, description = "`ehr_id` is not a UUID, `uid_based_id` is \
                                      not a well-formed OBJECT_VERSION_ID (a bare \
                                      container id is rejected here), or the \
                                      addressed COMPOSITION is ALREADY logically \
                                      deleted — ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`: \
                                      `400` is returned \"when the resource \
                                      identified by the request parameters is \
                                      already deleted\" (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row). The \
                                      already-deleted branch is evaluated BEFORE \
                                      the not-latest `409`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no COMPOSITION with \
                                      this `uid_based_id` in this EHR.",
         body = serde_json::Value),
        (status = 409, description = "The deletion was refused as conflicting \
                                      with the resource's current state. Two \
                                      triggers: (1) `uid_based_id` does not name \
                                      the latest version — ITS-REST \
                                      `specifications/responses/409_COMPOSITION_with_uid_based_id.yaml`: \
                                      \"`409 Conflict` is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header\" (the full three-part \
                                      identity is compared, so a fabricated \
                                      `creating_system_id` on the right version \
                                      tree does not match); (2) the EHR is not \
                                      modifiable (`EHR_STATUS.is_modifiable = \
                                      false`, RM ehr master04 §\"EHR Active \
                                      Status\") — the refusal is spec-required but \
                                      its status code is OUR OWN DESIGN, no \
                                      released text assigning it a branch.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<current latest version \
                             uid>\"` (the `409` response definition's \"Returns \
                             also latest `version_uid` in the `ETag` header\")."),
             ("Last-Modified" = String,
              description = "The commit instant of that current latest version as \
                             an HTTP-date, carried alongside the `ETag` \
                             (§\"ETag and Last-Modified\")."),
         )),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_delete",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── VERSIONED_COMPOSITION ─────────────────────────────────────────────────────

/// Retrieve the `VERSIONED_COMPOSITION` container
/// (`GET /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}`).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}", tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("versioned_object_uid" = String, Path,
         description = "VERSIONED_COMPOSITION identifier, taken from \
                        VERSIONED_COMPOSITION.uid.value — a HIER_OBJECT_ID in \
                        UUID form (`Resources.md` §\"Identifier types\": \"a \
                        `versioned_object_uid` for identifying a \
                        VERSIONED_OBJECT (i.e. a version container), stored under \
                        VERSIONED_OBJECT.uid.value, in a form of a \
                        HIER_OBJECT_ID\"). Unlike `/composition/{uid_based_id}`, \
                        this segment accepts only the container form — \
                        §\"Multiple identifiers for the same resource\" is scoped \
                        to that route.",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21")
    ),
    responses(
        (status = 200, description = "The VERSIONED_COMPOSITION container \
                                      (canonical JSON/XML per `Accept`): the \
                                      versioned-object identity, its `owner_id` \
                                      (the owning EHR) and `time_created` — the \
                                      commit time of the object's FIRST version \
                                      (RM common master06 §\"Versioned \
                                      Objects\") — not the version content. No \
                                      `Location`: `Requests_and_responses.md` \
                                      §Location forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag \
                             `W/\"<versioned_object_uid>\"` (§\"ETag and \
                             Last-Modified\": the value \"is usually taken from \
                             e.g. VERSIONED_OBJECT.uid.value\"; the `W/` weakness \
                             indicator is required since Release 1.1.0)."),
             ("Last-Modified" = String,
              description = "The commit instant of the container's most recent \
                             version as an HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\": both headers \"SHOULD \
                             be included in responses for VERSION, \
                             VERSIONED_OBJECT, or other resources that have \
                             versioning or unique state identifiers\")."),
         ),
         example = json!({
             "_type": "VERSIONED_COMPOSITION",
             "uid": { "_type": "HIER_OBJECT_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21" },
             "owner_id": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "EHR",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
             },
             "time_created": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" }
         })),
        (status = 400, description = "`ehr_id` or `versioned_object_uid` is not a \
                                      UUID (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\"). A syntactically valid but \
                                      unknown id is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no \
                                      VERSIONED_COMPOSITION with \
                                      `versioned_object_uid` in this EHR.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the VERSIONED_COMPOSITION container has only \
                                      the canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION content, \
                                      not for the version container).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_composition_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_composition_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve the COMPOSITION revision history
/// (`GET /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/revision_history`).
#[utoipa::path(
    get,
    path = "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/revision_history",
    tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("versioned_object_uid" = String, Path,
         description = "VERSIONED_COMPOSITION identifier, taken from \
                        VERSIONED_COMPOSITION.uid.value — a HIER_OBJECT_ID in \
                        UUID form (`Resources.md` §\"Identifier types\").",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21")
    ),
    responses(
        (status = 200, description = "The REVISION_HISTORY of the \
                                      VERSIONED_COMPOSITION (canonical JSON/XML \
                                      per `Accept`): one REVISION_HISTORY_ITEM \
                                      per committed version, most recent LAST \
                                      (RM common \
                                      `org.openehr.rm.common.revision_history.adoc`, \
                                      `REVISION_HISTORY.items`: \"The items in \
                                      this history in most-recent-last order\"). \
                                      Each item's `audits[0]` is that version's \
                                      commit audit; any post-committal \
                                      ATTESTATION on the version appears as a \
                                      further element of the same `audits` list \
                                      (RM common master06 §Attestation), never as \
                                      an extra history item. No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag \
                             `W/\"<versioned_object_uid>\"` of the container the \
                             history belongs to (§\"ETag and Last-Modified\": the \
                             value \"is usually taken from e.g. \
                             VERSIONED_OBJECT.uid.value\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the most recent revision as \
                             an HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "REVISION_HISTORY",
             "items": [
                 {
                     "_type": "REVISION_HISTORY_ITEM",
                     "version_id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" },
                     "audits": [ {
                         "_type": "AUDIT_DETAILS",
                         "system_id": "openEHRSys.example.com",
                         "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                         "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                         "change_type": {
                             "_type": "DV_CODED_TEXT",
                             "value": "creation",
                             "defining_code": {
                                 "_type": "CODE_PHRASE",
                                 "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                                 "code_string": "249"
                             }
                         }
                     } ]
                 },
                 {
                     "_type": "REVISION_HISTORY_ITEM",
                     "version_id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
                     "audits": [ {
                         "_type": "AUDIT_DETAILS",
                         "system_id": "openEHRSys.example.com",
                         "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                         "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                         "change_type": {
                             "_type": "DV_CODED_TEXT",
                             "value": "modification",
                             "defining_code": {
                                 "_type": "CODE_PHRASE",
                                 "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                                 "code_string": "251"
                             }
                         }
                     } ]
                 }
             ]
         })),
        (status = 400, description = "`ehr_id` or `versioned_object_uid` is not a \
                                      UUID (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`, or no \
                                      VERSIONED_COMPOSITION with \
                                      `versioned_object_uid` in this EHR.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the REVISION_HISTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST be \
                                      `406`; the Simplified Formats are not \
                                      defined for this resource).",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_composition_revision_history(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_composition_revision_history",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve a COMPOSITION version at a point in time
/// (`GET /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version`).
#[utoipa::path(
    get,
    path = "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version",
    tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("versioned_object_uid" = String, Path,
         description = "VERSIONED_COMPOSITION identifier, taken from \
                        VERSIONED_COMPOSITION.uid.value — a HIER_OBJECT_ID in \
                        UUID form (`Resources.md` §\"Identifier types\").",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21"),
        ("version_at_time" = Option<String>, Query,
         description = "A time in the extended ISO 8601 format; the VERSION \
                        extant at that time is returned. Absent means the latest \
                        VERSION. The timezone is optional — server-local when \
                        omitted (`Resources.md` §\"Datetime format\": query \
                        parameters \"MUST always use the _extended_ ISO 8601 \
                        format\" and \"Timezone SHOULD be only supplied when \
                        needed, otherwise the local timezone is assumed\").",
         example = "2026-07-26T09:12:44.512Z"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION envelope of the \
                                      COMPOSITION extant at that time (canonical \
                                      JSON/XML per `Accept`): the version \
                                      identity, its CONTRIBUTION reference, the \
                                      commit audit, the lifecycle state, and the \
                                      COMPOSITION itself under `data`. A version this server received from another system is served as its \
                                      IMPORTED_VERSION wrapper instead — the \
                                      local contribution and commit audit, with \
                                      the received ORIGINAL_VERSION under `item` \
                                      (RM common master06 §Committal and Audits). A \
                                      logically deleted version is served here as \
                                      a `200` whose `lifecycle_state` is \
                                      `523|deleted|` and which carries NO `data` \
                                      (RM common master06 §\"Logical Deletion\"; \
                                      `ORIGINAL_VERSION.data` is `0..1` in RM \
                                      common `org.openehr.rm.common.version`) — \
                                      unlike the bare `/composition` route, which \
                                      answers `204`. No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET` (\"It MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\"; the `W/` \
                             weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "The version's own `commit_audit.time_committed` as \
                             an HTTP-date — \"For openEHR resources, this value \
                             should be derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "ORIGINAL_VERSION",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
             "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" },
             "contribution": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "CONTRIBUTION",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" }
             },
             "commit_audit": {
                 "_type": "AUDIT_DETAILS",
                 "system_id": "openEHRSys.example.com",
                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                 "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                 "change_type": {
                     "_type": "DV_CODED_TEXT",
                     "value": "modification",
                     "defining_code": {
                         "_type": "CODE_PHRASE",
                         "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                         "code_string": "251"
                     }
                 }
             },
             "lifecycle_state": {
                 "_type": "DV_CODED_TEXT",
                 "value": "complete",
                 "defining_code": {
                     "_type": "CODE_PHRASE",
                     "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                     "code_string": "532"
                 }
             },
             "data": {
                 "_type": "COMPOSITION",
                 "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
                 "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                 "archetype_details": {
                     "_type": "ARCHETYPED",
                     "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                     "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                     "rm_version": "1.2.0"
                 },
                 "name": { "_type": "DV_TEXT", "value": "Encounter" },
                 "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                 "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                 "category": {
                     "_type": "DV_CODED_TEXT", "value": "event",
                     "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                 },
                 "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                 "context": {
                     "_type": "EVENT_CONTEXT",
                     "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                     "setting": {
                         "_type": "DV_CODED_TEXT", "value": "other care",
                         "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                     }
                 },
                 "content": [ {
                     "_type": "EVALUATION",
                     "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                     "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                     "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                     "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                     "subject": { "_type": "PARTY_SELF" },
                     "data": {
                         "_type": "ITEM_TREE",
                         "archetype_node_id": "at0001",
                         "name": { "_type": "DV_TEXT", "value": "Tree" },
                         "items": [ {
                             "_type": "ELEMENT",
                             "archetype_node_id": "at0002",
                             "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                             "value": { "_type": "DV_TEXT", "value": "Asthma — well controlled" }
                         } ]
                     }
                 } ]
             }
         })),
        (status = 400, description = "`ehr_id` or `versioned_object_uid` is not a \
                                      UUID, or `version_at_time` is not an \
                                      extended ISO 8601 datetime \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`; no VERSIONED_COMPOSITION \
                                      with `versioned_object_uid` in this EHR; or \
                                      the container holds no VERSION at the \
                                      requested `version_at_time` (an instant \
                                      before its first commit).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the ORIGINAL_VERSION envelope has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST be \
                                      `406`; the Simplified Formats describe the \
                                      COMPOSITION content, not the VERSION \
                                      envelope).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_composition_version_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_composition_version_get_at_time",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve a COMPOSITION version by id
/// (`GET /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}`).
///
/// Both id segments are load-bearing: the addressed version must belong to
/// the container in the path (RM common `org.openehr.rm.common.version`
/// invariant `Owner_id_valid`; `Resources.md` §"Identifier types"), and the
/// `version_uid` must match the served version's full three-part identity.
#[utoipa::path(
    get,
    path = "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}",
    tag = "COMPOSITION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("versioned_object_uid" = String, Path,
         description = "VERSIONED_COMPOSITION identifier, taken from \
                        VERSIONED_COMPOSITION.uid.value — a HIER_OBJECT_ID in \
                        UUID form (`Resources.md` §\"Identifier types\"). It must \
                        be the container that owns `version_uid`: the two \
                        segments are coherent only when the version's `object_id` \
                        part equals this segment.",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21"),
        ("version_uid" = String, Path,
         description = "VERSION identifier, taken from VERSION.uid.value — an \
                        OBJECT_VERSION_ID \
                        `{object_id}::{creating_system_id}::{version_tree_id}` \
                        (`Resources.md` §\"Identifier types\"). The addressed uid \
                        must name the served version's FULL three-part identity; \
                        a fabricated `creating_system_id` names no VERSION here \
                        and is `404`.",
         example = "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION envelope of the \
                                      COMPOSITION identified by `version_uid` \
                                      (canonical JSON/XML per `Accept`): the \
                                      version identity, its CONTRIBUTION \
                                      reference, the commit audit, the lifecycle \
                                      state, and the COMPOSITION itself under \
                                      `data`. A version this server received from another system is served as its \
                                      IMPORTED_VERSION wrapper instead — the \
                                      local contribution and commit audit, with \
                                      the received ORIGINAL_VERSION under `item` \
                                      (RM common master06 §Committal and Audits). A logically deleted version is \
                                      served here as a `200` whose \
                                      `lifecycle_state` is `523|deleted|` and \
                                      which carries NO `data` (RM common master06 \
                                      §\"Logical Deletion\"; \
                                      `ORIGINAL_VERSION.data` is `0..1` in RM \
                                      common `org.openehr.rm.common.version`). No \
                                      `Location`: `Requests_and_responses.md` \
                                      §Location forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is usually \
                             taken from e.g. … VERSION.uid.value\"; the `W/` \
                             weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "The version's own `commit_audit.time_committed` as \
                             an HTTP-date — \"For openEHR resources, this value \
                             should be derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "ORIGINAL_VERSION",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
             "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" },
             "contribution": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "CONTRIBUTION",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" }
             },
             "commit_audit": {
                 "_type": "AUDIT_DETAILS",
                 "system_id": "openEHRSys.example.com",
                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                 "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T11:04:02.880114Z" },
                 "change_type": {
                     "_type": "DV_CODED_TEXT",
                     "value": "modification",
                     "defining_code": {
                         "_type": "CODE_PHRASE",
                         "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                         "code_string": "251"
                     }
                 }
             },
             "lifecycle_state": {
                 "_type": "DV_CODED_TEXT",
                 "value": "complete",
                 "defining_code": {
                     "_type": "CODE_PHRASE",
                     "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                     "code_string": "532"
                 }
             },
             "data": {
                 "_type": "COMPOSITION",
                 "uid": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::2" },
                 "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                 "archetype_details": {
                     "_type": "ARCHETYPED",
                     "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                     "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                     "rm_version": "1.2.0"
                 },
                 "name": { "_type": "DV_TEXT", "value": "Encounter" },
                 "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                 "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                 "category": {
                     "_type": "DV_CODED_TEXT", "value": "event",
                     "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                 },
                 "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                 "context": {
                     "_type": "EVENT_CONTEXT",
                     "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                     "setting": {
                         "_type": "DV_CODED_TEXT", "value": "other care",
                         "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                     }
                 },
                 "content": [ {
                     "_type": "EVALUATION",
                     "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                     "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                     "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                     "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                     "subject": { "_type": "PARTY_SELF" },
                     "data": {
                         "_type": "ITEM_TREE",
                         "archetype_node_id": "at0001",
                         "name": { "_type": "DV_TEXT", "value": "Tree" },
                         "items": [ {
                             "_type": "ELEMENT",
                             "archetype_node_id": "at0002",
                             "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                             "value": { "_type": "DV_TEXT", "value": "Asthma — well controlled" }
                         } ]
                     }
                 } ]
             }
         })),
        (status = 400, description = "`ehr_id` or `versioned_object_uid` is not a \
                                      UUID, or `version_uid` is not a well-formed \
                                      OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `400` row: \"malformed request \
                                      syntax, syntactically invalid content\"). A \
                                      syntactically valid but unknown id is \
                                      `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id`; no VERSIONED_COMPOSITION \
                                      with `versioned_object_uid` in this EHR; no \
                                      VERSION with `version_uid`; or the two id \
                                      segments are INCOHERENT — a `version_uid` \
                                      whose `object_id` part does not name the \
                                      container in the path. The version then \
                                      belongs to a different VERSIONED_OBJECT and \
                                      this URL identifies nothing: RM common \
                                      `org.openehr.rm.common.version` invariant \
                                      `Owner_id_valid` \
                                      (`owner_id.value.is_equal (uid.object_id.value)`) \
                                      and `Resources.md` §\"Identifier types\" \
                                      (\"the `object_id` matches the \
                                      VERSIONED_OBJECT identifier, taken from \
                                      VERSIONED_OBJECT.uid.value\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the ORIGINAL_VERSION envelope has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST be \
                                      `406`; the Simplified Formats describe the \
                                      COMPOSITION content, not the VERSION \
                                      envelope).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_composition_version_get_by_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_composition_version_get_by_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── DIRECTORY ─────────────────────────────────────────────────────────────────

/// Retrieve the directory (FOLDER) at a point in time
/// (`GET /ehr/{ehr_id}/directory`).
///
/// The served body is the BARE FOLDER: the directory root, or — when `path` is
/// supplied — only the sub-FOLDER that path addresses. The directory is the
/// EHR's first folder hierarchy (`EHR.directory` = `folders.item(1)`, RM ehr
/// master04 §Folders) and is change-controlled like any other versioned
/// object, so the read carries the weak `ETag`/`Last-Modified` of the version
/// it served (`Requests_and_responses.md` §"`ETag` and Last-Modified").
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/directory", tag = "DIRECTORY",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_at_time" = Option<String>, Query,
         description = "A time in the extended ISO 8601 format; the directory \
                        version extant at that time is returned. Absent means \
                        the latest version (ITS-REST \
                        `specifications/operations/directory_get_at_time.yaml`: \
                        \"If `version_at_time` is supplied, retrieves the \
                        version extant _at specified time_, otherwise retrieves \
                        the _latest_ directory FOLDER version\"). The timezone \
                        is optional — server-local when omitted \
                        (`Resources.md` §\"Datetime format\": query parameters \
                        \"MUST always use the _extended_ ISO 8601 format\" and \
                        \"Timezone SHOULD be only supplied when needed, \
                        otherwise the local timezone is assumed\"). No released \
                        text defines the extancy algorithm itself, so the \
                        resolution is OURS, adjudicated: a version is extant from its own \
                        commit instant (inclusive) until the next commit, so a \
                        time at or after the newest commit serves that newest \
                        version — a future time is `200`, never `404` — and a \
                        time before the first held commit is `404`.",
         example = "2026-07-26T09:12:44.512Z"),
        ("path" = Option<String>, Query,
         description = "A path to a sub-folder. The released definition is one \
                        sentence — the path \"consists of slash-separated \
                        values of the name attribute of FOLDERs in the \
                        directory\" (ITS-REST \
                        `specifications/parameters/query/path.yaml`; SM \
                        `i_ehr_directory.adoc` `has_path` repeats it) — and \
                        only the addressed sub-FOLDER is returned. The \
                        resolution grammar beyond that sentence is OUR OWN \
                        DESIGN, adjudicated: the path is rooted at the directory root, \
                        which is implicit and is never named by a segment; a \
                        leading slash is tolerated and empty segments are \
                        skipped, so `a/b`, `/a/b` and `a//b` address the same \
                        node; each segment matches a child `FOLDER.name.value` \
                        under `folders` — the `items` OBJECT_REFs are never \
                        traversed, since they are references to other objects, \
                        not folders (RM common master05 §Overview); and where \
                        sibling names repeat, the first match wins. An empty \
                        `path` (or a bare `/`) addresses the root itself. A \
                        path that does not resolve is `404`.",
         example = "episodes/a/b/c"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The directory FOLDER extant at that time \
                                      — or, when `path` is supplied, only the \
                                      sub-FOLDER it addresses (canonical \
                                      JSON/XML per `Accept`). The version \
                                      headers always describe the DIRECTORY \
                                      version served, also when the body is a \
                                      sub-folder inside it. No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version uid>\"` of the \
                             directory version served (§\"ETag and \
                             Last-Modified\": the value \"is usually taken from \
                             e.g. … VERSION.uid.value\"; the `W/` weakness \
                             indicator is required since Release 1.1.0)."),
             ("Last-Modified" = String,
              description = "That version's commit instant as an HTTP-date — \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\": both headers \
                             \"SHOULD be included in responses for VERSION, \
                             VERSIONED_OBJECT, or other resources that have \
                             versioning or unique state identifiers\")."),
         ),
         example = json!({
             "_type": "FOLDER",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
             "name": { "_type": "DV_TEXT", "value": "root" },
             "folders": [ {
                 "_type": "FOLDER",
                 "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                 "name": { "_type": "DV_TEXT", "value": "episodes" },
                 "items": [ {
                     "_type": "OBJECT_REF",
                     "namespace": "local",
                     "type": "COMPOSITION",
                     "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                 } ]
             } ]
         })),
        (status = 204, description = "The addressed directory version is \
                                      logically deleted, so there is no FOLDER \
                                      to serve: a logical delete commits a new \
                                      version whose `data` is removed and whose \
                                      `lifecycle_state` is `523|deleted|` (RM \
                                      common master06 §\"Logical Deletion\"). \
                                      This server answers `204` for BOTH \
                                      addressing forms of this route — the \
                                      version extant at `version_at_time` and \
                                      the implicit latest. Only the first is \
                                      covered by released text (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`: \
                                      \"`204 No Content` is returned when the \
                                      resource identified by the request \
                                      parameters (at specified \
                                      `version_at_time`) time has been \
                                      deleted\"); the implicit-latest branch has \
                                      no released assignment at all, so \
                                      extending the same outcome to it is OUR \
                                      OWN reading, adjudicated."),
        (status = 400, description = "`ehr_id` is not a UUID, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 datetime (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\"). A syntactically valid \
                                      but unknown id is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when a \
                                      directory does not exist at the specified \
                                      `version_at_time`, or when `path` does \
                                      not exist within the directory\" (ITS-REST \
                                      `specifications/responses/404_directory_unknown_ehr_id_or_no_version_at_time_or_no_path.yaml`). \
                                      An EHR that never had a directory falls \
                                      under the middle clause — there is no \
                                      directory to serve at any time.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the DIRECTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION \
                                      content, and a FOLDER is not templated).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn directory_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "directory_get_at_time",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Update the directory (FOLDER) (`PUT /ehr/{ehr_id}/directory`).
///
/// The whole hierarchy is replaced by the submitted FOLDER tree: a new version
/// of the EHR's directory versioned object is committed inside a new
/// CONTRIBUTION (SM `i_ehr_directory.adoc` `update_directory`: "Create or
/// update a directory from a complete structure … Causes server-side creation
/// of a new `ORIGINAL_VERSION` and a new `CONTRIBUTION`"). The committal
/// headers `openehr-version` / `openehr-audit-details` are accepted and merged
/// into that commit (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}/directory", tag = "DIRECTORY",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("If-Match" = String, Header,
         description = "The latest directory version uid (which becomes the new \
                        version's `preceding_version_uid`), double-quoted; the \
                        weak `W/\"…\"` form the server emits is also accepted. \
                        Required, and this route is exactly the case the rule \
                        is written for: `/ehr/{ehr_id}/directory` carries NO \
                        version segment, so — per \
                        `Requests_and_responses.md` §\"If-Match and accidental \
                        overwrites\" — the header is required \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" and IS the precondition (ITS-REST \
                        `specifications/operations/directory_update.yaml`: \"The \
                        existing latest `version_uid` of directory FOLDER \
                        resource (i.e. the `preceding_version_uid`) must be \
                        specified in the `If-Match` header\"). A missing header \
                        is `400` (\"the service SHOULD respond with `400 Bad \
                        Request`\"); a value that is not the latest version MUST \
                        be `412`, never `409` — the opposite style from the \
                        COMPOSITION `DELETE`, whose precondition travels in the \
                        path.",
         example = "W/\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — no body, `204 No Content`; \
                        `return=identifier` — the body is only \
                        `{ \"uid\": \"<new version uid>\" }` at `200 OK`, never \
                        `204` (§\"Prefer only identifier\": \"a variant of \
                        preference that implies minimal response semantics, but \
                        with a non-empty response body\"); `return=representation` \
                        — the committed directory FOLDER at `200 OK`. An absent \
                        header means `return=minimal`; the token actually \
                        applied is echoed in `Preference-Applied`.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the new FOLDER VERSION, as an \
                        attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"553\"`; the default is \
                        `532|complete|` (RM common master06 §\"Version \
                        Lifecycle\"). Merged with the server defaults \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\": the committal headers are \
                        required on `PUT`, `POST` and `DELETE` for all \
                        change-controlled resources, FOLDER named explicitly, \
                        and whatever is provided \"MUST be merged with the \
                        default VERSION and VERSION.audit_details attributes on \
                        commit runtime\"). The released directory operations \
                        declare neither header — an OAS-side omission the docs \
                        text overrides.\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat — e.g. `description.value=\"Episode folder \
                        added\"`, `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` defaults to `251|modification|` and is \
                        client-overridable to any audit_change_type code \
                        consistent with a content-carrying new version of an \
                        existing object — `250|amendment|` and the other \
                        modification-class codes — but never `249|creation|` or \
                        `523|deleted|`, which belong to the `POST` and `DELETE` \
                        (RM common master06 §Contributions). `time_committed` \
                        \"is always set by the server\", and an omitted \
                        `system_id` defaults to the server's configured \
                        identifier."),
        ("openehr-item-tag" = Option<String>, Header,
         description = "Item tags to set on the directory's VERSIONED_OBJECT \
                        target; an empty value removes all \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", \"Usage in Requests\"). MAY \
                        be echoed back in the response header of the same name."),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "Item tags to set on the new FOLDER VERSION; an empty \
                        value removes all (same section, \"Usage in Requests\"). \
                        MAY be echoed back in the response header of the same \
                        name.")
    ),
    request_body(
        content(
            (serde_json::Value = "application/json", example = json!({
                "_type": "FOLDER",
                "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                "name": { "_type": "DV_TEXT", "value": "root" },
                "folders": [ {
                    "_type": "FOLDER",
                    "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                    "name": { "_type": "DV_TEXT", "value": "episodes" },
                    "items": [ {
                        "_type": "OBJECT_REF",
                        "namespace": "local",
                        "type": "COMPOSITION",
                        "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                    } ]
                } ]
            })),
            (serde_json::Value = "application/xml")
        ),
        description = "The COMPLETE new directory FOLDER tree — the update \
                       replaces the hierarchy, it does not patch it (SM \
                       `update_directory`: \"Directory structure with which to \
                       replace current structure\"). Canonical JSON or XML per \
                       `Content-Type`; the two Simplified Formats do NOT apply \
                       — they are defined for templated COMPOSITION content \
                       (`Resources.md` §\"Simplified Formats\") and a FOLDER is \
                       not templated, so a simplified `Content-Type` is `415` \
                       here. Sub-folders nest under `folders`; the contents of \
                       a folder are `items`, a list of OBJECT_REFs — \"Folder \
                       structures do not contain Compositions, only references \
                       to them\" (RM ehr master04 §Folders), so an inline \
                       LOCATABLE is `422`."
    ),
    responses(
        (
            status = 200, description = "Updated, with a body: the committed \
                                        directory FOLDER for \
                                        `Prefer: return=representation` (the \
                                        `representation` example) or the \
                                        single-`uid` object for \
                                        `return=identifier` (the `identifier` \
                                        example) — `Requests_and_responses.md` \
                                        §\"Prefer minimal, identifier or full \
                                        representation response\"; the released \
                                        `200_directory_updated` response says \
                                        the same. `ETag`, `Last-Modified` and \
                                        `Location` describe the newly committed \
                                        version, and `Preference-Applied` \
                                        declares the token honoured.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<new version uid>\"` \
                                (§\"ETag and Last-Modified\": the value \"is \
                                usually taken from e.g. … VERSION.uid.value\" \
                                and \"changes as soon as the resource \
                                changes\")."),
                ("Location" = String,
                 description = "The URL of the newly committed version, \
                                `<base_path>/ehr/<ehr_id>/directory/<version_uid>` \
                                (§\"Prefer minimal, identifier or full \
                                representation response\": the response \"SHOULD \
                                include a `Location` header pointing to the \
                                newly created or updated resource\"; the \
                                released `Location_directory` header \"indicates \
                                the URL of the directory FOLDER resource\")."),
                ("Last-Modified" = String,
                 description = "The commit instant of the new version as an \
                                HTTP-date — \"derived from \
                                VERSION.commit_audit.time_committed.value\" \
                                (§\"ETag and Last-Modified\")."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured (§\"Representation details \
                                negotiation\")."),
                ("openehr-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the \
                                directory's VERSIONED_OBJECT, present only when \
                                the request carried the header \
                                (§\"openehr-item-tag and \
                                openehr-version-item-tag\", \"Usage in \
                                Responses\")."),
                ("openehr-version-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the new \
                                FOLDER VERSION, present only when the request \
                                carried the header (same section, \"Usage in \
                                Responses\")."),
            ),
            content(
                (serde_json::Value = "application/json", examples(
                    ("representation" = (summary = "Prefer: return=representation — the committed directory FOLDER",
                     value = json!({
                        "_type": "FOLDER",
                        "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                        "name": { "_type": "DV_TEXT", "value": "root" },
                        "folders": [ {
                            "_type": "FOLDER",
                            "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                            "name": { "_type": "DV_TEXT", "value": "episodes" },
                            "items": [ {
                                "_type": "OBJECT_REF",
                                "namespace": "local",
                                "type": "COMPOSITION",
                                "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                            } ]
                        } ]
                     }))),
                    ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
                     value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
                )),
                (serde_json::Value = "application/xml")
            )
        ),
        (status = 204, description = "Updated with no body — the default \
                                      `Prefer: return=minimal` \
                                      (`Requests_and_responses.md` §\"Prefer \
                                      minimal, identifier or full representation \
                                      response\": \"If no response body is \
                                      returned, the service SHOULD use `204 No \
                                      Content`\"). The version headers are \
                                      carried exactly as on the `200`.",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new version uid>\"` \
                             (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "The URL of the newly committed version, \
                             `<base_path>/ehr/<ehr_id>/directory/<version_uid>` \
                             (§\"Prefer minimal, identifier or full \
                             representation response\")."),
             ("Last-Modified" = String,
              description = "The commit instant of the new version as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (§\"Representation details \
                             negotiation\")."),
             ("openehr-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the \
                             directory's VERSIONED_OBJECT, present only when the \
                             request carried the header (§\"openehr-item-tag and \
                             openehr-version-item-tag\", \"Usage in \
                             Responses\")."),
             ("openehr-version-item-tag" = String,
              description = "Echo of the ITEM_TAG list now stored on the new \
                             FOLDER VERSION, present only when the request \
                             carried the header (same section, \"Usage in \
                             Responses\")."),
         )),
        (status = 400, description = "`ehr_id` is not a UUID, the FOLDER payload \
                                      could not be parsed, `If-Match` is \
                                      missing/empty/not a well-formed \
                                      OBJECT_VERSION_ID, or a committal \
                                      `change_type` names a legal \
                                      audit_change_type code that contradicts an \
                                      update (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\"; §\"If-Match and \
                                      accidental overwrites\" for the missing-\
                                      `If-Match` case).",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id` (the released trigger, \
                                      `404_unknown_ehr_id`) — or the EHR exists \
                                      but indexes NO directory, so there is \
                                      nothing to update. The second branch has \
                                      no released assignment: \
                                      `404_unknown_ehr_id` does not cover it and \
                                      SM `update_directory`'s \
                                      `Pre_has_directory: has_directory(ehr_id)` \
                                      states the rule without a status code, so \
                                      answering `404` (the addressed resource \
                                      does not exist) is OUR OWN DESIGN, \
                                      adjudicated.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the DIRECTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST \
                                      be `406`; the Simplified Formats are \
                                      defined for templated COMPOSITION content, \
                                      and a FOLDER is not templated).",
         body = serde_json::Value),
        (status = 409, description = "The EHR is not modifiable \
                                      (`EHR_STATUS.is_modifiable = false`), so \
                                      its contents — the directory included — \
                                      cannot be updated (RM ehr master04 §\"EHR \
                                      Active Status\": the flag \"is used to \
                                      indicate whether the contents of an EHR \
                                      are modifiable\"). The refusal is \
                                      spec-required; the status code is OUR OWN \
                                      DESIGN — no released ITS-REST text assigns \
                                      a branch to it — chosen for the `409` \
                                      row's \"conflict\" meaning (§\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 412, description = "`If-Match` does not name the latest \
                                      directory version, so the update was not \
                                      performed (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\": \
                                      the service \"MUST NOT perform the \
                                      requested method\" and \"MUST respond with \
                                      HTTP status code `412 Precondition \
                                      Failed`\"; ITS-REST \
                                      `specifications/responses/412_directory.yaml` \
                                      says the same and adds the `ETag`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<current latest version \
                             uid>\"` — `412_directory`: \"Returns also latest \
                             `version_uid` in the `ETag` header\" (the same \
                             SHOULD as §\"If-Match and accidental \
                             overwrites\")."),
             ("Last-Modified" = String,
              description = "The commit instant of that current latest version \
                             as an HTTP-date, carried alongside the `ETag` \
                             (§\"ETag and Last-Modified\")."),
         )),
        (status = 415, description = "The request `Content-Type` is not one this \
                                      resource can process — anything outside \
                                      canonical JSON/XML, the Simplified Formats \
                                      included (`Resources.md` §\"Simplified \
                                      Formats\": \"If the service cannot process \
                                      the request payload as the simplified \
                                      format is not supported, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\"; §\"XML Format\"/§\"JSON \
                                      Format\" carry the same MUST for the \
                                      canonical types). A FOLDER is not \
                                      templated, so no simplified form of it \
                                      exists.",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed and converted \
                                      to a FOLDER tree, but the tree cannot be \
                                      followed as a directory \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row: \"The \
                                      request was well-formed but was unable to \
                                      be followed due to semantic errors\"). The \
                                      shape rules are RM, checked at every node \
                                      of the tree: `FOLDER.name` is mandatory \
                                      and `archetype_node_id` mandatory and \
                                      non-empty (LOCATABLE, RM common \
                                      master03), and every `items` member must \
                                      be an OBJECT_REF (`id` + `namespace` + \
                                      `type`, and never an inline LOCATABLE) — \
                                      \"Folder structures do not contain \
                                      Compositions, only references to them\" \
                                      (RM ehr master04 §Folders; RM common \
                                      master05 §Overview types `FOLDER.items` \
                                      `List<OBJECT_REF>`). A committal \
                                      `change_type`/`lifecycle_state` that is \
                                      not a member of its openEHR terminology \
                                      group is `422` too. An `items` OBJECT_REF \
                                      whose `namespace` claims this system \
                                      (`local`, or the configured system id) \
                                      must resolve to a versioned object in \
                                      this EHR; an unresolvable one is `422` \
                                      naming each dangling reference at its \
                                      tree path — no released text constrains \
                                      reference targets, so this is our own \
                                      extension, and foreign-namespace \
                                      references pass unchecked (BASE \
                                      `object_ref.adoc`: targets \"may exist \
                                      locally or be maintained outside the \
                                      current namespace\").",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn directory_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "directory_update",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Create the directory (FOLDER) (`POST /ehr/{ehr_id}/directory`).
///
/// Creates the EHR's directory hierarchy from a complete FOLDER tree: a new
/// `VERSIONED_OBJECT`, its first `ORIGINAL_VERSION` and a new CONTRIBUTION (SM
/// `i_ehr_directory.adoc` `create_directory`). The committal headers
/// `openehr-version` / `openehr-audit-details` are accepted and merged into
/// that commit (`Requests_and_responses.md` §openehr-version-and-audit-details).
#[utoipa::path(
    post, path = "/ehr/{ehr_id}/directory", tag = "DIRECTORY",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens: \
                        `return=minimal` — empty body; `return=identifier` — \
                        the body is only `{ \"uid\": \"<new version uid>\" }`; \
                        `return=representation` — the created directory FOLDER. \
                        An absent header means `return=minimal` (\"If no \
                        `Prefer` header is provided, the default behavior is \
                        assumed to be `return=minimal`\"), which the released \
                        `201_directory` response repeats; the token actually \
                        applied is echoed in `Preference-Applied`.",
         example = "return=representation"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the first FOLDER VERSION, as an \
                        attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"553\"`; the default is \
                        `532|complete|` (RM common master06 §\"Version \
                        Lifecycle\"). Merged with the server defaults \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\": the committal headers are \
                        required on `PUT`, `POST` and `DELETE` for all \
                        change-controlled resources — FOLDER named explicitly — \
                        and whatever is provided \"MUST be merged with the \
                        default VERSION and VERSION.audit_details attributes on \
                        commit runtime\"). The released directory operations \
                        declare neither header — an OAS-side omission the docs \
                        text overrides.\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this \
                        creation commits, as an attribute-path list; the header \
                        MAY repeat — e.g. `description.value=\"Initial episode \
                        index\"`, `committer.name=\"John Doe\",\
                        committer.external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\",\
                        committer.external_ref.namespace=\"demographic\",\
                        committer.external_ref.type=\"PERSON\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` is constrained to `249|creation|` (a \
                        create commits a first version — RM common master06 \
                        §Contributions); `time_committed` \"is always set by the \
                        server\", and an omitted `system_id` defaults to the \
                        server's configured identifier."),
        ("openehr-item-tag" = Option<String>, Header,
         description = "Item tags to set on the directory's VERSIONED_OBJECT \
                        target; an empty value removes all \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", \"Usage in Requests\"). MAY \
                        be echoed back in the response header of the same name."),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "Item tags to set on the created FOLDER VERSION; an empty \
                        value removes all (same section, \"Usage in Requests\"). \
                        MAY be echoed back in the response header of the same \
                        name.")
    ),
    request_body(
        content(
            (serde_json::Value = "application/json", example = json!({
                "_type": "FOLDER",
                "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                "name": { "_type": "DV_TEXT", "value": "root" },
                "folders": [ {
                    "_type": "FOLDER",
                    "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                    "name": { "_type": "DV_TEXT", "value": "episodes" },
                    "items": [ {
                        "_type": "OBJECT_REF",
                        "namespace": "local",
                        "type": "COMPOSITION",
                        "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                    } ]
                } ]
            })),
            (serde_json::Value = "application/xml")
        ),
        description = "The directory FOLDER tree, canonical JSON or XML per \
                       `Content-Type`. The two Simplified Formats do NOT apply \
                       — they are defined for templated COMPOSITION content \
                       (`Resources.md` §\"Simplified Formats\") and a FOLDER is \
                       not templated, so a simplified `Content-Type` is `415` \
                       here. Sub-folders nest under `folders`; the contents of \
                       a folder are `items`, a list of OBJECT_REFs \
                       (`{ id, namespace, type }`) — \"Folder structures do not \
                       contain Compositions, only references to them\" (RM ehr \
                       master04 §Folders), so an inline LOCATABLE is `422`."
    ),
    responses(
        (
            status = 201, description = "Created. `ETag` (weak `W/` form) \
                                        carries the new version uid, \
                                        `Last-Modified` its commit instant, \
                                        `Location` the directory version URL, \
                                        and `Preference-Applied` the `Prefer` \
                                        token actually honoured. The body is \
                                        `Prefer`-conditional (ITS-REST \
                                        `specifications/responses/201_directory.yaml`: \
                                        \"If `Prefer` header is \
                                        `return=representation`, the full \
                                        resource is included in the response \
                                        body; if is `return=identifier`, only \
                                        its unique identifier is included. If \
                                        the `Prefer` header is missing or set to \
                                        `return=minimal`, the body is empty\").",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<new version uid>\"` \
                                (§\"ETag and Last-Modified\": the value \"is \
                                usually taken from e.g. … VERSION.uid.value\"; \
                                the `W/` weakness indicator is required since \
                                Release 1.1.0)."),
                ("Location" = String,
                 description = "The URL of the newly created directory version, \
                                `<base_path>/ehr/<ehr_id>/directory/<version_uid>` \
                                (§Location: used \"in `201 Created` responses \
                                when a new resource is successfully created\"; \
                                the released `Location_directory` header \
                                \"indicates the URL of the directory FOLDER \
                                resource\")."),
                ("Last-Modified" = String,
                 description = "The creating CONTRIBUTION's commit instant as an \
                                HTTP-date — \"derived from \
                                VERSION.commit_audit.time_committed.value\" \
                                (§\"ETag and Last-Modified\")."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured (§\"Representation details \
                                negotiation\")."),
                ("openehr-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the \
                                directory's VERSIONED_OBJECT, present only when \
                                the request carried the header \
                                (§\"openehr-item-tag and \
                                openehr-version-item-tag\", \"Usage in \
                                Responses\": servers \"MAY include\" it \"to \
                                confirm the actual list of ITEM_TAGs stored on \
                                the server side\")."),
                ("openehr-version-item-tag" = String,
                 description = "Echo of the ITEM_TAG list now stored on the \
                                created FOLDER VERSION, present only when the \
                                request carried the header (same section, \
                                \"Usage in Responses\")."),
            ),
            content(
                (serde_json::Value = "application/json", examples(
                    ("representation" = (summary = "Prefer: return=representation — the created directory FOLDER",
                     value = json!({
                        "_type": "FOLDER",
                        "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                        "name": { "_type": "DV_TEXT", "value": "root" },
                        "folders": [ {
                            "_type": "FOLDER",
                            "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                            "name": { "_type": "DV_TEXT", "value": "episodes" },
                            "items": [ {
                                "_type": "OBJECT_REF",
                                "namespace": "local",
                                "type": "COMPOSITION",
                                "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                            } ]
                        } ]
                     }))),
                    ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
                     value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
                )),
                (serde_json::Value = "application/xml")
            )
        ),
        (status = 400, description = "`ehr_id` is not a UUID, the FOLDER payload \
                                      could not be parsed, or a committal \
                                      `change_type` names a legal \
                                      audit_change_type code that contradicts a \
                                      creation (only `249|creation|` is \
                                      compatible — RM common master06 \
                                      §Contributions) \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\").",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id` (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the DIRECTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": an unfulfillable `Accept` MUST \
                                      be `406`; the Simplified Formats are \
                                      defined for templated COMPOSITION content, \
                                      and a FOLDER is not templated).",
         body = serde_json::Value),
        (status = 409, description = "The creation conflicts with the EHR's \
                                      current state (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `409` row: the \
                                      request \"might generate a duplicate or a \
                                      conflict\"). Two triggers, both of which \
                                      are OUR OWN DESIGN — no released ITS-REST \
                                      text assigns either a status, and both are \
                                      adjudicated: (1) the EHR already holds a \
                                      LIVE directory — the rule itself is SM \
                                      `i_ehr_directory.adoc` `create_directory` \
                                      `Pre_no_directory: not has_directory \
                                      (ehr_id)`, which states the precondition \
                                      without a wire code; after a logical \
                                      delete the version container survives but \
                                      the directory slot is vacant (RM common \
                                      master06 §\"Logical Deletion\"), so a \
                                      re-create then opens a new hierarchy \
                                      rather than conflicting; (2) the EHR is \
                                      not modifiable (`EHR_STATUS.is_modifiable \
                                      = false`, RM ehr master04 §\"EHR Active \
                                      Status\": the flag \"is used to indicate \
                                      whether the contents of an EHR are \
                                      modifiable\") — the refusal is \
                                      spec-required, the status choice is ours.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one this \
                                      resource can process — anything outside \
                                      canonical JSON/XML, the Simplified Formats \
                                      included (`Resources.md` §\"Simplified \
                                      Formats\": \"If the service cannot process \
                                      the request payload as the simplified \
                                      format is not supported, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\"; §\"XML Format\"/§\"JSON \
                                      Format\" carry the same MUST for the \
                                      canonical types). A FOLDER is not \
                                      templated, so no simplified form of it \
                                      exists.",
         body = serde_json::Value),
        (status = 422, description = "The request was well-formed and converted \
                                      to a FOLDER tree, but the tree cannot be \
                                      followed as a directory \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row: \"The \
                                      request was well-formed but was unable to \
                                      be followed due to semantic errors\"). The \
                                      shape rules are RM, checked at every node \
                                      of the tree: `FOLDER.name` is mandatory \
                                      and `archetype_node_id` mandatory and \
                                      non-empty (LOCATABLE, RM common \
                                      master03), and every `items` member must \
                                      be an OBJECT_REF (`id` + `namespace` + \
                                      `type`, and never an inline LOCATABLE) — \
                                      \"Folder structures do not contain \
                                      Compositions, only references to them\" \
                                      (RM ehr master04 §Folders; RM common \
                                      master05 §Overview types `FOLDER.items` \
                                      `List<OBJECT_REF>`). A committal \
                                      `change_type`/`lifecycle_state` that is \
                                      not a member of its openEHR terminology \
                                      group is `422` too. An `items` OBJECT_REF \
                                      whose `namespace` claims this system \
                                      (`local`, or the configured system id) \
                                      must resolve to a versioned object in \
                                      this EHR; an unresolvable one is `422` \
                                      naming each dangling reference at its \
                                      tree path — no released text constrains \
                                      reference targets, so this is our own \
                                      extension, and foreign-namespace \
                                      references pass unchecked (BASE \
                                      `object_ref.adoc`: targets \"may exist \
                                      locally or be maintained outside the \
                                      current namespace\").",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn directory_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "directory_create",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Delete the directory (FOLDER) (`DELETE /ehr/{ehr_id}/directory`).
///
/// Deletion is LOGICAL: a new `523|deleted|` version of the directory with no
/// `data` is committed (RM common master06 §"Logical Deletion"; SM
/// `i_ehr_directory.adoc` `delete_directory`: "Logically delete the directory
/// by creating a new version in which the contents are removed"), and the
/// `204` reports THAT version in `ETag`/`Last-Modified`.
///
/// The precondition travels in the `If-Match` HEADER, not in the path — this
/// route has no version segment — so a stale precondition is `412`, never the
/// `409` the COMPOSITION `DELETE` answers (`Requests_and_responses.md`
/// §"If-Match and accidental overwrites"). The operation exchanges no body in
/// either direction, so nothing is content-negotiated here.
#[utoipa::path(
    delete, path = "/ehr/{ehr_id}/directory", tag = "DIRECTORY",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("If-Match" = String, Header,
         description = "The latest directory version uid — the \
                        `preceding_version_uid` the deletion supersedes — \
                        double-quoted; the weak `W/\"…\"` form the server emits \
                        is also accepted. Required: `/ehr/{ehr_id}/directory` \
                        carries no version segment, so \
                        `Requests_and_responses.md` §\"If-Match and accidental \
                        overwrites\" makes the header the precondition (\"when \
                        the `preceding_version_uid` is not part of the endpoint \
                        path segment\"), which ITS-REST \
                        `specifications/operations/directory_delete.yaml` \
                        repeats. A missing header is `400` (\"the service SHOULD \
                        respond with `400 Bad Request`\") and a value that is \
                        not the latest version is `412` — CONTRAST the \
                        COMPOSITION `DELETE`, which addresses the preceding \
                        version in the path and answers `409` for a non-latest \
                        uid.",
         example = "W/\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2\""),
        ("openehr-version" = Option<String>, Header,
         description = "Accepted for uniformity — `Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\" requires \
                        the committal headers on `PUT`, `POST` **and** `DELETE` \
                        for every change-controlled resource, FOLDER named \
                        explicitly. The deleting version's `lifecycle_state` is \
                        not client-selectable, though: a logical deletion sets \
                        it to `523|deleted|` by definition (RM common master06 \
                        §\"Logical Deletion\"). A supplied `lifecycle_state` \
                        naming any OTHER state contradicts the operation and is \
                        refused `400` rather than silently discarded — the same \
                        section makes the merge a MUST, and a value that cannot \
                        be merged is reported, not dropped; `523|deleted|` \
                        itself is accepted as the redundant statement it is."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this \
                        deletion commits, as an attribute-path list; the header \
                        MAY repeat — e.g. `description.value=\"Index no longer \
                        maintained\"`, `committer.name=\"John Doe\"`, \
                        `system_id=\"example.openehr.systemid\"`. \
                        `change_type` is constrained to `523|deleted|` (RM \
                        common master06 §Contributions: a deletion commits a \
                        deleted version); `time_committed` is always server-set, \
                        and an omitted `system_id` defaults to the server's \
                        configured identifier.")
    ),
    responses(
        (status = 204, description = "Logically deleted. Deletion is never \
                                      physical: a NEW version of the directory \
                                      is committed whose `data` is removed and \
                                      whose `lifecycle_state` — and whose \
                                      `commit_audit.change_type` — is \
                                      `523|deleted|` (RM common master06 \
                                      §\"Logical Deletion\": \"create a new \
                                      Version in the normal way; delete its \
                                      `data`…; set the `lifecycle_state` value \
                                      to the code for `deleted`; commit in the \
                                      normal way\"), inside a new CONTRIBUTION; \
                                      the version container survives, so the \
                                      directory slot is merely vacated. No \
                                      `Location`: `Requests_and_responses.md` \
                                      §\"HTTP headers\" records that \"the \
                                      `Location` response header was deprecated \
                                      from responses of `DELETE` methods\".",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<new deleted version \
                             uid>\"` — the identity of the version the deletion \
                             just committed, not the superseded one named in \
                             `If-Match`. The released `204_deleted` response \
                             declares no headers at all, but §\"ETag and \
                             Last-Modified\" SHOULDs both for versioned \
                             resources, so serving them is the docs text's rule; \
                             WHICH of the two identities the tag carries is \
                             unstated, and picking the new one is OURS, \
                             adjudicated — \
                             it follows from the same section (the tag \"changes \
                             as soon as the resource changes\", and the \
                             resource's current version is now the deleted \
                             one)."),
             ("Last-Modified" = String,
              description = "The deleting version's commit instant as an \
                             HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\"); a logical delete is \
                             a commit, so it IS the resource's last \
                             modification."),
         )),
        (status = 400, description = "`ehr_id` is not a UUID, or `If-Match` is \
                                      missing, empty, or not a well-formed \
                                      OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\"; §\"If-Match and accidental \
                                      overwrites\" for the missing-header case).",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id` (the released trigger, \
                                      `404_unknown_ehr_id`) — or the EHR exists \
                                      but indexes NO directory, so there is \
                                      nothing to delete. The second branch has \
                                      no released assignment: \
                                      `404_unknown_ehr_id` does not cover it and \
                                      SM `delete_directory`'s \
                                      `Pre_has_directory: has_directory(ehr_id)` \
                                      states the rule without a status code, so \
                                      answering `404` (the addressed resource \
                                      does not exist) is OUR OWN DESIGN, \
                                      adjudicated. It is evaluated before the \
                                      `If-Match` precondition — there is no \
                                      resource whose version could match.",
         body = serde_json::Value),
        (status = 409, description = "The EHR is not modifiable \
                                      (`EHR_STATUS.is_modifiable = false`), so \
                                      its contents — the directory included — \
                                      cannot be changed, and a logical delete is \
                                      a content write (RM ehr master04 §\"EHR \
                                      Active Status\": the flag \"is used to \
                                      indicate whether the contents of an EHR \
                                      are modifiable\"). The refusal is \
                                      spec-required; the status code is OUR OWN \
                                      DESIGN — no released ITS-REST text assigns \
                                      a branch to it — chosen for the `409` \
                                      row's \"conflict\" meaning (§\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 412, description = "`If-Match` does not name the latest \
                                      directory version, so the deletion was not \
                                      performed (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\": \
                                      the service \"MUST NOT perform the \
                                      requested method\" and \"MUST respond with \
                                      HTTP status code `412 Precondition \
                                      Failed`\"; ITS-REST \
                                      `specifications/responses/412_directory.yaml` \
                                      says the same and adds the `ETag`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<current latest version \
                             uid>\"` — `412_directory`: \"Returns also latest \
                             `version_uid` in the `ETag` header\"."),
             ("Last-Modified" = String,
              description = "The commit instant of that current latest version \
                             as an HTTP-date, carried alongside the `ETag` \
                             (§\"ETag and Last-Modified\")."),
         )),
        (status = 406, description = "A Simplified-Format `Accept` was sent: \
                                      the directory resource family has no \
                                      simplified mapping (the Simplified \
                                      Formats specification defines FLAT/\
                                      STRUCTURED for templated COMPOSITION \
                                      content only), so the request is refused \
                                      uniformly — even though this response \
                                      carries no body.",
         body = serde_json::Value),
        (status = 415, description = "A Simplified-Format `Content-Type` was \
                                      sent: no simplified mapping exists for \
                                      the directory resource family \
                                      (`Resources.md` §Simplified Formats), so \
                                      the request is refused uniformly — even \
                                      though this operation takes no body.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn directory_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "directory_delete",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve the directory (FOLDER) by version id
/// (`GET /ehr/{ehr_id}/directory/{version_uid}`).
///
/// The served body is the BARE FOLDER of that version — the directory root, or
/// only the sub-FOLDER `path` addresses. Unlike `/composition/{uid_based_id}`,
/// this segment has NO implicit-latest form: `Resources.md` §"Multiple
/// identifiers for the same resource" is scoped to the COMPOSITION route, and
/// the latest directory is read from `/ehr/{ehr_id}/directory` instead.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/directory/{version_uid}", tag = "DIRECTORY",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("version_uid" = String, Path,
         description = "VERSION identifier, taken from VERSION.uid.value — an \
                        OBJECT_VERSION_ID \
                        `{object_id}::{creating_system_id}::{version_tree_id}` \
                        (`Resources.md` §\"Identifier types\"; ITS-REST \
                        `specifications/parameters/path/version_uid.yaml`). \
                        This route takes ONLY the full three-part form — there \
                        is no implicit-latest addressing here, and a bare \
                        container id is `400`. The addressed uid must name the \
                        served version's FULL identity: a fabricated \
                        `creating_system_id` on an existing version tree names \
                        no VERSION in this repository and is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2"),
        ("path" = Option<String>, Query,
         description = "A path to a sub-folder. The released definition is one \
                        sentence — the path \"consists of slash-separated \
                        values of the name attribute of FOLDERs in the \
                        directory\" (ITS-REST \
                        `specifications/parameters/query/path.yaml`) — and only \
                        the addressed sub-FOLDER is returned (the operation \
                        adds: \"If `path` is supplied, retrieves from the \
                        directory only the sub-FOLDER that is associated with \
                        that path\"). The resolution grammar beyond that \
                        sentence is OUR OWN DESIGN, adjudicated: the path is rooted at the \
                        directory root, which is implicit and is never named by \
                        a segment; a leading slash is tolerated and empty \
                        segments are skipped, so `a/b`, `/a/b` and `a//b` \
                        address the same node; each segment matches a child \
                        `FOLDER.name.value` under `folders` — the `items` \
                        OBJECT_REFs are never traversed (RM common master05 \
                        §Overview); and where sibling names repeat, the first \
                        match wins. An empty `path` (or a bare `/`) addresses \
                        the root itself. A path that does not resolve is `404`.",
         example = "episodes/a/b/c"),
        ("expand_multimedia" = Option<bool>, Query,
         description = "OUR OWN EXTENSION — no openEHR spec governs this \
                        parameter. `true` transparently re-inlines DV_MULTIMEDIA \
                        content this deployment externalized to object storage, \
                        verifying its integrity, so the served body carries \
                        the original data again (the offload-added uri and \
                        integrity fields remain alongside it). A no-op when the \
                        body holds no external media; an error when the content \
                        cannot be restored, never a silent fall back to the \
                        stored reference.",
         example = json!(true))
    ),
    responses(
        (status = 200, description = "The directory FOLDER at that version — \
                                      or, when `path` is supplied, only the \
                                      sub-FOLDER it addresses (canonical \
                                      JSON/XML per `Accept`). The version \
                                      headers describe the addressed DIRECTORY \
                                      version, also when the body is a \
                                      sub-folder inside it. No `Location`: \
                                      `Requests_and_responses.md` §Location \
                                      forbids it on a `GET`.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` \
                             (§\"ETag and Last-Modified\": the value \"is \
                             usually taken from e.g. … VERSION.uid.value\"; the \
                             `W/` weakness indicator is required since Release \
                             1.1.0)."),
             ("Last-Modified" = String,
              description = "That version's own `commit_audit.time_committed` \
                             as an HTTP-date — \"For openEHR resources, this \
                             value should be derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\")."),
         ),
         example = json!({
             "_type": "FOLDER",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
             "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
             "name": { "_type": "DV_TEXT", "value": "root" },
             "folders": [ {
                 "_type": "FOLDER",
                 "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                 "name": { "_type": "DV_TEXT", "value": "episodes" },
                 "items": [ {
                     "_type": "OBJECT_REF",
                     "namespace": "local",
                     "type": "COMPOSITION",
                     "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                 } ]
             } ]
         })),
        (status = 204, description = "The addressed version is logically \
                                      deleted, so there is no FOLDER to serve: \
                                      the deleting commit removes `data` and \
                                      sets `lifecycle_state` to `523|deleted|` \
                                      (RM common master06 §\"Logical \
                                      Deletion\"). The released text assigns no \
                                      branch to an explicitly addressed deleted \
                                      version — its one deleted branch \
                                      (`204_deleted_at_time`) is textually \
                                      scoped to \"at specified \
                                      `version_at_time`\" — so answering the \
                                      same `204` here is OUR OWN reading, \
                                      adjudicated."),
        (status = 400, description = "`ehr_id` is not a UUID, or `version_uid` \
                                      is not a well-formed OBJECT_VERSION_ID — a \
                                      bare container id included, since this \
                                      route defines no implicit-latest form \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\"). A syntactically valid but \
                                      unknown id is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when a \
                                      directory with `version_uid` does not \
                                      exist, or when `path` does not exist \
                                      within the directory\" (ITS-REST \
                                      `specifications/responses/404_directory_unknown_ehr_id_or_no_version_uid_or_no_path.yaml`). \
                                      \"Does not exist\" is judged on the FULL \
                                      three-part identity: a `version_uid` whose \
                                      `creating_system_id` does not match the \
                                      stored version's names no VERSION here \
                                      (`Resources.md` §\"Identifier types\"), \
                                      and neither does one whose `object_id` \
                                      belongs to another EHR's directory.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the DIRECTORY resource has only the \
                                      canonical `application/json` / \
                                      `application/xml` representations \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\"; the Simplified Formats are \
                                      defined for templated COMPOSITION \
                                      content, and a FOLDER is not templated).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. Processing of the addressed \
                                      object is restricted, so the server holds \
                                      it in storage and serves nothing from it \
                                      (GDPR Art. 18(2) leaves storage as the \
                                      only processing a restriction admits). \
                                      Distinct from an authorization refusal: \
                                      the same caller with the same rights is \
                                      refused until the restriction is lifted, \
                                      and the body says so. The 401/403 rows of \
                                      the access layer are unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn directory_get_by_version_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "directory_get_by_version_id",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── CONTRIBUTION ──────────────────────────────────────────────────────────────

/// Create a CONTRIBUTION — the NATIVE change-set commit
/// (`POST /ehr/{ehr_id}/contribution`).
///
/// One CONTRIBUTION is one atomic change-set: `versions[]` MAY mix creations,
/// modifications, logical deletions and attestations of COMPOSITION /
/// `EHR_STATUS` / FOLDER, and either all of them commit or none ("Contributions
/// are similar to nested transactions. An attempt to commit a Contribution
/// should only succeed if each Version and/or Attestation in the Contribution
/// is committed successfully" — RM common master06 §"Committal and Audits";
/// the legal mixture is master06 §Contributions: "there might be any
/// combination of the logical change types in a single commit"). The
/// per-resource `POST`/`PUT`/`DELETE` routes are convenience wrappers over
/// exactly this operation — they "MUST internally be executed using the
/// 'native' way" (`Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details").
///
/// Which is why this route declares NO `openehr-version` /
/// `openehr-audit-details` parameters: those headers exist to carry committal
/// metadata for the convenience methods, and here that metadata is IN the body
/// — `versions[i].lifecycle_state`, `versions[i].commit_audit` and the
/// envelope `audit`. The section's merge MUST is scoped to the convenience
/// methods, and the released text states no precedence for those headers
/// arriving on the native route, so answering from the body alone is OUR OWN
/// reading, adjudicated.
#[utoipa::path(
    post, path = "/ehr/{ehr_id}/contribution", tag = "CONTRIBUTION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). Exactly one of the three tokens, and \
                        each names the CONTRIBUTION — never a member version: \
                        `return=minimal` — empty body; `return=identifier` — \
                        the body is only `{ \"uid\": \"<contribution_uid>\" }`; \
                        `return=representation` — the committed CONTRIBUTION. \
                        An absent header means `return=minimal` (\"If no \
                        `Prefer` header is provided, the default behavior is \
                        assumed to be `return=minimal`\", which ITS-REST \
                        `specifications/responses/201_CONTRIBUTION.yaml` \
                        repeats: \"If the `Prefer` header is missing or set to \
                        `return=minimal`, the body is empty\"); the token \
                        actually applied is echoed in `Preference-Applied`. \
                        CONTRAST the COMPOSITION commit: a Simplified-Format \
                        `Accept` does NOT force a body here — it only selects \
                        the inner `versions[i].data` form of a body `Prefer` \
                        already asked for (`201_CONTRIBUTION`: \"When the \
                        request `Accept` header selects a Simplified Formats \
                        MIME type … and `Prefer: return=representation`\").",
         example = "return=representation"),
        ("openehr-template-id" = Option<String>, Header,
         description = "The operational-template id the inner \
                        `versions[i].data` payloads are validated against — \
                        REQUIRED when the request `Content-Type` is one of the \
                        two Simplified Formats. `Requests_and_responses.md` \
                        §openehr-template-id: the header \"MUST be used \
                        whenever committing COMPOSITION (via `PUT` or `POST` \
                        methods) using a Simplified Format which does not \
                        support TEMPLATE_ID value under an equivalent \
                        `LOCATABLE.archetype_details.template_id` attribute of \
                        contained data\". A canonical-JSON envelope carries \
                        each COMPOSITION's own `archetype_details.template_id`, \
                        so the header is not needed there; a simplified commit \
                        without it cannot be resolved to a template and is \
                        `422` (the released text assigns the missing header no \
                        status — the code choice is ours). ONE template id \
                        applies to every simplified member of the change-set.",
         example = "problem_list.v1")
    ),
    request_body(
        // The envelope is always canonical JSON; a Simplified media type selects
        // ONLY the inner `versions[i].data` COMPOSITION form (SPECITS-84, the
        // Amendment_record entry of 27 Apr 2026 + `contribution_create.yaml`
        // §Simplified Formats). No canonical-XML CONTRIBUTION wire shape exists,
        // so `application/xml` is not offered (a canonical-XML `Content-Type` is
        // `415`).
        content(
            (serde_json::Value = "application/json", example = json!({
                "versions": [ {
                    "lifecycle_state": {
                        "_type": "DV_CODED_TEXT", "value": "complete",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "532" }
                    },
                    "data": {
                        "_type": "COMPOSITION",
                        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                        "archetype_details": {
                            "_type": "ARCHETYPED",
                            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
                            "template_id": { "_type": "TEMPLATE_ID", "value": "problem_list.v1" },
                            "rm_version": "1.2.0"
                        },
                        "name": { "_type": "DV_TEXT", "value": "Encounter" },
                        "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                        "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
                        "category": {
                            "_type": "DV_CODED_TEXT", "value": "event",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
                        },
                        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                        "context": {
                            "_type": "EVENT_CONTEXT",
                            "start_time": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                            "setting": {
                                "_type": "DV_CODED_TEXT", "value": "other care",
                                "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "238" }
                            }
                        },
                        "content": [ {
                            "_type": "EVALUATION",
                            "archetype_node_id": "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
                            "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis" },
                            "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
                            "encoding": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
                            "subject": { "_type": "PARTY_SELF" },
                            "data": {
                                "_type": "ITEM_TREE",
                                "archetype_node_id": "at0001",
                                "name": { "_type": "DV_TEXT", "value": "Tree" },
                                "items": [ {
                                    "_type": "ELEMENT",
                                    "archetype_node_id": "at0002",
                                    "name": { "_type": "DV_TEXT", "value": "Problem/Diagnosis name" },
                                    "value": { "_type": "DV_TEXT", "value": "Asthma" }
                                } ]
                            }
                        } ]
                    },
                    "commit_audit": {
                        "_type": "UPDATE_AUDIT",
                        "change_type": {
                            "_type": "DV_CODED_TEXT", "value": "creation",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "249" }
                        },
                        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" }
                    }
                }, {
                    "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31::openEHRSys.example.com::1" },
                    "lifecycle_state": {
                        "_type": "DV_CODED_TEXT", "value": "complete",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "532" }
                    },
                    "data": {
                        "_type": "EHR_STATUS",
                        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
                        "archetype_details": {
                            "_type": "ARCHETYPED",
                            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                            "rm_version": "1.2.0"
                        },
                        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
                        "subject": {
                            "_type": "PARTY_SELF",
                            "external_ref": {
                                "_type": "PARTY_REF",
                                "namespace": "demographic",
                                "type": "PERSON",
                                "id": { "_type": "GENERIC_ID", "value": "ins01", "scheme": "demographic" }
                            }
                        },
                        "is_queryable": true,
                        "is_modifiable": true
                    },
                    "commit_audit": {
                        "_type": "UPDATE_AUDIT",
                        "change_type": {
                            "_type": "DV_CODED_TEXT", "value": "modification",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" }
                        },
                        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" }
                    }
                } ],
                "audit": {
                    "_type": "UPDATE_AUDIT",
                    "change_type": {
                        "_type": "DV_CODED_TEXT", "value": "modification",
                        "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" }
                    },
                    "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                    "description": { "_type": "DV_TEXT", "value": "Encounter recorded at triage; EHR status refreshed" }
                }
            })),
            (serde_json::Value = "application/openehr.wt.flat+json"),
            (serde_json::Value = "application/openehr.wt.structured+json")
        ),
        description = "The un-committed CONTRIBUTION — the RELAXED envelope the \
                       released operation defines (ITS-REST \
                       `specifications/operations/contribution_create.yaml`): \
                       `versions[]` of UPDATE_VERSION \
                       (`preceding_version_uid`?, `signature`?, \
                       `lifecycle_state`, `attestations`?, `data`, \
                       `commit_audit`) plus the change-set `audit` \
                       (UPDATE_AUDIT: `change_type`, `committer`, \
                       `description`?, `system_id`?), and an OPTIONAL `uid` \
                       — the three relaxations quoted verbatim: `uid` \"when \
                       provided, it will be accepted in case is not in-use, \
                       otherwise error will be returned\"; \
                       `audit.time_committed` \"server will always set it\"; \
                       `audit.system_id` \"when provided, it will be \
                       validated\" (an omitted one defaults to the server's \
                       configured identifier). `audit` and each \
                       `versions[i].commit_audit` are UPDATE_AUDIT objects — \
                       \"Clients SHOULD send `_type: \"UPDATE_AUDIT\"`; for \
                       interoperability servers SHOULD additionally accept \
                       `_type: \"AUDIT_DETAILS\"` or an omitted `_type`\", \
                       and all three are accepted here. The CONTRIBUTION \
                       audit's `system_id`/`committer` are copied down into \
                       every member that omits them (RM common master06 \
                       §\"Committal and Audits\": those attributes \"should be \
                       copied into the corresponding attributes of the \
                       `commit_audit` of each VERSION included in the \
                       CONTRIBUTION\"), and the envelope `change_type` is the \
                       aggregate of the members' — \"This may sometimes be \
                       approximate, and is not expected to be used as a \
                       computable value\" (master06 §Contributions), so it is \
                       NOT cross-checked against them. `preceding_version_uid` \
                       is the OBJECT_VERSION_ID object shown; a bare string is \
                       also accepted. A `523|deleted|` member carries NO `data` \
                       — logical deletion \"delete its `data`… set the \
                       `lifecycle_state` value to the code for `deleted`\" (RM \
                       common master06 §\"Logical Deletion\"), which is why \
                       this server does not enforce the released UpdateVersion \
                       schema's `data: required` on such a member \
                       (adjudicated: we \
                       follow RM). The example is canonical JSON with two \
                       members (a COMPOSITION creation + an EHR_STATUS \
                       modification). Under a Simplified-Format \
                       `Content-Type`, SPECITS-84 fixes what changes: \"the \
                       CONTRIBUTION envelope itself remains canonical JSON \
                       (i.e. `uid`, `versions[]` metadata, and `audit` follow \
                       the canonical RM serialization). Only the inner \
                       versioned payload - each `versions[i].data` (the \
                       embedded `COMPOSITION`, `EHR_STATUS`, or `FOLDER`) - is \
                       serialized in the chosen FLAT or STRUCTURED form\" — and \
                       such a body additionally requires the \
                       `openehr-template-id` header."
    ),
    responses(
        (
            status = 201, description = "Created — every member version and \
                                        attestation committed, or none of them \
                                        (RM common master06 §\"Committal and \
                                        Audits\"). `ETag` (weak `W/` form) \
                                        carries the new `contribution_uid` \
                                        (NOT a version uid), `Location` the \
                                        CONTRIBUTION URL, and \
                                        `Preference-Applied` the `Prefer` token \
                                        actually honoured. The body is \
                                        `Prefer`-conditional \
                                        (`Requests_and_responses.md` §\"Prefer \
                                        minimal, identifier or full \
                                        representation response\"; ITS-REST \
                                        `specifications/responses/201_CONTRIBUTION.yaml`): \
                                        the committed CONTRIBUTION for \
                                        `return=representation` — its \
                                        `versions` are the OBJECT_REFs of the \
                                        versions this commit MINTED, so the \
                                        client learns each new version uid from \
                                        them (the `representation` example) —, \
                                        the single-`uid` object carrying the \
                                        CONTRIBUTION uid for \
                                        `return=identifier` (the `identifier` \
                                        example), and no body at all for the \
                                        default `return=minimal`.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<contribution_uid>\"` \
                                — ITS-REST \
                                `specifications/headers/ETag_CONTRIBUTION.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is the `contribution_uid` identifier\", whose \
                                own example already shows the `W/` form \
                                (required since Release 1.1.0, §\"ETag and \
                                Last-Modified\"). It is NOT a version uid: a \
                                CONTRIBUTION may have minted several."),
                ("Location" = String,
                 description = "The URL of the newly created CONTRIBUTION, \
                                `<base_path>/ehr/<ehr_id>/contribution/<contribution_uid>` \
                                (ITS-REST \
                                `specifications/headers/Location_CONTRIBUTION.yaml`; \
                                §Location: used \"in `201 Created` responses \
                                when a new resource is successfully \
                                created\")."),
                ("Last-Modified" = String,
                 description = "The commit instant of this CONTRIBUTION's \
                                audit, as an HTTP-date. §\"ETag and \
                                Last-Modified\": \"Both `ETag` and \
                                `Last-Modified` SHOULD be included in \
                                responses for VERSION, VERSIONED_OBJECT, or \
                                other resources that have versioning or unique \
                                state identifiers\", the value \"derived from \
                                `VERSION.commit_audit.time_committed.value`\" \
                                — a CONTRIBUTION is immutable and the 201 \
                                already names its unique identifier, so the \
                                committal is its one modification instant. \
                                Emitted under every `Prefer` setting; under \
                                `return=minimal` it is the only channel the \
                                instant has, since that branch returns no \
                                body."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured (§\"Representation details \
                                negotiation\")."),
            ),
            content(
                (serde_json::Value = "application/json", examples(
                    ("representation" = (summary = "Prefer: return=representation — the committed CONTRIBUTION",
                     value = json!({
                        "_type": "CONTRIBUTION",
                        "uid": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" },
                        "audit": {
                            "_type": "AUDIT_DETAILS",
                            "system_id": "openEHRSys.example.com",
                            "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                            "change_type": {
                                "_type": "DV_CODED_TEXT", "value": "modification",
                                "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" }
                            },
                            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                            "description": { "_type": "DV_TEXT", "value": "Encounter recorded at triage; EHR status refreshed" }
                        },
                        "versions": [ {
                            "_type": "OBJECT_REF",
                            "namespace": "local",
                            "type": "COMPOSITION",
                            "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                        }, {
                            "_type": "OBJECT_REF",
                            "namespace": "local",
                            "type": "EHR_STATUS",
                            "id": { "_type": "OBJECT_VERSION_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31::openEHRSys.example.com::2" }
                        } ]
                     }))),
                    ("identifier" = (summary = "Prefer: return=identifier — only the CONTRIBUTION uid",
                     value = json!({ "uid": "0826851c-c4c2-4d61-92b9-410fb8275ff0" })))
                )),
                (serde_json::Value = "application/openehr.wt.flat+json"),
                (serde_json::Value = "application/openehr.wt.structured+json")
            )
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content, or the modification type does \
                                      not match the operation - i.e. first \
                                      version of a MODIFICATION)\" (ITS-REST \
                                      `specifications/responses/400_CONTRIBUTION.yaml`). \
                                      Here that is: `ehr_id` is not a UUID; the \
                                      envelope is not parseable JSON; a \
                                      non-creation change type \
                                      (`250`/`251`/`252`/`253`/`816`/`817`, or \
                                      `523`/`666`) on a member with NO \
                                      `preceding_version_uid` — the released \
                                      first-version-of-a-MODIFICATION trigger \
                                      itself; or a member names a \
                                      `preceding_version_uid` whose \
                                      VERSIONED_OBJECT does not exist (the \
                                      modification matches no stored object — a \
                                      body-referenced target, which is why it \
                                      is not the `404` reserved for the URI's \
                                      `ehr_id`). A `249|creation|` member \
                                      carrying a `preceding_version_uid` is the \
                                      UNASSIGNED mirror of the directional \
                                      released trigger and answers `422` — see \
                                      that branch.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id` (the released trigger, \
                                      `404_unknown_ehr_id`) — the target EHR \
                                      must exist before a change-set can be \
                                      committed to it (SM \
                                      `i_ehr_contribution.adoc` \
                                      `commit_contribution` `Pre_has_ehr`). \
                                      Content the committed CONTRIBUTION merely \
                                      REFERS to is not covered by this branch \
                                      (see `400`/`412`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a CONTRIBUTION envelope is served as \
                                      canonical `application/json` only, \
                                      optionally with its inner \
                                      `versions[i].data` payloads in \
                                      `application/openehr.wt.flat+json` / \
                                      `application/openehr.wt.structured+json` \
                                      — there is no canonical-XML CONTRIBUTION \
                                      shape, so `application/xml` is refused \
                                      (`Resources.md` §\"JSON \
                                      Format\"/§\"Simplified Formats\": \"If the \
                                      service cannot fulfill this aspect of the \
                                      request, it MUST respond with HTTP status \
                                      code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 409, description = "The commit conflicts with existing state \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `409` row: the \
                                      request \"might generate a duplicate or a \
                                      conflict\"). The released trigger is the \
                                      client-supplied envelope `uid`: it \"will \
                                      be accepted in case is not in-use, \
                                      otherwise error will be returned\" \
                                      (`contribution_create.yaml`), and \
                                      `409.yaml` is that error — \"returned when \
                                      a resource with same identifier(s) \
                                      already exists\". The other three \
                                      triggers are OUR OWN DESIGN, \
                                      adjudicated, because no released text \
                                      assigns them a code: the EHR is not \
                                      modifiable (`EHR_STATUS.is_modifiable = \
                                      false` and the change-set touches \
                                      something other than the EHR_STATUS — RM \
                                      ehr master04 §\"EHR Active Status\", the \
                                      refusal is spec-required, the code is \
                                      ours); a member would create a SECOND \
                                      EHR_STATUS / EHR_ACCESS or re-create an \
                                      existing root FOLDER hierarchy (RM ehr, \
                                      EHR class: `ehr_status` 1..1); and a \
                                      `523|deleted|` member targeting the \
                                      EHR_STATUS, which the same 1..1 \
                                      cardinality forbids.",
         body = serde_json::Value),
        (status = 412, description = "A member's `preceding_version_uid` names \
                                      an EXISTING VERSIONED_OBJECT but a \
                                      version that does not exist or has \
                                      already been superseded, so the change-set \
                                      was not committed. No released text \
                                      assigns this branch a code — the \
                                      `400_CONTRIBUTION` trigger is about the \
                                      change TYPE, not a stale target — so the \
                                      choice is OURS, adjudicated: the member's \
                                      `preceding_version_uid` is the same \
                                      lost-update precondition the direct \
                                      routes carry in `If-Match`, which \
                                      §\"If-Match and accidental overwrites\" \
                                      answers with `412 Precondition Failed`. A \
                                      target VERSIONED_OBJECT that does not \
                                      exist at all is `400`, not `412`.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one \
                                      this resource can process — anything \
                                      outside `application/json` and the two \
                                      Simplified Formats, `application/xml` \
                                      included (a CONTRIBUTION has no canonical \
                                      XML wire shape), or a deprecated \
                                      `…schema+json` variant (`Resources.md` \
                                      §\"JSON Format\": \"If the service cannot \
                                      process the request payload as JSON \
                                      format, it MUST respond with HTTP status \
                                      code `415 Unsupported Media Type`\"; \
                                      §\"Simplified Formats\" carries the same \
                                      MUST for the simplified types).",
         body = serde_json::Value),
        (status = 422, description = "The CONTRIBUTION was well-formed but \
                                      cannot be followed \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row: \"The \
                                      request was well-formed but was unable to \
                                      be followed due to semantic errors\"). \
                                      Every trigger below is OUR OWN \
                                      assignment, adjudicated — the released \
                                      operation declares only \
                                      `400`/`404`/`409`: an empty `versions: \
                                      []` (NewContribution sets no `minItems` \
                                      and RM CONTRIBUTION declares no \
                                      invariant, so the rejection itself is \
                                      ours); a malformed envelope `uid` (not a \
                                      HIER_OBJECT_ID UUID); a `change_type` — \
                                      on a member or on the envelope — that is \
                                      not a code of the openEHR \
                                      `audit_change_type` group \
                                      (`AUDIT_DETAILS.Change_type_valid`); a \
                                      `249|creation|` member carrying a \
                                      `preceding_version_uid` (the UNASSIGNED \
                                      mirror of the released directional `400` \
                                      trigger — creation makes a NEW \
                                      VERSIONED_OBJECT, RM common master06 \
                                      §Contributions); `data` \
                                      on a `523|deleted|` member (its data \
                                      \"is set to Void\", RM common master06 \
                                      §Contributions) or on a `666|attestation|` \
                                      member (an attestation adds no content); \
                                      missing `data` on a creation or \
                                      modification member (a `523`/`666` \
                                      member with no `preceding_version_uid` \
                                      is the released first-version `400`); a \
                                      `666` \
                                      member with no `commit_audit`; a member \
                                      whose object kind is out of the \
                                      contribution's scope (a demographic PARTY \
                                      in an EHR CONTRIBUTION); a `commit_audit` \
                                      failing the AUDIT_DETAILS invariants \
                                      (empty `system_id`, an invalid \
                                      committer); a Simplified-Format body \
                                      without the `openehr-template-id` header \
                                      or not fitting that template; and any \
                                      member failing template or RM-invariant \
                                      validation — each member is validated \
                                      exactly as the direct commit route would \
                                      validate it, relaxed for a \
                                      `553|incomplete|` lifecycle (master06 \
                                      §\"Incomplete Content\"). A member that \
                                      OMITS the required `lifecycle_state`, or \
                                      that carries `other_input_version_uids` \
                                      (a property the released `UPDATE_VERSION` \
                                      does not declare — merge provenance is \
                                      served on reads only), is a `400` shape \
                                      failure, not a `422`.",
         body = serde_json::Value),
        (status = 403, description = "OUR OWN EXTENSION — no ITS-REST \
                                      operation defines restriction of \
                                      processing. The addressed object, or the \
                                      whole EHR, is restricted, so the write is \
                                      refused and the stored content stays \
                                      exactly as it is (GDPR Art. 18(2) leaves \
                                      storage as the only processing a \
                                      restriction admits). Distinct from an \
                                      authorization refusal, and from the 409 a \
                                      non-modifiable EHR answers with. The \
                                      401/403 rows of the access layer are \
                                      unchanged.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn contribution_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "contribution_create",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// List an EHR's CONTRIBUTIONs, newest-first, paged
/// (`GET /ehr/{ehr_id}/contribution`).
///
/// **OUR OWN EXTENSION — no openEHR spec governs this route.** ITS-REST 1.1.0
/// defines exactly one CONTRIBUTION read, the by-uid GET
/// (`specifications/operations/contribution_get.yaml`); there is no collection
/// GET on `/ehr/{ehr_id}/contribution`, so nothing about the shape below is
/// spec-derived. The SM does declare a `list_contributions` operation (SM
/// `i_ehr_contribution.adoc`) that the released REST API never surfaced — the
/// unrealized-operation gap is adjudicated in the conformance
/// catalogue — but this route is not a binding of it either: it answers a
/// SUMMARY row per CONTRIBUTION rather than the ids that operation returns.
///
/// The response is `{ "rows": [ { uid, time_committed, committer, change_type,
/// change_type_rubric } ], "total" }`. `committer` is the audit committer's
/// name only (a summary string — the by-uid GET returns the full `PARTY_PROXY`),
/// `change_type` the stored `audit_change_type` code and `change_type_rubric`
/// its display rubric from the same terminology bundle the by-uid GET's
/// `DV_CODED_TEXT.value` uses, so consumers never map codes locally. `total`
/// counts ALL of the EHR's CONTRIBUTIONs, not the returned window.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/contribution", tag = "CONTRIBUTION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\"). The one part of \
                        this route that is spec-shaped.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "OUR EXTENSION (no openEHR spec governs it) — row offset \
                        into the newest-first list; default 0. A negative or \
                        unparseable value is CLAMPED to the default rather than \
                        rejected, so this parameter never produces a `400`.",
         example = json!(0)),
        ("fetch" = Option<i64>, Query,
         description = "OUR EXTENSION (no openEHR spec governs it) — maximum \
                        rows to return; default 20, hard-capped at 100. A \
                        non-positive or unparseable value falls back to the \
                        default and a larger one is capped, so this parameter \
                        never produces a `400`.",
         example = json!(20))
    ),
    responses(
        (status = 200, description = "The EHR's CONTRIBUTIONs, newest first — \
                                      OUR OWN payload shape, canonical \
                                      `application/json` only. No `ETag` / \
                                      `Last-Modified`: a paged list is not a \
                                      resource with a version or a unique state \
                                      identifier (§\"ETag and Last-Modified\" \
                                      scopes the SHOULD to those).",
         body = serde_json::Value,
         example = json!({
             "rows": [ {
                 "uid": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                 "time_committed": "2026-07-26T09:12:44.512331Z",
                 "committer": "Dr Jane Roe",
                 "change_type_rubric": "modification",
                 "change_type": "251"
             } ],
             "total": 1
         })),
        (status = 400, description = "`ehr_id` is not a UUID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \"malformed \
                                      request syntax, syntactically invalid \
                                      content\"). `offset`/`fetch` never reach \
                                      this branch — they are clamped, not \
                                      rejected.",
         body = serde_json::Value),
        (status = 404, description = "Unknown `ehr_id` — the EHR is checked \
                                      before the page is read, so a missing EHR \
                                      is a clean `404` rather than an empty \
                                      list.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      this extension DTO is served as \
                                      `application/json` only — it is not an RM \
                                      type, so it has neither a canonical-XML \
                                      shape nor a Simplified-Format mapping \
                                      (`Resources.md` §\"JSON Format\": \"If the \
                                      service cannot fulfill this aspect of the \
                                      request, it MUST respond with HTTP status \
                                      code `406 Not Acceptable`\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn contribution_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "contribution_list",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve a CONTRIBUTION
/// (`GET /ehr/{ehr_id}/contribution/{contribution_uid}`).
///
/// The served body is the COMMITTED change-set: the CONTRIBUTION's own
/// `AUDIT_DETAILS` plus `versions` — by default the `OBJECT_REFs` of the versions
/// it affected ("a `CONTRIBUTION` object will be created, listing the affected
/// `VERSION` objects, and including its own audit object" — RM common master06
/// §Contributions), or, with `Prefer: return=representation, resolve_refs`, the
/// full `ORIGINAL_VERSIONs` those refs point at.
///
/// A `666|attestation|` member commits no new version, yet still affects the
/// `ORIGINAL_VERSION` it attests, so that version appears in `versions` too
/// (master06 §Contributions lists the affected versions, and §Attestation makes
/// an attestation a change to an existing one).
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/contribution/{contribution_uid}", tag = "CONTRIBUTION",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("contribution_uid" = String, Path,
         description = "The CONTRIBUTION uid — a PLAIN UUID (ITS-REST \
                        `specifications/parameters/path/contribution_uid.yaml`: \
                        `type: string, format: uuid`), i.e. the HIER_OBJECT_ID \
                        under `CONTRIBUTION.uid.value`. A CONTRIBUTION is not \
                        change-controlled, so this segment is NEVER the \
                        three-part `{object_id}::{creating_system_id}::{version_tree_id}` \
                        form and has no implicit-latest reading: a token that \
                        is not a UUID is syntactically invalid content and is \
                        `400` (`Requests_and_responses.md` §\"HTTP status \
                        codes\", the `400` row; the released text assigns the \
                        malformed-identifier case no branch of its own, so the \
                        400/404 split is OUR OWN policy, adjudicated).",
         example = "0826851c-c4c2-4d61-92b9-410fb8275ff0"),
        ("Prefer" = Option<String>, Header,
         description = "`return=representation, resolve_refs` asks for the \
                        member OBJECT_REFs to be resolved in place: \"Clients \
                        MAY request that object references (e.g., OBJECT_REF) \
                        be resolved into full or partial representations\" \
                        (`Requests_and_responses.md` §\"Prefer resolving Object \
                        references\"). Each `versions[i]` is then the full \
                        VERSION — envelope, `commit_audit`, \
                        `lifecycle_state`, `signature`, and the `data` payload \
                        — instead of a reference (an IMPORTED_VERSION member \
                        carries its wrapped ORIGINAL_VERSION under `item`). Without the token the members \
                        stay OBJECT_REFs and carry no `data` at all. That \
                        binding is also what makes a Simplified-Format `Accept` \
                        meaningful on this read: the released operation \
                        promises the simplified serialization of \
                        `versions[i].data` while the declared `200` body \
                        (`schemas/common/Contribution.yaml`) has no `data` \
                        anywhere, and resolving the refs is the only state in \
                        which both hold — OUR resolution of that released \
                        conflict, adjudicated.",
         example = "return=representation, resolve_refs")
    ),
    responses(
        (
            status = 200, description = "The CONTRIBUTION. The envelope is \
                                        ALWAYS canonical JSON — SPECITS-84 \
                                        (Amendment_record, 27 Apr 2026) and \
                                        ITS-REST \
                                        `specifications/responses/200_CONTRIBUTION.yaml`: \
                                        \"the response body is still a \
                                        canonical CONTRIBUTION envelope; only \
                                        each `versions[i].data` payload is \
                                        serialized in the requested FLAT or \
                                        STRUCTURED form\" — so a simplified \
                                        `Accept` changes only resolved inner \
                                        COMPOSITIONs; unresolved OBJECT_REF \
                                        members have no `data` and pass through \
                                        untouched, and a resolved non-COMPOSITION \
                                        payload (EHR_STATUS, FOLDER) is `406`, \
                                        the Simplified Formats being defined \
                                        for templated COMPOSITION content only. \
                                        The example is the default, canonical, \
                                        unresolved form: `versions` as \
                                        OBJECT_REFs whose `id` is an \
                                        OBJECT_VERSION_ID, `namespace` is \
                                        `local` and `type` is the affected DATA \
                                        class (COMPOSITION / EHR_STATUS / \
                                        FOLDER — the discipline of the released \
                                        `Contribution.yaml` example, even \
                                        though RM common master06 describes the \
                                        list as the affected VERSION objects), \
                                        with the full AUDIT_DETAILS whose \
                                        `description` is optional (0..1 in RM, \
                                        so it is absent when the committer sent \
                                        none).",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag `W/\"<contribution_uid>\"` \
                                — the same identity the creating `201` \
                                declared (ITS-REST \
                                `specifications/headers/ETag_CONTRIBUTION.yaml`). \
                                The released `200_CONTRIBUTION` declares only \
                                `Content-Type`; serving the tag here follows \
                                §\"ETag and Last-Modified\", whose SHOULD \
                                covers \"resources that have versioning or \
                                unique state identifiers\" — a CONTRIBUTION is \
                                immutable and has exactly one such identifier. \
                                Reading that SHOULD as reaching this resource \
                                is OURS, adjudicated."),
                ("Last-Modified" = String,
                 description = "The commit instant as an HTTP-date, taken from \
                                the CONTRIBUTION `audit.time_committed` — the \
                                same instant §\"ETag and Last-Modified\" points \
                                at for openEHR resources (\"derived from \
                                VERSION.commit_audit.time_committed.value\"), \
                                which for a CONTRIBUTION is the committal act \
                                its own audit records (RM common master06 \
                                §\"Committal and Audits\"). A CONTRIBUTION is \
                                never rewritten, so this value never changes."),
            ),
            content(
                (serde_json::Value = "application/json", example = json!({
                    "_type": "CONTRIBUTION",
                    "uid": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" },
                    "audit": {
                        "_type": "AUDIT_DETAILS",
                        "system_id": "openEHRSys.example.com",
                        "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                        "change_type": {
                            "_type": "DV_CODED_TEXT", "value": "modification",
                            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" }
                        },
                        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe" },
                        "description": { "_type": "DV_TEXT", "value": "Encounter recorded at triage; EHR status refreshed" }
                    },
                    "versions": [ {
                        "_type": "OBJECT_REF",
                        "namespace": "local",
                        "type": "COMPOSITION",
                        "id": { "_type": "OBJECT_VERSION_ID", "value": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21::openEHRSys.example.com::1" }
                    }, {
                        "_type": "OBJECT_REF",
                        "namespace": "local",
                        "type": "EHR_STATUS",
                        "id": { "_type": "OBJECT_VERSION_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31::openEHRSys.example.com::2" }
                    } ]
                })),
                (serde_json::Value = "application/openehr.wt.flat+json"),
                (serde_json::Value = "application/openehr.wt.structured+json")
            )
        ),
        (status = 400, description = "`ehr_id` or `contribution_uid` is not a \
                                      UUID (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `400` row: \
                                      \"malformed request syntax, syntactically \
                                      invalid content\") — including a \
                                      three-part OBJECT_VERSION_ID in the \
                                      `contribution_uid` segment, which names \
                                      no CONTRIBUTION identity. A well-formed \
                                      UUID that matches nothing is `404`, not \
                                      `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when a \
                                      CONTRIBUTION with `contribution_uid` does \
                                      not exist\" (ITS-REST \
                                      `specifications/responses/404_CONTRIBUTION.yaml`). \
                                      \"Does not exist\" is judged WITHIN the \
                                      addressed EHR: a CONTRIBUTION uid that \
                                      belongs to another EHR does not exist \
                                      under this `ehr_id` and is `404` too — \
                                      the released sentence does not spell the \
                                      cross-EHR case out, so reading it that \
                                      way is ours, and it follows from the \
                                      resource being addressed as a \
                                      sub-resource of the EHR.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the CONTRIBUTION envelope is served as \
                                      canonical `application/json` only \
                                      (there is no canonical-XML CONTRIBUTION \
                                      shape, so `application/xml` is refused), \
                                      and a Simplified-Format `Accept` is \
                                      refused when a resolved member payload is \
                                      not a COMPOSITION (`Resources.md` \
                                      §\"JSON Format\"/§\"Simplified Formats\": \
                                      \"If the service cannot fulfill this \
                                      aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\").",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime specification-generation \
                                      selection. The stored version body uses \
                                      openEHR specification surface this \
                                      deployment's active `spec_profile` does \
                                      not define, so it is refused rather than \
                                      served under a generation set that cannot \
                                      express it — and never down-converted. \
                                      Reachable only where `spec_profile = \"stable\"` \
                                      is configured; the body names switching \
                                      back to `development` as the remedy.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn contribution_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "contribution_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

// ── Item tags ─────────────────────────────────────────────────────────────────
// An ITEM_TAG carries no uid and no version of its own (RM `item_tag.adoc`), and
// ITS-REST `Requests_and_responses.md` §"ETag and Last-Modified" scopes those
// headers to resources "that have versioning or unique state identifiers" — so no
// tag route serves or accepts `ETag` / `Last-Modified` / `If-Match` / the
// committal headers, and a tag write commits no CONTRIBUTION. The canonical XML
// ITS defines no ITEM_TAG type, so tag routes are JSON-only: an XML `Accept` is
// `406`, an XML `Content-Type` on the two PUTs `415`.

/// Retrieve every `ITEM_TAG` in an EHR (`GET /ehr/{ehr_id}/tags`).
///
/// The EHR-wide aggregate read: "Retrieves the list of `ITEM_TAG` resources
/// associated with any target VERSION or `VERSIONED_OBJECT` within the EHR
/// identified by `ehr_id`" (ITS-REST
/// `specifications/operations/ehr_tags_get.yaml`). One list therefore spans
/// BOTH target forms (a `VERSIONED_OBJECT` container and a specific VERSION) and
/// every taggable kind (COMPOSITION, `EHR_STATUS`, FOLDER). Each row names its
/// own `target` by identifier, but NOT by RM class: the RM types `target` as a
/// bare `UID_BASED_ID` (`item_tag.adoc`), which carries no `type` member, so a
/// client that needs the target's kind resolves the uid. (The released OAS
/// models `target` as an `OBJECT_REF`, which would carry it — a real
/// conflict; the released RM wins.)
///
/// The list is unbounded: the released operation declares no `offset`/`fetch`,
/// ordering or limit parameter, so every matching tag is returned. The
/// (`key`, `target_path`) ordering the server applies is OURS — no openEHR spec
/// governs tag ordering.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/tags", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("tag_key" = Option<String>, Query,
         description = "Filter by ITEM_TAG `key`. The three filters are \
                        AND-combined, each an EXACT, case-sensitive match on \
                        the stored value; an omitted filter constrains \
                        nothing — \"In case no such parameter is provided then \
                        all ITEM_TAG resources will be retrieved\" \
                        (`ehr_tags_get.yaml`). None of exactness, case \
                        sensitivity or the combination rule is fixed by the \
                        released text, so those semantics are OURS, \
                        adjudicated. The \
                        parameter is SCALAR: the released description says the \
                        list \"can be filtered by the given one or more \
                        `tag_key`, `tag_value`, `tag_target_path` query \
                        parameters\" while each released parameter schema is a \
                        plain `type: string` — the plural reads as one or more \
                        OF THE THREE parameters (the mismatch is a released-text \
                        defect reported upstream), and a repeated parameter has \
                        no defined meaning.",
         example = "flag"),
        ("tag_value" = Option<String>, Query,
         description = "Filter by ITEM_TAG `value` — same semantics as \
                        `tag_key` (exact, case-sensitive, AND-combined, \
                        scalar).",
         example = "follow-up"),
        ("tag_target_path" = Option<String>, Query,
         description = "Filter by ITEM_TAG `target_path` — same semantics as \
                        `tag_key` (exact, case-sensitive, AND-combined, \
                        scalar). Tags stored WITHOUT a `target_path` (the \
                        absent 0..1 case) match no value of this filter; they \
                        are reached by omitting it.",
         example = "/context/start_time/value")
    ),
    responses(
        (
            status = 200, description = "The matching ITEM_TAG list. \"This \
                                        will return an empty list when there \
                                        is no matching ITEM_TAG associated \
                                        with any target within given EHR\" \
                                        (`ehr_tags_get.yaml`) — an EHR with no \
                                        matching tag is `200 []`, never `404`. \
                                        Every row carries the SERVER-ASSIGNED \
                                        `target` and `owner_id`: `target` is a \
                                        bare UID_BASED_ID (RM `item_tag.adoc`: \
                                        `target: UID_BASED_ID`, \"which may be \
                                        a `VERSIONED_OBJECT<T>` or a \
                                        `VERSION<T>`\") — a `HIER_OBJECT_ID` \
                                        for a container target, an \
                                        `OBJECT_VERSION_ID` for a VERSION \
                                        target — and `owner_id` is the RM \
                                        `OBJECT_REF` of the owning EHR \
                                        (`{namespace: local, type: EHR, id: \
                                        <ehr_id>}`). The released OAS `ItemTag` \
                                        schema types `target` as an OBJECT_REF \
                                        instead; the RM is the RELEASED \
                                        component and wins. The example shows \
                                        one container-targeted COMPOSITION tag \
                                        and one VERSION-targeted EHR_STATUS \
                                        tag — the aggregate spans kinds, even \
                                        though the released declaration reuses \
                                        `200_COMPOSITION_ItemTagList_retrieved` \
                                        (items typed `ItemTagOfComposition`) \
                                        for it, a released-text defect \
                                        reported upstream.",
            content((serde_json::Value = "application/json", example = json!([ {
                "_type": "ITEM_TAG",
                "key": "flag",
                "value": "follow-up",
                "target_path": "/context/start_time/value",
                "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            }, {
                "_type": "ITEM_TAG",
                "key": "reviewed",
                "value": "true",
                "target": { "_type": "OBJECT_VERSION_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31::openEHRSys.example.com::2" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            } ])))
        ),
        (status = 400, description = "`ehr_id` is not a UUID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \
                                      \"malformed request syntax, \
                                      syntactically invalid content\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id.yaml`). \
                                      An EXISTING EHR whose tags do not match \
                                      the filters is `200 []`, not `404`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      an ITEM_TAG list is served as canonical \
                                      `application/json` only. The canonical \
                                      XML ITS defines no ITEM_TAG type, so an \
                                      `application/xml` Accept — a member of \
                                      the released `Accept_canonical` enum, \
                                      stalled shape on this operation — is \
                                      refused (`Resources.md` §\"XML Format\": \
                                      \"If the service cannot fulfill this \
                                      aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_tags_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve a COMPOSITION's item tags
/// (`GET /ehr/{ehr_id}/composition/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// COMPOSITION version or `VERSIONED_COMPOSITION` identified by `uid_based_id`
/// and owned by EHR identified by `ehr_id`"
/// (`specifications/operations/composition_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag sets — see the parameter.
/// A tag carries exactly one `target` (RM `item_tag.adoc`), so a container tag
/// is never served by a version-form read and vice versa.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/composition/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: \"an \
                        OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`), or a form of \
                        a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the \
                        COMPOSITION version …, whereas the latter … is be used \
                        to get the tags of the target VERSIONED_COMPOSITION \
                        container\" (`composition_tags_get.yaml`). The two \
                        address DISJOINT sets: an ITEM_TAG has exactly one \
                        `target` (RM `item_tag.adoc`), so a tag written \
                        against the container is invisible to the version form \
                        and a tag written against \
                        `…::openEHRSys.example.com::1` is invisible both to \
                        the container form and to every other version. There \
                        is no implicit-latest reading of the container form \
                        here — it names the VERSIONED_COMPOSITION's own tag \
                        collection, not the latest version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1")
    ),
    responses(
        (
            status = 200, description = "The target's ITEM_TAG list. \"This \
                                        will return an empty list when there \
                                        is no ITEM_TAG associated with the \
                                        given target\" \
                                        (`composition_tags_get.yaml`) — an \
                                        EXISTING, untagged target is `200 []`; \
                                        a target that does not exist is `404`. \
                                        `target` and `owner_id` are \
                                        server-assigned from the route: \
                                        `target` is the bare UID_BASED_ID of \
                                        the addressed collection (RM \
                                        `item_tag.adoc`: \"may be a \
                                        `VERSIONED_OBJECT<T>` or a \
                                        `VERSION<T>`\") — `HIER_OBJECT_ID` for \
                                        the container form, `OBJECT_VERSION_ID` \
                                        for the version form — and `owner_id` \
                                        the OBJECT_REF of the owning EHR. \
                                        (The released OAS `ItemTag` schema \
                                        types `target` as an OBJECT_REF; the \
                                        RM is the RELEASED component and \
                                        wins.) `target_path` is present only \
                                        on tags that carry one — it is 0..1 in \
                                        the RM, and an empty string is stored \
                                        as absent.",
            content((serde_json::Value = "application/json", example = json!([ {
                "_type": "ITEM_TAG",
                "key": "reviewed",
                "value": "true",
                "target": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            }, {
                "_type": "ITEM_TAG",
                "key": "flag",
                "value": "follow-up",
                "target_path": "/context/start_time/value",
                "target": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            } ])))
        ),
        (status = 400, description = "`ehr_id` is not a UUID, or `uid_based_id` \
                                      is neither a UUID nor a well-formed \
                                      three-part OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \
                                      \"malformed request syntax, \
                                      syntactically invalid content\"). A \
                                      well-formed identifier that names \
                                      nothing is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id.yaml`). \
                                      Existence is judged as the operation \
                                      scopes it — \"owned by EHR identified by \
                                      `ehr_id`\" — so all of these are `404`: \
                                      an unknown versioned-object uid; one \
                                      that belongs to ANOTHER EHR; a version \
                                      form whose version does not exist; and a \
                                      uid whose stored kind is not a \
                                      COMPOSITION (an EHR_STATUS or FOLDER uid \
                                      on this route). The kind-mismatch \
                                      reading is OURS — the released sentence \
                                      does not spell it out — and follows from \
                                      the route family naming the target's \
                                      class; it is adjudicated. An EXISTING \
                                      COMPOSITION target with no tags is \
                                      `200 []`, not `404`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      an ITEM_TAG list is served as canonical \
                                      `application/json` only. The canonical \
                                      XML ITS defines no ITEM_TAG type, so an \
                                      `application/xml` Accept — a member of \
                                      the released `Accept_canonical` enum, \
                                      stalled shape on this operation — is \
                                      refused (`Resources.md` §\"XML Format\": \
                                      \"If the service cannot fulfill this \
                                      aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_tags_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Replace a COMPOSITION's item tags
/// (`PUT /ehr/{ehr_id}/composition/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// COMPOSITION version or `VERSIONED_COMPOSITION` identified by `uid_based_id`
/// and owned by EHR identified by `ehr_id`"
/// (`specifications/operations/composition_tags_update.yaml`). It is a FULL
/// COLLECTION REPLACE of the ADDRESSED collection — the container's or one
/// version's, never both: tags omitted from the body are removed, and
/// "providing an empty list will effectively remove all `ITEM_TAG` associated
/// with the given target".
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION,
/// mints no version, takes no `If-Match` and no committal headers, and serves
/// neither `ETag` nor `Last-Modified` (see the section note above).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}/composition/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: \"an \
                        OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`), or a form of \
                        a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular COMPOSITION version …, \
                        whereas the latter … is be used to update the tags of \
                        the target VERSIONED_COMPOSITION container\" \
                        (`composition_tags_update.yaml`). The two collections \
                        are DISJOINT — an ITEM_TAG has exactly one `target` \
                        (RM `item_tag.adoc`) — so replacing the container's \
                        list never touches any version's list, and replacing \
                        one version's list never touches the container's or a \
                        sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` (the default when the header is \
                        absent — `Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured — its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid — so the server applies, and \
                        declares, the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied`.",
         example = "return=representation")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                addressed target (required; there is no \
                                envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED, `value` and `target_path` are \
                                optional, and no other member is defined. \
                                `target` and `owner_id` are NOT client input — \
                                the server assigns them from the route \
                                (`target` = the addressed `uid_based_id`, \
                                `owner_id` = the addressed EHR), which is why \
                                the write schema omits them; a body that \
                                nonetheless carries them — or any other \
                                undeclared member — is REFUSED `400` naming \
                                the member. The schema declares \
                                `additionalProperties: false`, and the ITS-REST \
                                docs text says nothing about the write body's \
                                member set, so the released OAS grounds the \
                                expectation under the documented oracle order; \
                                the refusal is the released constraint, not our \
                                own strictness. A member of the wrong JSON type \
                                (a numeric `value`, say) is the same `400` — \
                                never a silently-absent attribute. `[]` is \
                                the clear-all form, never an error: \"Providing \
                                an empty list will effectively remove all \
                                ITEM_TAG associated with the given target\" \
                                (`composition_tags_update.yaml`). Identity \
                                inside the list is the (`key`, `target_path`) \
                                PAIR (`Requests_and_responses.md` §item-tag \
                                headers: \"More than one ITEM_TAG may be \
                                associated with a single target, in which case \
                                they are uniquely identified by their `key` \
                                and `target_path` pair attributes\"), so two \
                                entries may share a `key` when their \
                                `target_path` differs; a DUPLICATE pair inside \
                                one body is resolved last-wins (no released \
                                rule and no `uniqueItems` — ours, \
                                adjudicated). A `target_path` of `\"\"` \
                                normalizes to ABSENT, so it is the same \
                                identity as an entry with no `target_path` at \
                                all: the RM models `target_path` 0..1 with no \
                                non-empty invariant while the released \
                                EHR_STATUS example uses `\"\"` — reconciling \
                                the two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([ {
                     "key": "reviewed",
                     "value": "true"
                 }, {
                     "key": "flag",
                     "value": "follow-up",
                     "target_path": "/context/start_time/value"
                 } ])),
    responses(
        (
            status = 200, description = "Applied, with `Prefer: \
                                        return=representation`. The body is \
                                        the full RESULTING ITEM_TAG list of \
                                        the addressed collection — every tag \
                                        now stored on it, server-assigned \
                                        `target`/`owner_id` included — not \
                                        merely the entries just sent. (The \
                                        released \
                                        `200_COMPOSITION_ItemTagList_updated` \
                                        describes itself as \"returned when \
                                        the requested ITEM_TAG list is \
                                        successfully retrieved\", a \
                                        copy-and-paste of the `_retrieved` \
                                        response text; the trigger is the \
                                        update, as stated here.) The only \
                                        response header is \
                                        `Preference-Applied` — a tag \
                                        collection has no version and no uid, \
                                        so there is no `ETag`, no \
                                        `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\")."),
            ),
            content((serde_json::Value = "application/json", example = json!([ {
                "_type": "ITEM_TAG",
                "key": "reviewed",
                "value": "true",
                "target": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            }, {
                "_type": "ITEM_TAG",
                "key": "flag",
                "value": "follow-up",
                "target_path": "/context/start_time/value",
                "target": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            } ])))
        ),
        (status = 204, description = "Applied, with no body — \"`204 No \
                                      Content` is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`\" \
                                      (`responses/204_updated.yaml`); a \
                                      `return=identifier` request resolves \
                                      here too. This is the DEFAULT branch. It \
                                      carries no resource header of any kind — \
                                      no `ETag`, no `Last-Modified`, no \
                                      `Location` — only the \
                                      `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return)."),
         )),
        (status = 400, description = "`ehr_id` or `uid_based_id` is malformed, \
                                      or the body is not a JSON ARRAY / not \
                                      parseable as JSON \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \
                                      \"malformed request syntax, \
                                      syntactically invalid content\"). A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id.yaml`). \
                                      Existence is judged as the operation \
                                      scopes it — \"owned by EHR identified by \
                                      `ehr_id`\" — so an unknown uid, a uid \
                                      owned by another EHR, a non-existent \
                                      version of an existing container, and a \
                                      uid whose stored kind is not a \
                                      COMPOSITION are all `404`. The \
                                      kind-mismatch reading is OURS \
                                      (adjudicated): the released sentence does \
                                      not spell it out, and it follows from \
                                      the route family naming the target's \
                                      class.",
         body = serde_json::Value),
        (status = 406, description = "A `return=representation` request whose \
                                      `Accept` cannot be satisfied: the \
                                      ITEM_TAG list is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no ITEM_TAG type, so the \
                                      released `Accept_canonical` enum's \
                                      `application/xml` member is stalled \
                                      shape here) — `Resources.md` §\"XML \
                                      Format\": \"If the service cannot \
                                      fulfill this aspect of the request, it \
                                      MUST respond with HTTP status code `406 \
                                      Not Acceptable`\". The default `204` \
                                      branch returns no body and negotiates \
                                      nothing.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot process the request \
                                      payload as JSON format, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\"). An ABSENT `Content-Type` \
                                      declares nothing and is accepted as \
                                      JSON.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The released \
                                      operation declares only `400` for a bad \
                                      request; answering `422` for these \
                                      SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_tags_update",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Delete a COMPOSITION's item tags under one key
/// (`DELETE /ehr/{ehr_id}/composition/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target COMPOSITION version or `VERSIONED_COMPOSITION` identified by
/// `uid_based_id` and owned by EHR identified by `ehr_id`"
/// (`specifications/operations/composition_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the
/// (`key`, `target_path`) pair, the route carries no `target_path` selector,
/// and the released text says "resource(s)" — so EVERY tag under `key` on the
/// addressed collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/ehr/{ehr_id}/composition/{uid_based_id}/tags/{key}", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: an \
                        OBJECT_VERSION_ID (\"used to delete the tags a \
                        particular (target) version of the COMPOSITION \
                        version\") or a HIER_OBJECT_ID (\"used to delete the \
                        tags of the target VERSIONED_COMPOSITION container\" — \
                        `composition_tags_delete.yaml`). The two collections \
                        are DISJOINT (an ITEM_TAG has exactly one `target`, RM \
                        `item_tag.adoc`), so deleting a key from the container \
                        leaves the same key on every version untouched, and \
                        vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted on the \
                        addressed collection — \"The ITEM_TAG key\" \
                        (`parameters/path/key.yaml`, `type: string`), an \
                        UNCONSTRAINED string with no format, pattern or length \
                        bound, taken percent-decoded from the path segment (a \
                        key containing `/`, `?` or `#` must be percent-encoded \
                        by the client). It selects a SET: identity is the \
                        (`key`, `target_path`) pair and this route has no \
                        `target_path` selector, so all tags under the key go. \
                        (The released operation descriptions call this \
                        parameter `tag_key` in prose while the path parameter \
                        is `key` — a released-text inconsistency reported \
                        upstream; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The tags under `key` were deleted from \
                                      the addressed collection \
                                      (`responses/204_deleted.yaml`). No body \
                                      and no headers: an ITEM_TAG has no \
                                      version and no uid, so there is nothing \
                                      for an `ETag`/`Last-Modified` to carry, \
                                      and §\"HTTP headers\" records that \"the \
                                      `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\". The released response text \
                                      says \"(logically) deleted\", which is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tag simply ceases to exist."),
        (status = 400, description = "`ehr_id` is not a UUID, or \
                                      `uid_based_id` is neither a UUID nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row).",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id_or_key.yaml`). \
                                      The THIRD trigger makes this operation \
                                      deliberately NON-IDEMPOTENT at the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      existence is judged as the operation \
                                      scopes it (\"owned by EHR identified by \
                                      `ehr_id`\"): an unknown uid, a uid owned \
                                      by another EHR, a missing version, and a \
                                      uid whose stored kind is not a \
                                      COMPOSITION are all `404` — the \
                                      kind-mismatch reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn composition_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "composition_tags_delete",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Retrieve an `EHR_STATUS`'s item tags
/// (`GET /ehr/{ehr_id}/ehr_status/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `EHR_STATUS` version or `VERSIONED_EHR_STATUS` identified by `uid_based_id` and
/// owned by EHR identified by `ehr_id`"
/// (`specifications/operations/ehr_status_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag sets — see the parameter.
/// A tag carries exactly one `target` (RM `item_tag.adoc`), so a container tag
/// is never served by a version-form read and vice versa.
#[utoipa::path(
    get, path = "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: \"an \
                        OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`), or a form of \
                        a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the \
                        EHR_STATUS version …, whereas the latter … is be used \
                        to get the tags of the target VERSIONED_EHR_STATUS \
                        container\" (`ehr_status_tags_get.yaml`). The two \
                        address DISJOINT sets: an ITEM_TAG has exactly one \
                        `target` (RM `item_tag.adoc`), so a tag written \
                        against the container is invisible to the version form \
                        and a tag written against \
                        `…::openEHRSys.example.com::2` is invisible both to \
                        the container form and to every other version. The \
                        container form names the VERSIONED_EHR_STATUS's own \
                        tag collection — not the latest version's.",
         example = "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31")
    ),
    responses(
        (
            status = 200, description = "The target's ITEM_TAG list. \"This \
                                        will return an empty list when there \
                                        is no ITEM_TAG associated with the \
                                        given target\" \
                                        (`ehr_status_tags_get.yaml`) — an \
                                        EXISTING, untagged target is `200 []`; \
                                        a target that does not exist is `404`. \
                                        `target` and `owner_id` are \
                                        server-assigned from the route: \
                                        `target` is the bare UID_BASED_ID of \
                                        the addressed collection (RM \
                                        `item_tag.adoc`: \"may be a \
                                        `VERSIONED_OBJECT<T>` or a \
                                        `VERSION<T>`\") — `HIER_OBJECT_ID` for \
                                        the container form, `OBJECT_VERSION_ID` \
                                        for the version form — and `owner_id` \
                                        the OBJECT_REF of the owning EHR. \
                                        (The released OAS `ItemTag` schema \
                                        types `target` as an OBJECT_REF; the \
                                        RM is the RELEASED component and \
                                        wins.) `target_path` is present only \
                                        on tags that carry one: it is 0..1 in \
                                        the RM, and an empty string is stored \
                                        as absent — so the released \
                                        `ItemTagOfEhrStatus` example's \
                                        `target_path: \"\"` is served back as \
                                        no `target_path` at all.",
            content((serde_json::Value = "application/json", example = json!([ {
                "_type": "ITEM_TAG",
                "key": "category",
                "value": "final",
                "target": { "_type": "HIER_OBJECT_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            } ])))
        ),
        (status = 400, description = "`ehr_id` is not a UUID, or `uid_based_id` \
                                      is neither a UUID nor a well-formed \
                                      three-part OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \
                                      \"malformed request syntax, \
                                      syntactically invalid content\"). A \
                                      well-formed identifier that names \
                                      nothing is `404`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id.yaml`). \
                                      Existence is judged as the operation \
                                      scopes it — \"owned by EHR identified by \
                                      `ehr_id`\" — so all of these are `404`: \
                                      an unknown versioned-object uid; one \
                                      that belongs to ANOTHER EHR; a version \
                                      form whose version does not exist; and a \
                                      uid whose stored kind is not an \
                                      EHR_STATUS (a COMPOSITION or FOLDER uid \
                                      on this route). The kind-mismatch \
                                      reading is OURS — the released sentence \
                                      does not spell it out — and follows from \
                                      the route family naming the target's \
                                      class; it is adjudicated. An EXISTING \
                                      EHR_STATUS target with no tags is \
                                      `200 []`, not `404`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      an ITEM_TAG list is served as canonical \
                                      `application/json` only. The canonical \
                                      XML ITS defines no ITEM_TAG type, so an \
                                      `application/xml` Accept — a member of \
                                      the released `Accept_canonical` enum, \
                                      stalled shape on this operation — is \
                                      refused (`Resources.md` §\"XML Format\": \
                                      \"If the service cannot fulfill this \
                                      aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_tags_get",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Replace an `EHR_STATUS`'s item tags
/// (`PUT /ehr/{ehr_id}/ehr_status/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `EHR_STATUS` version or `VERSIONED_EHR_STATUS` identified by `uid_based_id` and
/// owned by EHR identified by `ehr_id`"
/// (`specifications/operations/ehr_status_tags_update.yaml`). It is a FULL
/// COLLECTION REPLACE of the ADDRESSED collection — the container's or one
/// version's, never both: tags omitted from the body are removed, and
/// "providing an empty list will effectively remove all `ITEM_TAG` associated
/// with the given target".
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION,
/// mints no `EHR_STATUS` version, takes no `If-Match` and no committal headers,
/// and serves neither `ETag` nor `Last-Modified` (see the section note above).
#[utoipa::path(
    put, path = "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: \"an \
                        OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`), or a form of \
                        a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular EHR_STATUS version …, whereas \
                        the latter … is be used to update the tags of the \
                        target VERSIONED_EHR_STATUS container\" \
                        (`ehr_status_tags_update.yaml`). The two collections \
                        are DISJOINT — an ITEM_TAG has exactly one `target` \
                        (RM `item_tag.adoc`) — so replacing the container's \
                        list never touches any version's list, and replacing \
                        one version's list never touches the container's or a \
                        sibling version's.",
         example = "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` (the default when the header is \
                        absent — `Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured — its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid — so the server applies, and \
                        declares, the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied`.",
         example = "return=representation")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                addressed target (required; there is no \
                                envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED, `value` and `target_path` are \
                                optional, and no other member is defined. \
                                `target` and `owner_id` are NOT client input — \
                                the server assigns them from the route \
                                (`target` = the addressed `uid_based_id`, \
                                `owner_id` = the addressed EHR), which is why \
                                the write schema omits them; a body that \
                                nonetheless carries them — or any other \
                                undeclared member — is REFUSED `400` naming \
                                the member. The schema declares \
                                `additionalProperties: false`, and the ITS-REST \
                                docs text says nothing about the write body's \
                                member set, so the released OAS grounds the \
                                expectation under the documented oracle order; \
                                the refusal is the released constraint, not our \
                                own strictness. A member of the wrong JSON type \
                                (a numeric `value`, say) is the same `400` — \
                                never a silently-absent attribute. `[]` is \
                                the clear-all form, never an error: \"Providing \
                                an empty list will effectively remove all \
                                ITEM_TAG associated with the given target\" \
                                (`ehr_status_tags_update.yaml`). Identity \
                                inside the list is the (`key`, `target_path`) \
                                PAIR (`Requests_and_responses.md` §item-tag \
                                headers: \"More than one ITEM_TAG may be \
                                associated with a single target, in which case \
                                they are uniquely identified by their `key` \
                                and `target_path` pair attributes\"), so two \
                                entries may share a `key` when their \
                                `target_path` differs; a DUPLICATE pair inside \
                                one body is resolved last-wins (no released \
                                rule and no `uniqueItems` — ours, \
                                adjudicated). A `target_path` of `\"\"` \
                                normalizes to ABSENT, so it is the same \
                                identity as an entry with no `target_path` at \
                                all: the RM models `target_path` 0..1 with no \
                                non-empty invariant while the released \
                                EHR_STATUS example uses `\"\"` — reconciling \
                                the two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([ {
                     "key": "category",
                     "value": "final"
                 }, {
                     "key": "flag",
                     "value": "follow-up",
                     "target_path": "/subject/external_ref/id/value"
                 } ])),
    responses(
        (
            status = 200, description = "Applied, with `Prefer: \
                                        return=representation`. The body is \
                                        the full RESULTING ITEM_TAG list of \
                                        the addressed collection — every tag \
                                        now stored on it, server-assigned \
                                        `target`/`owner_id` included — not \
                                        merely the entries just sent. (The \
                                        released \
                                        `200_EHR_STATUS_ItemTagList_updated` \
                                        describes itself as \"returned when \
                                        the requested ITEM_TAG list is \
                                        successfully retrieved\", a \
                                        copy-and-paste of the `_retrieved` \
                                        response text; the trigger is the \
                                        update, as stated here.) The only \
                                        response header is \
                                        `Preference-Applied` — a tag \
                                        collection has no version and no uid, \
                                        so there is no `ETag`, no \
                                        `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\")."),
            ),
            content((serde_json::Value = "application/json", example = json!([ {
                "_type": "ITEM_TAG",
                "key": "category",
                "value": "final",
                "target": { "_type": "HIER_OBJECT_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            }, {
                "_type": "ITEM_TAG",
                "key": "flag",
                "value": "follow-up",
                "target_path": "/subject/external_ref/id/value",
                "target": { "_type": "HIER_OBJECT_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31" },
                "owner_id": {
                    "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" }
                }
            } ])))
        ),
        (status = 204, description = "Applied, with no body — \"`204 No \
                                      Content` is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`\" \
                                      (`responses/204_updated.yaml`); a \
                                      `return=identifier` request resolves \
                                      here too. This is the DEFAULT branch. It \
                                      carries no resource header of any kind — \
                                      no `ETag`, no `Last-Modified`, no \
                                      `Location` — only the \
                                      `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return)."),
         )),
        (status = 400, description = "`ehr_id` or `uid_based_id` is malformed, \
                                      or the body is not a JSON ARRAY / not \
                                      parseable as JSON \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row: \
                                      \"malformed request syntax, \
                                      syntactically invalid content\"). A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id.yaml`). \
                                      Existence is judged as the operation \
                                      scopes it — \"owned by EHR identified by \
                                      `ehr_id`\" — so an unknown uid, a uid \
                                      owned by another EHR, a non-existent \
                                      version of an existing container, and a \
                                      uid whose stored kind is not an \
                                      EHR_STATUS are all `404`. The \
                                      kind-mismatch reading is OURS \
                                      (adjudicated): the released sentence does \
                                      not spell it out, and it follows from \
                                      the route family naming the target's \
                                      class.",
         body = serde_json::Value),
        (status = 406, description = "A `return=representation` request whose \
                                      `Accept` cannot be satisfied: the \
                                      ITEM_TAG list is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no ITEM_TAG type, so the \
                                      released `Accept_canonical` enum's \
                                      `application/xml` member is stalled \
                                      shape here) — `Resources.md` §\"XML \
                                      Format\": \"If the service cannot \
                                      fulfill this aspect of the request, it \
                                      MUST respond with HTTP status code `406 \
                                      Not Acceptable`\". The default `204` \
                                      branch returns no body and negotiates \
                                      nothing.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot process the request \
                                      payload as JSON format, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\"). An ABSENT `Content-Type` \
                                      declares nothing and is accepted as \
                                      JSON.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The released \
                                      operation declares only `400` for a bad \
                                      request; answering `422` for these \
                                      SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_tags_update",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}

/// Delete an `EHR_STATUS`'s item tags under one key
/// (`DELETE /ehr/{ehr_id}/ehr_status/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `EHR_STATUS` version or `VERSIONED_EHR_STATUS` identified by
/// `uid_based_id` and owned by EHR identified by `ehr_id`"
/// (`specifications/operations/ehr_status_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the
/// (`key`, `target_path`) pair, the route carries no `target_path` selector,
/// and the released text says "resource(s)" — so EVERY tag under `key` on the
/// addressed collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags/{key}", tag = "ITEM_TAG",
    params(
        ("ehr_id" = String, Path,
         description = "EHR identifier, taken from EHR.ehr_id.value — a UUID \
                        (`Resources.md` §\"Identifier types\").",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("uid_based_id" = String, Path,
         description = "The tagged target, in either released form: an \
                        OBJECT_VERSION_ID (\"used to delete the tags a \
                        particular (target) version of the EHR_STATUS \
                        version\") or a HIER_OBJECT_ID (\"used to delete the \
                        tags of the target VERSIONED_EHR_STATUS container\" — \
                        `ehr_status_tags_delete.yaml`). The two collections \
                        are DISJOINT (an ITEM_TAG has exactly one `target`, RM \
                        `item_tag.adoc`), so deleting a key from the container \
                        leaves the same key on every version untouched, and \
                        vice versa.",
         example = "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted on the \
                        addressed collection — \"The ITEM_TAG key\" \
                        (`parameters/path/key.yaml`, `type: string`), an \
                        UNCONSTRAINED string with no format, pattern or length \
                        bound, taken percent-decoded from the path segment (a \
                        key containing `/`, `?` or `#` must be percent-encoded \
                        by the client). It selects a SET: identity is the \
                        (`key`, `target_path`) pair and this route has no \
                        `target_path` selector, so all tags under the key go. \
                        (The released operation descriptions call this \
                        parameter `tag_key` in prose while the path parameter \
                        is `key` — a released-text inconsistency reported \
                        upstream; the wire name is `key`.)",
         example = "category")
    ),
    responses(
        (status = 204, description = "The tags under `key` were deleted from \
                                      the addressed collection \
                                      (`responses/204_deleted.yaml`). No body \
                                      and no headers: an ITEM_TAG has no \
                                      version and no uid, so there is nothing \
                                      for an `ETag`/`Last-Modified` to carry, \
                                      and §\"HTTP headers\" records that \"the \
                                      `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\". The released response text \
                                      says \"(logically) deleted\", which is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tag simply ceases to exist."),
        (status = 400, description = "`ehr_id` is not a UUID, or \
                                      `uid_based_id` is neither a UUID nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `400` row).",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when an EHR with \
                                      `ehr_id` does not exist, or when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_ehr_id_or_uid_based_id_or_key.yaml`). \
                                      The THIRD trigger makes this operation \
                                      deliberately NON-IDEMPOTENT at the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      existence is judged as the operation \
                                      scopes it (\"owned by EHR identified by \
                                      `ehr_id`\"): an unknown uid, a uid owned \
                                      by another EHR, a missing version, and a \
                                      uid whose stored kind is not an \
                                      EHR_STATUS are all `404` — the \
                                      kind-mismatch reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn ehr_status_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "ehr_status_tags_delete",
        parts,
        crate::api::ehr::dispatch::dispatch,
    )
    .await
}
