// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native `utoipa-axum` routing for the **standard Demographic API group**:
//! the party CRUD (`agent`/`group`/`organisation`/`person`/`role`), the
//! `versioned_party` reads, `contribution` create/get, and the `ITEM_TAG`
//! sub-resources (`demographic_tags_get` + the per-party `*_tags_*`).
//!
//! Each `#[utoipa::path]` handler single-sources its route and its `OpenAPI`
//! path, then forwards to the demographic group dispatcher
//! ([`super::dispatch::dispatch`]) through [`guarded_dispatch`], so the
//! `EHR_ACCESS` gate, the ABAC PEP and the ATNA audit tagging apply uniformly.
//!
//! The Demographic API is `DEVELOPMENT`-state within ITS-REST Release-1.1.0
//! (`docs/demographic/Description.md` §Status). That is a reporting qualifier
//! only: the BCP-14 requirement force of the released text is not
//! state-qualified, so every MUST and SHOULD below binds as it would on a STABLE
//! group, and the declarations are pinned to that release rather than to the
//! upstream development branch.
//!
//! The five party CRUD quintets are byte-identical across the kinds on the
//! released wire, so each kind mirrors the same
//! `operations/person_{create,get,update,delete}.yaml` and its `$ref`d
//! components, differing only in the RM type and its own `Location_*` header.
//! The `versioned_party_*`, `demographic_contribution_*` and `ITEM_TAG`
//! families are declared from their own operation files. Everything those files
//! leave open is filled from the released overview chapters, the docs text
//! winning every conflict: `Requests_and_responses.md` (the weak `W/` `ETag`
//! MUST, the `Prefer` triad, the committal-header MUST-accept rule, the
//! `If-Match` `400`/`412` rules, §Location's MUST-NOT on `GET`) and
//! `Resources.md` (the `415`/`406` format MUSTs). Two consequences show up on
//! every declaration:
//!
//! - `Location` is never declared on a read or a delete. The released responses
//!   slot `headers/Location_deprecated.yaml`, and §Location confines the header
//!   to "resource creation (e.g., `201 Created`) or redirect responses", so
//!   those slots are left undeclared rather than declared-as-deprecated.
//! - Party resources are canonical-only. The released operations reference
//!   `Accept_LOCATABLE.yaml`, whose enum admits the Simplified MIME types, but a
//!   PARTY is not templated and §openehr-template-id scopes the only
//!   template-naming header to "committing COMPOSITION", so a Simplified party
//!   payload cannot name the template it would be expanded against. Our handling
//!   of that gap, which the released text does not fix: a Simplified
//!   `Content-Type` is `415` and a Simplified-only `Accept` is `406`, the two
//!   MUSTs `Resources.md` §"Simplified Formats" states.
//!
//! The own-design `PARTY_RELATIONSHIP` extension lives in
//! [`super::relationship`]; no ITS-REST operation governs it.

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

/// The standard Demographic API group as a native `utoipa-axum` router.
///
/// Paths are group-relative, nested under the configured `base_path`, and every
/// operation runs through [`guarded_dispatch`] with the demographic group
/// [`dispatch`](super::dispatch::dispatch).
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(agent_create))
        .routes(routes!(agent_get, agent_update, agent_delete))
        .routes(routes!(group_create))
        .routes(routes!(group_get, group_update, group_delete))
        .routes(routes!(organisation_create))
        .routes(routes!(
            organisation_get,
            organisation_update,
            organisation_delete
        ))
        .routes(routes!(person_create))
        .routes(routes!(person_get, person_update, person_delete))
        .routes(routes!(role_create))
        .routes(routes!(role_get, role_update, role_delete))
        .routes(routes!(versioned_party_get))
        .routes(routes!(versioned_party_revision_history))
        .routes(routes!(versioned_party_version_get_at_time))
        .routes(routes!(versioned_party_version_get_by_id))
        .routes(routes!(contribution_create))
        .routes(routes!(contribution_get))
        .routes(routes!(demographic_tags_get))
        .routes(routes!(agent_tags_get, agent_tags_update))
        .routes(routes!(agent_tags_delete))
        .routes(routes!(group_tags_get, group_tags_update))
        .routes(routes!(group_tags_delete))
        .routes(routes!(organisation_tags_get, organisation_tags_update))
        .routes(routes!(organisation_tags_delete))
        .routes(routes!(person_tags_get, person_tags_update))
        .routes(routes!(person_tags_delete))
        .routes(routes!(role_tags_get, role_tags_update))
        .routes(routes!(role_tags_delete))
}

// ── Handlers ──────────────────────────────────────────────────────────────
// Each snapshots the request into `RequestParts` (identical to the generated-
// group `mount` adapter) and runs it through the demographic group dispatcher
// (`super::dispatch::dispatch`), so the EHR_ACCESS gate, ABAC PEP, and ATNA audit tagging
// apply uniformly.

// ── AGENT ───────────────────────────────────────────────────────────────────

/// Create an `AGENT` (`POST /demographic/agent`).
///
/// "Creates the first version of a new `AGENT`." (ITS-REST
/// `specifications/operations/agent_create.yaml`). The `uid` is server-minted:
/// a PARTY's `uid` is the containing `VERSION`'s `OBJECT_VERSION_ID`, which the
/// client cannot know at create time, so a `uid` in the submitted body does not
/// survive the write and the invariant `Uid_mandatory` (RM
/// `demographic/master02` §Party Identification, `PARTY.Uid_mandatory`) is
/// satisfied post-assignment. The released create declares no `409`, so a
/// client-supplied `uid` is never a conflict.
#[utoipa::path(
    post, path = "/demographic/agent", tag = "AGENT",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; enum \
                        `return=representation|return=minimal|return=identifier`, \
                        default `return=minimal`). An absent header is \
                        `return=minimal` — \"If no `Prefer` header is provided, \
                        the default behavior is assumed to be `return=minimal`\" \
                        — and `return=identifier` never answers `204`: \"the \
                        status will be `201 Created` or `200 OK`, never `204 No \
                        Content`\" (`Requests_and_responses.md` §\"Prefer only \
                        identifier\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`). \
                        An absent header reads as canonical JSON — `Resources.md` \
                        §\"JSON Format\" makes the header a client MAY, so its \
                        absence declares nothing to refuse. A Simplified \
                        `Content-Type` is `415` (see that response).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", and \
                        \"whatever is provided it MUST be merged with the default \
                        VERSION and VERSION.audit_details attributes on commit \
                        runtime\" (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\", which scopes the rule to \
                        \"all change-controlled resources\" — parties are \
                        version-controlled, RM `common/master06` §Change \
                        Control).",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. \"Through the `openehr-audit-details` header, \
                        clients MAY supply values for the AUDIT_DETAILS \
                        attributes `change_type`, `description`, `committer` and \
                        `system_id`. The `time_committed` attribute is always set \
                        by the server.\" — and \"when `system_id` is not provided \
                        by the client, the server MUST set it to its own \
                        configured system identifier\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "committer.name=\"John Doe\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — here the VERSIONED_PARTY. The tags are stored after the \
                        party exists and the stored set is echoed in the response \
                        header of the same name. \"Providing an empty value for \
                        this header will effectively remove all ITEM_TAGs \
                        associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", Usage in Requests); an absent \
                        header changes nothing.",
         example = "key=\"category\",value=\"final\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`). \
                        The two wrapper headers address DISTINCT collections \
                        (overview §\"openehr-item-tag and \
                        openehr-version-item-tag\"): this one replaces the \
                        just-committed VERSION's own tag set, `openehr-item-tag` \
                        the `VERSIONED_PARTY` container's; each response header \
                        echoes its own stored set.",
         example = "key=\"reviewed\",value=\"true\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The AGENT.\", `required: true` (ITS-REST \
                                `specifications/operations/agent_create.yaml`; \
                                schema `schemas/demographic/Agent.yaml`) as \
                                canonical JSON or XML. `PARTY.identities` is \
                                mandatory and non-empty (`Identities_valid`), and \
                                `name` carries the type designation \
                                (`Type_valid: type = name`, RM UML \
                                `org.openehr.rm.demographic.party`).",
                 example = json!({
                     "_type": "AGENT",
                     "name": { "_type": "DV_TEXT", "value": "AGENT" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.agent.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.agent.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Triage Assistant v2" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the AGENT is \
                                      successfully created. If `Prefer` header is \
                                      `return=representation`, the full resource \
                                      is included in the response body; if is \
                                      `return=identifier`, only its unique \
                                      identifier is included. If the `Prefer` \
                                      header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_AGENT.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is an \
                             identifier (e.g. a `version_uid` enclosed by double \
                             quotes) for a specific version of a resource.\" \
                             (ITS-REST `specifications/headers/ETag.yaml`), in the \
                             weak form the release requires — \"all `ETag` headers \
                             that hold a resource identifier MUST include a \
                             weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"<versioned_object_uid>::<system_id>::1\"`."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the AGENT resource.\" (ITS-REST \
                             `specifications/headers/Location_AGENT.yaml`), set to \
                             `<base_path>/demographic/agent/<version_uid>` — \
                             §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date; \"this value should be derived from \
                             VERSION.commit_audit.time_committed.value\", and both \
                             `ETag` and `Last-Modified` \"SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released `201_AGENT.yaml` does \
                             not slot it; the SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured. \"The service MAY include a \
                             `Preference-Applied` header in the response … to \
                             indicate that the client's preference has been \
                             honored\" (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) — the \
                             set as the server stored it; emitted only when the \
                             party carries tags (\"Servers MAY include the \
                             `openehr-item-tag` … header in responses to confirm \
                             the actual list of ITEM_TAGs stored on the server \
                             side\")."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created AGENT",
              value = json!({
                  "_type": "AGENT",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "AGENT" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.agent.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.agent.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Triage Assistant v2" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a body that is not well-formed canonical \
                                      JSON/XML. Content that parses but is not a \
                                      valid AGENT is the `422` below.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). On a \
                                      create the reachable trigger is a referenced \
                                      resource the commit resolves and does not \
                                      find.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied. A \
                                      PARTY is untemplated, so this server serves \
                                      it in the canonical formats only and refuses \
                                      a Simplified-only `Accept`: \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\" (`Resources.md` \
                                      §\"Simplified Formats\"; the same MUST is \
                                      stated for XML and JSON). The released \
                                      operation does not enumerate `406`; the MUST \
                                      is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A PARTY is not templated and \
                                      `openehr-template-id` — the only header that \
                                      can name a template — is scoped to \
                                      \"committing COMPOSITION\" \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified party \
                                      payload cannot be expanded: \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      AGENT (empty `identities`, a `name` that is \
                                      not the type designation), or a body whose \
                                      `_type` is a different PARTY subtype than \
                                      the route's — the routed kind's codec is the \
                                      one that decodes it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_create", parts, super::dispatch::dispatch).await
}

/// Retrieve an `AGENT` by uid-based id
/// (`GET /demographic/agent/{uid_based_id}`).
///
/// "Retrieves a version of the `AGENT` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/agent_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/agent/{uid_based_id}", tag = "AGENT",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An abstract \
                        identifier: it can take a form of an OBJECT_VERSION_ID \
                        identifier taken from VERSION.uid.value (i.e. a \
                        `version_uid`), or a form of a HIER_OBJECT_ID identifier \
                        taken from VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id.yaml`). The \
                        operation adds: \"When the `uid_based_id` has the form of \
                        a HIER_OBJECT_ID, if the `version_at_time` is supplied, \
                        retrieves the version extant _at specified time_, \
                        otherwise retrieves the _latest_ AGENT version.\" A \
                        syntactically unusable id is `400`; a well-formed id \
                        naming no AGENT (including a container of another PARTY \
                        kind) is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`). \
                        Selects the version extant at that instant when the path \
                        id is a `versioned_object_uid`; the latest version when \
                        omitted. The timezone is optional — server-local when \
                        absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested AGENT is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_AGENT_retrieved.yaml`). \
                                      That response slots \
                                      `headers/Location_deprecated.yaml`, and \
                                      §Location says the header \"MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource (e.g. via `GET` \
                                      method)\" — so no `Location` is emitted or \
                                      declared here.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (ITS-REST \
                             `specifications/headers/ETag.yaml`; \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\" makes resource-identifier `ETag`s \
                             weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\"; both \
                             headers \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`). \
                             \"When retrieving resources via `GET`, the server MAY \
                             also add these headers to the response\" \
                             (`Requests_and_responses.md` §\"openehr-item-tag and \
                             openehr-version-item-tag\", Usage in Responses); \
                             emitted only when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         example = json!({
             "_type": "AGENT",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "AGENT" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.agent.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.agent.v1" },
                 "rm_version": "1.2.0"
             },
             "identities": [
                 {
                     "_type": "PARTY_IDENTITY",
                     "name": { "_type": "DV_TEXT", "value": "legal identity" },
                     "archetype_node_id": "at0001",
                     "details": {
                         "_type": "ITEM_TREE",
                         "name": { "_type": "DV_TEXT", "value": "identity details" },
                         "archetype_node_id": "at0002",
                         "items": [
                             {
                                 "_type": "ELEMENT",
                                 "name": { "_type": "DV_TEXT", "value": "name" },
                                 "archetype_node_id": "at0003",
                                 "value": { "_type": "DV_TEXT", "value": "Triage Assistant v2" }
                             }
                         ]
                     }
                 }
             ]
         })),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters (at \
                                      specified `version_at_time`) time has been \
                                      deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`) \
                                      — the version selected by the request is a \
                                      deletion marker, which is a successful read \
                                      of a logically deleted resource, not a \
                                      `404`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `uid_based_id` that is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or a \
                                      `version_at_time` that is not an extended \
                                      ISO 8601 instant. The released get does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`). \
                                      A well-formed id whose stored container is a \
                                      different PARTY kind is this `404` too — the \
                                      route is kind-checked, and a VERSIONED_OBJECT \
                                      has one type (RM `common/master06` §Change \
                                      Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated, so it is served in the \
                                      canonical formats only and a Simplified-only \
                                      `Accept` is refused — \"If the service cannot \
                                      fulfill this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` carries no payload, \
                                      but the released operation still admits a \
                                      request `Content-Type`, and a party has no \
                                      template to expand a Simplified payload \
                                      against, so the declaration is refused \
                                      before the read: \"If the service cannot \
                                      process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
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
pub(crate) async fn agent_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_get", parts, super::dispatch::dispatch).await
}

/// Update an `AGENT` (`PUT /demographic/agent/{uid_based_id}`).
///
/// "Updates `AGENT` identified by `uid_based_id`." … "The existing latest
/// `version_uid` of `AGENT` resource (i.e. the `preceding_version_uid`) must be
/// specified in the `If-Match` header." (ITS-REST
/// `specifications/operations/agent_update.yaml`).
#[utoipa::path(
    put, path = "/demographic/agent/{uid_based_id}", tag = "AGENT",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_versioned_object_uid.yaml`) \
                        — the container, not a version. The operation adds: \"If \
                        the request body already contains an AGENT.uid.value, it \
                        must match the `uid_based_id` in the URL.\"",
         example = "8849182c-82ad-4088-a07f-48ead4180515"),
        ("If-Match" = String, Header,
         description = "The released parameter, verbatim: \"Header to make the \
                        request conditional. Together with `ETag` request tag, it \
                        helps to prevent simultaneous updates of a resource from \
                        overwriting each other (\"mid-air collisions\"). The \
                        format is always an `version_uid` identifier enclosed by \
                        double quotes. The operation will be performed only if \
                        the existing latest `version_uid` of the resource (i.e. \
                        the `preceding_version_uid`) matches this header's \
                        value.\" (ITS-REST \
                        `specifications/parameters/header/If-Match.yaml`, \
                        `required: true`). The weak `W/\"…\"` form this server \
                        emits in `ETag` is accepted too — the bare quoted form is \
                        the pre-1.1.0 shape the release keeps supported \
                        (`Requests_and_responses.md` §\"Deprecated headers\").",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`). `return=minimal` answers `204`; the \
                        other two answer `200` — \"the status will be `201 \
                        Created` or `200 OK`, never `204 No Content`\" for \
                        `return=identifier` (`Requests_and_responses.md` \
                        §\"Prefer only identifier\"). The token honoured is echoed \
                        in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`); \
                        absent reads as canonical JSON (`Resources.md` §\"JSON \
                        Format\" makes the header a client MAY). A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", merged \
                        with the server defaults \"on commit runtime\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `change_type`, `description`, `committer` and \
                        `system_id` MAY be supplied; \"The `time_committed` \
                        attribute is always set by the server\", and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\"). No \
                        released parameter file declares it.",
         example = "change_type.code_string=\"251\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`; \
                        the only tag parameter the released update declares). \
                        Demographic ITEM_TAGs are stored against the \
                        VERSIONED_PARTY with no version anchor, so the two tag \
                        sets coincide on this surface and this build takes the \
                        list to store from `openehr-item-tag`; both response \
                        headers then carry that one set.",
         example = "key=\"reviewed\",value=\"true\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — the VERSIONED_PARTY. Not declared on the released \
                        update operation, but it is the header this build reads \
                        as the tag list to store, and demographic tags are \
                        VERSIONED_OBJECT-anchored, so it is the accurate one to \
                        send here. An empty value \"will effectively remove all \
                        ITEM_TAGs associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\"); an absent header leaves the \
                        stored tags untouched.",
         example = "key=\"category\",value=\"final\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The new AGENT.\", `required: true` (ITS-REST \
                                `specifications/operations/agent_update.yaml`; \
                                schema `schemas/demographic/Agent.yaml`) as \
                                canonical JSON or XML. A `uid` in the body \"must \
                                match the `uid_based_id` in the URL\".",
                 example = json!({
                     "_type": "AGENT",
                     "name": { "_type": "DV_TEXT", "value": "AGENT" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.agent.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.agent.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Triage Assistant v3" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the AGENT is \
                                      successfully updated, with the full \
                                      resource in the response body when `Prefer` \
                                      header is `return=representation`, or only \
                                      its identifiers when `Prefer` header is \
                                      `return=identifier`.\" (ITS-REST \
                                      `specifications/responses/200_AGENT_updated.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             NEW version (ITS-REST \
                             `specifications/headers/ETag.yaml`; the weakness \
                             indicator is the release's MUST, \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the AGENT resource.\" (ITS-REST \
                             `specifications/headers/Location_AGENT.yaml`), set to \
                             `<base_path>/demographic/agent/<new version_uid>`. \
                             §Location scopes the header to \"resource creation … \
                             or redirect responses\" and §\"Prefer minimal, \
                             identifier or full representation response\" names \
                             the target as \"the newly created or updated \
                             resource\" — an openEHR update commits a NEW VERSION, \
                             which is that newly created resource."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"; both headers SHOULD accompany a \
                             versioned resource). The released \
                             `200_AGENT_updated.yaml` does not slot it; the \
                             SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured \
                             (`Requests_and_responses.md` §\"Representation \
                             details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) as the \
                             server stored it; emitted only when the party carries \
                             tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the same set, demographic tags having no version \
                             anchor.")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the updated AGENT",
              value = json!({
                  "_type": "AGENT",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "name": { "_type": "DV_TEXT", "value": "AGENT" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.agent.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.agent.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Triage Assistant v3" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the `Prefer` \
                                      header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_version_updated.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version (ITS-REST \
                             `specifications/headers/ETag.yaml`)."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the resource version resulted from the operation.\" \
                             (ITS-REST \
                             `specifications/headers/Location_version.yaml`), set \
                             to \
                             `<base_path>/demographic/agent/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`), \
                             emitted when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Two \
                                      reachable triggers here: an unparseable \
                                      `uid_based_id`/body, and an ABSENT \
                                      `If-Match` — \"When the service expects \
                                      `If-Match` for an operation, but the client \
                                      does not provide it, the service SHOULD \
                                      respond with `400 Bad Request`\" \
                                      (`Requests_and_responses.md` §\"If-Match and \
                                      accidental overwrites\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). A \
                                      `versioned_object_uid` whose stored \
                                      container is a different PARTY kind is this \
                                      `404` as well — the route is kind-checked \
                                      (a VERSIONED_OBJECT has one type, RM \
                                      `common/master06` §Change Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated and served in the \
                                      canonical formats only, so a \
                                      Simplified-only `Accept` MUST be refused \
                                      (`Resources.md` §\"Simplified Formats\"). \
                                      The released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 412, description = "The released trigger, verbatim: `412 \
                                      Precondition Failed` \"is returned when \
                                      `If-Match` request header doesn't match the \
                                      latest version on the service side. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\" (ITS-REST \
                                      `specifications/responses/412_AGENT.yaml`; \
                                      the same rule is the overview's own MUST — \
                                      \"it MUST NOT perform the requested method. \
                                      Instead, it MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` — the service \"SHOULD return also latest \
                             `version_uid` in the `ETag` response headers\" \
                             (`Requests_and_responses.md` §\"If-Match and \
                             accidental overwrites\"; ITS-REST \
                             `specifications/headers/ETag.yaml`). The released \
                             `412_AGENT.yaml` also slots \
                             `headers/Location_deprecated.yaml`; §Location \
                             forbids `Location` on a non-creation response, so \
                             none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which a party payload cannot \
                                      use (no template can be named for an \
                                      untemplated resource — \
                                      `Requests_and_responses.md` \
                                      §openehr-template-id): \"it MUST respond with \
                                      HTTP status code `415 Unsupported Media \
                                      Type`\" (`Resources.md` §\"Simplified \
                                      Formats\"). An absent `Content-Type` declares \
                                      nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      AGENT, or a body typed as a different PARTY \
                                      subtype than the route's.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_update", parts, super::dispatch::dispatch).await
}

/// Delete an `AGENT` (`DELETE /demographic/agent/{uid_based_id}`).
///
/// "Deletes the `AGENT` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/agent_delete.yaml`). The delete is LOGICAL: it
/// commits a new deletion `VERSION` rather than removing history — RM
/// `common/master06` §Change Control keeps every committed version, and a
/// subsequent read of the deleted current version answers `204`
/// (`responses/204_deleted_at_time.yaml`).
#[utoipa::path(
    delete, path = "/demographic/agent/{uid_based_id}", tag = "AGENT",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of an OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_version_uid.yaml`); \
                        the operation sharpens it: \"The `uid_based_id` MUST be in \
                        a form of an OBJECT_VERSION_ID identifier taken from the \
                        last (most recent) VERSION.uid.value, representing the \
                        `preceding_version_uid` to be deleted.\" A version that is \
                        not the latest is `409`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL here, and the released operation declares no \
                        `If-Match` parameter at all — by the spec's own carve-out: \
                        the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"), and on this operation it IS the \
                        path segment. A header that IS sent is still honoured — \
                        the same section makes a received precondition binding — \
                        as an alternative source of the preceding version; the \
                        weak `W/\"…\"` and bare quoted forms are both accepted.",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list. No released parameter file declares \
                        this header; the requirement is prose — \"services MUST \
                        also allow `PUT`, `POST` and `DELETE` methods directly on \
                        these change-controlled resources\" and \"services MUST \
                        accept `openehr-version` and `openehr-audit-details` \
                        custom request headers\" (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\"\
                        A `lifecycle_state` naming any state other than \
                        `523|deleted|` contradicts the operation and is refused \
                        `400` rather than silently discarded — the section makes \
                        the merge a MUST, and a value that cannot be merged is \
                        reported, not dropped (RM common master06 \
                        §\"Logical Deletion\")."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY repeat. \
                        `time_committed` is always server-set and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "description.value=\"merged into another record\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format (`application/json` by default). A \
                        Simplified-only `Accept` is `406` — a party is untemplated \
                        (`Resources.md` §\"Simplified Formats\").",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned for a successful \
                                      delete operation.\" (ITS-REST \
                                      `specifications/responses/204_version_deleted.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version the operation just committed — not \
                             the version named in the path. The released \
                             `204_version_deleted.yaml` slots \
                             `headers/ETag.yaml`, whose value \"is an identifier \
                             (e.g. a `version_uid` …) for a specific version of a \
                             resource\", and §\"ETag and Last-Modified\" adds that \
                             it \"changes as soon as the resource changes (i.e. \
                             when a new version is created)\" — a logical delete \
                             creates one. That same response slots \
                             `headers/Location_deprecated.yaml`; §\"Deprecated \
                             headers\" deprecates `Location` on `DELETE` \
                             responses, so none is emitted or declared.")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content) or when the resource identified by \
                                      the request parameters is already deleted.\" \
                                      (ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`) \
                                      — so a second delete of the same party is \
                                      this `400`, not a `404`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or one holding a \
                                      different PARTY kind than this route.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied \
                                      (a Simplified-only `Accept` on an \
                                      untemplated resource): \"it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header.\" (ITS-REST \
                                      `specifications/responses/409_AGENT_with_uid_based_id.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` (ITS-REST \
                             `specifications/headers/ETag.yaml`) — the client can \
                             retry the delete against it. The released response \
                             also slots `headers/Location_deprecated.yaml`; \
                             §Location forbids `Location` on a non-creation \
                             response, so none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is still refused before \
                                      the write, because a party has no template a \
                                      Simplified payload could be expanded against \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id; `Resources.md` \
                                      §\"Simplified Formats\" `415` MUST). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_delete", parts, super::dispatch::dispatch).await
}

// ── GROUP ─────────────────────────────────────────────────────────────────

/// Create a `GROUP` (`POST /demographic/group`).
///
/// "Creates the first version of a new `GROUP`." (ITS-REST
/// `specifications/operations/group_create.yaml`). The `uid` is server-minted:
/// a PARTY's `uid` is the containing `VERSION`'s `OBJECT_VERSION_ID`, which the
/// client cannot know at create time, so a `uid` in the submitted body does not
/// survive the write and the invariant `Uid_mandatory` (RM
/// `demographic/master02` §Party Identification, `PARTY.Uid_mandatory`) is
/// satisfied post-assignment. The released create declares no `409`, so a
/// client-supplied `uid` is never a conflict.
#[utoipa::path(
    post, path = "/demographic/group", tag = "GROUP",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; enum \
                        `return=representation|return=minimal|return=identifier`, \
                        default `return=minimal`). An absent header is \
                        `return=minimal` — \"If no `Prefer` header is provided, \
                        the default behavior is assumed to be `return=minimal`\" \
                        — and `return=identifier` never answers `204`: \"the \
                        status will be `201 Created` or `200 OK`, never `204 No \
                        Content`\" (`Requests_and_responses.md` §\"Prefer only \
                        identifier\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`). \
                        An absent header reads as canonical JSON — `Resources.md` \
                        §\"JSON Format\" makes the header a client MAY, so its \
                        absence declares nothing to refuse. A Simplified \
                        `Content-Type` is `415` (see that response).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", and \
                        \"whatever is provided it MUST be merged with the default \
                        VERSION and VERSION.audit_details attributes on commit \
                        runtime\" (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\", which scopes the rule to \
                        \"all change-controlled resources\" — parties are \
                        version-controlled, RM `common/master06` §Change \
                        Control).",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. \"Through the `openehr-audit-details` header, \
                        clients MAY supply values for the AUDIT_DETAILS \
                        attributes `change_type`, `description`, `committer` and \
                        `system_id`. The `time_committed` attribute is always set \
                        by the server.\" — and \"when `system_id` is not provided \
                        by the client, the server MUST set it to its own \
                        configured system identifier\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "committer.name=\"John Doe\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — here the VERSIONED_PARTY. The tags are stored after the \
                        party exists and the stored set is echoed in the response \
                        header of the same name. \"Providing an empty value for \
                        this header will effectively remove all ITEM_TAGs \
                        associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", Usage in Requests); an absent \
                        header changes nothing.",
         example = "key=\"category\",value=\"final\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`). \
                        The two wrapper headers address DISTINCT collections \
                        (overview §\"openehr-item-tag and \
                        openehr-version-item-tag\"): this one replaces the \
                        just-committed VERSION's own tag set, `openehr-item-tag` \
                        the `VERSIONED_PARTY` container's; each response header \
                        echoes its own stored set.",
         example = "key=\"reviewed\",value=\"true\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The GROUP.\", `required: true` (ITS-REST \
                                `specifications/operations/group_create.yaml`; \
                                schema `schemas/demographic/Group.yaml`) as \
                                canonical JSON or XML. `PARTY.identities` is \
                                mandatory and non-empty (`Identities_valid`), and \
                                `name` carries the type designation \
                                (`Type_valid: type = name`, RM UML \
                                `org.openehr.rm.demographic.party`).",
                 example = json!({
                     "_type": "GROUP",
                     "name": { "_type": "DV_TEXT", "value": "GROUP" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-GROUP.group.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Cardiology on-call rota" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the GROUP is \
                                      successfully created. If `Prefer` header is \
                                      `return=representation`, the full resource \
                                      is included in the response body; if is \
                                      `return=identifier`, only its unique \
                                      identifier is included. If the `Prefer` \
                                      header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_GROUP.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is an \
                             identifier (e.g. a `version_uid` enclosed by double \
                             quotes) for a specific version of a resource.\" \
                             (ITS-REST `specifications/headers/ETag.yaml`), in the \
                             weak form the release requires — \"all `ETag` headers \
                             that hold a resource identifier MUST include a \
                             weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"<versioned_object_uid>::<system_id>::1\"`."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the GROUP resource.\" (ITS-REST \
                             `specifications/headers/Location_GROUP.yaml`), set to \
                             `<base_path>/demographic/group/<version_uid>` — \
                             §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date; \"this value should be derived from \
                             VERSION.commit_audit.time_committed.value\", and both \
                             `ETag` and `Last-Modified` \"SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released `201_GROUP.yaml` does \
                             not slot it; the SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured. \"The service MAY include a \
                             `Preference-Applied` header in the response … to \
                             indicate that the client's preference has been \
                             honored\" (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) — the \
                             set as the server stored it; emitted only when the \
                             party carries tags (\"Servers MAY include the \
                             `openehr-item-tag` … header in responses to confirm \
                             the actual list of ITEM_TAGs stored on the server \
                             side\")."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created GROUP",
              value = json!({
                  "_type": "GROUP",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "GROUP" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-GROUP.group.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Cardiology on-call rota" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a body that is not well-formed canonical \
                                      JSON/XML. Content that parses but is not a \
                                      valid GROUP is the `422` below.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). On a \
                                      create the reachable trigger is a referenced \
                                      resource the commit resolves and does not \
                                      find.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied. A \
                                      PARTY is untemplated, so this server serves \
                                      it in the canonical formats only and refuses \
                                      a Simplified-only `Accept`: \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\" (`Resources.md` \
                                      §\"Simplified Formats\"; the same MUST is \
                                      stated for XML and JSON). The released \
                                      operation does not enumerate `406`; the MUST \
                                      is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A PARTY is not templated and \
                                      `openehr-template-id` — the only header that \
                                      can name a template — is scoped to \
                                      \"committing COMPOSITION\" \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified party \
                                      payload cannot be expanded: \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      GROUP (empty `identities`, a `name` that is \
                                      not the type designation), or a body whose \
                                      `_type` is a different PARTY subtype than \
                                      the route's — the routed kind's codec is the \
                                      one that decodes it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_create", parts, super::dispatch::dispatch).await
}

/// Retrieve a `GROUP` by uid-based id
/// (`GET /demographic/group/{uid_based_id}`).
///
/// "Retrieves a version of the `GROUP` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/group_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/group/{uid_based_id}", tag = "GROUP",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An abstract \
                        identifier: it can take a form of an OBJECT_VERSION_ID \
                        identifier taken from VERSION.uid.value (i.e. a \
                        `version_uid`), or a form of a HIER_OBJECT_ID identifier \
                        taken from VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id.yaml`). The \
                        operation adds: \"When the `uid_based_id` has the form of \
                        a HIER_OBJECT_ID, if the `version_at_time` is supplied, \
                        retrieves the version extant _at specified time_, \
                        otherwise retrieves the _latest_ GROUP version.\" A \
                        syntactically unusable id is `400`; a well-formed id \
                        naming no GROUP (including a container of another PARTY \
                        kind) is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`). \
                        Selects the version extant at that instant when the path \
                        id is a `versioned_object_uid`; the latest version when \
                        omitted. The timezone is optional — server-local when \
                        absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested GROUP is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_GROUP_retrieved.yaml`). \
                                      That response slots \
                                      `headers/Location_deprecated.yaml`, and \
                                      §Location says the header \"MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource (e.g. via `GET` \
                                      method)\" — so no `Location` is emitted or \
                                      declared here.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (ITS-REST \
                             `specifications/headers/ETag.yaml`; \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\" makes resource-identifier `ETag`s \
                             weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\"; both \
                             headers \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`). \
                             \"When retrieving resources via `GET`, the server MAY \
                             also add these headers to the response\" \
                             (`Requests_and_responses.md` §\"openehr-item-tag and \
                             openehr-version-item-tag\", Usage in Responses); \
                             emitted only when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         example = json!({
             "_type": "GROUP",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "GROUP" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-GROUP.group.v1" },
                 "rm_version": "1.2.0"
             },
             "identities": [
                 {
                     "_type": "PARTY_IDENTITY",
                     "name": { "_type": "DV_TEXT", "value": "legal identity" },
                     "archetype_node_id": "at0001",
                     "details": {
                         "_type": "ITEM_TREE",
                         "name": { "_type": "DV_TEXT", "value": "identity details" },
                         "archetype_node_id": "at0002",
                         "items": [
                             {
                                 "_type": "ELEMENT",
                                 "name": { "_type": "DV_TEXT", "value": "name" },
                                 "archetype_node_id": "at0003",
                                 "value": { "_type": "DV_TEXT", "value": "Cardiology on-call rota" }
                             }
                         ]
                     }
                 }
             ]
         })),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters (at \
                                      specified `version_at_time`) time has been \
                                      deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`) \
                                      — the version selected by the request is a \
                                      deletion marker, which is a successful read \
                                      of a logically deleted resource, not a \
                                      `404`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `uid_based_id` that is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or a \
                                      `version_at_time` that is not an extended \
                                      ISO 8601 instant. The released get does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`). \
                                      A well-formed id whose stored container is a \
                                      different PARTY kind is this `404` too — the \
                                      route is kind-checked, and a VERSIONED_OBJECT \
                                      has one type (RM `common/master06` §Change \
                                      Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated, so it is served in the \
                                      canonical formats only and a Simplified-only \
                                      `Accept` is refused — \"If the service cannot \
                                      fulfill this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` carries no payload, \
                                      but the released operation still admits a \
                                      request `Content-Type`, and a party has no \
                                      template to expand a Simplified payload \
                                      against, so the declaration is refused \
                                      before the read: \"If the service cannot \
                                      process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
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
pub(crate) async fn group_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_get", parts, super::dispatch::dispatch).await
}

/// Update a `GROUP` (`PUT /demographic/group/{uid_based_id}`).
///
/// "Updates `GROUP` identified by `uid_based_id`." … "The existing latest
/// `version_uid` of `GROUP` resource (i.e. the `preceding_version_uid`) must be
/// specified in the `If-Match` header." (ITS-REST
/// `specifications/operations/group_update.yaml`).
#[utoipa::path(
    put, path = "/demographic/group/{uid_based_id}", tag = "GROUP",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_versioned_object_uid.yaml`) \
                        — the container, not a version. The operation adds: \"If \
                        the request body already contains a GROUP.uid.value, it \
                        must match the `uid_based_id` in the URL.\"",
         example = "8849182c-82ad-4088-a07f-48ead4180515"),
        ("If-Match" = String, Header,
         description = "The released parameter, verbatim: \"Header to make the \
                        request conditional. Together with `ETag` request tag, it \
                        helps to prevent simultaneous updates of a resource from \
                        overwriting each other (\"mid-air collisions\"). The \
                        format is always an `version_uid` identifier enclosed by \
                        double quotes. The operation will be performed only if \
                        the existing latest `version_uid` of the resource (i.e. \
                        the `preceding_version_uid`) matches this header's \
                        value.\" (ITS-REST \
                        `specifications/parameters/header/If-Match.yaml`, \
                        `required: true`). The weak `W/\"…\"` form this server \
                        emits in `ETag` is accepted too — the bare quoted form is \
                        the pre-1.1.0 shape the release keeps supported \
                        (`Requests_and_responses.md` §\"Deprecated headers\").",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`). `return=minimal` answers `204`; the \
                        other two answer `200` — \"the status will be `201 \
                        Created` or `200 OK`, never `204 No Content`\" for \
                        `return=identifier` (`Requests_and_responses.md` \
                        §\"Prefer only identifier\"). The token honoured is echoed \
                        in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`); \
                        absent reads as canonical JSON (`Resources.md` §\"JSON \
                        Format\" makes the header a client MAY). A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", merged \
                        with the server defaults \"on commit runtime\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `change_type`, `description`, `committer` and \
                        `system_id` MAY be supplied; \"The `time_committed` \
                        attribute is always set by the server\", and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\"). No \
                        released parameter file declares it.",
         example = "change_type.code_string=\"251\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`; \
                        the only tag parameter the released update declares). \
                        Demographic ITEM_TAGs are stored against the \
                        VERSIONED_PARTY with no version anchor, so the two tag \
                        sets coincide on this surface and this build takes the \
                        list to store from `openehr-item-tag`; both response \
                        headers then carry that one set.",
         example = "key=\"reviewed\",value=\"true\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — the VERSIONED_PARTY. Not declared on the released \
                        update operation, but it is the header this build reads \
                        as the tag list to store, and demographic tags are \
                        VERSIONED_OBJECT-anchored, so it is the accurate one to \
                        send here. An empty value \"will effectively remove all \
                        ITEM_TAGs associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\"); an absent header leaves the \
                        stored tags untouched.",
         example = "key=\"category\",value=\"final\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The new GROUP.\", `required: true` (ITS-REST \
                                `specifications/operations/group_update.yaml`; \
                                schema `schemas/demographic/Group.yaml`) as \
                                canonical JSON or XML. A `uid` in the body \"must \
                                match the `uid_based_id` in the URL\".",
                 example = json!({
                     "_type": "GROUP",
                     "name": { "_type": "DV_TEXT", "value": "GROUP" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-GROUP.group.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Cardiology on-call rota (2026)" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the GROUP is \
                                      successfully updated, with the full \
                                      resource in the response body when `Prefer` \
                                      header is `return=representation`, or only \
                                      its identifiers when `Prefer` header is \
                                      `return=identifier`.\" (ITS-REST \
                                      `specifications/responses/200_GROUP_updated.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             NEW version (ITS-REST \
                             `specifications/headers/ETag.yaml`; the weakness \
                             indicator is the release's MUST, \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the GROUP resource.\" (ITS-REST \
                             `specifications/headers/Location_GROUP.yaml`), set to \
                             `<base_path>/demographic/group/<new version_uid>`. \
                             §Location scopes the header to \"resource creation … \
                             or redirect responses\" and §\"Prefer minimal, \
                             identifier or full representation response\" names \
                             the target as \"the newly created or updated \
                             resource\" — an openEHR update commits a NEW VERSION, \
                             which is that newly created resource."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"; both headers SHOULD accompany a \
                             versioned resource). The released \
                             `200_GROUP_updated.yaml` does not slot it; the \
                             SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured \
                             (`Requests_and_responses.md` §\"Representation \
                             details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) as the \
                             server stored it; emitted only when the party carries \
                             tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the same set, demographic tags having no version \
                             anchor.")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the updated GROUP",
              value = json!({
                  "_type": "GROUP",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "name": { "_type": "DV_TEXT", "value": "GROUP" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-GROUP.group.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Cardiology on-call rota (2026)" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the `Prefer` \
                                      header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_version_updated.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version (ITS-REST \
                             `specifications/headers/ETag.yaml`)."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the resource version resulted from the operation.\" \
                             (ITS-REST \
                             `specifications/headers/Location_version.yaml`), set \
                             to \
                             `<base_path>/demographic/group/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`), \
                             emitted when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Two \
                                      reachable triggers here: an unparseable \
                                      `uid_based_id`/body, and an ABSENT \
                                      `If-Match` — \"When the service expects \
                                      `If-Match` for an operation, but the client \
                                      does not provide it, the service SHOULD \
                                      respond with `400 Bad Request`\" \
                                      (`Requests_and_responses.md` §\"If-Match and \
                                      accidental overwrites\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). A \
                                      `versioned_object_uid` whose stored \
                                      container is a different PARTY kind is this \
                                      `404` as well — the route is kind-checked \
                                      (a VERSIONED_OBJECT has one type, RM \
                                      `common/master06` §Change Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated and served in the \
                                      canonical formats only, so a \
                                      Simplified-only `Accept` MUST be refused \
                                      (`Resources.md` §\"Simplified Formats\"). \
                                      The released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 412, description = "The released trigger, verbatim: `412 \
                                      Precondition Failed` \"is returned when \
                                      `If-Match` request header doesn't match the \
                                      latest version on the service side. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\" (ITS-REST \
                                      `specifications/responses/412_GROUP.yaml`; \
                                      the same rule is the overview's own MUST — \
                                      \"it MUST NOT perform the requested method. \
                                      Instead, it MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` — the service \"SHOULD return also latest \
                             `version_uid` in the `ETag` response headers\" \
                             (`Requests_and_responses.md` §\"If-Match and \
                             accidental overwrites\"; ITS-REST \
                             `specifications/headers/ETag.yaml`). The released \
                             `412_GROUP.yaml` also slots \
                             `headers/Location_deprecated.yaml`; §Location \
                             forbids `Location` on a non-creation response, so \
                             none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which a party payload cannot \
                                      use (no template can be named for an \
                                      untemplated resource — \
                                      `Requests_and_responses.md` \
                                      §openehr-template-id): \"it MUST respond with \
                                      HTTP status code `415 Unsupported Media \
                                      Type`\" (`Resources.md` §\"Simplified \
                                      Formats\"). An absent `Content-Type` declares \
                                      nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      GROUP, or a body typed as a different PARTY \
                                      subtype than the route's.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_update", parts, super::dispatch::dispatch).await
}

/// Delete a `GROUP` (`DELETE /demographic/group/{uid_based_id}`).
///
/// "Deletes the `GROUP` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/group_delete.yaml`). The delete is LOGICAL: it
/// commits a new deletion `VERSION` rather than removing history — RM
/// `common/master06` §Change Control keeps every committed version, and a
/// subsequent read of the deleted current version answers `204`
/// (`responses/204_deleted_at_time.yaml`).
#[utoipa::path(
    delete, path = "/demographic/group/{uid_based_id}", tag = "GROUP",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of an OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_version_uid.yaml`); \
                        the operation sharpens it: \"The `uid_based_id` MUST be in \
                        a form of an OBJECT_VERSION_ID identifier taken from the \
                        last (most recent) VERSION.uid.value, representing the \
                        `preceding_version_uid` to be deleted.\" A version that is \
                        not the latest is `409`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL here, and the released operation declares no \
                        `If-Match` parameter at all — by the spec's own carve-out: \
                        the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"), and on this operation it IS the \
                        path segment. A header that IS sent is still honoured — \
                        the same section makes a received precondition binding — \
                        as an alternative source of the preceding version; the \
                        weak `W/\"…\"` and bare quoted forms are both accepted.",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list. No released parameter file declares \
                        this header; the requirement is prose — \"services MUST \
                        also allow `PUT`, `POST` and `DELETE` methods directly on \
                        these change-controlled resources\" and \"services MUST \
                        accept `openehr-version` and `openehr-audit-details` \
                        custom request headers\" (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\"\
                        A `lifecycle_state` naming any state other than \
                        `523|deleted|` contradicts the operation and is refused \
                        `400` rather than silently discarded — the section makes \
                        the merge a MUST, and a value that cannot be merged is \
                        reported, not dropped (RM common master06 \
                        §\"Logical Deletion\")."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY repeat. \
                        `time_committed` is always server-set and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "description.value=\"merged into another record\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format (`application/json` by default). A \
                        Simplified-only `Accept` is `406` — a party is untemplated \
                        (`Resources.md` §\"Simplified Formats\").",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned for a successful \
                                      delete operation.\" (ITS-REST \
                                      `specifications/responses/204_version_deleted.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version the operation just committed — not \
                             the version named in the path. The released \
                             `204_version_deleted.yaml` slots \
                             `headers/ETag.yaml`, whose value \"is an identifier \
                             (e.g. a `version_uid` …) for a specific version of a \
                             resource\", and §\"ETag and Last-Modified\" adds that \
                             it \"changes as soon as the resource changes (i.e. \
                             when a new version is created)\" — a logical delete \
                             creates one. That same response slots \
                             `headers/Location_deprecated.yaml`; §\"Deprecated \
                             headers\" deprecates `Location` on `DELETE` \
                             responses, so none is emitted or declared.")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content) or when the resource identified by \
                                      the request parameters is already deleted.\" \
                                      (ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`) \
                                      — so a second delete of the same party is \
                                      this `400`, not a `404`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or one holding a \
                                      different PARTY kind than this route.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied \
                                      (a Simplified-only `Accept` on an \
                                      untemplated resource): \"it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header.\" (ITS-REST \
                                      `specifications/responses/409_GROUP_with_uid_based_id.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` (ITS-REST \
                             `specifications/headers/ETag.yaml`) — the client can \
                             retry the delete against it. The released response \
                             also slots `headers/Location_deprecated.yaml`; \
                             §Location forbids `Location` on a non-creation \
                             response, so none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is still refused before \
                                      the write, because a party has no template a \
                                      Simplified payload could be expanded against \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id; `Resources.md` \
                                      §\"Simplified Formats\" `415` MUST). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_delete", parts, super::dispatch::dispatch).await
}

// ── ORGANISATION ────────────────────────────────────────────────────────────

/// Create an `ORGANISATION` (`POST /demographic/organisation`).
///
/// "Creates the first version of a new `ORGANISATION`." (ITS-REST
/// `specifications/operations/organisation_create.yaml`). The `uid` is server-minted:
/// a PARTY's `uid` is the containing `VERSION`'s `OBJECT_VERSION_ID`, which the
/// client cannot know at create time, so a `uid` in the submitted body does not
/// survive the write and the invariant `Uid_mandatory` (RM
/// `demographic/master02` §Party Identification, `PARTY.Uid_mandatory`) is
/// satisfied post-assignment. The released create declares no `409`, so a
/// client-supplied `uid` is never a conflict.
#[utoipa::path(
    post, path = "/demographic/organisation", tag = "ORGANISATION",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; enum \
                        `return=representation|return=minimal|return=identifier`, \
                        default `return=minimal`). An absent header is \
                        `return=minimal` — \"If no `Prefer` header is provided, \
                        the default behavior is assumed to be `return=minimal`\" \
                        — and `return=identifier` never answers `204`: \"the \
                        status will be `201 Created` or `200 OK`, never `204 No \
                        Content`\" (`Requests_and_responses.md` §\"Prefer only \
                        identifier\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`). \
                        An absent header reads as canonical JSON — `Resources.md` \
                        §\"JSON Format\" makes the header a client MAY, so its \
                        absence declares nothing to refuse. A Simplified \
                        `Content-Type` is `415` (see that response).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", and \
                        \"whatever is provided it MUST be merged with the default \
                        VERSION and VERSION.audit_details attributes on commit \
                        runtime\" (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\", which scopes the rule to \
                        \"all change-controlled resources\" — parties are \
                        version-controlled, RM `common/master06` §Change \
                        Control).",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. \"Through the `openehr-audit-details` header, \
                        clients MAY supply values for the AUDIT_DETAILS \
                        attributes `change_type`, `description`, `committer` and \
                        `system_id`. The `time_committed` attribute is always set \
                        by the server.\" — and \"when `system_id` is not provided \
                        by the client, the server MUST set it to its own \
                        configured system identifier\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "committer.name=\"John Doe\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — here the VERSIONED_PARTY. The tags are stored after the \
                        party exists and the stored set is echoed in the response \
                        header of the same name. \"Providing an empty value for \
                        this header will effectively remove all ITEM_TAGs \
                        associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", Usage in Requests); an absent \
                        header changes nothing.",
         example = "key=\"category\",value=\"final\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`). \
                        The two wrapper headers address DISTINCT collections \
                        (overview §\"openehr-item-tag and \
                        openehr-version-item-tag\"): this one replaces the \
                        just-committed VERSION's own tag set, `openehr-item-tag` \
                        the `VERSIONED_PARTY` container's; each response header \
                        echoes its own stored set.",
         example = "key=\"reviewed\",value=\"true\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The ORGANISATION.\", `required: true` (ITS-REST \
                                `specifications/operations/organisation_create.yaml`; \
                                schema `schemas/demographic/Organisation.yaml`) as \
                                canonical JSON or XML. `PARTY.identities` is \
                                mandatory and non-empty (`Identities_valid`), and \
                                `name` carries the type designation \
                                (`Type_valid: type = name`, RM UML \
                                `org.openehr.rm.demographic.party`).",
                 example = json!({
                     "_type": "ORGANISATION",
                     "name": { "_type": "DV_TEXT", "value": "ORGANISATION" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "St Elsewhere Hospital" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the ORGANISATION is \
                                      successfully created. If `Prefer` header is \
                                      `return=representation`, the full resource \
                                      is included in the response body; if is \
                                      `return=identifier`, only its unique \
                                      identifier is included. If the `Prefer` \
                                      header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_ORGANISATION.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is an \
                             identifier (e.g. a `version_uid` enclosed by double \
                             quotes) for a specific version of a resource.\" \
                             (ITS-REST `specifications/headers/ETag.yaml`), in the \
                             weak form the release requires — \"all `ETag` headers \
                             that hold a resource identifier MUST include a \
                             weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"<versioned_object_uid>::<system_id>::1\"`."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the ORGANISATION resource.\" (ITS-REST \
                             `specifications/headers/Location_ORGANISATION.yaml`), set to \
                             `<base_path>/demographic/organisation/<version_uid>` — \
                             §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date; \"this value should be derived from \
                             VERSION.commit_audit.time_committed.value\", and both \
                             `ETag` and `Last-Modified` \"SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released `201_ORGANISATION.yaml` does \
                             not slot it; the SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured. \"The service MAY include a \
                             `Preference-Applied` header in the response … to \
                             indicate that the client's preference has been \
                             honored\" (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) — the \
                             set as the server stored it; emitted only when the \
                             party carries tags (\"Servers MAY include the \
                             `openehr-item-tag` … header in responses to confirm \
                             the actual list of ITEM_TAGs stored on the server \
                             side\")."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created ORGANISATION",
              value = json!({
                  "_type": "ORGANISATION",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "ORGANISATION" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "St Elsewhere Hospital" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a body that is not well-formed canonical \
                                      JSON/XML. Content that parses but is not a \
                                      valid ORGANISATION is the `422` below.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). On a \
                                      create the reachable trigger is a referenced \
                                      resource the commit resolves and does not \
                                      find.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied. A \
                                      PARTY is untemplated, so this server serves \
                                      it in the canonical formats only and refuses \
                                      a Simplified-only `Accept`: \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\" (`Resources.md` \
                                      §\"Simplified Formats\"; the same MUST is \
                                      stated for XML and JSON). The released \
                                      operation does not enumerate `406`; the MUST \
                                      is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A PARTY is not templated and \
                                      `openehr-template-id` — the only header that \
                                      can name a template — is scoped to \
                                      \"committing COMPOSITION\" \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified party \
                                      payload cannot be expanded: \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      ORGANISATION (empty `identities`, a `name` that is \
                                      not the type designation), or a body whose \
                                      `_type` is a different PARTY subtype than \
                                      the route's — the routed kind's codec is the \
                                      one that decodes it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_create",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve an `ORGANISATION` by uid-based id
/// (`GET /demographic/organisation/{uid_based_id}`).
///
/// "Retrieves a version of the `ORGANISATION` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/organisation_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/organisation/{uid_based_id}", tag = "ORGANISATION",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An abstract \
                        identifier: it can take a form of an OBJECT_VERSION_ID \
                        identifier taken from VERSION.uid.value (i.e. a \
                        `version_uid`), or a form of a HIER_OBJECT_ID identifier \
                        taken from VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id.yaml`). The \
                        operation adds: \"When the `uid_based_id` has the form of \
                        a HIER_OBJECT_ID, if the `version_at_time` is supplied, \
                        retrieves the version extant _at specified time_, \
                        otherwise retrieves the _latest_ ORGANISATION version.\" A \
                        syntactically unusable id is `400`; a well-formed id \
                        naming no ORGANISATION (including a container of another PARTY \
                        kind) is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`). \
                        Selects the version extant at that instant when the path \
                        id is a `versioned_object_uid`; the latest version when \
                        omitted. The timezone is optional — server-local when \
                        absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested ORGANISATION is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_ORGANISATION_retrieved.yaml`). \
                                      That response slots \
                                      `headers/Location_deprecated.yaml`, and \
                                      §Location says the header \"MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource (e.g. via `GET` \
                                      method)\" — so no `Location` is emitted or \
                                      declared here.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (ITS-REST \
                             `specifications/headers/ETag.yaml`; \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\" makes resource-identifier `ETag`s \
                             weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\"; both \
                             headers \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`). \
                             \"When retrieving resources via `GET`, the server MAY \
                             also add these headers to the response\" \
                             (`Requests_and_responses.md` §\"openehr-item-tag and \
                             openehr-version-item-tag\", Usage in Responses); \
                             emitted only when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         example = json!({
             "_type": "ORGANISATION",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "ORGANISATION" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
                 "rm_version": "1.2.0"
             },
             "identities": [
                 {
                     "_type": "PARTY_IDENTITY",
                     "name": { "_type": "DV_TEXT", "value": "legal identity" },
                     "archetype_node_id": "at0001",
                     "details": {
                         "_type": "ITEM_TREE",
                         "name": { "_type": "DV_TEXT", "value": "identity details" },
                         "archetype_node_id": "at0002",
                         "items": [
                             {
                                 "_type": "ELEMENT",
                                 "name": { "_type": "DV_TEXT", "value": "name" },
                                 "archetype_node_id": "at0003",
                                 "value": { "_type": "DV_TEXT", "value": "St Elsewhere Hospital" }
                             }
                         ]
                     }
                 }
             ]
         })),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters (at \
                                      specified `version_at_time`) time has been \
                                      deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`) \
                                      — the version selected by the request is a \
                                      deletion marker, which is a successful read \
                                      of a logically deleted resource, not a \
                                      `404`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `uid_based_id` that is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or a \
                                      `version_at_time` that is not an extended \
                                      ISO 8601 instant. The released get does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`). \
                                      A well-formed id whose stored container is a \
                                      different PARTY kind is this `404` too — the \
                                      route is kind-checked, and a VERSIONED_OBJECT \
                                      has one type (RM `common/master06` §Change \
                                      Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated, so it is served in the \
                                      canonical formats only and a Simplified-only \
                                      `Accept` is refused — \"If the service cannot \
                                      fulfill this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` carries no payload, \
                                      but the released operation still admits a \
                                      request `Content-Type`, and a party has no \
                                      template to expand a Simplified payload \
                                      against, so the declaration is refused \
                                      before the read: \"If the service cannot \
                                      process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
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
pub(crate) async fn organisation_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "organisation_get", parts, super::dispatch::dispatch).await
}

/// Update an `ORGANISATION` (`PUT /demographic/organisation/{uid_based_id}`).
///
/// "Updates `ORGANISATION` identified by `uid_based_id`." … "The existing latest
/// `version_uid` of `ORGANISATION` resource (i.e. the `preceding_version_uid`) must be
/// specified in the `If-Match` header." (ITS-REST
/// `specifications/operations/organisation_update.yaml`).
#[utoipa::path(
    put, path = "/demographic/organisation/{uid_based_id}", tag = "ORGANISATION",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_versioned_object_uid.yaml`) \
                        — the container, not a version. The operation adds: \"If \
                        the request body already contains an ORGANISATION.uid.value, it \
                        must match the `uid_based_id` in the URL.\"",
         example = "8849182c-82ad-4088-a07f-48ead4180515"),
        ("If-Match" = String, Header,
         description = "The released parameter, verbatim: \"Header to make the \
                        request conditional. Together with `ETag` request tag, it \
                        helps to prevent simultaneous updates of a resource from \
                        overwriting each other (\"mid-air collisions\"). The \
                        format is always an `version_uid` identifier enclosed by \
                        double quotes. The operation will be performed only if \
                        the existing latest `version_uid` of the resource (i.e. \
                        the `preceding_version_uid`) matches this header's \
                        value.\" (ITS-REST \
                        `specifications/parameters/header/If-Match.yaml`, \
                        `required: true`). The weak `W/\"…\"` form this server \
                        emits in `ETag` is accepted too — the bare quoted form is \
                        the pre-1.1.0 shape the release keeps supported \
                        (`Requests_and_responses.md` §\"Deprecated headers\").",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`). `return=minimal` answers `204`; the \
                        other two answer `200` — \"the status will be `201 \
                        Created` or `200 OK`, never `204 No Content`\" for \
                        `return=identifier` (`Requests_and_responses.md` \
                        §\"Prefer only identifier\"). The token honoured is echoed \
                        in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`); \
                        absent reads as canonical JSON (`Resources.md` §\"JSON \
                        Format\" makes the header a client MAY). A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", merged \
                        with the server defaults \"on commit runtime\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `change_type`, `description`, `committer` and \
                        `system_id` MAY be supplied; \"The `time_committed` \
                        attribute is always set by the server\", and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\"). No \
                        released parameter file declares it.",
         example = "change_type.code_string=\"251\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`; \
                        the only tag parameter the released update declares). \
                        Demographic ITEM_TAGs are stored against the \
                        VERSIONED_PARTY with no version anchor, so the two tag \
                        sets coincide on this surface and this build takes the \
                        list to store from `openehr-item-tag`; both response \
                        headers then carry that one set.",
         example = "key=\"reviewed\",value=\"true\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — the VERSIONED_PARTY. Not declared on the released \
                        update operation, but it is the header this build reads \
                        as the tag list to store, and demographic tags are \
                        VERSIONED_OBJECT-anchored, so it is the accurate one to \
                        send here. An empty value \"will effectively remove all \
                        ITEM_TAGs associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\"); an absent header leaves the \
                        stored tags untouched.",
         example = "key=\"category\",value=\"final\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The new ORGANISATION.\", `required: true` (ITS-REST \
                                `specifications/operations/organisation_update.yaml`; \
                                schema `schemas/demographic/Organisation.yaml`) as \
                                canonical JSON or XML. A `uid` in the body \"must \
                                match the `uid_based_id` in the URL\".",
                 example = json!({
                     "_type": "ORGANISATION",
                     "name": { "_type": "DV_TEXT", "value": "ORGANISATION" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "St Elsewhere Hospital NHS Trust" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the ORGANISATION is \
                                      successfully updated, with the full \
                                      resource in the response body when `Prefer` \
                                      header is `return=representation`, or only \
                                      its identifiers when `Prefer` header is \
                                      `return=identifier`.\" (ITS-REST \
                                      `specifications/responses/200_ORGANISATION_updated.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             NEW version (ITS-REST \
                             `specifications/headers/ETag.yaml`; the weakness \
                             indicator is the release's MUST, \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the ORGANISATION resource.\" (ITS-REST \
                             `specifications/headers/Location_ORGANISATION.yaml`), set to \
                             `<base_path>/demographic/organisation/<new version_uid>`. \
                             §Location scopes the header to \"resource creation … \
                             or redirect responses\" and §\"Prefer minimal, \
                             identifier or full representation response\" names \
                             the target as \"the newly created or updated \
                             resource\" — an openEHR update commits a NEW VERSION, \
                             which is that newly created resource."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"; both headers SHOULD accompany a \
                             versioned resource). The released \
                             `200_ORGANISATION_updated.yaml` does not slot it; the \
                             SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured \
                             (`Requests_and_responses.md` §\"Representation \
                             details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) as the \
                             server stored it; emitted only when the party carries \
                             tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the same set, demographic tags having no version \
                             anchor.")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the updated ORGANISATION",
              value = json!({
                  "_type": "ORGANISATION",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "name": { "_type": "DV_TEXT", "value": "ORGANISATION" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "St Elsewhere Hospital NHS Trust" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the `Prefer` \
                                      header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_version_updated.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version (ITS-REST \
                             `specifications/headers/ETag.yaml`)."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the resource version resulted from the operation.\" \
                             (ITS-REST \
                             `specifications/headers/Location_version.yaml`), set \
                             to \
                             `<base_path>/demographic/organisation/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`), \
                             emitted when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Two \
                                      reachable triggers here: an unparseable \
                                      `uid_based_id`/body, and an ABSENT \
                                      `If-Match` — \"When the service expects \
                                      `If-Match` for an operation, but the client \
                                      does not provide it, the service SHOULD \
                                      respond with `400 Bad Request`\" \
                                      (`Requests_and_responses.md` §\"If-Match and \
                                      accidental overwrites\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). A \
                                      `versioned_object_uid` whose stored \
                                      container is a different PARTY kind is this \
                                      `404` as well — the route is kind-checked \
                                      (a VERSIONED_OBJECT has one type, RM \
                                      `common/master06` §Change Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated and served in the \
                                      canonical formats only, so a \
                                      Simplified-only `Accept` MUST be refused \
                                      (`Resources.md` §\"Simplified Formats\"). \
                                      The released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 412, description = "The released trigger, verbatim: `412 \
                                      Precondition Failed` \"is returned when \
                                      `If-Match` request header doesn't match the \
                                      latest version on the service side. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\" (ITS-REST \
                                      `specifications/responses/412_ORGANISATION.yaml`; \
                                      the same rule is the overview's own MUST — \
                                      \"it MUST NOT perform the requested method. \
                                      Instead, it MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` — the service \"SHOULD return also latest \
                             `version_uid` in the `ETag` response headers\" \
                             (`Requests_and_responses.md` §\"If-Match and \
                             accidental overwrites\"; ITS-REST \
                             `specifications/headers/ETag.yaml`). The released \
                             `412_ORGANISATION.yaml` also slots \
                             `headers/Location_deprecated.yaml`; §Location \
                             forbids `Location` on a non-creation response, so \
                             none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which a party payload cannot \
                                      use (no template can be named for an \
                                      untemplated resource — \
                                      `Requests_and_responses.md` \
                                      §openehr-template-id): \"it MUST respond with \
                                      HTTP status code `415 Unsupported Media \
                                      Type`\" (`Resources.md` §\"Simplified \
                                      Formats\"). An absent `Content-Type` declares \
                                      nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      ORGANISATION, or a body typed as a different PARTY \
                                      subtype than the route's.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_update",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Delete an `ORGANISATION` (`DELETE /demographic/organisation/{uid_based_id}`).
///
/// "Deletes the `ORGANISATION` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/organisation_delete.yaml`). The delete is LOGICAL: it
/// commits a new deletion `VERSION` rather than removing history — RM
/// `common/master06` §Change Control keeps every committed version, and a
/// subsequent read of the deleted current version answers `204`
/// (`responses/204_deleted_at_time.yaml`).
#[utoipa::path(
    delete, path = "/demographic/organisation/{uid_based_id}", tag = "ORGANISATION",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of an OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_version_uid.yaml`); \
                        the operation sharpens it: \"The `uid_based_id` MUST be in \
                        a form of an OBJECT_VERSION_ID identifier taken from the \
                        last (most recent) VERSION.uid.value, representing the \
                        `preceding_version_uid` to be deleted.\" A version that is \
                        not the latest is `409`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL here, and the released operation declares no \
                        `If-Match` parameter at all — by the spec's own carve-out: \
                        the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"), and on this operation it IS the \
                        path segment. A header that IS sent is still honoured — \
                        the same section makes a received precondition binding — \
                        as an alternative source of the preceding version; the \
                        weak `W/\"…\"` and bare quoted forms are both accepted.",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list. No released parameter file declares \
                        this header; the requirement is prose — \"services MUST \
                        also allow `PUT`, `POST` and `DELETE` methods directly on \
                        these change-controlled resources\" and \"services MUST \
                        accept `openehr-version` and `openehr-audit-details` \
                        custom request headers\" (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\"\
                        A `lifecycle_state` naming any state other than \
                        `523|deleted|` contradicts the operation and is refused \
                        `400` rather than silently discarded — the section makes \
                        the merge a MUST, and a value that cannot be merged is \
                        reported, not dropped (RM common master06 \
                        §\"Logical Deletion\")."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY repeat. \
                        `time_committed` is always server-set and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "description.value=\"merged into another record\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format (`application/json` by default). A \
                        Simplified-only `Accept` is `406` — a party is untemplated \
                        (`Resources.md` §\"Simplified Formats\").",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned for a successful \
                                      delete operation.\" (ITS-REST \
                                      `specifications/responses/204_version_deleted.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version the operation just committed — not \
                             the version named in the path. The released \
                             `204_version_deleted.yaml` slots \
                             `headers/ETag.yaml`, whose value \"is an identifier \
                             (e.g. a `version_uid` …) for a specific version of a \
                             resource\", and §\"ETag and Last-Modified\" adds that \
                             it \"changes as soon as the resource changes (i.e. \
                             when a new version is created)\" — a logical delete \
                             creates one. That same response slots \
                             `headers/Location_deprecated.yaml`; §\"Deprecated \
                             headers\" deprecates `Location` on `DELETE` \
                             responses, so none is emitted or declared.")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content) or when the resource identified by \
                                      the request parameters is already deleted.\" \
                                      (ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`) \
                                      — so a second delete of the same party is \
                                      this `400`, not a `404`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or one holding a \
                                      different PARTY kind than this route.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied \
                                      (a Simplified-only `Accept` on an \
                                      untemplated resource): \"it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header.\" (ITS-REST \
                                      `specifications/responses/409_ORGANISATION_with_uid_based_id.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` (ITS-REST \
                             `specifications/headers/ETag.yaml`) — the client can \
                             retry the delete against it. The released response \
                             also slots `headers/Location_deprecated.yaml`; \
                             §Location forbids `Location` on a non-creation \
                             response, so none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is still refused before \
                                      the write, because a party has no template a \
                                      Simplified payload could be expanded against \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id; `Resources.md` \
                                      §\"Simplified Formats\" `415` MUST). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

// ── PERSON ────────────────────────────────────────────────────────────────

/// Create a `PERSON` (`POST /demographic/person`).
///
/// "Creates the first version of a new `PERSON`." (ITS-REST
/// `specifications/operations/person_create.yaml`). The `uid` is server-minted:
/// a PARTY's `uid` is the containing `VERSION`'s `OBJECT_VERSION_ID`, which the
/// client cannot know at create time, so a `uid` in the submitted body does not
/// survive the write and the invariant `Uid_mandatory` (RM
/// `demographic/master02` §Party Identification, `PARTY.Uid_mandatory`) is
/// satisfied post-assignment. The released create declares no `409`, so a
/// client-supplied `uid` is never a conflict.
#[utoipa::path(
    post, path = "/demographic/person", tag = "PERSON",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; enum \
                        `return=representation|return=minimal|return=identifier`, \
                        default `return=minimal`). An absent header is \
                        `return=minimal` — \"If no `Prefer` header is provided, \
                        the default behavior is assumed to be `return=minimal`\" \
                        — and `return=identifier` never answers `204`: \"the \
                        status will be `201 Created` or `200 OK`, never `204 No \
                        Content`\" (`Requests_and_responses.md` §\"Prefer only \
                        identifier\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`). \
                        An absent header reads as canonical JSON — `Resources.md` \
                        §\"JSON Format\" makes the header a client MAY, so its \
                        absence declares nothing to refuse. A Simplified \
                        `Content-Type` is `415` (see that response).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", and \
                        \"whatever is provided it MUST be merged with the default \
                        VERSION and VERSION.audit_details attributes on commit \
                        runtime\" (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\", which scopes the rule to \
                        \"all change-controlled resources\" — parties are \
                        version-controlled, RM `common/master06` §Change \
                        Control).",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. \"Through the `openehr-audit-details` header, \
                        clients MAY supply values for the AUDIT_DETAILS \
                        attributes `change_type`, `description`, `committer` and \
                        `system_id`. The `time_committed` attribute is always set \
                        by the server.\" — and \"when `system_id` is not provided \
                        by the client, the server MUST set it to its own \
                        configured system identifier\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "committer.name=\"John Doe\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — here the VERSIONED_PARTY. The tags are stored after the \
                        party exists and the stored set is echoed in the response \
                        header of the same name. \"Providing an empty value for \
                        this header will effectively remove all ITEM_TAGs \
                        associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", Usage in Requests); an absent \
                        header changes nothing.",
         example = "key=\"category\",value=\"final\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`). \
                        The two wrapper headers address DISTINCT collections \
                        (overview §\"openehr-item-tag and \
                        openehr-version-item-tag\"): this one replaces the \
                        just-committed VERSION's own tag set, `openehr-item-tag` \
                        the `VERSIONED_PARTY` container's; each response header \
                        echoes its own stored set.",
         example = "key=\"reviewed\",value=\"true\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The PERSON.\", `required: true` (ITS-REST \
                                `specifications/operations/person_create.yaml`; \
                                schema `schemas/demographic/Person.yaml`) as \
                                canonical JSON or XML. `PARTY.identities` is \
                                mandatory and non-empty (`Identities_valid`), and \
                                `name` carries the type designation \
                                (`Type_valid: type = name`, RM UML \
                                `org.openehr.rm.demographic.party`).",
                 example = json!({
                     "_type": "PERSON",
                     "name": { "_type": "DV_TEXT", "value": "PERSON" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Jane Doe" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the PERSON is \
                                      successfully created. If `Prefer` header is \
                                      `return=representation`, the full resource \
                                      is included in the response body; if is \
                                      `return=identifier`, only its unique \
                                      identifier is included. If the `Prefer` \
                                      header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_PERSON.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is an \
                             identifier (e.g. a `version_uid` enclosed by double \
                             quotes) for a specific version of a resource.\" \
                             (ITS-REST `specifications/headers/ETag.yaml`), in the \
                             weak form the release requires — \"all `ETag` headers \
                             that hold a resource identifier MUST include a \
                             weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"<versioned_object_uid>::<system_id>::1\"`."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the PERSON resource.\" (ITS-REST \
                             `specifications/headers/Location_PERSON.yaml`), set to \
                             `<base_path>/demographic/person/<version_uid>` — \
                             §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date; \"this value should be derived from \
                             VERSION.commit_audit.time_committed.value\", and both \
                             `ETag` and `Last-Modified` \"SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released `201_PERSON.yaml` does \
                             not slot it; the SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured. \"The service MAY include a \
                             `Preference-Applied` header in the response … to \
                             indicate that the client's preference has been \
                             honored\" (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) — the \
                             set as the server stored it; emitted only when the \
                             party carries tags (\"Servers MAY include the \
                             `openehr-item-tag` … header in responses to confirm \
                             the actual list of ITEM_TAGs stored on the server \
                             side\")."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created PERSON",
              value = json!({
                  "_type": "PERSON",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "PERSON" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Jane Doe" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a body that is not well-formed canonical \
                                      JSON/XML. Content that parses but is not a \
                                      valid PERSON is the `422` below.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). On a \
                                      create the reachable trigger is a referenced \
                                      resource the commit resolves and does not \
                                      find.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied. A \
                                      PARTY is untemplated, so this server serves \
                                      it in the canonical formats only and refuses \
                                      a Simplified-only `Accept`: \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\" (`Resources.md` \
                                      §\"Simplified Formats\"; the same MUST is \
                                      stated for XML and JSON). The released \
                                      operation does not enumerate `406`; the MUST \
                                      is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A PARTY is not templated and \
                                      `openehr-template-id` — the only header that \
                                      can name a template — is scoped to \
                                      \"committing COMPOSITION\" \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified party \
                                      payload cannot be expanded: \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      PERSON (empty `identities`, a `name` that is \
                                      not the type designation), or a body whose \
                                      `_type` is a different PARTY subtype than \
                                      the route's — the routed kind's codec is the \
                                      one that decodes it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "person_create", parts, super::dispatch::dispatch).await
}

/// Retrieve a `PERSON` by uid-based id
/// (`GET /demographic/person/{uid_based_id}`).
///
/// "Retrieves a version of the `PERSON` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/person_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/person/{uid_based_id}", tag = "PERSON",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An abstract \
                        identifier: it can take a form of an OBJECT_VERSION_ID \
                        identifier taken from VERSION.uid.value (i.e. a \
                        `version_uid`), or a form of a HIER_OBJECT_ID identifier \
                        taken from VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id.yaml`). The \
                        operation adds: \"When the `uid_based_id` has the form of \
                        a HIER_OBJECT_ID, if the `version_at_time` is supplied, \
                        retrieves the version extant _at specified time_, \
                        otherwise retrieves the _latest_ PERSON version.\" A \
                        syntactically unusable id is `400`; a well-formed id \
                        naming no PERSON (including a container of another PARTY \
                        kind) is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`). \
                        Selects the version extant at that instant when the path \
                        id is a `versioned_object_uid`; the latest version when \
                        omitted. The timezone is optional — server-local when \
                        absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested PERSON is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_PERSON_retrieved.yaml`). \
                                      That response slots \
                                      `headers/Location_deprecated.yaml`, and \
                                      §Location says the header \"MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource (e.g. via `GET` \
                                      method)\" — so no `Location` is emitted or \
                                      declared here.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (ITS-REST \
                             `specifications/headers/ETag.yaml`; \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\" makes resource-identifier `ETag`s \
                             weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\"; both \
                             headers \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`). \
                             \"When retrieving resources via `GET`, the server MAY \
                             also add these headers to the response\" \
                             (`Requests_and_responses.md` §\"openehr-item-tag and \
                             openehr-version-item-tag\", Usage in Responses); \
                             emitted only when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         example = json!({
             "_type": "PERSON",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "PERSON" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                 "rm_version": "1.2.0"
             },
             "identities": [
                 {
                     "_type": "PARTY_IDENTITY",
                     "name": { "_type": "DV_TEXT", "value": "legal identity" },
                     "archetype_node_id": "at0001",
                     "details": {
                         "_type": "ITEM_TREE",
                         "name": { "_type": "DV_TEXT", "value": "identity details" },
                         "archetype_node_id": "at0002",
                         "items": [
                             {
                                 "_type": "ELEMENT",
                                 "name": { "_type": "DV_TEXT", "value": "name" },
                                 "archetype_node_id": "at0003",
                                 "value": { "_type": "DV_TEXT", "value": "Jane Doe" }
                             }
                         ]
                     }
                 }
             ]
         })),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters (at \
                                      specified `version_at_time`) time has been \
                                      deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`) \
                                      — the version selected by the request is a \
                                      deletion marker, which is a successful read \
                                      of a logically deleted resource, not a \
                                      `404`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `uid_based_id` that is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or a \
                                      `version_at_time` that is not an extended \
                                      ISO 8601 instant. The released get does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`). \
                                      A well-formed id whose stored container is a \
                                      different PARTY kind is this `404` too — the \
                                      route is kind-checked, and a VERSIONED_OBJECT \
                                      has one type (RM `common/master06` §Change \
                                      Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated, so it is served in the \
                                      canonical formats only and a Simplified-only \
                                      `Accept` is refused — \"If the service cannot \
                                      fulfill this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` carries no payload, \
                                      but the released operation still admits a \
                                      request `Content-Type`, and a party has no \
                                      template to expand a Simplified payload \
                                      against, so the declaration is refused \
                                      before the read: \"If the service cannot \
                                      process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
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
pub(crate) async fn person_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "person_get", parts, super::dispatch::dispatch).await
}

/// Update a `PERSON` (`PUT /demographic/person/{uid_based_id}`).
///
/// "Updates `PERSON` identified by `uid_based_id`." … "The existing latest
/// `version_uid` of `PERSON` resource (i.e. the `preceding_version_uid`) must be
/// specified in the `If-Match` header." (ITS-REST
/// `specifications/operations/person_update.yaml`).
#[utoipa::path(
    put, path = "/demographic/person/{uid_based_id}", tag = "PERSON",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_versioned_object_uid.yaml`) \
                        — the container, not a version. The operation adds: \"If \
                        the request body already contains a PERSON.uid.value, it \
                        must match the `uid_based_id` in the URL.\"",
         example = "8849182c-82ad-4088-a07f-48ead4180515"),
        ("If-Match" = String, Header,
         description = "The released parameter, verbatim: \"Header to make the \
                        request conditional. Together with `ETag` request tag, it \
                        helps to prevent simultaneous updates of a resource from \
                        overwriting each other (\"mid-air collisions\"). The \
                        format is always an `version_uid` identifier enclosed by \
                        double quotes. The operation will be performed only if \
                        the existing latest `version_uid` of the resource (i.e. \
                        the `preceding_version_uid`) matches this header's \
                        value.\" (ITS-REST \
                        `specifications/parameters/header/If-Match.yaml`, \
                        `required: true`). The weak `W/\"…\"` form this server \
                        emits in `ETag` is accepted too — the bare quoted form is \
                        the pre-1.1.0 shape the release keeps supported \
                        (`Requests_and_responses.md` §\"Deprecated headers\").",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`). `return=minimal` answers `204`; the \
                        other two answer `200` — \"the status will be `201 \
                        Created` or `200 OK`, never `204 No Content`\" for \
                        `return=identifier` (`Requests_and_responses.md` \
                        §\"Prefer only identifier\"). The token honoured is echoed \
                        in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`); \
                        absent reads as canonical JSON (`Resources.md` §\"JSON \
                        Format\" makes the header a client MAY). A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", merged \
                        with the server defaults \"on commit runtime\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `change_type`, `description`, `committer` and \
                        `system_id` MAY be supplied; \"The `time_committed` \
                        attribute is always set by the server\", and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\"). No \
                        released parameter file declares it.",
         example = "change_type.code_string=\"251\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`; \
                        the only tag parameter the released update declares). \
                        Demographic ITEM_TAGs are stored against the \
                        VERSIONED_PARTY with no version anchor, so the two tag \
                        sets coincide on this surface and this build takes the \
                        list to store from `openehr-item-tag`; both response \
                        headers then carry that one set.",
         example = "key=\"reviewed\",value=\"true\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — the VERSIONED_PARTY. Not declared on the released \
                        update operation, but it is the header this build reads \
                        as the tag list to store, and demographic tags are \
                        VERSIONED_OBJECT-anchored, so it is the accurate one to \
                        send here. An empty value \"will effectively remove all \
                        ITEM_TAGs associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\"); an absent header leaves the \
                        stored tags untouched.",
         example = "key=\"category\",value=\"final\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The new PERSON.\", `required: true` (ITS-REST \
                                `specifications/operations/person_update.yaml`; \
                                schema `schemas/demographic/Person.yaml`) as \
                                canonical JSON or XML. A `uid` in the body \"must \
                                match the `uid_based_id` in the URL\".",
                 example = json!({
                     "_type": "PERSON",
                     "name": { "_type": "DV_TEXT", "value": "PERSON" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Jane Doe (married name)" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the PERSON is \
                                      successfully updated, with the full \
                                      resource in the response body when `Prefer` \
                                      header is `return=representation`, or only \
                                      its identifiers when `Prefer` header is \
                                      `return=identifier`.\" (ITS-REST \
                                      `specifications/responses/200_PERSON_updated.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             NEW version (ITS-REST \
                             `specifications/headers/ETag.yaml`; the weakness \
                             indicator is the release's MUST, \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the PERSON resource.\" (ITS-REST \
                             `specifications/headers/Location_PERSON.yaml`), set to \
                             `<base_path>/demographic/person/<new version_uid>`. \
                             §Location scopes the header to \"resource creation … \
                             or redirect responses\" and §\"Prefer minimal, \
                             identifier or full representation response\" names \
                             the target as \"the newly created or updated \
                             resource\" — an openEHR update commits a NEW VERSION, \
                             which is that newly created resource."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"; both headers SHOULD accompany a \
                             versioned resource). The released \
                             `200_PERSON_updated.yaml` does not slot it; the \
                             SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured \
                             (`Requests_and_responses.md` §\"Representation \
                             details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) as the \
                             server stored it; emitted only when the party carries \
                             tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the same set, demographic tags having no version \
                             anchor.")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the updated PERSON",
              value = json!({
                  "_type": "PERSON",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "name": { "_type": "DV_TEXT", "value": "PERSON" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Jane Doe (married name)" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the `Prefer` \
                                      header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_version_updated.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version (ITS-REST \
                             `specifications/headers/ETag.yaml`)."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the resource version resulted from the operation.\" \
                             (ITS-REST \
                             `specifications/headers/Location_version.yaml`), set \
                             to \
                             `<base_path>/demographic/person/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`), \
                             emitted when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Two \
                                      reachable triggers here: an unparseable \
                                      `uid_based_id`/body, and an ABSENT \
                                      `If-Match` — \"When the service expects \
                                      `If-Match` for an operation, but the client \
                                      does not provide it, the service SHOULD \
                                      respond with `400 Bad Request`\" \
                                      (`Requests_and_responses.md` §\"If-Match and \
                                      accidental overwrites\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). A \
                                      `versioned_object_uid` whose stored \
                                      container is a different PARTY kind is this \
                                      `404` as well — the route is kind-checked \
                                      (a VERSIONED_OBJECT has one type, RM \
                                      `common/master06` §Change Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated and served in the \
                                      canonical formats only, so a \
                                      Simplified-only `Accept` MUST be refused \
                                      (`Resources.md` §\"Simplified Formats\"). \
                                      The released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 412, description = "The released trigger, verbatim: `412 \
                                      Precondition Failed` \"is returned when \
                                      `If-Match` request header doesn't match the \
                                      latest version on the service side. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\" (ITS-REST \
                                      `specifications/responses/412_PERSON.yaml`; \
                                      the same rule is the overview's own MUST — \
                                      \"it MUST NOT perform the requested method. \
                                      Instead, it MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` — the service \"SHOULD return also latest \
                             `version_uid` in the `ETag` response headers\" \
                             (`Requests_and_responses.md` §\"If-Match and \
                             accidental overwrites\"; ITS-REST \
                             `specifications/headers/ETag.yaml`). The released \
                             `412_PERSON.yaml` also slots \
                             `headers/Location_deprecated.yaml`; §Location \
                             forbids `Location` on a non-creation response, so \
                             none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which a party payload cannot \
                                      use (no template can be named for an \
                                      untemplated resource — \
                                      `Requests_and_responses.md` \
                                      §openehr-template-id): \"it MUST respond with \
                                      HTTP status code `415 Unsupported Media \
                                      Type`\" (`Resources.md` §\"Simplified \
                                      Formats\"). An absent `Content-Type` declares \
                                      nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      PERSON, or a body typed as a different PARTY \
                                      subtype than the route's.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "person_update", parts, super::dispatch::dispatch).await
}

/// Delete a `PERSON` (`DELETE /demographic/person/{uid_based_id}`).
///
/// "Deletes the `PERSON` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/person_delete.yaml`). The delete is LOGICAL: it
/// commits a new deletion `VERSION` rather than removing history — RM
/// `common/master06` §Change Control keeps every committed version, and a
/// subsequent read of the deleted current version answers `204`
/// (`responses/204_deleted_at_time.yaml`).
#[utoipa::path(
    delete, path = "/demographic/person/{uid_based_id}", tag = "PERSON",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of an OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_version_uid.yaml`); \
                        the operation sharpens it: \"The `uid_based_id` MUST be in \
                        a form of an OBJECT_VERSION_ID identifier taken from the \
                        last (most recent) VERSION.uid.value, representing the \
                        `preceding_version_uid` to be deleted.\" A version that is \
                        not the latest is `409`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL here, and the released operation declares no \
                        `If-Match` parameter at all — by the spec's own carve-out: \
                        the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"), and on this operation it IS the \
                        path segment. A header that IS sent is still honoured — \
                        the same section makes a received precondition binding — \
                        as an alternative source of the preceding version; the \
                        weak `W/\"…\"` and bare quoted forms are both accepted.",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list. No released parameter file declares \
                        this header; the requirement is prose — \"services MUST \
                        also allow `PUT`, `POST` and `DELETE` methods directly on \
                        these change-controlled resources\" and \"services MUST \
                        accept `openehr-version` and `openehr-audit-details` \
                        custom request headers\" (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\"\
                        A `lifecycle_state` naming any state other than \
                        `523|deleted|` contradicts the operation and is refused \
                        `400` rather than silently discarded — the section makes \
                        the merge a MUST, and a value that cannot be merged is \
                        reported, not dropped (RM common master06 \
                        §\"Logical Deletion\")."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY repeat. \
                        `time_committed` is always server-set and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "description.value=\"merged into another record\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format (`application/json` by default). A \
                        Simplified-only `Accept` is `406` — a party is untemplated \
                        (`Resources.md` §\"Simplified Formats\").",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned for a successful \
                                      delete operation.\" (ITS-REST \
                                      `specifications/responses/204_version_deleted.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version the operation just committed — not \
                             the version named in the path. The released \
                             `204_version_deleted.yaml` slots \
                             `headers/ETag.yaml`, whose value \"is an identifier \
                             (e.g. a `version_uid` …) for a specific version of a \
                             resource\", and §\"ETag and Last-Modified\" adds that \
                             it \"changes as soon as the resource changes (i.e. \
                             when a new version is created)\" — a logical delete \
                             creates one. That same response slots \
                             `headers/Location_deprecated.yaml`; §\"Deprecated \
                             headers\" deprecates `Location` on `DELETE` \
                             responses, so none is emitted or declared.")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content) or when the resource identified by \
                                      the request parameters is already deleted.\" \
                                      (ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`) \
                                      — so a second delete of the same party is \
                                      this `400`, not a `404`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or one holding a \
                                      different PARTY kind than this route.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied \
                                      (a Simplified-only `Accept` on an \
                                      untemplated resource): \"it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header.\" (ITS-REST \
                                      `specifications/responses/409_PERSON_with_uid_based_id.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` (ITS-REST \
                             `specifications/headers/ETag.yaml`) — the client can \
                             retry the delete against it. The released response \
                             also slots `headers/Location_deprecated.yaml`; \
                             §Location forbids `Location` on a non-creation \
                             response, so none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is still refused before \
                                      the write, because a party has no template a \
                                      Simplified payload could be expanded against \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id; `Resources.md` \
                                      §\"Simplified Formats\" `415` MUST). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "person_delete", parts, super::dispatch::dispatch).await
}

// ── ROLE ────────────────────────────────────────────────────────────────────

/// Create a `ROLE` (`POST /demographic/role`).
///
/// "Creates the first version of a new `ROLE`." (ITS-REST
/// `specifications/operations/role_create.yaml`). The `uid` is server-minted:
/// a PARTY's `uid` is the containing `VERSION`'s `OBJECT_VERSION_ID`, which the
/// client cannot know at create time, so a `uid` in the submitted body does not
/// survive the write and the invariant `Uid_mandatory` (RM
/// `demographic/master02` §Party Identification, `PARTY.Uid_mandatory`) is
/// satisfied post-assignment. The released create declares no `409`, so a
/// client-supplied `uid` is never a conflict.
#[utoipa::path(
    post, path = "/demographic/role", tag = "ROLE",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; enum \
                        `return=representation|return=minimal|return=identifier`, \
                        default `return=minimal`). An absent header is \
                        `return=minimal` — \"If no `Prefer` header is provided, \
                        the default behavior is assumed to be `return=minimal`\" \
                        — and `return=identifier` never answers `204`: \"the \
                        status will be `201 Created` or `200 OK`, never `204 No \
                        Content`\" (`Requests_and_responses.md` §\"Prefer only \
                        identifier\"). The token honoured is echoed in \
                        `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`). \
                        An absent header reads as canonical JSON — `Resources.md` \
                        §\"JSON Format\" makes the header a client MAY, so its \
                        absence declares nothing to refuse. A Simplified \
                        `Content-Type` is `415` (see that response).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this create commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", and \
                        \"whatever is provided it MUST be merged with the default \
                        VERSION and VERSION.audit_details attributes on commit \
                        runtime\" (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\", which scopes the rule to \
                        \"all change-controlled resources\" — parties are \
                        version-controlled, RM `common/master06` §Change \
                        Control).",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this create \
                        commits, as an attribute-path list; the header MAY \
                        repeat. \"Through the `openehr-audit-details` header, \
                        clients MAY supply values for the AUDIT_DETAILS \
                        attributes `change_type`, `description`, `committer` and \
                        `system_id`. The `time_committed` attribute is always set \
                        by the server.\" — and \"when `system_id` is not provided \
                        by the client, the server MUST set it to its own \
                        configured system identifier\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "committer.name=\"John Doe\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — here the VERSIONED_PARTY. The tags are stored after the \
                        party exists and the stored set is echoed in the response \
                        header of the same name. \"Providing an empty value for \
                        this header will effectively remove all ITEM_TAGs \
                        associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\", Usage in Requests); an absent \
                        header changes nothing.",
         example = "key=\"category\",value=\"final\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`). \
                        The two wrapper headers address DISTINCT collections \
                        (overview §\"openehr-item-tag and \
                        openehr-version-item-tag\"): this one replaces the \
                        just-committed VERSION's own tag set, `openehr-item-tag` \
                        the `VERSIONED_PARTY` container's; each response header \
                        echoes its own stored set.",
         example = "key=\"reviewed\",value=\"true\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The ROLE.\", `required: true` (ITS-REST \
                                `specifications/operations/role_create.yaml`; \
                                schema `schemas/demographic/Role.yaml`) as \
                                canonical JSON or XML. `PARTY.identities` is \
                                mandatory and non-empty (`Identities_valid`), and \
                                `name` carries the type designation \
                                (`Type_valid: type = name`, RM UML \
                                `org.openehr.rm.demographic.party`).",
                 example = json!({
                     "_type": "ROLE",
                     "name": { "_type": "DV_TEXT", "value": "ROLE" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "General practitioner" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the ROLE is \
                                      successfully created. If `Prefer` header is \
                                      `return=representation`, the full resource \
                                      is included in the response body; if is \
                                      `return=identifier`, only its unique \
                                      identifier is included. If the `Prefer` \
                                      header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_ROLE.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is an \
                             identifier (e.g. a `version_uid` enclosed by double \
                             quotes) for a specific version of a resource.\" \
                             (ITS-REST `specifications/headers/ETag.yaml`), in the \
                             weak form the release requires — \"all `ETag` headers \
                             that hold a resource identifier MUST include a \
                             weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"<versioned_object_uid>::<system_id>::1\"`."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the ROLE resource.\" (ITS-REST \
                             `specifications/headers/Location_ROLE.yaml`), set to \
                             `<base_path>/demographic/role/<version_uid>` — \
                             §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The creating VERSION's commit instant as an \
                             HTTP-date; \"this value should be derived from \
                             VERSION.commit_audit.time_committed.value\", and both \
                             `ETag` and `Last-Modified` \"SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released `201_ROLE.yaml` does \
                             not slot it; the SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured. \"The service MAY include a \
                             `Preference-Applied` header in the response … to \
                             indicate that the client's preference has been \
                             honored\" (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) — the \
                             set as the server stored it; emitted only when the \
                             party carries tags (\"Servers MAY include the \
                             `openehr-item-tag` … header in responses to confirm \
                             the actual list of ITEM_TAGs stored on the server \
                             side\")."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the created ROLE",
              value = json!({
                  "_type": "ROLE",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
                  "name": { "_type": "DV_TEXT", "value": "ROLE" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "General practitioner" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a body that is not well-formed canonical \
                                      JSON/XML. Content that parses but is not a \
                                      valid ROLE is the `422` below.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). On a \
                                      create the reachable trigger is a referenced \
                                      resource the commit resolves and does not \
                                      find.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied. A \
                                      PARTY is untemplated, so this server serves \
                                      it in the canonical formats only and refuses \
                                      a Simplified-only `Accept`: \"If the service \
                                      cannot fulfill this aspect of the request, \
                                      it MUST respond with HTTP status code `406 \
                                      Not Acceptable`\" (`Resources.md` \
                                      §\"Simplified Formats\"; the same MUST is \
                                      stated for XML and JSON). The released \
                                      operation does not enumerate `406`; the MUST \
                                      is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A PARTY is not templated and \
                                      `openehr-template-id` — the only header that \
                                      can name a template — is scoped to \
                                      \"committing COMPOSITION\" \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified party \
                                      payload cannot be expanded: \"If the service \
                                      cannot process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      ROLE (empty `identities`, a `name` that is \
                                      not the type designation), or a body whose \
                                      `_type` is a different PARTY subtype than \
                                      the route's — the routed kind's codec is the \
                                      one that decodes it.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_create", parts, super::dispatch::dispatch).await
}

/// Retrieve a `ROLE` by uid-based id
/// (`GET /demographic/role/{uid_based_id}`).
///
/// "Retrieves a version of the `ROLE` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/role_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/role/{uid_based_id}", tag = "ROLE",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An abstract \
                        identifier: it can take a form of an OBJECT_VERSION_ID \
                        identifier taken from VERSION.uid.value (i.e. a \
                        `version_uid`), or a form of a HIER_OBJECT_ID identifier \
                        taken from VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id.yaml`). The \
                        operation adds: \"When the `uid_based_id` has the form of \
                        a HIER_OBJECT_ID, if the `version_at_time` is supplied, \
                        retrieves the version extant _at specified time_, \
                        otherwise retrieves the _latest_ ROLE version.\" A \
                        syntactically unusable id is `400`; a well-formed id \
                        naming no ROLE (including a container of another PARTY \
                        kind) is `404`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`). \
                        Selects the version extant at that instant when the path \
                        id is a `versioned_object_uid`; the latest version when \
                        omitted. The timezone is optional — server-local when \
                        absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406` (see that response).",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested ROLE is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_ROLE_retrieved.yaml`). \
                                      That response slots \
                                      `headers/Location_deprecated.yaml`, and \
                                      §Location says the header \"MUST NOT be used \
                                      to indicate an alternate representation of \
                                      an existing resource (e.g. via `GET` \
                                      method)\" — so no `Location` is emitted or \
                                      declared here.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served version (ITS-REST \
                             `specifications/headers/ETag.yaml`; \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\" makes resource-identifier `ETag`s \
                             weak-type)."),
             ("Last-Modified" = String,
              description = "The served version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\"; both \
                             headers \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`). \
                             \"When retrieving resources via `GET`, the server MAY \
                             also add these headers to the response\" \
                             (`Requests_and_responses.md` §\"openehr-item-tag and \
                             openehr-version-item-tag\", Usage in Responses); \
                             emitted only when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the served VERSION's own collection, distinct from \
                             the container set `openehr-item-tag` carries \
                             (overview §\"openehr-item-tag and \
                             openehr-version-item-tag\").")
         ),
         example = json!({
             "_type": "ROLE",
             "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" },
             "name": { "_type": "DV_TEXT", "value": "ROLE" },
             "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
             "archetype_details": {
                 "_type": "ARCHETYPED",
                 "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                 "rm_version": "1.2.0"
             },
             "identities": [
                 {
                     "_type": "PARTY_IDENTITY",
                     "name": { "_type": "DV_TEXT", "value": "legal identity" },
                     "archetype_node_id": "at0001",
                     "details": {
                         "_type": "ITEM_TREE",
                         "name": { "_type": "DV_TEXT", "value": "identity details" },
                         "archetype_node_id": "at0002",
                         "items": [
                             {
                                 "_type": "ELEMENT",
                                 "name": { "_type": "DV_TEXT", "value": "name" },
                                 "archetype_node_id": "at0003",
                                 "value": { "_type": "DV_TEXT", "value": "General practitioner" }
                             }
                         ]
                     }
                 }
             ]
         })),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters (at \
                                      specified `version_at_time`) time has been \
                                      deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted_at_time.yaml`) \
                                      — the version selected by the request is a \
                                      deletion marker, which is a successful read \
                                      of a logically deleted resource, not a \
                                      `404`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `uid_based_id` that is neither an \
                                      OBJECT_VERSION_ID nor a HIER_OBJECT_ID, or a \
                                      `version_at_time` that is not an extended \
                                      ISO 8601 instant. The released get does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`). \
                                      A well-formed id whose stored container is a \
                                      different PARTY kind is this `404` too — the \
                                      route is kind-checked, and a VERSIONED_OBJECT \
                                      has one type (RM `common/master06` §Change \
                                      Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated, so it is served in the \
                                      canonical formats only and a Simplified-only \
                                      `Accept` is refused — \"If the service cannot \
                                      fulfill this aspect of the request, it MUST \
                                      respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `GET` carries no payload, \
                                      but the released operation still admits a \
                                      request `Content-Type`, and a party has no \
                                      template to expand a Simplified payload \
                                      against, so the declaration is refused \
                                      before the read: \"If the service cannot \
                                      process the request payload as the \
                                      simplified format is not supported, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"Simplified Formats\"). An absent \
                                      `Content-Type` declares nothing to refuse.",
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
pub(crate) async fn role_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_get", parts, super::dispatch::dispatch).await
}

/// Update a `ROLE` (`PUT /demographic/role/{uid_based_id}`).
///
/// "Updates `ROLE` identified by `uid_based_id`." … "The existing latest
/// `version_uid` of `ROLE` resource (i.e. the `preceding_version_uid`) must be
/// specified in the `If-Match` header." (ITS-REST
/// `specifications/operations/role_update.yaml`).
#[utoipa::path(
    put, path = "/demographic/role/{uid_based_id}", tag = "ROLE",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_versioned_object_uid.yaml`) \
                        — the container, not a version. The operation adds: \"If \
                        the request body already contains a ROLE.uid.value, it \
                        must match the `uid_based_id` in the URL.\"",
         example = "8849182c-82ad-4088-a07f-48ead4180515"),
        ("If-Match" = String, Header,
         description = "The released parameter, verbatim: \"Header to make the \
                        request conditional. Together with `ETag` request tag, it \
                        helps to prevent simultaneous updates of a resource from \
                        overwriting each other (\"mid-air collisions\"). The \
                        format is always an `version_uid` identifier enclosed by \
                        double quotes. The operation will be performed only if \
                        the existing latest `version_uid` of the resource (i.e. \
                        the `preceding_version_uid`) matches this header's \
                        value.\" (ITS-REST \
                        `specifications/parameters/header/If-Match.yaml`, \
                        `required: true`). The weak `W/\"…\"` form this server \
                        emits in `ETag` is accepted too — the bare quoted form is \
                        the pre-1.1.0 shape the release keeps supported \
                        (`Requests_and_responses.md` §\"Deprecated headers\").",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`). `return=minimal` answers `204`; the \
                        other two answer `200` — \"the status will be `201 \
                        Created` or `200 OK`, never `204 No Content`\" for \
                        `return=identifier` (`Requests_and_responses.md` \
                        §\"Prefer only identifier\"). The token honoured is echoed \
                        in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format, `application/json` or \
                        `application/xml` (ITS-REST \
                        `specifications/parameters/header/ContentType_LOCATABLE.yaml`); \
                        absent reads as canonical JSON (`Resources.md` §\"JSON \
                        Format\" makes the header a client MAY). A Simplified \
                        `Content-Type` is `415`.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format, `application/json` \
                        (default) or `application/xml` (ITS-REST \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml`). \
                        A Simplified-only `Accept` is `406`.",
         example = "application/json"),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the VERSION this update commits, \
                        as an attribute-path list — e.g. \
                        `lifecycle_state.code_string=\"532\"`. No released \
                        parameter file declares this header; the requirement is \
                        prose: \"services MUST accept `openehr-version` and \
                        `openehr-audit-details` custom request headers\", merged \
                        with the server defaults \"on commit runtime\" \
                        (`Requests_and_responses.md` §\"openehr-version and \
                        openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"532\"\
                        A `lifecycle_state` of `523|deleted|` is REFUSED here \
                        (`422`): logical deletion removes the version's data and \
                        sets that state in one act (RM common master06 \
                        §\"Logical Deletion\"), so a commit that carries content \
                        cannot claim it."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this update \
                        commits, as an attribute-path list; the header MAY \
                        repeat. `change_type`, `description`, `committer` and \
                        `system_id` MAY be supplied; \"The `time_committed` \
                        attribute is always set by the server\", and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\"). No \
                        released parameter file declares it.",
         example = "change_type.code_string=\"251\""),
        ("openehr-version-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSION\" (ITS-REST \
                        `specifications/parameters/header/openehr-version-item-tag.yaml`; \
                        the only tag parameter the released update declares). \
                        Demographic ITEM_TAGs are stored against the \
                        VERSIONED_PARTY with no version anchor, so the two tag \
                        sets coincide on this surface and this build takes the \
                        list to store from `openehr-item-tag`; both response \
                        headers then carry that one set.",
         example = "key=\"reviewed\",value=\"true\""),
        ("openehr-item-tag" = Option<String>, Header,
         description = "\"The list of all ITEM_TAG to be set and associated with \
                        the current VERSIONED_OBJECT\" (ITS-REST \
                        `specifications/parameters/header/openehr-item-tag.yaml`) \
                        — the VERSIONED_PARTY. Not declared on the released \
                        update operation, but it is the header this build reads \
                        as the tag list to store, and demographic tags are \
                        VERSIONED_OBJECT-anchored, so it is the accurate one to \
                        send here. An empty value \"will effectively remove all \
                        ITEM_TAGs associated with the given target\" \
                        (`Requests_and_responses.md` §\"openehr-item-tag and \
                        openehr-version-item-tag\"); an absent header leaves the \
                        stored tags untouched.",
         example = "key=\"category\",value=\"final\"")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The new ROLE.\", `required: true` (ITS-REST \
                                `specifications/operations/role_update.yaml`; \
                                schema `schemas/demographic/Role.yaml`) as \
                                canonical JSON or XML. A `uid` in the body \"must \
                                match the `uid_based_id` in the URL\".",
                 example = json!({
                     "_type": "ROLE",
                     "name": { "_type": "DV_TEXT", "value": "ROLE" },
                     "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
                     "archetype_details": {
                         "_type": "ARCHETYPED",
                         "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                         "rm_version": "1.2.0"
                     },
                     "identities": [
                         {
                             "_type": "PARTY_IDENTITY",
                             "name": { "_type": "DV_TEXT", "value": "legal identity" },
                             "archetype_node_id": "at0001",
                             "details": {
                                 "_type": "ITEM_TREE",
                                 "name": { "_type": "DV_TEXT", "value": "identity details" },
                                 "archetype_node_id": "at0002",
                                 "items": [
                                     {
                                         "_type": "ELEMENT",
                                         "name": { "_type": "DV_TEXT", "value": "name" },
                                         "archetype_node_id": "at0003",
                                         "value": { "_type": "DV_TEXT", "value": "Senior general practitioner" }
                                     }
                                 ]
                             }
                         }
                     ]
                 })),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the ROLE is \
                                      successfully updated, with the full \
                                      resource in the response body when `Prefer` \
                                      header is `return=representation`, or only \
                                      its identifiers when `Prefer` header is \
                                      `return=identifier`.\" (ITS-REST \
                                      `specifications/responses/200_ROLE_updated.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             NEW version (ITS-REST \
                             `specifications/headers/ETag.yaml`; the weakness \
                             indicator is the release's MUST, \
                             `Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the ROLE resource.\" (ITS-REST \
                             `specifications/headers/Location_ROLE.yaml`), set to \
                             `<base_path>/demographic/role/<new version_uid>`. \
                             §Location scopes the header to \"resource creation … \
                             or redirect responses\" and §\"Prefer minimal, \
                             identifier or full representation response\" names \
                             the target as \"the newly created or updated \
                             resource\" — an openEHR update commits a NEW VERSION, \
                             which is that newly created resource."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"; both headers SHOULD accompany a \
                             versioned resource). The released \
                             `200_ROLE_updated.yaml` does not slot it; the \
                             SHOULD is cross-cutting."),
             ("Preference-Applied" = String,
              description = "`return=identifier` | `return=representation` — the \
                             preference the service honoured \
                             (`Requests_and_responses.md` §\"Representation \
                             details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`) as the \
                             server stored it; emitted only when the party carries \
                             tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`); \
                             the same set, demographic tags having no version \
                             anchor.")
         ),
         examples(
             ("representation" = (summary = "Prefer: return=representation — the updated ROLE",
              value = json!({
                  "_type": "ROLE",
                  "uid": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" },
                  "name": { "_type": "DV_TEXT", "value": "ROLE" },
                  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
                  "archetype_details": {
                      "_type": "ARCHETYPED",
                      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                      "rm_version": "1.2.0"
                  },
                  "identities": [
                      {
                          "_type": "PARTY_IDENTITY",
                          "name": { "_type": "DV_TEXT", "value": "legal identity" },
                          "archetype_node_id": "at0001",
                          "details": {
                              "_type": "ITEM_TREE",
                              "name": { "_type": "DV_TEXT", "value": "identity details" },
                              "archetype_node_id": "at0002",
                              "items": [
                                  {
                                      "_type": "ELEMENT",
                                      "name": { "_type": "DV_TEXT", "value": "name" },
                                      "archetype_node_id": "at0003",
                                      "value": { "_type": "DV_TEXT", "value": "Senior general practitioner" }
                                  }
                              ]
                          }
                      }
                  ]
              }))),
             ("identifier" = (summary = "Prefer: return=identifier — only the new version uid",
              value = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2" })))
         )),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the `Prefer` \
                                      header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_version_updated.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the new \
                             version (ITS-REST \
                             `specifications/headers/ETag.yaml`)."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the resource version resulted from the operation.\" \
                             (ITS-REST \
                             `specifications/headers/Location_version.yaml`), set \
                             to \
                             `<base_path>/demographic/role/<new version_uid>`."),
             ("Last-Modified" = String,
              description = "The new version's commit instant as an HTTP-date \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\")."),
             ("Preference-Applied" = String,
              description = "`return=minimal` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\")."),
             ("openehr-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSIONED_OBJECT\" (ITS-REST \
                             `specifications/headers/openehr-item-tag.yaml`), \
                             emitted when the party carries tags."),
             ("openehr-version-item-tag" = String,
              description = "\"The list of all ITEM_TAG associated with the \
                             current VERSION\" (ITS-REST \
                             `specifications/headers/openehr-version-item-tag.yaml`).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Two \
                                      reachable triggers here: an unparseable \
                                      `uid_based_id`/body, and an ABSENT \
                                      `If-Match` — \"When the service expects \
                                      `If-Match` for an operation, but the client \
                                      does not provide it, the service SHOULD \
                                      respond with `400 Bad Request`\" \
                                      (`Requests_and_responses.md` §\"If-Match and \
                                      accidental overwrites\").",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`). A \
                                      `versioned_object_uid` whose stored \
                                      container is a different PARTY kind is this \
                                      `404` as well — the route is kind-checked \
                                      (a VERSIONED_OBJECT has one type, RM \
                                      `common/master06` §Change Control).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      PARTY is untemplated and served in the \
                                      canonical formats only, so a \
                                      Simplified-only `Accept` MUST be refused \
                                      (`Resources.md` §\"Simplified Formats\"). \
                                      The released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 412, description = "The released trigger, verbatim: `412 \
                                      Precondition Failed` \"is returned when \
                                      `If-Match` request header doesn't match the \
                                      latest version on the service side. Returns \
                                      also latest `version_uid` in the `ETag` \
                                      header.\" (ITS-REST \
                                      `specifications/responses/412_ROLE.yaml`; \
                                      the same rule is the overview's own MUST — \
                                      \"it MUST NOT perform the requested method. \
                                      Instead, it MUST respond with HTTP status \
                                      code `412 Precondition Failed`\").",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` — the service \"SHOULD return also latest \
                             `version_uid` in the `ETag` response headers\" \
                             (`Requests_and_responses.md` §\"If-Match and \
                             accidental overwrites\"; ITS-REST \
                             `specifications/headers/ETag.yaml`). The released \
                             `412_ROLE.yaml` also slots \
                             `headers/Location_deprecated.yaml`; §Location \
                             forbids `Location` on a non-creation response, so \
                             none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`, which a party payload cannot \
                                      use (no template can be named for an \
                                      untemplated resource — \
                                      `Requests_and_responses.md` \
                                      §openehr-template-id): \"it MUST respond with \
                                      HTTP status code `415 Unsupported Media \
                                      Type`\" (`Resources.md` §\"Simplified \
                                      Formats\"). An absent `Content-Type` declares \
                                      nothing to refuse.",
         body = serde_json::Value),
        (status = 422, description = "The released trigger, verbatim: `422 \
                                      Unprocessable Entity` \"is returned when the \
                                      content type and syntax is correct, could be \
                                      converted to a resource, but there are \
                                      semantic validation errors\" (ITS-REST \
                                      `specifications/responses/422.yaml`). Here: \
                                      an RM invariant violation on the submitted \
                                      ROLE, or a body typed as a different PARTY \
                                      subtype than the route's.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_update", parts, super::dispatch::dispatch).await
}

/// Delete a `ROLE` (`DELETE /demographic/role/{uid_based_id}`).
///
/// "Deletes the `ROLE` identified by `uid_based_id`." (ITS-REST
/// `specifications/operations/role_delete.yaml`). The delete is LOGICAL: it
/// commits a new deletion `VERSION` rather than removing history — RM
/// `common/master06` §Change Control keeps every committed version, and a
/// subsequent read of the deleted current version answers `204`
/// (`responses/204_deleted_at_time.yaml`).
#[utoipa::path(
    delete, path = "/demographic/role/{uid_based_id}", tag = "ROLE",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"An identifier in a \
                        form of an OBJECT_VERSION_ID identifier taken from \
                        VERSION.uid.value (i.e. a `version_uid`).\" (ITS-REST \
                        `specifications/parameters/path/uid_based_id_as_version_uid.yaml`); \
                        the operation sharpens it: \"The `uid_based_id` MUST be in \
                        a form of an OBJECT_VERSION_ID identifier taken from the \
                        last (most recent) VERSION.uid.value, representing the \
                        `preceding_version_uid` to be deleted.\" A version that is \
                        not the latest is `409`.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("If-Match" = Option<String>, Header,
         description = "OPTIONAL here, and the released operation declares no \
                        `If-Match` parameter at all — by the spec's own carve-out: \
                        the precondition is required only \"when the \
                        `preceding_version_uid` is not part of the endpoint path \
                        segment\" (`Requests_and_responses.md` §\"If-Match and \
                        accidental overwrites\"), and on this operation it IS the \
                        path segment. A header that IS sent is still honoured — \
                        the same section makes a received precondition binding — \
                        as an alternative source of the preceding version; the \
                        weak `W/\"…\"` and bare quoted forms are both accepted.",
         example = "\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1\""),
        ("openehr-version" = Option<String>, Header,
         description = "Committal metadata for the deletion VERSION, as an \
                        attribute-path list. No released parameter file declares \
                        this header; the requirement is prose — \"services MUST \
                        also allow `PUT`, `POST` and `DELETE` methods directly on \
                        these change-controlled resources\" and \"services MUST \
                        accept `openehr-version` and `openehr-audit-details` \
                        custom request headers\" (`Requests_and_responses.md` \
                        §\"openehr-version and openehr-audit-details\").",
         example = "lifecycle_state.code_string=\"523\"\
                        A `lifecycle_state` naming any state other than \
                        `523|deleted|` contradicts the operation and is refused \
                        `400` rather than silently discarded — the section makes \
                        the merge a MUST, and a value that cannot be merged is \
                        reported, not dropped (RM common master06 \
                        §\"Logical Deletion\")."),
        ("openehr-audit-details" = Option<String>, Header,
         description = "Committal AUDIT_DETAILS for the CONTRIBUTION this delete \
                        commits, as an attribute-path list; the header MAY repeat. \
                        `time_committed` is always server-set and an omitted \
                        `system_id` MUST default to the server's configured \
                        identifier (`Requests_and_responses.md` §\"openehr-version \
                        and openehr-audit-details\"). No released parameter file \
                        declares it.",
         example = "description.value=\"merged into another record\""),
        ("Accept" = Option<String>, Header,
         description = "A successful delete has no body, so this only selects the \
                        error-body format (`application/json` by default). A \
                        Simplified-only `Accept` is `406` — a party is untemplated \
                        (`Resources.md` §\"Simplified Formats\").",
         example = "application/json")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned for a successful \
                                      delete operation.\" (ITS-REST \
                                      `specifications/responses/204_version_deleted.yaml`).",
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             DELETION version the operation just committed — not \
                             the version named in the path. The released \
                             `204_version_deleted.yaml` slots \
                             `headers/ETag.yaml`, whose value \"is an identifier \
                             (e.g. a `version_uid` …) for a specific version of a \
                             resource\", and §\"ETag and Last-Modified\" adds that \
                             it \"changes as soon as the resource changes (i.e. \
                             when a new version is created)\" — a logical delete \
                             creates one. That same response slots \
                             `headers/Location_deprecated.yaml`; §\"Deprecated \
                             headers\" deprecates `Location` on `DELETE` \
                             responses, so none is emitted or declared.")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content) or when the resource identified by \
                                      the request parameters is already deleted.\" \
                                      (ITS-REST \
                                      `specifications/responses/400_already_deleted.yaml`) \
                                      — so a second delete of the same party is \
                                      this `400`, not a `404`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or one holding a \
                                      different PARTY kind than this route.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied \
                                      (a Simplified-only `Accept` on an \
                                      untemplated resource): \"it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"Simplified \
                                      Formats\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when supplied \
                                      `uid_based_id` doesn't match the latest \
                                      version. Returns also latest `version_uid` \
                                      in the `ETag` header.\" (ITS-REST \
                                      `specifications/responses/409_ROLE_with_uid_based_id.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The CURRENT latest `version_uid`, weak form \
                             `W/\"…\"` (ITS-REST \
                             `specifications/headers/ETag.yaml`) — the client can \
                             retry the delete against it. The released response \
                             also slots `headers/Location_deprecated.yaml`; \
                             §Location forbids `Location` on a non-creation \
                             response, so none is emitted."),
             ("Last-Modified" = String,
              description = "The current latest version's commit instant as an \
                             HTTP-date, from the same metadata the `ETag` is read \
                             off (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 415, description = "The request DECLARES a Simplified \
                                      `Content-Type`. A `DELETE` sends no payload, \
                                      but the declaration is still refused before \
                                      the write, because a party has no template a \
                                      Simplified payload could be expanded against \
                                      (`Requests_and_responses.md` \
                                      §openehr-template-id; `Resources.md` \
                                      §\"Simplified Formats\" `415` MUST). An \
                                      absent `Content-Type` declares nothing to \
                                      refuse.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_delete", parts, super::dispatch::dispatch).await
}

// ── VERSIONED_PARTY ──────────────────────────────────────────────────────────

/// Retrieve the `VERSIONED_PARTY` container
/// (`GET /demographic/versioned_party/{versioned_object_uid}`).
///
/// "Retrieves a `VERSIONED_PARTY` identified by `versioned_object_uid`."
/// (ITS-REST `specifications/operations/versioned_party_get.yaml`). The four
/// `versioned_party_*` reads are canonical-JSON-only on this server: the
/// released operations reference `parameters/header/Accept_canonical.yaml`
/// (JSON + XML), and `Resources.md` §"Data representation" requires only that
/// "Services MUST support at least one of the openEHR **XML** or **JSON**
/// canonical formats" — JSON satisfies it, and an exclusively-XML `Accept` is
/// the `406` the same chapter mandates. That is an honest boundary of this
/// build, not a spec allowance to serve less.
#[utoipa::path(
    get, path = "/demographic/versioned_party/{versioned_object_uid}", tag = "VERSIONED_PARTY",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The released parameter, verbatim: \"VERSIONED_PARTY \
                        identifier taken from VERSIONED_PARTY.uid.value.\" \
                        (ITS-REST \
                        `specifications/parameters/path/versioned_object_uid_PARTY.yaml`, \
                        `format: uuid`) — the version container, i.e. the \
                        HIER_OBJECT_ID form of a party id.",
         example = "6cb19121-4307-4648-9da0-d62e4d51f19b"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        This build serves the container as `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested \
                                      VERSIONED_PARTY is successfully \
                                      retrieved.\" (ITS-REST \
                                      `specifications/responses/200_VERSIONED_PARTY.yaml`; \
                                      schema \
                                      `schemas/demographic/VersionedParty.yaml`). \
                                      `owner_id` is emitted in the shape that \
                                      schema's own example uses — the plain \
                                      `OBJECT_REF` the released \
                                      `ObjectRefOfHierObjectId` schema titles, \
                                      with `namespace: local`, `type: SYSTEM` \
                                      and a `HIER_OBJECT_ID` id — because a \
                                      demographic party has no containing EHR \
                                      to own it and no released text names \
                                      another referent; the id carries this \
                                      server's configured system identifier.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<versioned_object_uid>\"`. \
                             The released `200_VERSIONED_PARTY.yaml` declares no \
                             `ETag`; the overview does — the value \"is usually \
                             taken from e.g. VERSIONED_OBJECT.uid.value, \
                             VERSION.uid.value\" and both `ETag` and \
                             `Last-Modified` \"SHOULD be included in responses for \
                             VERSION, VERSIONED_OBJECT, or other resources that \
                             have versioning\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\"). No `Last-Modified` \
                             accompanies it: a VERSIONED_OBJECT container body \
                             exposes no `commit_audit.time_committed` to derive \
                             it from.")
         ),
         example = json!({
             "_type": "VERSIONED_PARTY",
             "uid": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
             "owner_id": {
                 "_type": "OBJECT_REF",
                 "namespace": "local",
                 "type": "SYSTEM",
                 "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" }
             },
             "time_created": { "_type": "DV_DATE_TIME", "value": "2015-01-20T19:30:22.765+01:00" }
         })),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      a `versioned_object_uid` that is not a \
                                      well-formed party id. The released operation \
                                      does not enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      this build serves the VERSIONED_PARTY \
                                      container as canonical JSON only, so an \
                                      `Accept` that excludes `application/json` is \
                                      refused — \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not Acceptable`\" \
                                      (`Resources.md` §\"XML Format\"/§\"JSON \
                                      Format\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_party_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_party_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve the party's `REVISION_HISTORY`
/// (`GET /demographic/versioned_party/{versioned_object_uid}/revision_history`).
///
/// "Retrieves revision history of the `VERSIONED_PARTY` identified by
/// `versioned_object_uid`." (ITS-REST
/// `specifications/operations/versioned_party_revision_history.yaml`).
#[utoipa::path(
    get, path = "/demographic/versioned_party/{versioned_object_uid}/revision_history", tag = "VERSIONED_PARTY",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The released parameter, verbatim: \"VERSIONED_PARTY \
                        identifier taken from VERSIONED_PARTY.uid.value.\" \
                        (ITS-REST \
                        `specifications/parameters/path/versioned_object_uid_PARTY.yaml`, \
                        `format: uuid`).",
         example = "6cb19121-4307-4648-9da0-d62e4d51f19b"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        This build serves the history as `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested \
                                      REVISION_HISTORY is successfully \
                                      retrieved.\" (ITS-REST \
                                      `specifications/responses/200_REVISION_HISTORY.yaml`; \
                                      schema `schemas/common/RevisionHistory.yaml`). \
                                      `items` runs oldest-first — \
                                      `REVISION_HISTORY.most_recent_version` is \
                                      the last item (RM `common/master04` \
                                      §REVISION_HISTORY).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<versioned_object_uid>\"`. A \
                             REVISION_HISTORY carries no `uid` of its own, so the \
                             addressed container's id is the `ETag` source the \
                             overview names — the value \"is usually taken from \
                             e.g. VERSIONED_OBJECT.uid.value, VERSION.uid.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). The released \
                             `200_REVISION_HISTORY.yaml` declares no `ETag`; the \
                             SHOULD is cross-cutting."),
             ("Last-Modified" = String,
              description = "The most recent revision's commit instant as an \
                             HTTP-date — \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"), taken from the last \
                             `items[]`/`audits[0]` entry.")
         )),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`) — here a \
                                      `versioned_object_uid` that is not a \
                                      well-formed party id.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      REVISION_HISTORY is served as canonical JSON \
                                      only on this build, so an `Accept` excluding \
                                      `application/json` MUST be refused \
                                      (`Resources.md` §\"JSON Format\"). The \
                                      released operation does not enumerate `406`; \
                                      the MUST is cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn versioned_party_revision_history(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_party_revision_history",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve the party `VERSION` at a point in time
/// (`GET /demographic/versioned_party/{versioned_object_uid}/version`).
///
/// "Retrieves a `VERSION` from the `VERSIONED_PARTY` identified by
/// `versioned_object_uid`." … "If `version_at_time` is supplied, retrieves the
/// `VERSION` extant _at specified time_, otherwise retrieves the _latest_
/// `VERSION`." (ITS-REST
/// `specifications/operations/versioned_party_version_get_at_time.yaml`).
#[utoipa::path(
    get, path = "/demographic/versioned_party/{versioned_object_uid}/version", tag = "VERSIONED_PARTY",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The released parameter, verbatim: \"VERSIONED_PARTY \
                        identifier taken from VERSIONED_PARTY.uid.value.\" \
                        (ITS-REST \
                        `specifications/parameters/path/versioned_object_uid_PARTY.yaml`, \
                        `format: uuid`).",
         example = "6cb19121-4307-4648-9da0-d62e4d51f19b"),
        ("version_at_time" = Option<String>, Query,
         description = "\"A given time in the extended ISO 8601 format.\" \
                        (ITS-REST \
                        `specifications/parameters/query/version_at_time.yaml`); \
                        the latest VERSION when omitted. The timezone is optional \
                        — server-local when absent.",
         example = "2015-01-20T19:30:22.765+01:00"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        This build serves the VERSION as `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested VERSION is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_VERSION_of_PARTY_at_time.yaml`; \
                                      schema \
                                      `schemas/demographic/UVersionOfParty.yaml`) \
                                      — the ORIGINAL_VERSION wrapper, `data` \
                                      carrying the party itself.",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is the \
                             VERSION identifier (i.e. the `version_uid`) enclosed \
                             by double quotes.\" (ITS-REST \
                             `specifications/headers/ETag_VERSION.yaml`, which this \
                             response slots), in the weak `W/\"…\"` form the \
                             release requires. That response also slots \
                             `headers/Location_deprecated.yaml`; §Location forbids \
                             `Location` on a `GET`, so none is emitted."),
             ("Last-Modified" = String,
              description = "The served VERSION's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`) — here a \
                                      malformed `versioned_object_uid` or a \
                                      `version_at_time` that is not an extended ISO \
                                      8601 instant.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when either the URL \
                                      configured doesn't exist at all, or the \
                                      targeted resource doesn't exist, or when a \
                                      VERSION of the resource does not exist at \
                                      the specified `version_at_time`\" (ITS-REST \
                                      `specifications/responses/404_not_found_or_no_version_at_time.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      VERSION is served as canonical JSON only on \
                                      this build, so an `Accept` excluding \
                                      `application/json` MUST be refused \
                                      (`Resources.md` §\"JSON Format\"). The \
                                      released operation does not enumerate `406`; \
                                      the MUST is cross-cutting.",
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
pub(crate) async fn versioned_party_version_get_at_time(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_party_version_get_at_time",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a specific party `VERSION` by version uid
/// (`GET /demographic/versioned_party/{versioned_object_uid}/version/{version_uid}`).
///
/// "Retrieves a `VERSION` identified by `version_uid` of a `VERSIONED_PARTY`
/// identified by `versioned_object_uid`." (ITS-REST
/// `specifications/operations/versioned_party_version_get_by_id.yaml`).
#[utoipa::path(
    get, path = "/demographic/versioned_party/{versioned_object_uid}/version/{version_uid}", tag = "VERSIONED_PARTY",
    params(
        ("versioned_object_uid" = String, Path,
         description = "The released parameter, verbatim: \"VERSIONED_PARTY \
                        identifier taken from VERSIONED_PARTY.uid.value.\" \
                        (ITS-REST \
                        `specifications/parameters/path/versioned_object_uid_PARTY.yaml`, \
                        `format: uuid`).",
         example = "6cb19121-4307-4648-9da0-d62e4d51f19b"),
        ("version_uid" = String, Path,
         description = "The released parameter, verbatim: \"VERSION identifier \
                        taken from VERSION.uid.value.\" (ITS-REST \
                        `specifications/parameters/path/version_uid.yaml`) — the \
                        OBJECT_VERSION_ID form, whose `object_id` segment is the \
                        `versioned_object_uid` above (`Resources.md` §\"Identifier \
                        types\").",
         example = "6cb19121-4307-4648-9da0-d62e4d51f19b::openEHRSys.example.com::2"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        This build serves the VERSION as `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the requested VERSION is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_VERSION_of_PARTY_by_id.yaml`; \
                                      schema \
                                      `schemas/demographic/UVersionOfParty.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "The weak entity tag `W/\"<version_uid>\"` of the \
                             served VERSION. The released \
                             `200_VERSION_of_PARTY_by_id.yaml` declares NO `ETag` \
                             while its at-time sibling declares \
                             `headers/ETag_VERSION.yaml` — an asymmetry between two \
                             responses that serve the same resource shape. The \
                             overview settles it: `ETag` and `Last-Modified` \
                             \"SHOULD be included in responses for VERSION, \
                             VERSIONED_OBJECT, or other resources that have \
                             versioning\", the value \"usually taken from e.g. … \
                             VERSION.uid.value\" (`Requests_and_responses.md` \
                             §\"ETag and Last-Modified\"), so both are emitted \
                             here."),
             ("Last-Modified" = String,
              description = "The served VERSION's commit instant as an HTTP-date, \
                             \"derived from \
                             VERSION.commit_audit.time_committed.value\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\").")
         )),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`) — here a \
                                      `versioned_object_uid` or `version_uid` that \
                                      is not well-formed.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when, based on the \
                                      request parameters, the server did not find \
                                      a current representation of a target \
                                      resource, or is not willing to disclose that \
                                      one exists\" (ITS-REST \
                                      `specifications/responses/404.yaml`) — an \
                                      unknown container, or a `version_uid` that \
                                      names no version of it.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      VERSION is served as canonical JSON only on \
                                      this build, so an `Accept` excluding \
                                      `application/json` MUST be refused \
                                      (`Resources.md` §\"JSON Format\"). The \
                                      released operation does not enumerate `406`; \
                                      the MUST is cross-cutting.",
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
pub(crate) async fn versioned_party_version_get_by_id(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "versioned_party_version_get_by_id",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

// ── CONTRIBUTION ─────────────────────────────────────────────────────────────

/// Create a demographic `CONTRIBUTION` (`POST /demographic/contribution`).
///
/// The released operation defines the relaxed commit envelope verbatim: "We
/// will use the relaxed CONTRIBUTION with the following optional attributes:
/// `uid`: when provided, it will be accepted in case is not in-use, otherwise
/// error will be returned; `audit.time_committed`: server will always set it;
/// `audit.system_id`: when provided, it will be validated" (ITS-REST
/// `specifications/operations/demographic_contribution_create.yaml`). "The
/// `audit` and each `versions[i].commit_audit` are `UPDATE_AUDIT` objects …
/// Clients SHOULD send `_type: \"UPDATE_AUDIT\"`; for interoperability servers
/// SHOULD additionally accept `_type: \"AUDIT_DETAILS\"` or an omitted `_type`
/// for this attribute."
///
/// The commit envelope is canonical **JSON** only on this build: the released
/// operation references `parameters/header/ContentType_canonical.yaml`
/// (JSON + XML), and `Resources.md` §"Data representation" requires only that
/// "Services MUST support at least one of the openEHR **XML** or **JSON**
/// canonical formats" — an honest boundary, not a spec allowance.
#[utoipa::path(
    post, path = "/demographic/contribution", tag = "CONTRIBUTION",
    params(
        ("Prefer" = Option<String>, Header,
         description = "The released parameter, verbatim: \"Request header to \
                        indicate the preference over response details. The \
                        response will contain the entire resource when the \
                        `Prefer` header has a value of `return=representation`, \
                        or only the resource identifier (e.g., the `uid`) when \
                        the value is `return=identifier`.\" (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`; default \
                        `return=minimal`, which answers an empty `201`). The \
                        token honoured is echoed in `Preference-Applied`.",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "`application/json` — the CONTRIBUTION commit envelope is \
                        canonical JSON on this build (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml` \
                        also lists `application/xml`, which this envelope does \
                        not serve). Any other DECLARED type is `415`; an absent \
                        header reads as JSON (`Resources.md` §\"JSON Format\" \
                        makes the header a client MAY).",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        The `201` body is served as `application/json`; an \
                        `Accept` that excludes JSON is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "\"The CONTRIBUTION.\", `required: true` (ITS-REST \
                                `specifications/operations/demographic_contribution_create.yaml`; \
                                schema \
                                `schemas/demographic/NewContribution.yaml` — \
                                `versions` and `audit` required, `uid` optional). \
                                Each `versions[i]` is an `UPDATE_VERSION` \
                                (`lifecycle_state`, `data`, `commit_audit` \
                                required; `preceding_version_uid` on a \
                                modification). NOTE: the release publishes no \
                                demographic CONTRIBUTION example, so the example \
                                below is OURS, constructed from those schemas — \
                                it is not released spec text.",
                 example = json!({
                     "uid": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" },
                     "versions": [
                         {
                             "_type": "ORIGINAL_VERSION",
                             "lifecycle_state": {
                                 "_type": "DV_CODED_TEXT",
                                 "value": "complete",
                                 "defining_code": {
                                     "terminology_id": { "value": "openehr" },
                                     "code_string": "532"
                                 }
                             },
                             "commit_audit": {
                                 "_type": "UPDATE_AUDIT",
                                 "change_type": {
                                     "_type": "DV_CODED_TEXT",
                                     "value": "creation",
                                     "defining_code": {
                                         "terminology_id": { "value": "openehr" },
                                         "code_string": "249"
                                     }
                                 },
                                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "A user name" }
                             },
                             "data": {
                                 "_type": "PERSON",
                                 "name": { "_type": "DV_TEXT", "value": "PERSON" },
                                 "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
                                 "archetype_details": {
                                     "_type": "ARCHETYPED",
                                     "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                                     "rm_version": "1.2.0"
                                 },
                                 "identities": [
                                     {
                                         "_type": "PARTY_IDENTITY",
                                         "name": { "_type": "DV_TEXT", "value": "legal identity" },
                                         "archetype_node_id": "at0001",
                                         "details": {
                                             "_type": "ITEM_TREE",
                                             "name": { "_type": "DV_TEXT", "value": "identity details" },
                                             "archetype_node_id": "at0002",
                                             "items": [
                                                 {
                                                     "_type": "ELEMENT",
                                                     "name": { "_type": "DV_TEXT", "value": "name" },
                                                     "archetype_node_id": "at0003",
                                                     "value": { "_type": "DV_TEXT", "value": "Jane Doe" }
                                                 }
                                             ]
                                         }
                                     }
                                 ]
                             }
                         }
                     ],
                     "audit": {
                         "_type": "UPDATE_AUDIT",
                         "change_type": {
                             "_type": "DV_CODED_TEXT",
                             "value": "creation",
                             "defining_code": {
                                 "terminology_id": { "value": "openehr" },
                                 "code_string": "249"
                             }
                         },
                         "description": { "_type": "DV_TEXT", "value": "Description text" },
                         "committer": { "_type": "PARTY_IDENTIFIED", "name": "A user name" }
                     }
                 })),
    responses(
        (status = 201, description = "The released trigger, verbatim: `201 \
                                      Created` \"is returned when the \
                                      CONTRIBUTION is successfully created. If \
                                      `Prefer` header is `return=representation`, \
                                      the full resource is included in the \
                                      response body; if is `return=identifier`, \
                                      only its unique identifier is included. If \
                                      the `Prefer` header is missing or set to \
                                      `return=minimal`, the body is empty.\" \
                                      (ITS-REST \
                                      `specifications/responses/201_demographic_CONTRIBUTION.yaml`).",
         body = serde_json::Value,
         headers(
             ("ETag" = String,
              description = "\"The `ETag` (i.e. entity tag) response header is the \
                             `contribution_uid` identifier, enclosed by double \
                             quotes.\" (ITS-REST \
                             `specifications/headers/ETag_CONTRIBUTION.yaml`), in \
                             the weak form the release requires — \"all `ETag` \
                             headers that hold a resource identifier MUST include \
                             a weakness indicator `W/`\" \
                             (`Requests_and_responses.md` §\"ETag and \
                             Last-Modified\"). Shape: \
                             `W/\"0826851c-c4c2-4d61-92b9-410fb8275ff0\"`. It is \
                             the CONTRIBUTION's own uid, not a version uid."),
             ("Location" = String,
              description = "\"The `Location` response header indicates the URL of \
                             the CONTRIBUTION resource.\" (ITS-REST \
                             `specifications/headers/Location_demographic_CONTRIBUTION.yaml`), \
                             set to \
                             `<base_path>/demographic/contribution/<contribution_uid>` \
                             — §Location: used \"in `201 Created` responses when a \
                             new resource is successfully created\"."),
             ("Last-Modified" = String,
              description = "The commit instant of this CONTRIBUTION's audit, as \
                             an HTTP-date. §\"ETag and Last-Modified\": \"Both \
                             `ETag` and `Last-Modified` SHOULD be included in \
                             responses for VERSION, VERSIONED_OBJECT, or other \
                             resources that have versioning or unique state \
                             identifiers\", the value \"derived from \
                             `VERSION.commit_audit.time_committed.value`\" — a \
                             CONTRIBUTION is immutable and this response already \
                             names its unique identifier, so the committal is its \
                             one modification instant. Emitted under every \
                             `Prefer` setting, matching the EHR-scoped commit and \
                             the CONTRIBUTION read."),
             ("Preference-Applied" = String,
              description = "`return=minimal` | `return=identifier` | \
                             `return=representation` — the preference the service \
                             honoured (`Requests_and_responses.md` \
                             §\"Representation details negotiation\").")
         ),
         examples(
             ("identifier" = (summary = "Prefer: return=identifier — only the contribution uid",
              value = json!({ "uid": "0826851c-c4c2-4d61-92b9-410fb8275ff0" }))),
             ("representation" = (summary = "Prefer: return=representation — the committed CONTRIBUTION (constructed example — the release publishes none)",
              value = json!({
                  "_type": "CONTRIBUTION",
                  "uid": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" },
                  "versions": [
                      {
                          "_type": "OBJECT_REF",
                          "namespace": "local",
                          "type": "PERSON",
                          "id": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" }
                      }
                  ],
                  "audit": {
                      "_type": "AUDIT_DETAILS",
                      "system_id": "openEHRSys.example.com",
                      "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                      "change_type": {
                          "_type": "DV_CODED_TEXT",
                          "value": "creation",
                          "defining_code": {
                              "terminology_id": { "value": "openehr" },
                              "code_string": "249"
                          }
                      },
                      "description": { "_type": "DV_TEXT", "value": "Description text" },
                      "committer": { "_type": "PARTY_IDENTIFIED", "name": "A user name" }
                  }
              })))
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content, or the modification type does not \
                                      match the operation - i.e. first version of \
                                      a MODIFICATION)\" (ITS-REST \
                                      `specifications/responses/400_CONTRIBUTION.yaml`). \
                                      Here that is: the envelope is not parseable \
                                      JSON; the envelope `audit` or its \
                                      `change_type` is absent \
                                      (`NewContribution.yaml` and \
                                      `UpdateAudit.yaml` list both under \
                                      `required`); the `audit`, a member \
                                      `commit_audit` or an attestation states no \
                                      `committer`, or one that does not decode as \
                                      a canonical `PARTY_PROXY` \
                                      (`UpdateAudit.yaml` lists it under \
                                      `required` and types it `UPartyProxy`); a \
                                      member, an attestation (`666`) member \
                                      included, states no `lifecycle_state` (SM \
                                      master03 §\"Version Update Semantics\": \
                                      \"must be supplied in all cases\"); a \
                                      non-creation change type on a member with \
                                      no `preceding_version_uid` (the released \
                                      first-version-of-a-MODIFICATION trigger); \
                                      a member carrying a property \
                                      `UpdateVersion.yaml` does not declare \
                                      (`other_input_version_uids` and `item` \
                                      included); or a member names a \
                                      `preceding_version_uid` whose \
                                      VERSIONED_OBJECT does not exist.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      `201` body is served as canonical JSON only, \
                                      so an `Accept` excluding `application/json` \
                                      MUST be refused (`Resources.md` §\"JSON \
                                      Format\"). The released operation does not \
                                      enumerate `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when a resource with \
                                      same identifier(s) already exists\" (ITS-REST \
                                      `specifications/responses/409.yaml`) — a \
                                      client-supplied `uid` that is already in \
                                      use, the operation's \"accepted in case is \
                                      not in-use, otherwise error will be \
                                      returned\".",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a payload media type \
                                      this envelope cannot be processed as: the \
                                      demographic CONTRIBUTION commit is canonical \
                                      JSON on this build, so an XML or Simplified \
                                      `Content-Type` is refused — \"If the service \
                                      cannot process the request payload as JSON \
                                      format, it MUST respond with HTTP status \
                                      code `415 Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"; the same \
                                      MUST for the other formats). An absent \
                                      `Content-Type` declares nothing to refuse. \
                                      The released operation does not enumerate \
                                      `415`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The CONTRIBUTION was well-formed but \
                                      cannot be followed \
                                      (`Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row: \"The \
                                      request was well-formed but was unable to \
                                      be followed due to semantic errors\"). The \
                                      released operation enumerates no `422`; \
                                      the same section permits it (\"Additional \
                                      status codes MAY be used as long as they \
                                      do not conflict with the predefined \
                                      codes\"), and every trigger below is OUR \
                                      OWN assignment, shared with the EHR-scoped \
                                      commit: an empty `versions: []`; a \
                                      malformed envelope `uid` (not a \
                                      HIER_OBJECT_ID UUID); a `change_type` — on \
                                      a member or on the envelope — that is not \
                                      a code of the openEHR `audit_change_type` \
                                      group (`AUDIT_DETAILS.Change_type_valid`); \
                                      a `249|creation|` member carrying a \
                                      `preceding_version_uid`; `data` on a \
                                      `523|deleted|` or `666|attestation|` \
                                      member, or missing `data` on a creation or \
                                      modification member; a member whose \
                                      object kind is out of the contribution's \
                                      scope (an EHR-scoped COMPOSITION, \
                                      EHR_STATUS, EHR_ACCESS or FOLDER in a \
                                      demographic CONTRIBUTION); a \
                                      `commit_audit` failing the AUDIT_DETAILS \
                                      invariants (empty `system_id`, a \
                                      committer that decodes but breaks its own \
                                      PARTY_PROXY invariants); a \
                                      `lifecycle_state` outside the openEHR \
                                      `version_lifecycle_state` group, a \
                                      `666|attestation|` member's included (it \
                                      is checked against the group although an \
                                      attestation stores no lifecycle state); an \
                                      `UPDATE_ATTESTATION` failing its \
                                      `ATTESTATION` attributes (`reason`, \
                                      `is_pending`, `items`); and any member \
                                      failing RM-invariant or archetype \
                                      validation, exactly as the direct party \
                                      routes would validate it.",
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
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a demographic `CONTRIBUTION` by uid
/// (`GET /demographic/contribution/{contribution_uid}`).
///
/// "Retrieves a CONTRIBUTION identified by `contribution_uid`." (ITS-REST
/// `specifications/operations/demographic_contribution_get.yaml`).
#[utoipa::path(
    get, path = "/demographic/contribution/{contribution_uid}", tag = "CONTRIBUTION",
    params(
        ("contribution_uid" = String, Path,
         description = "The released parameter, verbatim: \"The CONTRIBUTION \
                        uid.\" (ITS-REST \
                        `specifications/parameters/path/contribution_uid.yaml`, \
                        `format: uuid`). A value that is not a UUID is `400`.",
         example = "0826851c-c4c2-4d61-92b9-410fb8275ff0"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`). \
                        This build serves the CONTRIBUTION envelope as \
                        `application/json`; an `Accept` that excludes JSON — \
                        including the Simplified types the released `200` \
                        describes — is `406`.",
         example = "application/json")
    ),
    responses(
        (status = 200, description = "The released trigger, verbatim: `200 OK` \
                                      \"is returned when the CONTRIBUTION is \
                                      successfully retrieved.\" (ITS-REST \
                                      `specifications/responses/200_CONTRIBUTION.yaml`; \
                                      schema `schemas/common/Contribution.yaml`). \
                                      That response also describes a Simplified \
                                      arm — \"When the request `Accept` header \
                                      selects a Simplified Formats MIME type …, \
                                      the response body is still a canonical \
                                      CONTRIBUTION envelope; only each \
                                      `versions[i].data` payload is serialized in \
                                      the requested FLAT or STRUCTURED form\" — \
                                      which this build does NOT serve on the \
                                      demographic surface: a demographic \
                                      CONTRIBUTION's versions carry untemplated \
                                      PARTY data with no template to expand \
                                      against (`Requests_and_responses.md` \
                                      §openehr-template-id), so a Simplified \
                                      `Accept` is the `406` below. No `ETag` or \
                                      `Last-Modified` is emitted: the released \
                                      response declares neither, and a \
                                      CONTRIBUTION is not itself a versioned \
                                      resource.",
         body = serde_json::Value,
         example = json!({
             "_type": "CONTRIBUTION",
             "uid": { "_type": "HIER_OBJECT_ID", "value": "0826851c-c4c2-4d61-92b9-410fb8275ff0" },
             "versions": [
                 {
                     "_type": "OBJECT_REF",
                     "namespace": "local",
                     "type": "PERSON",
                     "id": { "_type": "OBJECT_VERSION_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" }
                 }
             ],
             "audit": {
                 "_type": "AUDIT_DETAILS",
                 "system_id": "openEHRSys.example.com",
                 "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-26T09:12:44.512331Z" },
                 "change_type": {
                     "_type": "DV_CODED_TEXT",
                     "value": "creation",
                     "defining_code": {
                         "terminology_id": { "value": "openehr" },
                         "code_string": "249"
                     }
                 },
                 "description": { "_type": "DV_TEXT", "value": "Description text" },
                 "committer": { "_type": "PARTY_IDENTIFIED", "name": "A user name" }
             }
         })),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is returned \
                                      when the request could not be parsed or is \
                                      invalid (e.g. malformed request URL syntax, \
                                      missing required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`) — here a \
                                      `contribution_uid` that is not a UUID, the \
                                      `format: uuid` the released path parameter \
                                      declares. The released operation does not \
                                      enumerate `400`; the trigger is the \
                                      cross-cutting one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a CONTRIBUTION \
                                      with `contribution_uid` does not exist\" \
                                      (ITS-REST \
                                      `specifications/responses/404_demographic_CONTRIBUTION.yaml`). \
                                      An EHR-scoped CONTRIBUTION is this `404` \
                                      too — the demographic surface addresses only \
                                      the ehr-less ones, and the EHR API has its \
                                      own contribution endpoints.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: the \
                                      CONTRIBUTION envelope is served as canonical \
                                      JSON only on this build, so an `Accept` \
                                      excluding `application/json` (XML, or one of \
                                      the Simplified types the released `200` \
                                      describes) MUST be refused — \"If the service \
                                      cannot fulfill this aspect of the request, it \
                                      MUST respond with HTTP status code `406 Not \
                                      Acceptable`\" (`Resources.md` §\"JSON \
                                      Format\"/§\"Simplified Formats\"). The \
                                      released operation does not enumerate `406`; \
                                      the MUST is cross-cutting.",
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
    guarded_dispatch(state, "contribution_get", parts, super::dispatch::dispatch).await
}
// ── ITEM_TAG sub-resources ───────────────────────────────────────────────────
// Sixteen released operations: five byte-identical typed quintets
// (`{person,agent,group,organisation,role}_tags_{get,update,delete}.yaml`) plus
// the space-wide `demographic_tags_get.yaml`. Canonical-JSON only — the
// canonical XML ITS defines no ITEM_TAG type (`Resources.md` §"XML Format"), so
// an XML `Accept` is `406` and an XML `Content-Type` on the five PUTs `415`.
// Tags are not change-controlled: no CONTRIBUTION, version, `If-Match`, `ETag`
// or `Location`; the only response header is `Preference-Applied` on the PUTs.
// NOTE: `target` is the bare RM `UID_BASED_ID` (`item_tag.adoc`), which wins
// over the released OAS `UObjectRefOfUidBasedId` envelope; no released text
// fixes an EHR-less tag's `owner_id`, so it follows the `ItemTagOf*` examples.

/// List `ITEM_TAG`s across the whole Demographic space
/// (`GET /demographic/tags`).
///
/// "Retrieves the list of `ITEM_TAG` resources associated with any target `VERSION`
/// or `VERSIONED_PARTY` within the Demographic space." (ITS-REST
/// `specifications/operations/demographic_tags_get.yaml`).
///
/// The ONLY tag route on the whole surface with NO scoping parameter: its
/// EHR-side twin is bounded by one EHR, this one is a whole-space scan. The
/// released operation declares no scoping parameter, no `offset`/`fetch`, no
/// ordering and no limit, and no released sentence bounds who may read the
/// space — it is served whole under the deployment's authorization layer, a
/// position adjudicated (distinct from the
/// filter semantics, which are their own entry).
#[utoipa::path(
    get, path = "/demographic/tags", tag = "ITEM_TAG",
    params(
        ("tag_key" = Option<String>, Query,
         description = "Filter by ITEM_TAG `key`. The released parameter file \
                        (`specifications/parameters/query/tag_key.yaml`) \
                        carries NO description at all — only `name`, `in: \
                        query`, `style: form`, `explode: true` and `schema: \
                        {type: string}` — so everything below is read off the \
                        operation description or is OURS. The three filters \
                        are AND-combined, each an EXACT, case-sensitive match \
                        on the stored value; an omitted filter constrains \
                        nothing — \"In case no such parameter is provided then \
                        all ITEM_TAG resources will be retrieved\" \
                        (`demographic_tags_get.yaml`). None of exactness, case \
                        sensitivity or the combination rule is fixed by the \
                        released text, so those semantics are OURS, \
                        adjudicated. The \
                        parameter is SCALAR: the released description says the \
                        list \"can be filtered by the given one or more \
                        `tag_key`, `tag_value`, `tag_target_path` query \
                        parameters\" while each released parameter schema is a \
                        plain `type: string` — the plural reads as one or more \
                        OF THE THREE parameters (the mismatch is a \
                        released-text defect, adjudicated), and a \
                        repeated parameter has no defined meaning.",
         example = "flag"),
        ("tag_value" = Option<String>, Query,
         description = "Filter by ITEM_TAG `value`. The released parameter \
                        file \
                        (`specifications/parameters/query/tag_value.yaml`) \
                        carries NO description at all — only `name`, `in: \
                        query`, `style: form`, `explode: true` and `schema: \
                        {type: string}` — so everything below is read off the \
                        operation description or is OURS. Same semantics as \
                        `tag_key`: exact, case-sensitive, AND-combined, \
                        scalar.",
         example = "follow-up"),
        ("tag_target_path" = Option<String>, Query,
         description = "Filter by ITEM_TAG `target_path`. The released \
                        parameter file \
                        (`specifications/parameters/query/tag_target_path.yaml`) \
                        carries NO description at all — only `name`, `in: \
                        query`, `style: form`, `explode: true` and `schema: \
                        {type: string}` — so everything below is read off the \
                        operation description or is OURS. Same semantics as \
                        `tag_key`: exact, case-sensitive, AND-combined, \
                        scalar. Tags stored WITHOUT a `target_path` (the \
                        absent 0..1 case, which is also where an empty string \
                        normalizes to) match no value of this filter; they are \
                        reached by omitting it.",
         example = "/details/items[at0001]/value"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The matching ITEM_TAG list, across \
                                         every party kind and both target \
                                         forms. An empty array when nothing \
                                         matches — the released empty-list \
                                         sentence for this operation still \
                                         names an EHR, a copy-paste from the \
                                         EHR-scoped twin onto a route that has \
                                         no EHR at all; it is read as the \
                                         Demographic space and the defect is \
                                         adjudicated. A no-match \
                                         result is `200 []`, never `404`. \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         tagged target — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The released operation reuses \
                                         `responses/200_PERSON_ItemTagList_retrieved.yaml` \
                                         (items typed \
                                         `schemas/demographic/ItemTagOfPerson.yaml`) \
                                         for this CROSS-KIND list, another \
                                         released-text defect — each row's own \
                                         `target.type` names its kind, and the \
                                         example below shows an AGENT tag \
                                         beside a PERSON one. The (`key`, \
                                         `target_path`) ordering the server \
                                         applies is OURS: no openEHR spec \
                                         governs tag ordering.",
            content((serde_json::Value = "application/json", example = json!([
                {
                    "_type": "ITEM_TAG",
                    "key": "flag",
                    "value": "follow-up",
                    "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                    "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                },
                {
                    "_type": "ITEM_TAG",
                    "key": "reviewed",
                    "value": "true",
                    "target_path": "/details/items[at0001]/value",
                    "target": { "_type": "HIER_OBJECT_ID", "value": "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31" },
                    "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                }
            ])))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      This route takes no path parameter and \
                                      every filter is an unconstrained string, \
                                      so the only reachable trigger is an \
                                      unparseable query string. The released \
                                      operation declares `200` and `400` ONLY \
                                      — with no scoping parameter there is \
                                      nothing to fail to find, so there is no \
                                      `404` on this route.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn demographic_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "demographic_tags_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve an `AGENT`'s `ITEM_TAG`s
/// (`GET /demographic/agent/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `AGENT` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/agent_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag collections — see the
/// parameter.
#[utoipa::path(
    get, path = "/demographic/agent/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the AGENT \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        get the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/agent_tags_get.yaml`; the \
                        released path parameter file \
                        `parameters/path/uid_based_id.yaml` carries the same \
                        dual-form sentence with VERSIONED_OBJECT in place of \
                        VERSIONED_PARTY). The two forms address DISJOINT \
                        collections: an ITEM_TAG carries exactly one `target` \
                        (RM `item_tag.adoc`: `target: UID_BASED_ID`, \"which \
                        may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), \
                        so a tag written against the container is invisible to \
                        the version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                         OK` \"is returned when the requested \
                                         ITEM_TAG list is successfully \
                                         retrieved.\" (ITS-REST \
                                         `specifications/responses/200_AGENT_ItemTagList_retrieved.yaml`; \
                                         items typed \
                                         `schemas/demographic/ItemTagOfAgent.yaml`). \
                                         \"This will return an empty list when \
                                         there is no ITEM_TAG associated with \
                                         the given target\" \
                                         (`agent_tags_get.yaml`) — an \
                                         EXISTING, untagged target is `200 \
                                         []`; a target that does not exist is \
                                         `404`. \"More than one ITEM_TAG may \
                                         be associated with a single target \
                                         AGENT or VERSIONED_PARTY, in which \
                                         case they are uniquely identified by \
                                         their `key` and `target_path` pair \
                                         attributes\". Every row carries the \
                                         SERVER-ASSIGNED `target` and \
                                         `owner_id`; neither is client input. \
                                         `target` names the ADDRESSED \
                                         collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. No `ETag`, `Last-Modified` or \
                                         `Location` accompanies the list: a \
                                         tag collection is not \
                                         change-controlled and has no version \
                                         and no uid.",
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      All of these are that non-existence: an \
                                      unknown `versioned_object_uid`; a \
                                      version form whose version does not \
                                      exist; and a well-formed uid whose \
                                      stored container is NOT an AGENT — \
                                      another PARTY kind, or a \
                                      COMPOSITION/EHR_STATUS/FOLDER uid from \
                                      the EHR space. The kind-checked reading \
                                      is OURS (the released sentence does not \
                                      spell it out) and follows from the route \
                                      naming the target's class — a \
                                      VERSIONED_OBJECT has one type (RM \
                                      `common/master06` §Change Control); it \
                                      is adjudicated. An EXISTING \
                                      target with no tags is `200 []`, never \
                                      `404`.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_tags_get", parts, super::dispatch::dispatch).await
}

/// Replace an `AGENT`'s `ITEM_TAG`s
/// (`PUT /demographic/agent/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `AGENT` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/agent_tags_update.yaml`). It is a FULL COLLECTION
/// REPLACE of the ADDRESSED collection — the container's or one version's,
/// never both.
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION, mints
/// no version, takes no `If-Match` and no committal headers, and serves neither
/// `ETag` nor `Last-Modified`.
#[utoipa::path(
    put, path = "/demographic/agent/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular AGENT version (e.g. one \
                        identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        update the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/agent_tags_update.yaml`). \
                        The update sentence sources the HIER_OBJECT_ID from \
                        VERSIONED_OBJECT while the get and delete of the same \
                        family source it from VERSIONED_PARTY, and all three \
                        end on \"the target VERSIONED_PARTY container\" — an \
                        editorial split inside one operation family, \
                        adjudicated; both \
                        name the same container. The two forms address \
                        DISJOINT collections: an ITEM_TAG carries exactly one \
                        `target` (RM `item_tag.adoc`: `target: UID_BASED_ID`, \
                        \"which may be a `VERSIONED_OBJECT<T>` or a \
                        `VERSION<T>`\"), so a tag written against the \
                        container is invisible to the version form and a tag \
                        written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So replacing the container's list never touches any \
                        version's list, and replacing one version's list never \
                        touches the container's or a sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` — the default when the header is \
                        absent (`Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") — answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured: its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid, so the server applies — and \
                        declares — the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied` (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`).",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml`, \
                        enum `application/json` | `application/xml`). The tag \
                        list has no XML and no Simplified-Format shape, so \
                        only `application/json` is processable and any other \
                        declared type is `415`; an ABSENT `Content-Type` \
                        declares nothing and is read as canonical JSON.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                ADDRESSED collection (`required: true`; there \
                                is no envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED (\"Tag key (identifier)\"), `value` \
                                (\"Tag value\") and `target_path` (\"An AQL \
                                path withing the `target` used to tag a \
                                fine-grained element\") are optional, and \
                                `additionalProperties: false` defines no other \
                                member. `target` and `owner_id` are NOT client \
                                input — the server assigns them from the route \
                                — which is why the write schema omits them; a \
                                body that nonetheless carries them — or any \
                                other undeclared member — is REFUSED `400` \
                                naming the member, because \
                                `additionalProperties: false` is a released \
                                constraint and the ITS-REST docs text is silent \
                                on the write body's member set, so the OAS \
                                grounds it under the documented oracle order. A \
                                member of the wrong JSON type (a numeric \
                                `value`, say) is the same `400` — never a \
                                silently-absent attribute. This is a FULL COLLECTION REPLACE: \
                                tags omitted from the body are removed, and \
                                \"Providing an empty list will effectively \
                                remove all ITEM_TAG associated with the given \
                                target\" (`agent_tags_update.yaml`), so `[]` \
                                is the clear-all form and never an error. \
                                Identity inside the list is the (`key`, \
                                `target_path`) PAIR (\"More than one ITEM_TAG \
                                may be associated with a single target, in \
                                which case they are uniquely identified by \
                                their `key` and `target_path` pair \
                                attributes\"), so two entries may share a \
                                `key` when their `target_path` differs; a \
                                DUPLICATE pair inside one body is resolved \
                                last-wins (no released rule and no \
                                `uniqueItems` — ours, adjudicated). A \
                                `target_path` of `\"\"` normalizes to ABSENT, \
                                the same identity as an entry with no \
                                `target_path` at all: the RM models \
                                `target_path` 0..1 with no non-empty invariant \
                                while all five released `ItemTagOf*` examples \
                                carry `target_path: \"\"` — reconciling the \
                                two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([
                     { "key": "flag", "value": "follow-up" },
                     { "key": "reviewed", "value": "true", "target_path": "/details/items[at0001]/value" }
                 ])),
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
                                         `responses/200_AGENT_ItemTagList_updated.yaml` \
                                         describes itself as \"returned when \
                                         the requested ITEM_TAG list is \
                                         successfully retrieved\", a \
                                         copy-and-paste of its `_retrieved` \
                                         sibling; the trigger is the update, \
                                         as stated here. Items are typed \
                                         `schemas/demographic/ItemTagOfAgent.yaml`.) \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The only response header is \
                                         `Preference-Applied`: a tag \
                                         collection is not change-controlled, \
                                         so there is no `ETag`, no \
                                         `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\").")
            ),
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_updated.yaml`) \
                                      — the DEFAULT branch; a \
                                      `return=identifier` request resolves \
                                      here too. No body and no resource header \
                                      of any kind — no `ETag`, no \
                                      `Last-Modified`, no `Location` — only \
                                      the `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      UUID nor a well-formed three-part \
                                      OBJECT_VERSION_ID, or a body that is not \
                                      parseable JSON / not a JSON ARRAY. A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      An unknown `versioned_object_uid`, a \
                                      version form whose version does not \
                                      exist, and a well-formed uid whose \
                                      stored container is NOT an AGENT \
                                      (another PARTY kind, or a uid from the \
                                      EHR space) are all this `404` — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
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
                                      nothing. The released operation does not \
                                      enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused: \
                                      \"If the service cannot process the \
                                      request payload as JSON format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"). An \
                                      ABSENT `Content-Type` declares nothing \
                                      and is accepted as JSON. The released \
                                      operation does not enumerate `415`; the \
                                      MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The invariants are \
                                      checked before any write, so a rejected \
                                      list leaves the stored collection \
                                      untouched. The released operation \
                                      declares only `400`; answering `422` for \
                                      these SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_tags_update", parts, super::dispatch::dispatch).await
}

/// Delete an `AGENT`'s `ITEM_TAG`s under one key
/// (`DELETE /demographic/agent/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `AGENT` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/agent_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair, the route carries no `target_path` selector, and the
/// released text says "resource(s)" — so every tag under `key` on the addressed
/// collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/demographic/agent/{uid_based_id}/tags/{key}",
    tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to delete \
                        the tags a particular (target) version of the AGENT \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        delete the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/agent_tags_delete.yaml`). \
                        The two forms address DISJOINT collections: an \
                        ITEM_TAG carries exactly one `target` (RM \
                        `item_tag.adoc`: `target: UID_BASED_ID`, \"which may \
                        be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), so a \
                        tag written against the container is invisible to the \
                        version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So deleting a key from the container leaves the same \
                        key on every version untouched, and vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted from the \
                        addressed collection — \"The ITEM_TAG key\" (ITS-REST \
                        `specifications/parameters/path/key.yaml`, `type: \
                        string`), an UNCONSTRAINED string with no format, \
                        pattern or length bound, taken percent-decoded from \
                        the path segment (a key containing `/`, `?` or `#` \
                        must be percent-encoded by the client). It selects a \
                        SET, not one resource: identity is the (`key`, \
                        `target_path`) pair and this route has no \
                        `target_path` selector, so EVERY tag under the key \
                        goes — which is why the released description says \
                        \"Deletes the ITEM_TAG resource(s) identified by \
                        `tag_key`\" (`agent_tags_delete.yaml`). (That \
                        description calls the parameter `tag_key` in prose \
                        while the path parameter is `key` — a released-text \
                        inconsistency, adjudicated; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters has \
                                      been (logically) deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted.yaml`). \
                                      \"(logically) deleted\" is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tags simply cease to exist. No body and \
                                      no headers: an ITEM_TAG has no version \
                                      and no uid, so there is nothing for an \
                                      `ETag`/`Last-Modified` to carry, and \
                                      \"the `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\" (`Requests_and_responses.md` \
                                      §\"Deprecated headers\"). The released \
                                      operation declares no `Accept` either, \
                                      and the empty body negotiates nothing — \
                                      so this route has no `406`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist.\" (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id_or_key.yaml`). \
                                      The SECOND trigger makes this operation \
                                      deliberately NON-IDEMPOTENT on the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      non-existence covers an unknown \
                                      `versioned_object_uid`, a version form \
                                      whose version does not exist, and a \
                                      well-formed uid whose stored container \
                                      is NOT an AGENT (another PARTY kind, or \
                                      a uid from the EHR space) — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn agent_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "agent_tags_delete", parts, super::dispatch::dispatch).await
}

/// Retrieve a `GROUP`'s `ITEM_TAG`s
/// (`GET /demographic/group/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `GROUP` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/group_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag collections — see the
/// parameter.
#[utoipa::path(
    get, path = "/demographic/group/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the GROUP \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        get the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/group_tags_get.yaml`; the \
                        released path parameter file \
                        `parameters/path/uid_based_id.yaml` carries the same \
                        dual-form sentence with VERSIONED_OBJECT in place of \
                        VERSIONED_PARTY). The two forms address DISJOINT \
                        collections: an ITEM_TAG carries exactly one `target` \
                        (RM `item_tag.adoc`: `target: UID_BASED_ID`, \"which \
                        may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), \
                        so a tag written against the container is invisible to \
                        the version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                         OK` \"is returned when the requested \
                                         ITEM_TAG list is successfully \
                                         retrieved.\" (ITS-REST \
                                         `specifications/responses/200_GROUP_ItemTagList_retrieved.yaml`; \
                                         items typed \
                                         `schemas/demographic/ItemTagOfGroup.yaml`). \
                                         \"This will return an empty list when \
                                         there is no ITEM_TAG associated with \
                                         the given target\" \
                                         (`group_tags_get.yaml`) — an \
                                         EXISTING, untagged target is `200 \
                                         []`; a target that does not exist is \
                                         `404`. \"More than one ITEM_TAG may \
                                         be associated with a single target \
                                         GROUP or VERSIONED_PARTY, in which \
                                         case they are uniquely identified by \
                                         their `key` and `target_path` pair \
                                         attributes\". Every row carries the \
                                         SERVER-ASSIGNED `target` and \
                                         `owner_id`; neither is client input. \
                                         `target` names the ADDRESSED \
                                         collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. No `ETag`, `Last-Modified` or \
                                         `Location` accompanies the list: a \
                                         tag collection is not \
                                         change-controlled and has no version \
                                         and no uid.",
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      All of these are that non-existence: an \
                                      unknown `versioned_object_uid`; a \
                                      version form whose version does not \
                                      exist; and a well-formed uid whose \
                                      stored container is NOT a GROUP — \
                                      another PARTY kind, or a \
                                      COMPOSITION/EHR_STATUS/FOLDER uid from \
                                      the EHR space. The kind-checked reading \
                                      is OURS (the released sentence does not \
                                      spell it out) and follows from the route \
                                      naming the target's class — a \
                                      VERSIONED_OBJECT has one type (RM \
                                      `common/master06` §Change Control); it \
                                      is adjudicated. An EXISTING \
                                      target with no tags is `200 []`, never \
                                      `404`.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_tags_get", parts, super::dispatch::dispatch).await
}

/// Replace a `GROUP`'s `ITEM_TAG`s
/// (`PUT /demographic/group/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `GROUP` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/group_tags_update.yaml`). It is a FULL COLLECTION
/// REPLACE of the ADDRESSED collection — the container's or one version's,
/// never both.
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION, mints
/// no version, takes no `If-Match` and no committal headers, and serves neither
/// `ETag` nor `Last-Modified`.
#[utoipa::path(
    put, path = "/demographic/group/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular GROUP version (e.g. one \
                        identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        update the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/group_tags_update.yaml`). \
                        The update sentence sources the HIER_OBJECT_ID from \
                        VERSIONED_OBJECT while the get and delete of the same \
                        family source it from VERSIONED_PARTY, and all three \
                        end on \"the target VERSIONED_PARTY container\" — an \
                        editorial split inside one operation family, \
                        adjudicated; both \
                        name the same container. The two forms address \
                        DISJOINT collections: an ITEM_TAG carries exactly one \
                        `target` (RM `item_tag.adoc`: `target: UID_BASED_ID`, \
                        \"which may be a `VERSIONED_OBJECT<T>` or a \
                        `VERSION<T>`\"), so a tag written against the \
                        container is invisible to the version form and a tag \
                        written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So replacing the container's list never touches any \
                        version's list, and replacing one version's list never \
                        touches the container's or a sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` — the default when the header is \
                        absent (`Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") — answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured: its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid, so the server applies — and \
                        declares — the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied` (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`).",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml`, \
                        enum `application/json` | `application/xml`). The tag \
                        list has no XML and no Simplified-Format shape, so \
                        only `application/json` is processable and any other \
                        declared type is `415`; an ABSENT `Content-Type` \
                        declares nothing and is read as canonical JSON.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                ADDRESSED collection (`required: true`; there \
                                is no envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED (\"Tag key (identifier)\"), `value` \
                                (\"Tag value\") and `target_path` (\"An AQL \
                                path withing the `target` used to tag a \
                                fine-grained element\") are optional, and \
                                `additionalProperties: false` defines no other \
                                member. `target` and `owner_id` are NOT client \
                                input — the server assigns them from the route \
                                — which is why the write schema omits them; a \
                                body that nonetheless carries them — or any \
                                other undeclared member — is REFUSED `400` \
                                naming the member, because \
                                `additionalProperties: false` is a released \
                                constraint and the ITS-REST docs text is silent \
                                on the write body's member set, so the OAS \
                                grounds it under the documented oracle order. A \
                                member of the wrong JSON type (a numeric \
                                `value`, say) is the same `400` — never a \
                                silently-absent attribute. This is a FULL COLLECTION REPLACE: \
                                tags omitted from the body are removed, and \
                                \"Providing an empty list will effectively \
                                remove all ITEM_TAG associated with the given \
                                target\" (`group_tags_update.yaml`), so `[]` \
                                is the clear-all form and never an error. \
                                Identity inside the list is the (`key`, \
                                `target_path`) PAIR (\"More than one ITEM_TAG \
                                may be associated with a single target, in \
                                which case they are uniquely identified by \
                                their `key` and `target_path` pair \
                                attributes\"), so two entries may share a \
                                `key` when their `target_path` differs; a \
                                DUPLICATE pair inside one body is resolved \
                                last-wins (no released rule and no \
                                `uniqueItems` — ours, adjudicated). A \
                                `target_path` of `\"\"` normalizes to ABSENT, \
                                the same identity as an entry with no \
                                `target_path` at all: the RM models \
                                `target_path` 0..1 with no non-empty invariant \
                                while all five released `ItemTagOf*` examples \
                                carry `target_path: \"\"` — reconciling the \
                                two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([
                     { "key": "flag", "value": "follow-up" },
                     { "key": "reviewed", "value": "true", "target_path": "/details/items[at0001]/value" }
                 ])),
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
                                         `responses/200_GROUP_ItemTagList_updated.yaml` \
                                         describes itself as \"returned when \
                                         the requested ITEM_TAG list is \
                                         successfully retrieved\", a \
                                         copy-and-paste of its `_retrieved` \
                                         sibling; the trigger is the update, \
                                         as stated here. Items are typed \
                                         `schemas/demographic/ItemTagOfGroup.yaml`.) \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The only response header is \
                                         `Preference-Applied`: a tag \
                                         collection is not change-controlled, \
                                         so there is no `ETag`, no \
                                         `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\").")
            ),
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_updated.yaml`) \
                                      — the DEFAULT branch; a \
                                      `return=identifier` request resolves \
                                      here too. No body and no resource header \
                                      of any kind — no `ETag`, no \
                                      `Last-Modified`, no `Location` — only \
                                      the `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      UUID nor a well-formed three-part \
                                      OBJECT_VERSION_ID, or a body that is not \
                                      parseable JSON / not a JSON ARRAY. A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      An unknown `versioned_object_uid`, a \
                                      version form whose version does not \
                                      exist, and a well-formed uid whose \
                                      stored container is NOT a GROUP (another \
                                      PARTY kind, or a uid from the EHR space) \
                                      are all this `404` — the kind-checked \
                                      reading being OURS, adjudicated.",
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
                                      nothing. The released operation does not \
                                      enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused: \
                                      \"If the service cannot process the \
                                      request payload as JSON format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"). An \
                                      ABSENT `Content-Type` declares nothing \
                                      and is accepted as JSON. The released \
                                      operation does not enumerate `415`; the \
                                      MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The invariants are \
                                      checked before any write, so a rejected \
                                      list leaves the stored collection \
                                      untouched. The released operation \
                                      declares only `400`; answering `422` for \
                                      these SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_tags_update", parts, super::dispatch::dispatch).await
}

/// Delete a `GROUP`'s `ITEM_TAG`s under one key
/// (`DELETE /demographic/group/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `GROUP` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/group_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair, the route carries no `target_path` selector, and the
/// released text says "resource(s)" — so every tag under `key` on the addressed
/// collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/demographic/group/{uid_based_id}/tags/{key}",
    tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to delete \
                        the tags a particular (target) version of the GROUP \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        delete the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/group_tags_delete.yaml`). \
                        The two forms address DISJOINT collections: an \
                        ITEM_TAG carries exactly one `target` (RM \
                        `item_tag.adoc`: `target: UID_BASED_ID`, \"which may \
                        be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), so a \
                        tag written against the container is invisible to the \
                        version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So deleting a key from the container leaves the same \
                        key on every version untouched, and vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted from the \
                        addressed collection — \"The ITEM_TAG key\" (ITS-REST \
                        `specifications/parameters/path/key.yaml`, `type: \
                        string`), an UNCONSTRAINED string with no format, \
                        pattern or length bound, taken percent-decoded from \
                        the path segment (a key containing `/`, `?` or `#` \
                        must be percent-encoded by the client). It selects a \
                        SET, not one resource: identity is the (`key`, \
                        `target_path`) pair and this route has no \
                        `target_path` selector, so EVERY tag under the key \
                        goes — which is why the released description says \
                        \"Deletes the ITEM_TAG resource(s) identified by \
                        `tag_key`\" (`group_tags_delete.yaml`). (That \
                        description calls the parameter `tag_key` in prose \
                        while the path parameter is `key` — a released-text \
                        inconsistency, adjudicated; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters has \
                                      been (logically) deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted.yaml`). \
                                      \"(logically) deleted\" is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tags simply cease to exist. No body and \
                                      no headers: an ITEM_TAG has no version \
                                      and no uid, so there is nothing for an \
                                      `ETag`/`Last-Modified` to carry, and \
                                      \"the `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\" (`Requests_and_responses.md` \
                                      §\"Deprecated headers\"). The released \
                                      operation declares no `Accept` either, \
                                      and the empty body negotiates nothing — \
                                      so this route has no `406`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist.\" (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id_or_key.yaml`). \
                                      The SECOND trigger makes this operation \
                                      deliberately NON-IDEMPOTENT on the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      non-existence covers an unknown \
                                      `versioned_object_uid`, a version form \
                                      whose version does not exist, and a \
                                      well-formed uid whose stored container \
                                      is NOT a GROUP (another PARTY kind, or a \
                                      uid from the EHR space) — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn group_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "group_tags_delete", parts, super::dispatch::dispatch).await
}

/// Retrieve an `ORGANISATION`'s `ITEM_TAG`s
/// (`GET /demographic/organisation/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `ORGANISATION` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/organisation_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag collections — see the
/// parameter.
#[utoipa::path(
    get, path = "/demographic/organisation/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the \
                        ORGANISATION version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        get the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/organisation_tags_get.yaml`; \
                        the released path parameter file \
                        `parameters/path/uid_based_id.yaml` carries the same \
                        dual-form sentence with VERSIONED_OBJECT in place of \
                        VERSIONED_PARTY). The two forms address DISJOINT \
                        collections: an ITEM_TAG carries exactly one `target` \
                        (RM `item_tag.adoc`: `target: UID_BASED_ID`, \"which \
                        may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), \
                        so a tag written against the container is invisible to \
                        the version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                         OK` \"is returned when the requested \
                                         ITEM_TAG list is successfully \
                                         retrieved.\" (ITS-REST \
                                         `specifications/responses/200_ORGANISATION_ItemTagList_retrieved.yaml`; \
                                         items typed \
                                         `schemas/demographic/ItemTagOfOrganisation.yaml`). \
                                         \"This will return an empty list when \
                                         there is no ITEM_TAG associated with \
                                         the given target\" \
                                         (`organisation_tags_get.yaml`) — an \
                                         EXISTING, untagged target is `200 \
                                         []`; a target that does not exist is \
                                         `404`. \"More than one ITEM_TAG may \
                                         be associated with a single target \
                                         ORGANISATION or VERSIONED_PARTY, in \
                                         which case they are uniquely \
                                         identified by their `key` and \
                                         `target_path` pair attributes\". \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. No `ETag`, `Last-Modified` or \
                                         `Location` accompanies the list: a \
                                         tag collection is not \
                                         change-controlled and has no version \
                                         and no uid.",
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      All of these are that non-existence: an \
                                      unknown `versioned_object_uid`; a \
                                      version form whose version does not \
                                      exist; and a well-formed uid whose \
                                      stored container is NOT an ORGANISATION \
                                      — another PARTY kind, or a \
                                      COMPOSITION/EHR_STATUS/FOLDER uid from \
                                      the EHR space. The kind-checked reading \
                                      is OURS (the released sentence does not \
                                      spell it out) and follows from the route \
                                      naming the target's class — a \
                                      VERSIONED_OBJECT has one type (RM \
                                      `common/master06` §Change Control); it \
                                      is adjudicated. An EXISTING \
                                      target with no tags is `200 []`, never \
                                      `404`.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_tags_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Replace an `ORGANISATION`'s `ITEM_TAG`s
/// (`PUT /demographic/organisation/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `ORGANISATION` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/organisation_tags_update.yaml`). It is
/// a FULL COLLECTION REPLACE of the ADDRESSED collection — the container's or
/// one version's, never both.
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION, mints
/// no version, takes no `If-Match` and no committal headers, and serves neither
/// `ETag` nor `Last-Modified`.
#[utoipa::path(
    put, path = "/demographic/organisation/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular ORGANISATION version (e.g. \
                        one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        update the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/organisation_tags_update.yaml`). \
                        The update sentence sources the HIER_OBJECT_ID from \
                        VERSIONED_OBJECT while the get and delete of the same \
                        family source it from VERSIONED_PARTY, and all three \
                        end on \"the target VERSIONED_PARTY container\" — an \
                        editorial split inside one operation family, \
                        adjudicated; both \
                        name the same container. The two forms address \
                        DISJOINT collections: an ITEM_TAG carries exactly one \
                        `target` (RM `item_tag.adoc`: `target: UID_BASED_ID`, \
                        \"which may be a `VERSIONED_OBJECT<T>` or a \
                        `VERSION<T>`\"), so a tag written against the \
                        container is invisible to the version form and a tag \
                        written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So replacing the container's list never touches any \
                        version's list, and replacing one version's list never \
                        touches the container's or a sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` — the default when the header is \
                        absent (`Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") — answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured: its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid, so the server applies — and \
                        declares — the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied` (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`).",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml`, \
                        enum `application/json` | `application/xml`). The tag \
                        list has no XML and no Simplified-Format shape, so \
                        only `application/json` is processable and any other \
                        declared type is `415`; an ABSENT `Content-Type` \
                        declares nothing and is read as canonical JSON.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                ADDRESSED collection (`required: true`; there \
                                is no envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED (\"Tag key (identifier)\"), `value` \
                                (\"Tag value\") and `target_path` (\"An AQL \
                                path withing the `target` used to tag a \
                                fine-grained element\") are optional, and \
                                `additionalProperties: false` defines no other \
                                member. `target` and `owner_id` are NOT client \
                                input — the server assigns them from the route \
                                — which is why the write schema omits them; a \
                                body that nonetheless carries them — or any \
                                other undeclared member — is REFUSED `400` \
                                naming the member, because \
                                `additionalProperties: false` is a released \
                                constraint and the ITS-REST docs text is silent \
                                on the write body's member set, so the OAS \
                                grounds it under the documented oracle order. A \
                                member of the wrong JSON type (a numeric \
                                `value`, say) is the same `400` — never a \
                                silently-absent attribute. This is a FULL COLLECTION REPLACE: \
                                tags omitted from the body are removed, and \
                                \"Providing an empty list will effectively \
                                remove all ITEM_TAG associated with the given \
                                target\" (`organisation_tags_update.yaml`), so \
                                `[]` is the clear-all form and never an error. \
                                Identity inside the list is the (`key`, \
                                `target_path`) PAIR (\"More than one ITEM_TAG \
                                may be associated with a single target, in \
                                which case they are uniquely identified by \
                                their `key` and `target_path` pair \
                                attributes\"), so two entries may share a \
                                `key` when their `target_path` differs; a \
                                DUPLICATE pair inside one body is resolved \
                                last-wins (no released rule and no \
                                `uniqueItems` — ours, adjudicated). A \
                                `target_path` of `\"\"` normalizes to ABSENT, \
                                the same identity as an entry with no \
                                `target_path` at all: the RM models \
                                `target_path` 0..1 with no non-empty invariant \
                                while all five released `ItemTagOf*` examples \
                                carry `target_path: \"\"` — reconciling the \
                                two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([
                     { "key": "flag", "value": "follow-up" },
                     { "key": "reviewed", "value": "true", "target_path": "/details/items[at0001]/value" }
                 ])),
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
                                         `responses/200_ORGANISATION_ItemTagList_updated.yaml` \
                                         describes itself as \"returned when \
                                         the requested ITEM_TAG list is \
                                         successfully retrieved\", a \
                                         copy-and-paste of its `_retrieved` \
                                         sibling; the trigger is the update, \
                                         as stated here. Items are typed \
                                         `schemas/demographic/ItemTagOfOrganisation.yaml`.) \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The only response header is \
                                         `Preference-Applied`: a tag \
                                         collection is not change-controlled, \
                                         so there is no `ETag`, no \
                                         `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\").")
            ),
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_updated.yaml`) \
                                      — the DEFAULT branch; a \
                                      `return=identifier` request resolves \
                                      here too. No body and no resource header \
                                      of any kind — no `ETag`, no \
                                      `Last-Modified`, no `Location` — only \
                                      the `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      UUID nor a well-formed three-part \
                                      OBJECT_VERSION_ID, or a body that is not \
                                      parseable JSON / not a JSON ARRAY. A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      An unknown `versioned_object_uid`, a \
                                      version form whose version does not \
                                      exist, and a well-formed uid whose \
                                      stored container is NOT an ORGANISATION \
                                      (another PARTY kind, or a uid from the \
                                      EHR space) are all this `404` — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
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
                                      nothing. The released operation does not \
                                      enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused: \
                                      \"If the service cannot process the \
                                      request payload as JSON format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"). An \
                                      ABSENT `Content-Type` declares nothing \
                                      and is accepted as JSON. The released \
                                      operation does not enumerate `415`; the \
                                      MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The invariants are \
                                      checked before any write, so a rejected \
                                      list leaves the stored collection \
                                      untouched. The released operation \
                                      declares only `400`; answering `422` for \
                                      these SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_tags_update",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Delete an `ORGANISATION`'s `ITEM_TAG`s under one key
/// (`DELETE /demographic/organisation/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `ORGANISATION` version or `VERSIONED_PARTY` identified by
/// `uid_based_id`" (ITS-REST
/// `specifications/operations/organisation_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair, the route carries no `target_path` selector, and the
/// released text says "resource(s)" — so every tag under `key` on the addressed
/// collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/demographic/organisation/{uid_based_id}/tags/{key}",
    tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to delete \
                        the tags a particular (target) version of the \
                        ORGANISATION version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        delete the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/organisation_tags_delete.yaml`). \
                        The two forms address DISJOINT collections: an \
                        ITEM_TAG carries exactly one `target` (RM \
                        `item_tag.adoc`: `target: UID_BASED_ID`, \"which may \
                        be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), so a \
                        tag written against the container is invisible to the \
                        version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So deleting a key from the container leaves the same \
                        key on every version untouched, and vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted from the \
                        addressed collection — \"The ITEM_TAG key\" (ITS-REST \
                        `specifications/parameters/path/key.yaml`, `type: \
                        string`), an UNCONSTRAINED string with no format, \
                        pattern or length bound, taken percent-decoded from \
                        the path segment (a key containing `/`, `?` or `#` \
                        must be percent-encoded by the client). It selects a \
                        SET, not one resource: identity is the (`key`, \
                        `target_path`) pair and this route has no \
                        `target_path` selector, so EVERY tag under the key \
                        goes — which is why the released description says \
                        \"Deletes the ITEM_TAG resource(s) identified by \
                        `tag_key`\" (`organisation_tags_delete.yaml`). (That \
                        description calls the parameter `tag_key` in prose \
                        while the path parameter is `key` — a released-text \
                        inconsistency, adjudicated; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters has \
                                      been (logically) deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted.yaml`). \
                                      \"(logically) deleted\" is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tags simply cease to exist. No body and \
                                      no headers: an ITEM_TAG has no version \
                                      and no uid, so there is nothing for an \
                                      `ETag`/`Last-Modified` to carry, and \
                                      \"the `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\" (`Requests_and_responses.md` \
                                      §\"Deprecated headers\"). The released \
                                      operation declares no `Accept` either, \
                                      and the empty body negotiates nothing — \
                                      so this route has no `406`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist.\" (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id_or_key.yaml`). \
                                      The SECOND trigger makes this operation \
                                      deliberately NON-IDEMPOTENT on the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      non-existence covers an unknown \
                                      `versioned_object_uid`, a version form \
                                      whose version does not exist, and a \
                                      well-formed uid whose stored container \
                                      is NOT an ORGANISATION (another PARTY \
                                      kind, or a uid from the EHR space) — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn organisation_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "organisation_tags_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a `PERSON`'s `ITEM_TAG`s
/// (`GET /demographic/person/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `PERSON` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/person_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag collections — see the
/// parameter.
#[utoipa::path(
    get, path = "/demographic/person/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the PERSON \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        get the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/person_tags_get.yaml`; the \
                        released path parameter file \
                        `parameters/path/uid_based_id.yaml` carries the same \
                        dual-form sentence with VERSIONED_OBJECT in place of \
                        VERSIONED_PARTY). The two forms address DISJOINT \
                        collections: an ITEM_TAG carries exactly one `target` \
                        (RM `item_tag.adoc`: `target: UID_BASED_ID`, \"which \
                        may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), \
                        so a tag written against the container is invisible to \
                        the version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                         OK` \"is returned when the requested \
                                         ITEM_TAG list is successfully \
                                         retrieved.\" (ITS-REST \
                                         `specifications/responses/200_PERSON_ItemTagList_retrieved.yaml`; \
                                         items typed \
                                         `schemas/demographic/ItemTagOfPerson.yaml`). \
                                         \"This will return an empty list when \
                                         there is no ITEM_TAG associated with \
                                         the given target\" \
                                         (`person_tags_get.yaml`) — an \
                                         EXISTING, untagged target is `200 \
                                         []`; a target that does not exist is \
                                         `404`. \"More than one ITEM_TAG may \
                                         be associated with a single target \
                                         PERSON or VERSIONED_PARTY, in which \
                                         case they are uniquely identified by \
                                         their `key` and `target_path` pair \
                                         attributes\". Every row carries the \
                                         SERVER-ASSIGNED `target` and \
                                         `owner_id`; neither is client input. \
                                         `target` names the ADDRESSED \
                                         collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. No `ETag`, `Last-Modified` or \
                                         `Location` accompanies the list: a \
                                         tag collection is not \
                                         change-controlled and has no version \
                                         and no uid.",
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      All of these are that non-existence: an \
                                      unknown `versioned_object_uid`; a \
                                      version form whose version does not \
                                      exist; and a well-formed uid whose \
                                      stored container is NOT a PERSON — \
                                      another PARTY kind, or a \
                                      COMPOSITION/EHR_STATUS/FOLDER uid from \
                                      the EHR space. The kind-checked reading \
                                      is OURS (the released sentence does not \
                                      spell it out) and follows from the route \
                                      naming the target's class — a \
                                      VERSIONED_OBJECT has one type (RM \
                                      `common/master06` §Change Control); it \
                                      is adjudicated. An EXISTING \
                                      target with no tags is `200 []`, never \
                                      `404`.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "person_tags_get", parts, super::dispatch::dispatch).await
}

/// Replace a `PERSON`'s `ITEM_TAG`s
/// (`PUT /demographic/person/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `PERSON` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/person_tags_update.yaml`). It is a FULL
/// COLLECTION REPLACE of the ADDRESSED collection — the container's or one
/// version's, never both.
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION, mints
/// no version, takes no `If-Match` and no committal headers, and serves neither
/// `ETag` nor `Last-Modified`.
#[utoipa::path(
    put, path = "/demographic/person/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular PERSON version (e.g. one \
                        identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        update the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/person_tags_update.yaml`). \
                        The update sentence sources the HIER_OBJECT_ID from \
                        VERSIONED_OBJECT while the get and delete of the same \
                        family source it from VERSIONED_PARTY, and all three \
                        end on \"the target VERSIONED_PARTY container\" — an \
                        editorial split inside one operation family, \
                        adjudicated; both \
                        name the same container. The two forms address \
                        DISJOINT collections: an ITEM_TAG carries exactly one \
                        `target` (RM `item_tag.adoc`: `target: UID_BASED_ID`, \
                        \"which may be a `VERSIONED_OBJECT<T>` or a \
                        `VERSION<T>`\"), so a tag written against the \
                        container is invisible to the version form and a tag \
                        written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So replacing the container's list never touches any \
                        version's list, and replacing one version's list never \
                        touches the container's or a sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` — the default when the header is \
                        absent (`Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") — answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured: its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid, so the server applies — and \
                        declares — the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied` (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`).",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml`, \
                        enum `application/json` | `application/xml`). The tag \
                        list has no XML and no Simplified-Format shape, so \
                        only `application/json` is processable and any other \
                        declared type is `415`; an ABSENT `Content-Type` \
                        declares nothing and is read as canonical JSON.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                ADDRESSED collection (`required: true`; there \
                                is no envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED (\"Tag key (identifier)\"), `value` \
                                (\"Tag value\") and `target_path` (\"An AQL \
                                path withing the `target` used to tag a \
                                fine-grained element\") are optional, and \
                                `additionalProperties: false` defines no other \
                                member. `target` and `owner_id` are NOT client \
                                input — the server assigns them from the route \
                                — which is why the write schema omits them; a \
                                body that nonetheless carries them — or any \
                                other undeclared member — is REFUSED `400` \
                                naming the member, because \
                                `additionalProperties: false` is a released \
                                constraint and the ITS-REST docs text is silent \
                                on the write body's member set, so the OAS \
                                grounds it under the documented oracle order. A \
                                member of the wrong JSON type (a numeric \
                                `value`, say) is the same `400` — never a \
                                silently-absent attribute. This is a FULL COLLECTION REPLACE: \
                                tags omitted from the body are removed, and \
                                \"Providing an empty list will effectively \
                                remove all ITEM_TAG associated with the given \
                                target\" (`person_tags_update.yaml`), so `[]` \
                                is the clear-all form and never an error. \
                                Identity inside the list is the (`key`, \
                                `target_path`) PAIR (\"More than one ITEM_TAG \
                                may be associated with a single target, in \
                                which case they are uniquely identified by \
                                their `key` and `target_path` pair \
                                attributes\"), so two entries may share a \
                                `key` when their `target_path` differs; a \
                                DUPLICATE pair inside one body is resolved \
                                last-wins (no released rule and no \
                                `uniqueItems` — ours, adjudicated). A \
                                `target_path` of `\"\"` normalizes to ABSENT, \
                                the same identity as an entry with no \
                                `target_path` at all: the RM models \
                                `target_path` 0..1 with no non-empty invariant \
                                while all five released `ItemTagOf*` examples \
                                carry `target_path: \"\"` — reconciling the \
                                two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([
                     { "key": "flag", "value": "follow-up" },
                     { "key": "reviewed", "value": "true", "target_path": "/details/items[at0001]/value" }
                 ])),
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
                                         `responses/200_PERSON_ItemTagList_updated.yaml` \
                                         describes itself as \"returned when \
                                         the requested ITEM_TAG list is \
                                         successfully retrieved\", a \
                                         copy-and-paste of its `_retrieved` \
                                         sibling; the trigger is the update, \
                                         as stated here. Items are typed \
                                         `schemas/demographic/ItemTagOfPerson.yaml`.) \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The only response header is \
                                         `Preference-Applied`: a tag \
                                         collection is not change-controlled, \
                                         so there is no `ETag`, no \
                                         `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\").")
            ),
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_updated.yaml`) \
                                      — the DEFAULT branch; a \
                                      `return=identifier` request resolves \
                                      here too. No body and no resource header \
                                      of any kind — no `ETag`, no \
                                      `Last-Modified`, no `Location` — only \
                                      the `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      UUID nor a well-formed three-part \
                                      OBJECT_VERSION_ID, or a body that is not \
                                      parseable JSON / not a JSON ARRAY. A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      An unknown `versioned_object_uid`, a \
                                      version form whose version does not \
                                      exist, and a well-formed uid whose \
                                      stored container is NOT a PERSON \
                                      (another PARTY kind, or a uid from the \
                                      EHR space) are all this `404` — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
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
                                      nothing. The released operation does not \
                                      enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused: \
                                      \"If the service cannot process the \
                                      request payload as JSON format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"). An \
                                      ABSENT `Content-Type` declares nothing \
                                      and is accepted as JSON. The released \
                                      operation does not enumerate `415`; the \
                                      MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The invariants are \
                                      checked before any write, so a rejected \
                                      list leaves the stored collection \
                                      untouched. The released operation \
                                      declares only `400`; answering `422` for \
                                      these SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "person_tags_update",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Delete a `PERSON`'s `ITEM_TAG`s under one key
/// (`DELETE /demographic/person/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `PERSON` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/person_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair, the route carries no `target_path` selector, and the
/// released text says "resource(s)" — so every tag under `key` on the addressed
/// collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/demographic/person/{uid_based_id}/tags/{key}",
    tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to delete \
                        the tags a particular (target) version of the PERSON \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        delete the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/person_tags_delete.yaml`). \
                        The two forms address DISJOINT collections: an \
                        ITEM_TAG carries exactly one `target` (RM \
                        `item_tag.adoc`: `target: UID_BASED_ID`, \"which may \
                        be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), so a \
                        tag written against the container is invisible to the \
                        version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So deleting a key from the container leaves the same \
                        key on every version untouched, and vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted from the \
                        addressed collection — \"The ITEM_TAG key\" (ITS-REST \
                        `specifications/parameters/path/key.yaml`, `type: \
                        string`), an UNCONSTRAINED string with no format, \
                        pattern or length bound, taken percent-decoded from \
                        the path segment (a key containing `/`, `?` or `#` \
                        must be percent-encoded by the client). It selects a \
                        SET, not one resource: identity is the (`key`, \
                        `target_path`) pair and this route has no \
                        `target_path` selector, so EVERY tag under the key \
                        goes — which is why the released description says \
                        \"Deletes the ITEM_TAG resource(s) identified by \
                        `tag_key`\" (`person_tags_delete.yaml`). (That \
                        description calls the parameter `tag_key` in prose \
                        while the path parameter is `key` — a released-text \
                        inconsistency, adjudicated; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters has \
                                      been (logically) deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted.yaml`). \
                                      \"(logically) deleted\" is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tags simply cease to exist. No body and \
                                      no headers: an ITEM_TAG has no version \
                                      and no uid, so there is nothing for an \
                                      `ETag`/`Last-Modified` to carry, and \
                                      \"the `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\" (`Requests_and_responses.md` \
                                      §\"Deprecated headers\"). The released \
                                      operation declares no `Accept` either, \
                                      and the empty body negotiates nothing — \
                                      so this route has no `406`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist.\" (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id_or_key.yaml`). \
                                      The SECOND trigger makes this operation \
                                      deliberately NON-IDEMPOTENT on the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      non-existence covers an unknown \
                                      `versioned_object_uid`, a version form \
                                      whose version does not exist, and a \
                                      well-formed uid whose stored container \
                                      is NOT a PERSON (another PARTY kind, or \
                                      a uid from the EHR space) — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn person_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "person_tags_delete",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a `ROLE`'s `ITEM_TAG`s
/// (`GET /demographic/role/{uid_based_id}/tags`).
///
/// "Retrieves the list of all `ITEM_TAG` resources associated with a given target
/// `ROLE` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/role_tags_get.yaml`).
///
/// The two `uid_based_id` forms address DISJOINT tag collections — see the
/// parameter.
#[utoipa::path(
    get, path = "/demographic/role/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to get the \
                        tags of a particular (target) version of the ROLE \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        get the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/role_tags_get.yaml`; the \
                        released path parameter file \
                        `parameters/path/uid_based_id.yaml` carries the same \
                        dual-form sentence with VERSIONED_OBJECT in place of \
                        VERSIONED_PARTY). The two forms address DISJOINT \
                        collections: an ITEM_TAG carries exactly one `target` \
                        (RM `item_tag.adoc`: `target: UID_BASED_ID`, \"which \
                        may be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), \
                        so a tag written against the container is invisible to \
                        the version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                         OK` \"is returned when the requested \
                                         ITEM_TAG list is successfully \
                                         retrieved.\" (ITS-REST \
                                         `specifications/responses/200_ROLE_ItemTagList_retrieved.yaml`; \
                                         items typed \
                                         `schemas/demographic/ItemTagOfRole.yaml`). \
                                         \"This will return an empty list when \
                                         there is no ITEM_TAG associated with \
                                         the given target\" \
                                         (`role_tags_get.yaml`) — an EXISTING, \
                                         untagged target is `200 []`; a target \
                                         that does not exist is `404`. \"More \
                                         than one ITEM_TAG may be associated \
                                         with a single target ROLE or \
                                         VERSIONED_PARTY, in which case they \
                                         are uniquely identified by their \
                                         `key` and `target_path` pair \
                                         attributes\". Every row carries the \
                                         SERVER-ASSIGNED `target` and \
                                         `owner_id`; neither is client input. \
                                         `target` names the ADDRESSED \
                                         collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. No `ETag`, `Last-Modified` or \
                                         `Location` accompanies the list: a \
                                         tag collection is not \
                                         change-controlled and has no version \
                                         and no uid.",
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      All of these are that non-existence: an \
                                      unknown `versioned_object_uid`; a \
                                      version form whose version does not \
                                      exist; and a well-formed uid whose \
                                      stored container is NOT a ROLE — another \
                                      PARTY kind, or a \
                                      COMPOSITION/EHR_STATUS/FOLDER uid from \
                                      the EHR space. The kind-checked reading \
                                      is OURS (the released sentence does not \
                                      spell it out) and follows from the route \
                                      naming the target's class — a \
                                      VERSIONED_OBJECT has one type (RM \
                                      `common/master06` §Change Control); it \
                                      is adjudicated. An EXISTING \
                                      target with no tags is `200 []`, never \
                                      `404`.",
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
                                      Acceptable`\"). The released operation \
                                      does not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_tags_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_tags_get", parts, super::dispatch::dispatch).await
}

/// Replace a `ROLE`'s `ITEM_TAG`s
/// (`PUT /demographic/role/{uid_based_id}/tags`).
///
/// "Updates the list of all `ITEM_TAG` resources associated with a given target
/// `ROLE` version or `VERSIONED_PARTY` identified by `uid_based_id`" (ITS-REST
/// `specifications/operations/role_tags_update.yaml`). It is a FULL COLLECTION
/// REPLACE of the ADDRESSED collection — the container's or one version's,
/// never both.
///
/// Tags are not change-controlled, so this write commits no CONTRIBUTION, mints
/// no version, takes no `If-Match` and no committal headers, and serves neither
/// `ETag` nor `Last-Modified`.
#[utoipa::path(
    put, path = "/demographic/role/{uid_based_id}/tags", tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_OBJECT.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to update \
                        the tags of a particular ROLE version (e.g. one \
                        identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        update the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/role_tags_update.yaml`). \
                        The update sentence sources the HIER_OBJECT_ID from \
                        VERSIONED_OBJECT while the get and delete of the same \
                        family source it from VERSIONED_PARTY, and all three \
                        end on \"the target VERSIONED_PARTY container\" — an \
                        editorial split inside one operation family, \
                        adjudicated; both \
                        name the same container. The two forms address \
                        DISJOINT collections: an ITEM_TAG carries exactly one \
                        `target` (RM `item_tag.adoc`: `target: UID_BASED_ID`, \
                        \"which may be a `VERSIONED_OBJECT<T>` or a \
                        `VERSION<T>`\"), so a tag written against the \
                        container is invisible to the version form and a tag \
                        written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So replacing the container's list never touches any \
                        version's list, and replacing one version's list never \
                        touches the container's or a sibling version's.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("Prefer" = Option<String>, Header,
         description = "`return=minimal` — the default when the header is \
                        absent (`Requests_and_responses.md` §\"Representation \
                        details negotiation\": \"If no `Prefer` header is \
                        provided, the default behavior is assumed to be \
                        `return=minimal`\") — answers `204 No Content`; \
                        `return=representation` answers `200` with the full \
                        RESULTING tag list of the addressed collection. \
                        `return=identifier` cannot be honoured: its released \
                        contract is a body carrying \"only the identifier \
                        (e.g., the `uid`) of the affected resource\" and an \
                        ITEM_TAG has no uid, so the server applies — and \
                        declares — the default `return=minimal`; that \
                        resolution is OURS, adjudicated. Whichever branch runs, the \
                        response states it in `Preference-Applied` (ITS-REST \
                        `specifications/parameters/header/Prefer.yaml`).",
         example = "return=representation"),
        ("Content-Type" = Option<String>, Header,
         description = "The canonical payload format (ITS-REST \
                        `specifications/parameters/header/ContentType_canonical.yaml`, \
                        enum `application/json` | `application/xml`). The tag \
                        list has no XML and no Simplified-Format shape, so \
                        only `application/json` is processable and any other \
                        declared type is `415`; an ABSENT `Content-Type` \
                        declares nothing and is read as canonical JSON.",
         example = "application/json"),
        ("Accept" = Option<String>, Header,
         description = "The canonical response format (ITS-REST \
                        `specifications/parameters/header/Accept_canonical.yaml`, \
                        enum `application/json` | `application/xml`). An \
                        ITEM_TAG list is served as `application/json` only — \
                        the canonical XML ITS defines no ITEM_TAG type, so the \
                        enum's `application/xml` member is stalled shape on \
                        this operation and asking for it is `406`.",
         example = "application/json")
    ),
    request_body(content = serde_json::Value,
                 description = "A BARE JSON ARRAY of UPDATE_ITEM_TAG objects — \
                                the complete tag list to associate with the \
                                ADDRESSED collection (`required: true`; there \
                                is no envelope object). Per the released \
                                `schemas/common/UpdateItemTag.yaml`: `key` is \
                                REQUIRED (\"Tag key (identifier)\"), `value` \
                                (\"Tag value\") and `target_path` (\"An AQL \
                                path withing the `target` used to tag a \
                                fine-grained element\") are optional, and \
                                `additionalProperties: false` defines no other \
                                member. `target` and `owner_id` are NOT client \
                                input — the server assigns them from the route \
                                — which is why the write schema omits them; a \
                                body that nonetheless carries them — or any \
                                other undeclared member — is REFUSED `400` \
                                naming the member, because \
                                `additionalProperties: false` is a released \
                                constraint and the ITS-REST docs text is silent \
                                on the write body's member set, so the OAS \
                                grounds it under the documented oracle order. A \
                                member of the wrong JSON type (a numeric \
                                `value`, say) is the same `400` — never a \
                                silently-absent attribute. This is a FULL COLLECTION REPLACE: \
                                tags omitted from the body are removed, and \
                                \"Providing an empty list will effectively \
                                remove all ITEM_TAG associated with the given \
                                target\" (`role_tags_update.yaml`), so `[]` is \
                                the clear-all form and never an error. \
                                Identity inside the list is the (`key`, \
                                `target_path`) PAIR (\"More than one ITEM_TAG \
                                may be associated with a single target, in \
                                which case they are uniquely identified by \
                                their `key` and `target_path` pair \
                                attributes\"), so two entries may share a \
                                `key` when their `target_path` differs; a \
                                DUPLICATE pair inside one body is resolved \
                                last-wins (no released rule and no \
                                `uniqueItems` — ours, adjudicated). A \
                                `target_path` of `\"\"` normalizes to ABSENT, \
                                the same identity as an entry with no \
                                `target_path` at all: the RM models \
                                `target_path` 0..1 with no non-empty invariant \
                                while all five released `ItemTagOf*` examples \
                                carry `target_path: \"\"` — reconciling the \
                                two on one identity is ours, \
                                adjudicated. Canonical JSON only: an \
                                XML (or Simplified-Format) `Content-Type` is \
                                `415`.",
                 example = json!([
                     { "key": "flag", "value": "follow-up" },
                     { "key": "reviewed", "value": "true", "target_path": "/details/items[at0001]/value" }
                 ])),
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
                                         `responses/200_ROLE_ItemTagList_updated.yaml` \
                                         describes itself as \"returned when \
                                         the requested ITEM_TAG list is \
                                         successfully retrieved\", a \
                                         copy-and-paste of its `_retrieved` \
                                         sibling; the trigger is the update, \
                                         as stated here. Items are typed \
                                         `schemas/demographic/ItemTagOfRole.yaml`.) \
                                         Every row carries the SERVER-ASSIGNED \
                                         `target` and `owner_id`; neither is \
                                         client input. `target` names the \
                                         ADDRESSED collection — `{namespace: \
                                         \"demographic\", type: <PARTY kind>, \
                                         id: <the addressed uid>}`, whose `id` \
                                         is a `HIER_OBJECT_ID` for the \
                                         container form and an \
                                         `OBJECT_VERSION_ID` for the version \
                                         form — and `owner_id` names the \
                                         owning VERSIONED_PARTY. That \
                                         `owner_id` is OUR OWN DESIGN: RM \
                                         `item_tag.adoc` says only \
                                         \"Identifier of owner object, such as \
                                         EHR\" and a demographic party has no \
                                         EHR, so no released sentence fixes \
                                         it; the released `ItemTagOf*` \
                                         examples show `{namespace: local, \
                                         type: SYSTEM}` instead, which nothing \
                                         requires. The position is \
                                         adjudicated. `target_path` \
                                         is present only on tags that carry \
                                         one — it is 0..1 in the RM — and the \
                                         empty string normalizes to ABSENT, so \
                                         a stored tag never echoes the \
                                         `target_path: \"\"` the released \
                                         `ItemTagOf*` examples all show; that \
                                         reconciliation is adjudicated \
                                         too. The only response header is \
                                         `Preference-Applied`: a tag \
                                         collection is not change-controlled, \
                                         so there is no `ETag`, no \
                                         `Last-Modified` and no `Location`.",
            headers(
                ("Preference-Applied" = String,
                 description = "`return=representation` — the honoured \
                                preference (`Requests_and_responses.md` \
                                §\"Representation details negotiation\": the \
                                service MAY include this header \"to indicate \
                                that the client's preference has been \
                                honored\").")
            ),
            content((serde_json::Value = "application/json", example =
                json!([
                    {
                        "_type": "ITEM_TAG",
                        "key": "flag",
                        "value": "follow-up",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    },
                    {
                        "_type": "ITEM_TAG",
                        "key": "reviewed",
                        "value": "true",
                        "target_path": "/details/items[at0001]/value",
                        "target": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
                        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "SYSTEM", "id": { "_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com" } }
                    }
                ])
            ))
        ),
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the update \
                                      operation was successful and the \
                                      `Prefer` header is missing or is set to \
                                      `return=minimal`.\" (ITS-REST \
                                      `specifications/responses/204_updated.yaml`) \
                                      — the DEFAULT branch; a \
                                      `return=identifier` request resolves \
                                      here too. No body and no resource header \
                                      of any kind — no `ETag`, no \
                                      `Last-Modified`, no `Location` — only \
                                      the `Preference-Applied` declaration.",
         headers(
             ("Preference-Applied" = String,
              description = "`return=minimal` — the applied preference, \
                             including when the request asked for \
                             `return=identifier` (an ITEM_TAG has no uid to \
                             return).")
         )),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter \
                                      or content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      UUID nor a well-formed three-part \
                                      OBJECT_VERSION_ID, or a body that is not \
                                      parseable JSON / not a JSON ARRAY. A \
                                      well-formed array whose entries break an \
                                      ITEM_TAG rule is `422`, not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist.\" \
                                      (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id.yaml`). \
                                      An unknown `versioned_object_uid`, a \
                                      version form whose version does not \
                                      exist, and a well-formed uid whose \
                                      stored container is NOT a ROLE (another \
                                      PARTY kind, or a uid from the EHR space) \
                                      are all this `404` — the kind-checked \
                                      reading being OURS, adjudicated.",
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
                                      nothing. The released operation does not \
                                      enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      canonical JSON. The tag list has no XML \
                                      and no Simplified-Format shape, so any \
                                      other declared media type is refused: \
                                      \"If the service cannot process the \
                                      request payload as JSON format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" \
                                      (`Resources.md` §\"JSON Format\"). An \
                                      ABSENT `Content-Type` declares nothing \
                                      and is accepted as JSON. The released \
                                      operation does not enumerate `415`; the \
                                      MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 422, description = "The body is well-formed but an entry \
                                      breaks an ITEM_TAG rule: a missing or \
                                      empty `key`, a `key` with leading or \
                                      trailing whitespace (RM `item_tag.adoc` \
                                      __Inv_key_valid__: \"not key.is_empty \
                                      and key.is_justified\"), or an EMPTY \
                                      `value` (__Inv_value_valid__: \"value /= \
                                      Void implies not value.is_empty\" — omit \
                                      the member instead). The invariants are \
                                      checked before any write, so a rejected \
                                      list leaves the stored collection \
                                      untouched. The released operation \
                                      declares only `400`; answering `422` for \
                                      these SEMANTIC failures follows \
                                      `Requests_and_responses.md` §\"HTTP \
                                      status codes\", the `422` row (\"The \
                                      request was well-formed but was unable \
                                      to be followed due to semantic errors\") \
                                      and is OURS, adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_tags_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_tags_update", parts, super::dispatch::dispatch).await
}

/// Delete a `ROLE`'s `ITEM_TAG`s under one key
/// (`DELETE /demographic/role/{uid_based_id}/tags/{key}`).
///
/// "Deletes the `ITEM_TAG` resource(s) identified by `tag_key`, associated with a
/// given target `ROLE` version or `VERSIONED_PARTY` identified by `uid_based_id`"
/// (ITS-REST `specifications/operations/role_tags_delete.yaml`).
///
/// A SET delete, not a single-resource delete: `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair, the route carries no `target_path` selector, and the
/// released text says "resource(s)" — so every tag under `key` on the addressed
/// collection goes, however many paths they carry.
#[utoipa::path(
    delete, path = "/demographic/role/{uid_based_id}/tags/{key}",
    tag = "ITEM_TAG",
    params(
        ("uid_based_id" = String, Path,
         description = "The released parameter, verbatim: \"The `uid_based_id` \
                        can take a form of an OBJECT_VERSION_ID identifier \
                        taken from VERSION.uid.value (i.e. a `version_uid`), \
                        or a form of a HIER_OBJECT_ID identifier taken from \
                        VERSIONED_PARTY.uid.value (i.e. a \
                        `versioned_object_uid`). The former is used to delete \
                        the tags a particular (target) version of the ROLE \
                        version (e.g. one identified by \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1`), \
                        whereas the latter (e.g. an identifier like \
                        `8849182c-82ad-4088-a07f-48ead4180515`) is be used to \
                        delete the tags of the target VERSIONED_PARTY \
                        container.\" (ITS-REST \
                        `specifications/operations/role_tags_delete.yaml`). \
                        The two forms address DISJOINT collections: an \
                        ITEM_TAG carries exactly one `target` (RM \
                        `item_tag.adoc`: `target: UID_BASED_ID`, \"which may \
                        be a `VERSIONED_OBJECT<T>` or a `VERSION<T>`\"), so a \
                        tag written against the container is invisible to the \
                        version form and a tag written against \
                        `8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1` \
                        is invisible both to the container form and to every \
                        other version. The container form names the \
                        VERSIONED_PARTY's OWN tag collection, not the latest \
                        version's — there is no implicit-latest reading here. \
                        So deleting a key from the container leaves the same \
                        key on every version untouched, and vice versa.",
         example = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"),
        ("key" = String, Path,
         description = "The ITEM_TAG `key` whose tags are deleted from the \
                        addressed collection — \"The ITEM_TAG key\" (ITS-REST \
                        `specifications/parameters/path/key.yaml`, `type: \
                        string`), an UNCONSTRAINED string with no format, \
                        pattern or length bound, taken percent-decoded from \
                        the path segment (a key containing `/`, `?` or `#` \
                        must be percent-encoded by the client). It selects a \
                        SET, not one resource: identity is the (`key`, \
                        `target_path`) pair and this route has no \
                        `target_path` selector, so EVERY tag under the key \
                        goes — which is why the released description says \
                        \"Deletes the ITEM_TAG resource(s) identified by \
                        `tag_key`\" (`role_tags_delete.yaml`). (That \
                        description calls the parameter `tag_key` in prose \
                        while the path parameter is `key` — a released-text \
                        inconsistency, adjudicated; the wire name is `key`.)",
         example = "flag")
    ),
    responses(
        (status = 204, description = "The released trigger, verbatim: `204 No \
                                      Content` \"is returned when the resource \
                                      identified by the request parameters has \
                                      been (logically) deleted.\" (ITS-REST \
                                      `specifications/responses/204_deleted.yaml`). \
                                      \"(logically) deleted\" is \
                                      change-control vocabulary that cannot \
                                      apply here — a tag is not \
                                      change-controlled, so removal is plain: \
                                      no deleted version is committed and the \
                                      tags simply cease to exist. No body and \
                                      no headers: an ITEM_TAG has no version \
                                      and no uid, so there is nothing for an \
                                      `ETag`/`Last-Modified` to carry, and \
                                      \"the `Location` response header was \
                                      deprecated from responses of `DELETE` \
                                      methods\" (`Requests_and_responses.md` \
                                      §\"Deprecated headers\"). The released \
                                      operation declares no `Accept` either, \
                                      and the empty body negotiates nothing — \
                                      so this route has no `406`."),
        (status = 400, description = "The released cross-cutting trigger, \
                                      verbatim: `400 Bad Request` \"is \
                                      returned when the request could not be \
                                      parsed or is invalid (e.g. malformed \
                                      request URL syntax, missing required \
                                      header or parameter, or syntactically \
                                      invalid header, parameter or content)\" \
                                      (ITS-REST \
                                      `specifications/responses/400.yaml`). \
                                      Here: a `uid_based_id` that is neither a \
                                      HIER_OBJECT_ID (a UUID) nor a \
                                      well-formed three-part \
                                      OBJECT_VERSION_ID. A well-formed \
                                      identifier that names nothing is `404`, \
                                      not `400`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when the \
                                      `uid_based_id` does not exist, or when \
                                      the ITEM_TAG identified by the `key` \
                                      does not exist.\" (ITS-REST \
                                      `specifications/responses/404_unknown_uid_based_id_or_key.yaml`). \
                                      The SECOND trigger makes this operation \
                                      deliberately NON-IDEMPOTENT on the wire: \
                                      the second identical `DELETE` answers \
                                      `404`, because after the first one no \
                                      ITEM_TAG under that key exists on the \
                                      addressed collection. A key that exists \
                                      only on the OTHER collection of the same \
                                      versioned object (container vs version) \
                                      does not exist here either. Target \
                                      non-existence covers an unknown \
                                      `versioned_object_uid`, a version form \
                                      whose version does not exist, and a \
                                      well-formed uid whose stored container \
                                      is NOT a ROLE (another PARTY kind, or a \
                                      uid from the EHR space) — the \
                                      kind-checked reading being OURS, \
                                      adjudicated.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn role_tags_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "role_tags_delete", parts, super::dispatch::dispatch).await
}
