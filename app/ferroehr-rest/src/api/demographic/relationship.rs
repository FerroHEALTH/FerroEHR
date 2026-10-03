// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `PARTY_RELATIONSHIP` wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs this: the vendored Demographic API
//! (`demographic.openapi.yaml`) defines **no** `party_relationship` /
//! `versioned_party_relationship` paths anywhere. These routes are our own
//! extension realizing SM `I_PARTY_RELATIONSHIP`
//! (`docs/specs/openehr/SM/.../i_party_relationship.adoc` — the *service*
//! basis, not a *wire* basis) and are **excluded from any
//! conformance-profile claim**. The envelope mirrors the party CRUD + versioned
//! reads with one fixed `party_relationship` / `versioned_party_relationship`
//! segment; because there is no wire contract, the generated party `*Params`
//! structs are reused by analogy.
//!
//! NOTE (no openEHR spec governs role semantics on an unspecified route — our
//! own design/extension): the shared authentication + RBAC layer answers before
//! any handler runs — no valid principal is `401`, the configured read-only role
//! is `403` on the writes (create, update, delete) and unaffected on the reads,
//! and the coarse operation class is `Clinical` (not under `/admin/`, so no
//! ADMIN role). Both branches are declared per operation below.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::extract::State;
use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::generated::demographic::{
    AgentCreateParams, AgentGetParams, AgentUpdateParams, VersionedPartyGetParams,
    VersionedPartyVersionGetAtTimeParams, VersionedPartyVersionGetByIdParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::PartyRelationship;

use crate::api::{RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::{RestError, sm_api_error};
use crate::state::AppState;
use ferroehr::service::response::ServiceResponse;

/// The `PARTY_RELATIONSHIP` extension routes as a native `utoipa-axum` router,
/// realizing SM `I_PARTY_RELATIONSHIP` on our own wire with no ITS-REST contract
/// (see the module docs).
///
/// Paths are group-relative, and every operation runs through
/// [`guarded_dispatch`] with the demographic group
/// [`dispatch`](super::dispatch::dispatch), which routes relationship ops back
/// into [`run`].
pub(crate) fn relationship_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (the macro composes one method-router — handlers
    // in a single call must share the path; mixing paths panics at build with
    // "Overlapping method route").
    OpenApiRouter::new()
        .routes(routes!(party_relationship_create))
        .routes(routes!(
            party_relationship_get,
            party_relationship_update,
            party_relationship_delete
        ))
        .routes(routes!(versioned_party_relationship_get))
        .routes(routes!(party_relationship_revision_history))
        .routes(routes!(party_relationship_version_get_at_time))
        .routes(routes!(party_relationship_version_get_by_id))
}

// ── Handlers (our own wire; no ITS-REST operation governs these) ──────────────
// Each snapshots the request and runs it through the demographic group
// dispatcher (`super::dispatch::dispatch`), which routes relationship ops into [`run`].

/// Create a `PARTY_RELATIONSHIP` (`POST /demographic/party_relationship`).
///
/// **Our own extension — no ITS-REST operation governs this.** The released
/// Demographic API defines no `party_relationship` path anywhere; this route
/// realizes SM `I_PARTY_RELATIONSHIP`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_party_relationship.adoc` — a
/// *service* basis, not a *wire* basis) over an envelope that deliberately
/// mirrors the released party CRUD so clients see one consistent surface. It is
/// **excluded from any conformance-profile claim**: none of the branches below
/// is a released ITS-REST requirement, and the overview citations describe the
/// convention we chose to follow, not an obligation this route inherits. The RM
/// itself carries relationships inline on the source PARTY
/// (`PARTY.relationships`, RM `demographic/master02` §Party Relationships), so a
/// standalone relationship resource is ours by construction.
#[utoipa::path(
    post, path = "/demographic/party_relationship", tag = "demographic-relationship",
    params(
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference, following the released \
                        convention: `return=representation` — the created \
                        PARTY_RELATIONSHIP; `return=identifier` — only \
                        `{ \"uid\": \"…\" }`; absent or `return=minimal` — an \
                        empty body (`Requests_and_responses.md` §\"Representation \
                        details negotiation\"; §\"Prefer only identifier\" keeps \
                        the identifier variant off `204`). The token honoured is \
                        echoed in `Preference-Applied`. Extension route — the \
                        convention is borrowed, not mandated.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "`application/json` (the default when absent) or \
                        `application/xml` — the canonical formats. A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "`application/json` (default) or `application/xml`. A \
                        Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. Accepted in the \
                        shape the released committal headers use \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"), applied here to an extension \
                        resource.",
         example = "lifecycle_state.code_string=\"532\""),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `time_committed` is always server-set and an \
                        omitted `system_id` falls back to the server's configured \
                        identifier, as in the released committal-header rule \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "committer.name=\"John Doe\"")
    ),
    request_body(content = serde_json::Value,
                 description = "An RM PARTY_RELATIONSHIP as canonical JSON or \
                                XML. `source` and `target` are `PARTY_REF`s and \
                                the relationship's `name` is its type \
                                (`Type_validity: type = name`, RM UML \
                                `org.openehr.rm.demographic.party_relationship`). \
                                No released schema governs this body — the shape \
                                is the RM class itself.",
                 example = json!({
                     "_type": "PARTY_RELATIONSHIP",
                     "name": { "_type": "DV_TEXT", "value": "carer" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.carer.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.carer.v1" },
                         "rm_version": "1.2.0"
                     },
                     "source": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "PERSON",
                         "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
                     },
                     "target": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "PERSON",
                         "id": { "_type": "HIER_OBJECT_ID", "value": "6cb19121-4307-4648-9da0-d62e4d51f19b" }
                     }
                 })),
    responses(
        (status = 201, description = "Created. The body follows `Prefer` (the \
                                      full relationship, the `{uid}` object, or \
                                      empty). Extension route — the status and \
                                      body rules mirror the released party \
                                      create; no released response file governs \
                                      them.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version, in the weak form the release requires of \
                             identifier `ETag`s (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "`<base_path>/demographic/party_relationship/<version_uid>` \
                             — the URL of the newly created resource (§Location: \
                             used \"in `201 Created` responses when a new resource \
                             is successfully created\")."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured (§\"Representation details negotiation\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created relationship",
              value = json!({
                  "_type": "PARTY_RELATIONSHIP",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "carer" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.carer.v1",
                  "source": {
                      "_type": "PARTY_REF",
                      "namespace": "demographic",
                      "type": "PERSON",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
                  },
                  "target": {
                      "_type": "PARTY_REF",
                      "namespace": "demographic",
                      "type": "PERSON",
                      "id": { "_type": "HIER_OBJECT_ID", "value": "6cb19121-4307-4648-9da0-d62e4d51f19b" }
                  }
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The request could not be parsed: a body that \
                                      is not well-formed canonical JSON/XML. \
                                      Status assignment follows the overview \
                                      table's `400` row — \"malformed request \
                                      syntax, syntactically invalid content\" \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 403, description = "The authenticated principal holds the \
                                      configured read-only role: this route \
                                      creates a version, so it is refused before \
                                      the resource is touched.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY_RELATIONSHIP is untemplated, so it is \
                                      served in the canonical formats only and a \
                                      Simplified-only `Accept` is refused \
                                      (`Resources.md` §\"Simplified Formats\": an \
                                      unfulfillable `Accept` is `406`).",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which an untemplated \
                                      resource cannot use — no header can name a \
                                      template for it (`Requests_and_responses.md` \
                                      §openehr-template-id scopes \
                                      `openehr-template-id` to \"committing \
                                      COMPOSITION\"); `Resources.md` §\"Simplified \
                                      Formats\" makes an unprocessable payload \
                                      format a `415`. An absent `Content-Type` \
                                      declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The body parses but is not a usable \
                                      PARTY_RELATIONSHIP (missing or invalid \
                                      `source`/`target` `PARTY_REF`s, or another \
                                      RM invariant violation) — the overview \
                                      table's `422` row, \"the request was \
                                      well-formed but was unable to be followed \
                                      due to semantic errors\" \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_create",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a `PARTY_RELATIONSHIP` by uid-based id
/// (`GET /demographic/party_relationship/{uid_based_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    get, path = "/demographic/party_relationship/{uid_based_id}", tag = "demographic-relationship",
    params(
        ("uid_based_id" = String, Path,
         description = "Either an OBJECT_VERSION_ID (a specific `version_uid`) or \
                        a HIER_OBJECT_ID (`versioned_object_uid`) for the latest \
                        / at-time version — the dual form the released party \
                        reads use (`Resources.md` §\"Identifier types\" and \
                        §\"Multiple identifiers for the same resource\"), applied \
                        here to an extension resource.",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "A given time in the extended ISO 8601 format; when the \
                        path id is a `versioned_object_uid`, selects the version \
                        extant at that instant (latest when omitted). The \
                        timezone is optional — server-local when absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "`application/json` (default) or `application/xml`. A \
                        Simplified-only `Accept` is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The relationship as canonical JSON/XML. No \
                                      `Location`: §Location says the header \"MUST \
                                      NOT be used to indicate an alternate \
                                      representation of an existing resource (e.g. \
                                      via `GET` method)\" \
                                      (`Requests_and_responses.md`), a rule this \
                                      extension follows.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (§\"ETag and Last-Modified\": \
                             identifier `ETag`s are weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\").")
         ),
         example = json!({
             "_type": "PARTY_RELATIONSHIP",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "carer" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.carer.v1",
             "source": {
                 "_type": "PARTY_REF",
                 "namespace": "demographic",
                 "type": "PERSON",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
             },
             "target": {
                 "_type": "PARTY_REF",
                 "namespace": "demographic",
                 "type": "PERSON",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "6cb19121-4307-4648-9da0-d62e4d51f19b" }
             }
         })),
        (status = 204, description = "The version the request selects is a \
                                      deletion marker — a successful read of a \
                                      logically deleted resource, mirroring the \
                                      released party read's own `204` branch \
                                      (`specifications/responses/204_deleted_at_time.yaml`)."),
        (status = 400, description = "The `uid_based_id` is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 instant — the overview table's `400` \
                                      row, \"syntactically invalid content\" \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship, or no version at the \
                                      requested `version_at_time` — the overview \
                                      table's `404` row, \"The origin service did \
                                      not find the target resource or is not \
                                      willing to disclose that one exists\".",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: an \
                                      untemplated resource is served in the \
                                      canonical formats only, so a Simplified-only \
                                      `Accept` is refused (`Resources.md` \
                                      §\"Simplified Formats\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` sends no payload, \
                                      but the declaration is refused before the \
                                      read because an untemplated resource has no \
                                      template to expand one against \
                                      (`Resources.md` §\"Simplified Formats\"). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Update a `PARTY_RELATIONSHIP`
/// (`PUT /demographic/party_relationship/{uid_based_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    put, path = "/demographic/party_relationship/{uid_based_id}", tag = "demographic-relationship",
    params(
        ("uid_based_id" = String, Path,
         description = "The HIER_OBJECT_ID `versioned_object_uid` of the \
                        relationship container to update — the same container \
                        form the released party update takes \
                        (`Resources.md` §\"Identifier types\").",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
        ("If-Match" = String, Header,
         description = "REQUIRED: the existing latest `version_uid` (the \
                        `preceding_version_uid`), double-quoted; the weak \
                        `W/\"…\"` form this server emits in `ETag` is accepted \
                        too. The precondition is required because the preceding \
                        version is NOT in the path — \"This is only required by a \
                        small set of versioned resources in this specification, \
                        when the `preceding_version_uid` is not part of the \
                        endpoint path segment\" (`Requests_and_responses.md` \
                        §\"If-Match and accidental overwrites\"), the convention \
                        this extension follows.",
         example = "\"1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "`return=representation` — the updated relationship at \
                        `200`; `return=identifier` — `{ \"uid\": \"…\" }` at \
                        `200` (never `204`, §\"Prefer only identifier\"); absent \
                        or `return=minimal` — an empty `204` \
                        (`Requests_and_responses.md` §\"Representation details \
                        negotiation\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "`application/json` (the default when absent) or \
                        `application/xml`. A Simplified `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "`application/json` (default) or `application/xml`. A \
                        Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list, in the shape of the released \
                        committal headers (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\""),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `time_committed` is always server-set \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "change_type.code_string=\"251\"")
    ),
    request_body(content = serde_json::Value,
                 description = "The updated RM PARTY_RELATIONSHIP as canonical \
                                JSON or XML; no released schema governs this \
                                body.",
                 example = json!({
                     "_type": "PARTY_RELATIONSHIP",
                     "name": { "_type": "DV_TEXT", "value": "carer" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.carer.v1",
                     "source": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "PERSON",
                         "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" }
                     },
                     "target": {
                         "_type": "PARTY_REF",
                         "namespace": "demographic",
                         "type": "ORGANISATION",
                         "id": { "_type": "HIER_OBJECT_ID", "value": "6cb19121-4307-4648-9da0-d62e4d51f19b" }
                     }
                 })),
    responses(
        (status = 200, description = "Updated, with the body `Prefer` asked for \
                                      (`return=representation` — the full \
                                      relationship; `return=identifier` — the \
                                      `{uid}` object).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the NEW \
                             version (§\"ETag and Last-Modified\")."),
             ("Location" = String,
              description = "`<base_path>/demographic/party_relationship/<new version_uid>` \
                             — the version this update created (§Location + \
                             §\"Prefer minimal, identifier or full representation \
                             response\": \"the newly created or updated \
                             resource\")."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (§\"ETag and Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured (§\"Representation \
                             details negotiation\").")
         )),
        (status = 204, description = "Updated with no body — the default \
                                      `return=minimal` (§\"Prefer minimal, \
                                      identifier or full representation \
                                      response\": \"If no response body is \
                                      returned, the service SHOULD use `204 No \
                                      Content`\").",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version."),
             ("Location" = String,
              description = "`<base_path>/demographic/party_relationship/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured.")
         )),
        (status = 400, description = "The request could not be parsed, or the \
                                      required `If-Match` is absent — \"When the \
                                      service expects `If-Match` for an operation, \
                                      but the client does not provide it, the \
                                      service SHOULD respond with `400 Bad \
                                      Request`\" (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 403, description = "The authenticated principal holds the \
                                      configured read-only role: this route \
                                      creates a version, so it is refused before \
                                      the resource is touched.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship container — the \
                                      overview table's `404` row \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: an \
                                      untemplated resource is served in the \
                                      canonical formats only, so a Simplified-only \
                                      `Accept` is refused (`Resources.md` \
                                      §\"Simplified Formats\").",
         body = serde_json::Value),
        (status = 412, description = "`If-Match` does not match the latest version \
                                      on the service side: \"it MUST NOT perform \
                                      the requested method. Instead, it MUST \
                                      respond with HTTP status code `412 \
                                      Precondition Failed`, and SHOULD return also \
                                      latest `version_uid` in the `ETag` response \
                                      headers\" (`Requests_and_responses.md` \
                                      §\"If-Match and accidental overwrites\") — \
                                      the convention this extension follows.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"`, so the client can retry against it. No \
                             `Location`: §Location scopes the header to \
                             creation/redirect responses."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off.")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which an untemplated \
                                      resource cannot use (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The body parses but is not a usable \
                                      PARTY_RELATIONSHIP (missing or invalid \
                                      `source`/`target` `PARTY_REF`s) — the \
                                      overview table's `422` row, \"the request \
                                      was well-formed but was unable to be \
                                      followed due to semantic errors\".",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_update",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Delete a `PARTY_RELATIONSHIP`
/// (`DELETE /demographic/party_relationship/{uid_based_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
/// The delete is LOGICAL — it commits a deletion VERSION rather than removing
/// history (RM `common/master06` §Change Control), matching the released party
/// deletes.
#[utoipa::path(
    delete, path = "/demographic/party_relationship/{uid_based_id}", tag = "demographic-relationship",
    params(
        ("uid_based_id" = String, Path,
         description = "The OBJECT_VERSION_ID `version_uid` of the latest version \
                        — the `preceding_version_uid` to delete — as in the \
                        released party delete \
                        (`specifications/parameters/path/uid_based_id_as_version_uid.yaml`).",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL: the preceding version is already the path \
                        segment, and the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"). A header that IS sent is \
                        honoured as an alternative source of the preceding \
                        version; the weak `W/\"…\"` and bare quoted forms are \
                        both accepted.",
         example = "\"1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list, in the shape of the released \
                        committal headers (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\""),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `time_committed` is always server-set \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "description.value=\"relationship ended\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format. A Simplified-only `Accept` is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "Logically deleted — a deletion VERSION was \
                                      committed and there is no body to return \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `204` row).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version just committed — the `ETag` \
                             \"changes as soon as the resource changes (i.e. when \
                             a new version is created)\" (§\"ETag and \
                             Last-Modified\"), and a logical delete creates one. \
                             No `Location`: §\"Deprecated headers\" deprecates it \
                             on `DELETE` responses.")
         )),
        (status = 400, description = "The request could not be parsed, or the \
                                      relationship is already deleted — the \
                                      branch the released party delete assigns to \
                                      `400` \
                                      (`specifications/responses/400_already_deleted.yaml`), \
                                      followed here by convention.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 403, description = "The authenticated principal holds the \
                                      configured read-only role: this route \
                                      commits a deletion version, so it is refused before \
                                      the resource is touched.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship — the overview table's \
                                      `404` row (`Requests_and_responses.md` \
                                      §\"HTTP status codes\").",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: an \
                                      untemplated resource is served in the \
                                      canonical formats only, so a Simplified-only \
                                      `Accept` is refused (`Resources.md` \
                                      §\"Simplified Formats\").",
         body = serde_json::Value),
        (status = 409, description = "The supplied `uid_based_id` is not the \
                                      latest version — the branch the released \
                                      party delete assigns to `409` \
                                      (`specifications/responses/409_PERSON_with_uid_based_id.yaml`: \
                                      \"returned when supplied `uid_based_id` \
                                      doesn't match the latest version. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\"), followed here by convention — \
                                      except that this extension route does NOT \
                                      echo the latest `version_uid` in an `ETag` \
                                      on the conflict, so no such header is \
                                      declared; the message body names the latest \
                                      version instead.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is refused before the \
                                      write because an untemplated resource has no \
                                      template to expand one against \
                                      (`Resources.md` §\"Simplified Formats\"). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve the `VERSIONED_PARTY_RELATIONSHIP` container
/// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}", tag = "demographic-relationship",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The version-container id of the relationship (a \
                        HIER_OBJECT_ID / `versioned_object_uid`, `Resources.md` \
                        §\"Identifier types\").",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
        ("Accept" = Option<String>, Header,
         description = "This container is served as canonical `application/json`; \
                        an `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The VERSIONED_OBJECT container of the \
                                      relationship, as canonical JSON. No \
                                      `Location`: §Location restricts the header \
                                      to creation/redirect responses \
                                      (`Requests_and_responses.md`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<versioned_object_uid>\"` — \
                             the `ETag` value \"is usually taken from e.g. \
                             VERSIONED_OBJECT.uid.value, VERSION.uid.value\" \
                             (§\"ETag and Last-Modified\"). A container body \
                             exposes no commit audit, so no `Last-Modified` \
                             accompanies it.")
         )),
        (status = 400, description = "The `versioned_object_uid` is not a \
                                      well-formed id — the overview table's `400` \
                                      row, \"syntactically invalid content\" \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship container — the \
                                      overview table's `404` row.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      container is served as canonical JSON only, \
                                      so an `Accept` excluding `application/json` \
                                      is refused (`Resources.md` §\"JSON \
                                      Format\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, refused before the read \
                                      because an untemplated resource has no \
                                      template to expand one against \
                                      (`Resources.md` §\"Simplified Formats\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_party_relationship_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_party_relationship_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve the relationship's `REVISION_HISTORY`
/// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/revision_history`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/revision_history", tag = "demographic-relationship",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The version-container id of the relationship (a \
                        HIER_OBJECT_ID / `versioned_object_uid`).",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
        ("Accept" = Option<String>, Header,
         description = "The history is served as canonical `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The relationship's REVISION_HISTORY as \
                                      canonical JSON; `items` runs oldest-first \
                                      (`REVISION_HISTORY.most_recent_version` is \
                                      the last item, RM `common/master04` \
                                      §REVISION_HISTORY).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<versioned_object_uid>\"` — \
                             a REVISION_HISTORY carries no `uid` of its own, so \
                             the addressed container's id is the `ETag` source \
                             (§\"ETag and Last-Modified\" names \
                             VERSIONED_OBJECT.uid.value as one)."),
             ("Last-Modified" = String,
              description = "The most recent revision's commit instant as an \
                             HTTP-date, \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\").")
         )),
        (status = 400, description = "The `versioned_object_uid` is not a \
                                      well-formed id — the overview table's `400` \
                                      row (`Requests_and_responses.md` §\"HTTP \
                                      status codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship container — the \
                                      overview table's `404` row.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      history is served as canonical JSON only \
                                      (`Resources.md` §\"JSON Format\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, refused before the read \
                                      (`Resources.md` §\"Simplified Formats\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_revision_history(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_revision_history",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve the relationship VERSION at a point in time
/// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/version`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/version", tag = "demographic-relationship",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The version-container id of the relationship (a \
                        HIER_OBJECT_ID / `versioned_object_uid`).",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
        ("version_at_time" = Option<String>, Query,
         description = "A given time in the extended ISO 8601 format; selects the \
                        VERSION extant at that instant (the latest when omitted). \
                        The timezone is optional — server-local when absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The VERSION is served as canonical `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION wrapper as canonical \
                                      JSON, `data` carrying the relationship. No \
                                      `Location` — §Location restricts the header \
                                      to creation/redirect responses \
                                      (`Requests_and_responses.md`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served VERSION (§\"ETag and Last-Modified\")."),
             ("Last-Modified" = String,
              description = "The served VERSION's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\").")
         )),
        (status = 400, description = "The `versioned_object_uid` is malformed, or \
                                      `version_at_time` is not an extended ISO \
                                      8601 instant — the overview table's `400` \
                                      row (`Requests_and_responses.md` §\"HTTP \
                                      status codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship container, or no \
                                      version at the requested `version_at_time` \
                                      — the overview table's `404` row.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      VERSION is served as canonical JSON only \
                                      (`Resources.md` §\"JSON Format\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, refused before the read \
                                      (`Resources.md` §\"Simplified Formats\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_version_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_version_get_at_time",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a specific relationship VERSION by version uid
/// (`GET /demographic/versioned_party_relationship/{versioned_object_uid}/version/{version_uid}`).
///
/// **Our own extension — no ITS-REST operation governs this** (see
/// [`party_relationship_create`] and the module docs); realizes SM
/// `I_PARTY_RELATIONSHIP` and is excluded from any conformance-profile claim.
#[utoipa::path(
    get, path = "/demographic/versioned_party_relationship/{versioned_object_uid}/version/{version_uid}", tag = "demographic-relationship",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The version-container id of the relationship (a \
                        HIER_OBJECT_ID / `versioned_object_uid`).",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8"),
        ("version_uid" = String, Path,
         description = "The VERSION identifier (an OBJECT_VERSION_ID) whose \
                        `object_id` segment is the `versioned_object_uid` above \
                        (`Resources.md` §\"Identifier types\").",
         example = "1f2a3b4c-5d6e-4f70-8192-a3b4c5d6e7f8::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The VERSION is served as canonical `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The ORIGINAL_VERSION wrapper as canonical \
                                      JSON, `data` carrying the relationship. No \
                                      `Location` — §Location restricts the header \
                                      to creation/redirect responses \
                                      (`Requests_and_responses.md`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served VERSION (§\"ETag and Last-Modified\")."),
             ("Last-Modified" = String,
              description = "The served VERSION's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (§\"ETag and Last-Modified\").")
         )),
        (status = 400, description = "The `versioned_object_uid` or `version_uid` \
                                      is not well-formed — the overview table's \
                                      `400` row (`Requests_and_responses.md` \
                                      §\"HTTP status codes\").",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal) — refused before the request \
                                      reaches the resource.",
         body = serde_json::Value),
        (status = 404, description = "No such relationship container, or a \
                                      `version_uid` that names no version of it — \
                                      the overview table's `404` row.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      VERSION is served as canonical JSON only \
                                      (`Resources.md` §\"JSON Format\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, refused before the read \
                                      (`Resources.md` §\"Simplified Formats\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn party_relationship_version_get_by_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "party_relationship_version_get_by_id",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// `PARTY_RELATIONSHIP` operations — same envelope/header rules as the party
/// routes, one fixed `party_relationship` segment (our own wire; no ITS-REST
/// operation governs it — see the module docs).
#[expect(
    clippy::too_many_lines,
    reason = "one arm per PARTY_RELATIONSHIP operation, like `party::run`: a flat \
              match keeps every operation's wire behaviour readable in one place"
)]
pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    const SEG: &str = "party_relationship";
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let base = state.config().server.base_path.clone();

    // PARTY_RELATIONSHIP is not templated → no Simplified-Formats mapping;
    // reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;

    match op {
        "party_relationship_create" => {
            let _p = AgentCreateParams::from_request(parts.route()?, q, h)?;
            let body = negotiate::rm_value::<PartyRelationship>(h, &parts.body)?;
            let resp = state
                .backend()
                .party_relationship_create(
                    body,
                    crate::overview::committal::committal_commit(
                        h,
                        crate::api::ehr::committer_proxy(),
                    )?,
                )
                .await?;
            Ok(write_relationship(
                h,
                &base,
                SEG,
                StatusCode::CREATED,
                StatusCode::CREATED,
                &resp,
            ))
        }
        "party_relationship_get" => {
            let p = AgentGetParams::from_request(parts.route()?, q, h)?;
            let resp = state
                .backend()
                .party_relationship_get(p.uid_based_id, p.version_at_time)
                .await?;
            if resp.is_empty() {
                return Ok(negotiate::empty(StatusCode::NO_CONTENT));
            }
            Ok(read_relationship(h, &resp))
        }
        "party_relationship_update" => {
            let p = AgentUpdateParams::from_request(parts.route()?, q, h)?;
            let uid = p.uid_based_id.clone();
            let body = negotiate::rm_value::<PartyRelationship>(h, &parts.body)?;
            match state
                .backend()
                .party_relationship_update(
                    p.uid_based_id,
                    // Decode the `W/"…"`/quoted ETag syntax at the adapter seam
                    // so the service compares a bare OBJECT_VERSION_ID.
                    super::if_match_token(&p.if_match),
                    body,
                    crate::overview::committal::committal_commit(
                        h,
                        crate::api::ehr::committer_proxy(),
                    )?,
                )
                .await
            {
                Ok(resp) => Ok(write_relationship(
                    h,
                    &base,
                    SEG,
                    StatusCode::NO_CONTENT,
                    StatusCode::OK,
                    &resp,
                )),
                Err(e) if super::is_precondition(&e) => {
                    let meta = state
                        .backend()
                        .party_relationship_latest_meta(uid)
                        .await
                        .ok()
                        .flatten();
                    Ok(super::error_with_meta(sm_api_error(e), meta.as_ref()))
                }
                Err(e) => Err(RestError::from(e)),
            }
        }
        // A stale `uid_based_id` answers `409` AND echoes the latest
        // `version_uid` in `ETag`, matching the party delete this extension
        // mirrors (`409_PERSON_with_uid_based_id.yaml`'s convention).
        "party_relationship_delete" => {
            let p = AgentGetParams::from_request(parts.route()?, q, h)?;
            let preceding = p.uid_based_id.clone();
            match state
                .backend()
                .party_relationship_delete(
                    p.uid_based_id,
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
                Err(e) if super::is_precondition(&e) => {
                    let meta = state
                        .backend()
                        .party_relationship_latest_meta(preceding)
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
        // The versioned reads carry the weak `ETag` (+ `Last-Modified` where the
        // served body exposes a commit audit) the overview §"`ETag` and
        // Last-Modified" asks of VERSION / VERSIONED_OBJECT responses, and no
        // `Location` (§Location — creation/redirect only).
        "versioned_party_relationship_get" => {
            let p = VersionedPartyGetParams::from_request(parts.route()?, q, h)?;
            let vo = p.versioned_object_uid.clone();
            let resp = state
                .backend()
                .versioned_party_relationship_get(p.versioned_object_uid)
                .await?;
            Ok(super::read_versioned(h, &vo, &resp.body))
        }
        "party_relationship_revision_history" => {
            let p = VersionedPartyGetParams::from_request(parts.route()?, q, h)?;
            let vo = p.versioned_object_uid.clone();
            let resp = state
                .backend()
                .party_relationship_revision_history(p.versioned_object_uid)
                .await?;
            Ok(super::read_versioned(h, &vo, &resp.body))
        }
        "party_relationship_version_get_at_time" => {
            let p = VersionedPartyVersionGetAtTimeParams::from_request(parts.route()?, q, h)?;
            let resp = state
                .backend()
                .party_relationship_version_get_at_time(p.versioned_object_uid, p.version_at_time)
                .await?;
            let mut out = negotiate::respond(h, StatusCode::OK, &resp.body);
            super::set_versioning_headers(&mut out, resp.meta.as_ref());
            Ok(out)
        }
        "party_relationship_version_get_by_id" => {
            let p = VersionedPartyVersionGetByIdParams::from_request(parts.route()?, q, h)?;
            let vo = p.versioned_object_uid.clone();
            let resp = state
                .backend()
                .party_relationship_version_get_by_id(p.versioned_object_uid, p.version_uid)
                .await?;
            Ok(super::read_versioned(h, &vo, &resp.body))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted demographic relationship operation: {other}"
        )))),
    }
}

/// A create/update relationship response honouring `Prefer` (canonical
/// JSON/XML via the concrete `PARTY_RELATIONSHIP` type) and setting the
/// demographic `ETag`/`Location`.
fn write_relationship(
    h: &http::HeaderMap,
    base: &str,
    segment: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    resp: &ServiceResponse,
) -> Response {
    // The full `Prefer` triad (overview §Prefer): representation → the RM
    // body; identifier → `{uid}` only, never `204`; minimal (default) → empty.
    // One seam for all three, and for the `Preference-Applied` declaration.
    let mut out = negotiate::write_negotiated(
        h,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| {
            negotiate::respond_rm::<PartyRelationship>(h, status, &resp.body, "party_relationship")
        },
    );
    super::set_write_headers(&mut out, base, segment, resp.meta.as_ref());
    out
}

/// A `200 OK` read of a relationship, setting the demographic
/// `ETag`/`Last-Modified`. No `Location` — overview §Location: the header "MUST
/// NOT be used to indicate an alternate representation of an existing resource
/// (e.g. via `GET` method)".
fn read_relationship(h: &http::HeaderMap, resp: &ServiceResponse) -> Response {
    let mut out = negotiate::respond_rm::<PartyRelationship>(
        h,
        StatusCode::OK,
        &resp.body,
        "party_relationship",
    );
    super::set_versioning_headers(&mut out, resp.meta.as_ref());
    out
}
