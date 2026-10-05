// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native `utoipa-axum` routing for the Definition API group (ADL 1.4 / ADL 2
//! templates + stored queries).
//!
//! Operation semantics are the ITS-REST Definition API
//! (`docs/specs/openehr/ITS-REST`); no openEHR spec governs the OAS layout. Each
//! handler forwards to the group dispatcher through [`guarded_dispatch`].
//!
//! NOTE (operation ids): a few generated operation ids carry `.` (e.g.
//! `definition_template_adl1.4_list`, `definition_query_store.yaml`) — invalid
//! Rust identifiers, so the handler fn names sanitise `.` to `_` while the op
//! string passed to the dispatcher is the verbatim generated id it matches on.
//!
//! ## Prose-vs-OAS reconciliations (documented real wire, per handler)
//!
//! - **ADL 2 upload** (`definition_template_adl2_upload.yaml`): the OAS declares
//!   `201/400/409`; the REST adapter emits the `409` on a duplicate HRID (the
//!   native SM `upload_artefact` would replace — this seam does not), and
//!   additionally returns `422` when the ADL2/AOM2 artefact is invalid on the
//!   store path (the OAS folds that under `400`). A request DECLARING a media
//!   type other than the operation's single `text/plain` body type is refused
//!   `415` before parsing — the same cross-cutting rule that grounds the ADL 1.4
//!   guard (`specifications/docs/overview/Resources.md` §"XML Format"/§"JSON
//!   Format": a payload the service cannot process as the declared format "MUST
//!   respond with HTTP status code `415 Unsupported Media Type`"); an absent
//!   `Content-Type` declares nothing to refuse (the header is a client MAY). The
//!   `at_version` query parameter is `deprecated: true` and is dropped
//!   (spec-permitted); only `Prefer` is honoured.
//! - **ADL 2 get / version get** (`definition_template_adl2_get.yaml`,
//!   `_version_get.yaml` — the latter `deprecated: true`): the OAS lists a `400`
//!   that is unreachable here, since `template_id` and `version` are path
//!   segments. The build serves the `text/plain` ADL2 source and the
//!   `application/json` `OperationalTemplateV2` projection, and declares no
//!   `application/xml` body, so an `Accept` naming only XML is a `406`.
//! - **ADL 2 example get** (`_example_get`): the stored template is compiled to
//!   its operational template and turned into a Web Template, which the shared
//!   example generator walks into an example COMPOSITION, served across the four
//!   `Accept_LOCATABLE` forms exactly as the ADL 1.4 example endpoint.
//! - **ADL 1.4 upload** (`definition_template_adl1.4_upload.yaml`): the OAS
//!   declares `201/400/409`; our wire additionally returns `422` when the OPT
//!   parses as XML but is structurally invalid (the OAS folds that under `400`).
//! - **ADL 1.4 get / example** (`_get`, `_example_get`): the OAS `400` on `_get`
//!   is unreachable (only a path parameter); the reachable statuses are the
//!   ones documented on each operation below.
//! - **Stored-query list by name** (`definition_query_list.yaml`): the qualified
//!   name is a prefix pattern, so an unmatched name yields an empty `200` list,
//!   never `404` — the released operation declares only `200`
//!   (`specifications/responses/200_QueryList.yaml`). Its "when is empty, it
//!   will be treated as 'wildcard' in the search" clause is unreachable on the
//!   released wire: `specifications/parameters/path/qualified_query_name.yaml`
//!   is `required: true` and the release defines no bare `/definition/query`
//!   operation — this build serves that empty-prefix case on an extension route
//!   of its own (flagged as such on the declaration).

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

/// The Definition API group as a native `utoipa-axum` router (group-relative
/// paths; nested under the configured base path).
// `definition_template_adl2_version_get` carries `#[deprecated]` — the released
// operation is `deprecated: true` under SPECITS-87
// (`ITS-REST specifications/operations/definition_template_adl2_version_get.yaml`
// + `docs/overview/Amendment_record.md`), and utoipa reflects the Rust attribute
// into the served OpenAPI. It is the only handler in this group so marked; the
// `routes!` macro references it by name, so the deprecation lint is allowed here.
#[expect(
    deprecated,
    reason = "`definition_template_adl2_version_get` is `#[deprecated]` so utoipa \
              reflects `deprecated: true` into the served document; the \
              `routes!` macro has to reference it by name"
)]
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            definition_template_adl1_4_list,
            definition_template_adl1_4_upload
        ))
        .routes(routes!(definition_template_adl1_4_get))
        .routes(routes!(definition_template_adl1_4_example_get))
        .routes(routes!(
            definition_template_adl2_list,
            definition_template_adl2_upload
        ))
        .routes(routes!(definition_template_adl2_get))
        .routes(routes!(definition_template_adl2_example_get))
        .routes(routes!(definition_template_adl2_version_get))
        .routes(routes!(definition_query_list_all))
        .routes(routes!(definition_query_list, definition_query_store_yaml))
        .routes(routes!(
            definition_query_version_get,
            definition_query_version_store_yaml
        ))
}

/// List the stored ADL 1.4 operational templates
/// (`GET /definition/template/adl1.4`).
#[utoipa::path(
    get, path = "/definition/template/adl1.4", tag = "ADL1.4",
    params(
        ("template_id" = Option<String>, Query,
         description = "\"Pattern for matching `template_id` (supports wildcards \
                        `*`)\" (ITS-REST \
                        `specifications/parameters/query/filter_template_id.yaml`); \
                        omit to match any.",
         example = "vital*"),
        ("concept" = Option<String>, Query,
         description = "\"Pattern for matching `concept` (supports wildcards \
                        `*`)\" (ITS-REST \
                        `specifications/parameters/query/concept.yaml`); omit to \
                        match any.",
         example = "*signs*"),
        ("version" = Option<String>, Query,
         description = "A glob (`*` wildcard) matched against the whole \
                        `template_id`, which is where an ADL 1.4 template's \
                        version lives (`vital_signs.v1`). The ITS-REST docs \
                        text is silent, so the RELEASED OAS governs: \"Filter \
                        by version (e.g. `1.2.*` or use `*` for all versions), \
                        taken from `template_id`; if missing, then only the \
                        latest version will be returned\" \
                        (`specifications/parameters/query/filter_version.yaml`) \
                        — an ABSENT `version` collapses the listing to the \
                        latest `.vN` axis of each template; `*` lists every \
                        stored version. All three filters AND together.",
         example = "*.v1"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). An offset \
                        past the end of the match set yields an empty list, not \
                        an error — the released text fixes no other handling. A \
                        negative value is ignored (read as absent).",
         example = 0),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); absent or \
                        `0` returns every match.",
         example = 10)
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template list is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_TemplateList_adl1_4.yaml`) \
                                        — retrieval, not matching: a filter that \
                                        matches nothing is this `200` with an \
                                        empty array, never a `404`. Each row is a \
                                        `TemplateMetadata` whose `template_id`, \
                                        `concept`, `archetype_id` and \
                                        `created_timestamp` are all required \
                                        (`schemas/definition/TemplateMetadata.yaml`); \
                                        its `version` member is `deprecated: \
                                        true` there, so this server emits it only \
                                        when the stored `template_id` carries a \
                                        derivable version and never as an \
                                        explicit `null`.",
            content((serde_json::Value = "application/json", example = json!([
                {
                    "template_id": "Vital Signs",
                    "concept": "Vital Signs",
                    "archetype_id": "openEHR-EHR-COMPOSITION.encounter.v1",
                    "created_timestamp": "2017-08-14T19:24:56.639Z"
                }
            ])))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here \
                                      the only reachable trigger is a \
                                      syntactically invalid parameter: `offset` \
                                      or `fetch` that is not an integer. The \
                                      released list operation does not enumerate \
                                      `400`; the trigger is the cross-cutting \
                                      one.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the template list has the single \
                                      `application/json` representation the \
                                      released response declares \
                                      (`200_TemplateList_adl1_4.yaml`, and \
                                      `parameters/header/Accept_JSON.yaml` on the \
                                      operation), so an exclusively-XML `Accept` \
                                      is refused (`Resources.md` §\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"). The released operation does \
                                      not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl1_4_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl1.4_list",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Upload an ADL 1.4 operational template
/// (`POST /definition/template/adl1.4`).
///
/// The template arrives as canonical OPT XML (`Content-Type: application/xml`);
/// a request declaring another payload type is `415`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §XML Format).
#[utoipa::path(
    post, path = "/definition/template/adl1.4", tag = "ADL1.4",
    params(
        ("Content-Type" = Option<String>, Header,
         description = "`application/xml` (or `text/xml`) — the operation's only \
                        declared payload format \
                        (`specifications/operations/definition_template_adl1.4_upload.yaml`, \
                        whose `Content-Type` enum is the single value \
                        `application/xml`). Any other DECLARED type is `415`; an \
                        absent header declares nothing to refuse (`Resources.md` \
                        §\"XML Format\" makes the header a client MAY) and reads \
                        as XML.",
         example = "application/xml"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`specifications/parameters/header/Prefer.yaml`; \
                        `Requests_and_responses.md` §\"Representation details \
                        negotiation\"). `return=representation` — the stored OPT \
                        XML; `return=identifier` — \"only its unique identifier \
                        is included\" (`responses/201_Template_adl1_4_upload.yaml`), \
                        i.e. the `TemplateIdentifier` object; missing or \
                        `return=minimal` — \"the body is empty\". The token \
                        actually honoured is echoed in `Preference-Applied`.",
         example = "return=representation")
    ),
    request_body(content((String = "application/xml")),
                 description = "The operational template as canonical OPT XML \
                                (`schemas/aom/OperationalTemplate.yaml`); \
                                `required: true` on the released operation."),
    responses(
        (
            status = 201, description = "The released trigger, verbatim: `201 \
                                        Created` \"is returned when the template \
                                        has been successfully uploaded\" \
                                        (ITS-REST \
                                        `specifications/responses/201_Template_adl1_4_upload.yaml`). \
                                        The body follows `Prefer`, per that same \
                                        response: \"If `Prefer` header is \
                                        `return=representation`, the full \
                                        resource is included in the response \
                                        body; if is `return=identifier`, only its \
                                        unique identifier is included. If the \
                                        `Prefer` header is missing or set to \
                                        `return=minimal`, the body is empty.\" A \
                                        template carries no `uid` on this wire, \
                                        so the identifier body is the released \
                                        `TemplateIdentifier` object \
                                        (`schemas/others/TemplateIdentifier.yaml`), \
                                        not the generic `{uid}` shape.",
            headers(
                ("Location" = String,
                 description = "\"The `Location` response header indicates the \
                                URL of the Template resource\" (ITS-REST \
                                `specifications/headers/Location_Template_adl1_4.yaml`, \
                                whose own example percent-encodes the id: \
                                `…/v1/definition/template/adl1.4/Vital%20Signs`) \
                                — set here to \
                                `<base_path>/definition/template/adl1.4/<percent-encoded \
                                template_id>` on every `Prefer` outcome \
                                (`Requests_and_responses.md` §Location: used \
                                \"in `201 Created` responses when a new resource \
                                is successfully created\")."),
                ("ETag" = String,
                 description = "\"The `ETag` (i.e. entity tag) response header is \
                                an identifier of Template\" (ITS-REST \
                                `specifications/headers/ETag_Template_adl1_4.yaml`), \
                                carrying the stored `template_id` in the weak \
                                form the release requires — \"all `ETag` headers \
                                that hold a resource identifier MUST include a \
                                weakness indicator `W/`\" \
                                (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\"). Shape: `W/\"Vital Signs\"`. \
                                Keyed on the same `template_id` the retrieval \
                                `ETag` uses, so a client's `If-None-Match` \
                                round-trips."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured. \"The service MAY include a \
                                `Preference-Applied` header in the response … to \
                                indicate that the client's preference has been \
                                honored\" (`Requests_and_responses.md` \
                                §\"Representation details negotiation\").")
            ),
            content(
                (String = "application/xml", example = r#"<template xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns="http://schemas.openehr.org/v1">
    <language>
        <terminology_id><value>ISO_639-1</value></terminology_id>
        <code_string>en</code_string>
    </language>
    <uid><value>b4d7f203-b329-4e89-a58a-c605b19e94de</value></uid>
    <template_id><value>Vital Signs</value></template_id>
    <concept>Vital Signs</concept>
    <definition>
        <rm_type_name>COMPOSITION</rm_type_name>
        <node_id>at0000</node_id>
        ...
    </definition>
</template>
"#),
                (serde_json::Value = "application/json",
                 example = json!({ "template_id": "Vital Signs" }))
            )
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      the body is not UTF-8 text, or is not \
                                      well-formed XML (mismatched tags, an empty \
                                      document with no root element) — \
                                      \"syntactically invalid … content\" is \
                                      precisely that branch. Everything past XML \
                                      well-formedness (a document that is not an \
                                      OPT, an AOM2 rule violation) is the \
                                      semantic `422` below.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when a template \
                                      with same `template_id` already exists\" \
                                      (ITS-REST \
                                      `specifications/responses/409_template_already_exists.yaml`).",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a payload media type \
                                      the service cannot process as this \
                                      operation's XML body: \"A client MAY use \
                                      the header `Content-Type: application/xml` \
                                      in the requests to specify the XML payload \
                                      format. If the service cannot process the \
                                      request payload as XML format, it MUST \
                                      respond with HTTP status code `415 \
                                      Unsupported Media Type`\" (`Resources.md` \
                                      §\"XML Format\"). An absent `Content-Type` \
                                      declares nothing to refuse and is accepted. \
                                      The released operation does not enumerate \
                                      `415`; the MUST is cross-cutting \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `415` row).",
         body = serde_json::Value),
        (status = 422, description = "OUR WIRE — the payload is well-formed XML but \
                                      is not a usable operational template: it \
                                      does not decode as an OPT document; a \
                                      foreign or duplicated top-level element; a \
                                      blank `template_id` or an empty `concept` \
                                      (both mandatory `OPERATIONAL_TEMPLATE` \
                                      attributes); or an AOM2 standalone-artefact \
                                      validity-rule violation, the rule code \
                                      carried in `validationErrors` \
                                      (`AM/docs/AOM2/master08-validation.adoc` \
                                      §Validation; `schemas/others/Error.yaml`). No released \
                                      response file covers the semantic branch on \
                                      this operation, so the assignment is the \
                                      overview status table's own `422` row — \
                                      \"the request was well-formed but was \
                                      unable to be followed due to semantic \
                                      errors\" (`Requests_and_responses.md` \
                                      §\"HTTP status codes\") — and \"additional \
                                      status codes MAY be used as long as they do \
                                      not conflict with the predefined codes\". \
                                      Syntactically-unparseable content is the \
                                      `400` above, not this branch.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl1_4_upload(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl1.4_upload",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve one ADL 1.4 template by id
/// (`GET /definition/template/adl1.4/{template_id}`).
#[utoipa::path(
    get, path = "/definition/template/adl1.4/{template_id}", tag = "ADL1.4",
    params(
        ("template_id" = String, Path,
         description = "\"Template identifier or partial reference. A partial \
                        `template_id` will resolve to “latest” major version of \
                        that template\" (ITS-REST \
                        `specifications/parameters/path/template_id.yaml`, whose \
                        own examples run from the legacy `Vital Signs` to the \
                        HRID `org.highmed::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0`). \
                        This server resolves an ADL 1.4 id by EXACT match \
                        (case-insensitive): no released text defines a version \
                        grammar for legacy OPT 1.4 identifiers, so the \
                        partial-reference rule has nothing to resolve against \
                        here — OUR OWN handling of that silence, registered as a \
                        conformance boundary.",
         example = "Vital Signs"),
        ("Accept" = Option<String>, Header,
         description = "`application/xml` (the canonical OPT; also the default \
                        for absent/`*/*`), or `application/openehr.wt+json` / \
                        `application/json` (the Web Template document — the only \
                        JSON projection of an OPT). All three are enumerated by \
                        `specifications/parameters/header/Accept_Template.yaml`. \
                        The response `Content-Type` is always the negotiated type \
                        (`Resources.md` §\"JSON Format\": the proper \
                        `Content-Type` \"MUST be present in the response of the \
                        service\").",
         example = "application/xml")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_Template_adl1_4_retrieved.yaml`). \
                                        The body follows `Accept`: the stored \
                                        canonical OPT XML served back verbatim \
                                        — \"the original (canonical) `XML` based \
                                        OPT format\" \
                                        (`operations/definition_template_adl1.4_get.yaml`) \
                                        — or \"the simplified `JSON`-based \
                                        \u{201c}web template\u{201d} format\" \
                                        under whichever of \
                                        `application/openehr.wt+json` / \
                                        `application/json` was negotiated. \
                                        `application/json` is honoured rather \
                                        than refused because \
                                        `parameters/header/Accept_Template.yaml` \
                                        and `headers/ContentType_Template.yaml` \
                                        both enumerate it while the released \
                                        `200` declares no schema for it; the Web \
                                        Template document is the only JSON \
                                        template representation the release \
                                        names (`Resources.md` §Simplified \
                                        Formats). The Web Template document's \
                                        internal shape follows the Better \
                                        `web-template` model — no openEHR spec \
                                        defines a schema for it, our own \
                                        design/extension.",
            headers(
                ("ETag" = String,
                 description = "\"The `ETag` (i.e. entity tag) response header is \
                                an identifier of Template\" (ITS-REST \
                                `specifications/headers/ETag_Template_adl1_4.yaml`), \
                                carrying the `template_id` in the weak form the \
                                release requires — \"all `ETag` headers that hold \
                                a resource identifier MUST include a weakness \
                                indicator `W/`\" (`Requests_and_responses.md` \
                                §\"ETag and Last-Modified\"). Shape: `W/\"Vital \
                                Signs\"`, identical across all three \
                                representations because \"the `ETag` value is \
                                independent of its resource serialization format \
                                (JSON/XML)\" (same section). No `Last-Modified`: \
                                a stored OPT has no released version or state \
                                identifier to date.")
            ),
            content(
                (String = "application/xml", example = r#"<template xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns="http://schemas.openehr.org/v1">
    <language>
        <terminology_id><value>ISO_639-1</value></terminology_id>
        <code_string>en</code_string>
    </language>
    <uid><value>b4d7f203-b329-4e89-a58a-c605b19e94de</value></uid>
    <template_id><value>Vital Signs</value></template_id>
    <concept>Vital Signs</concept>
    <definition>
        <rm_type_name>COMPOSITION</rm_type_name>
        <node_id>at0000</node_id>
        ...
    </definition>
</template>
"#),
                (serde_json::Value = "application/openehr.wt+json"),
                (serde_json::Value = "application/json")
            )
        ),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a template with \
                                      the specified `template_id` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_template_id.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The released trigger, verbatim: `406 Not \
                                      Acceptable` \"is returned when the service \
                                      cannot produce a response matching `Accept` \
                                      request header, i.e. content type or format \
                                      is not supported\" (ITS-REST \
                                      `specifications/responses/406.yaml`) — here, \
                                      an `Accept` outside `application/xml`, \
                                      `application/openehr.wt+json` and \
                                      `application/json`. Resolved BEFORE storage \
                                      is touched, so an unacceptable `Accept` on \
                                      an unknown template is still this `406`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl1_4_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl1.4_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Generate an example COMPOSITION for an ADL 1.4 template
/// (`GET /definition/template/adl1.4/{template_id}/example`).
#[utoipa::path(
    get, path = "/definition/template/adl1.4/{template_id}/example", tag = "ADL1.4",
    params(
        ("template_id" = String, Path,
         description = "\"Template identifier or partial reference\" (ITS-REST \
                        `specifications/parameters/path/template_id.yaml`), \
                        resolved by EXACT (case-insensitive) match as on the \
                        sibling `_get` — no released text defines a version \
                        grammar for legacy OPT 1.4 identifiers.",
         example = "Vital Signs"),
        ("type" = Option<String>, Query,
         description = "\"Type of use for the data example\" (ITS-REST \
                        `specifications/parameters/query/example_type.yaml`, \
                        enum `[input, output]`, default `input`): \"either of \
                        type `input` (i.e., ready to be submitted to the \
                        repository) or `output` (as it would appear when \
                        retrieved from the repository)\" \
                        (`operations/definition_template_adl1.4_example_get.yaml`).",
         example = "input"),
        ("detail_level" = Option<String>, Query,
         description = "\"The level of data points details of the example\" \
                        (ITS-REST \
                        `specifications/parameters/query/example_detail_level.yaml`, \
                        enum `[required, medium, complete]`, default \
                        `required`). The released meanings, from the operation \
                        text: `required` — \"a minimal composition including \
                        only mandatory data points\"; `medium` — \"a fairly \
                        realistic set of data points, including some optional RM \
                        attributes and elements\"; `complete` — \"a full \
                        representation of all possible data points; not expected \
                        to be committable or realistic\". \"The implementation \
                        and completeness of these examples are not specified, \
                        and vendors may produce different results.\"",
         example = "required"),
        ("Accept" = Option<String>, Header,
         description = "One of `application/json` (default), `application/xml`, \
                        `application/openehr.wt.flat+json`, or \
                        `application/openehr.wt.structured+json` — the four \
                        forms \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml` \
                        enumerates and \
                        `responses/200_Template_example_retrieved.yaml` declares \
                        content for (`Resources.md` §Simplified Formats fixes \
                        the two `openehr.wt.*` media types).",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_Template_example_retrieved.yaml`) \
                                        — here the GENERATED example data \
                                        instance, an RM COMPOSITION serialized \
                                        per `Accept` (canonical JSON/XML, or one \
                                        of the two Simplified Formats). No \
                                        `ETag`: the example is derived output, \
                                        not a stored resource with an identifier \
                                        (`Requests_and_responses.md` §\"ETag and \
                                        Last-Modified\" scopes the header to \
                                        \"resources that have versioning or \
                                        unique state identifiers\").",
            content((serde_json::Value = "application/json"), (serde_json::Value = "application/xml"), (serde_json::Value = "application/openehr.wt.flat+json"), (serde_json::Value = "application/openehr.wt.structured+json"))
        ),
        (status = 400, description = "`type` or `detail_level` is outside its \
                                      enumerated set. The released operation \
                                      leaves the handling open — when the server \
                                      \"does not support the requested `type` or \
                                      `detail_level`, it will fall back to the \
                                      closest supported level, or it may return \
                                      an error (typically `400 Bad Request`)\" — \
                                      and this server takes the error arm for an \
                                      out-of-enum value (an out-of-enum value is \
                                      also \"syntactically invalid … parameter\" \
                                      under `responses/400.yaml`). Which arm of \
                                      that either/or applies is OURS.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a template with \
                                      the specified `template_id` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_template_id.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The released trigger, verbatim: `406 Not \
                                      Acceptable` \"is returned when the service \
                                      cannot produce a response matching `Accept` \
                                      request header, i.e. content type or format \
                                      is not supported\" (ITS-REST \
                                      `specifications/responses/406.yaml`) — an \
                                      `Accept` outside the four \
                                      `Accept_LOCATABLE` representations.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl1_4_example_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl1.4_example_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// List the stored ADL 2 templates (`GET /definition/template/adl2`).
#[utoipa::path(
    get, path = "/definition/template/adl2", tag = "ADL2",
    params(
        ("template_id" = Option<String>, Query,
         description = "\"Pattern for matching `template_id` (supports wildcards \
                        `*`)\" (ITS-REST \
                        `specifications/parameters/query/filter_template_id.yaml`); \
                        omit to match any. An ADL2 `template_id` is the \
                        artefact's `ARCHETYPE_HRID`.",
         example = "openEHR-EHR-COMPOSITION.t_vital_signs.*"),
        ("concept" = Option<String>, Query,
         description = "\"Pattern for matching `concept` (supports wildcards \
                        `*`)\" (ITS-REST \
                        `specifications/parameters/query/concept.yaml`); omit to \
                        match any.",
         example = "*signs*"),
        ("version" = Option<String>, Query,
         description = "A glob (`*` wildcard) matched against the whole \
                        `template_id`, which for ADL2 is the HRID carrying the \
                        artefact's SEMVER `release_version` \
                        (`…t_vital_signs.v1.0.0`). The ITS-REST docs text is \
                        silent, so the RELEASED OAS governs: \"Filter by \
                        version …, taken from `template_id`; if missing, then \
                        only the latest version will be returned\" \
                        (`specifications/parameters/query/filter_version.yaml`) \
                        — an ABSENT `version` collapses the listing to the \
                        latest `.vN` axis of each template; `*` lists every \
                        stored version. All three filters AND together.",
         example = "*.v1.*"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). An offset \
                        past the end of the match set yields an empty list, not \
                        an error. A negative value is ignored (read as absent).",
         example = 0),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); absent or \
                        `0` returns every match.",
         example = 10)
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template list is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_TemplateList_adl2.yaml`) \
                                        — retrieval, not matching: a filter that \
                                        matches nothing is this `200` with an \
                                        empty array, never a `404`. Each row is a \
                                        `TemplateMetadata` whose `template_id`, \
                                        `concept`, `archetype_id` and \
                                        `created_timestamp` are all required \
                                        (`schemas/definition/TemplateMetadata.yaml`); \
                                        the schema gives no member descriptions \
                                        and no released text says what \
                                        `archetype_id` holds for an ADL2 \
                                        operational template, so this server \
                                        emits the artefact's own `ARCHETYPE_HRID` \
                                        there (the released example instead shows \
                                        the specialisation parent) — OUR OWN \
                                        reading of that silence. `concept` is \
                                        derived from the HRID's concept segment. \
                                        The `deprecated: true` `version` member \
                                        is not emitted on this interface.",
            content((serde_json::Value = "application/json", example = json!([
                {
                    "template_id": "openEHR-EHR-COMPOSITION.t_clinical_info_ds_sf.v1.0.0",
                    "concept": "t_clinical_info_ds_sf",
                    "archetype_id": "openEHR-EHR-COMPOSITION.t_clinical_info_ds_sf.v1.0.0",
                    "created_timestamp": "2017-08-14T19:24:56.639Z"
                },
                {
                    "template_id": "openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0",
                    "concept": "t_vital_signs",
                    "archetype_id": "openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0",
                    "created_timestamp": "2017-08-14T19:24:56.639Z"
                }
            ])))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here \
                                      the only reachable trigger is a \
                                      syntactically invalid parameter: `offset` \
                                      or `fetch` that is not an integer. The \
                                      released list operation does not enumerate \
                                      `400`; the trigger is the cross-cutting \
                                      one.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the template list has the single \
                                      `application/json` representation the \
                                      released response declares \
                                      (`200_TemplateList_adl2.yaml`, and \
                                      `parameters/header/Accept_JSON.yaml` on the \
                                      operation), so an exclusively-XML `Accept` \
                                      is refused (`Resources.md` §\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"). The released operation does \
                                      not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl2_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl2_list",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Upload an ADL 2 operational template (`POST /definition/template/adl2`).
///
/// The template arrives as `text/plain` ADL2 source; a request declaring another
/// payload type is `415`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §"XML Format"/§"JSON Format" — the cross-cutting format rule). The deprecated
/// `at_version` query parameter is dropped (spec-permitted).
#[utoipa::path(
    post, path = "/definition/template/adl2", tag = "ADL2",
    params(
        ("Content-Type" = Option<String>, Header,
         description = "`text/plain` — the operation's only declared payload \
                        format \
                        (`specifications/operations/definition_template_adl2_upload.yaml`, \
                        whose `Content-Type` enum is the single value \
                        `text/plain`). Any other DECLARED type is `415`; an \
                        absent header declares nothing to refuse (`Resources.md` \
                        makes the header a client MAY) and reads as ADL2 source.",
         example = "text/plain"),
        ("Prefer" = Option<String>, Header,
         description = "Response-verbosity preference \
                        (`specifications/parameters/header/Prefer.yaml`; \
                        `Requests_and_responses.md` §\"Representation details \
                        negotiation\"). `return=representation` — the stored ADL2 \
                        source (`text/plain`); `return=identifier` — \"only its \
                        unique identifier is included\" \
                        (`responses/201_Template_adl2_upload.yaml`), i.e. the \
                        `TemplateIdentifier` object; missing or `return=minimal` \
                        — \"the body is empty\". The token actually honoured is \
                        echoed in `Preference-Applied`.",
         example = "return=identifier")
    ),
    request_body(content((String = "text/plain")),
                 description = "The ADL2 operational-template source \
                                (`schemas/aom/OperationalTemplateV2.yaml` or a \
                                plain string); `required: true` on the released \
                                operation."),
    responses(
        (
            status = 201, description = "The released trigger, verbatim: `201 \
                                        Created` \"is returned when the template \
                                        has been successfully uploaded\" \
                                        (ITS-REST \
                                        `specifications/responses/201_Template_adl2_upload.yaml`). \
                                        The body follows `Prefer`, per that same \
                                        response: \"If `Prefer` header is \
                                        `return=representation`, the full \
                                        resource is included in the response \
                                        body; if is `return=identifier`, only its \
                                        unique identifier is included. If the \
                                        `Prefer` header is missing or set to \
                                        `return=minimal`, the body is empty.\" \
                                        The identifier body is the released \
                                        `TemplateIdentifier` object \
                                        (`schemas/others/TemplateIdentifier.yaml`), \
                                        carrying the stored `ARCHETYPE_HRID`.",
            headers(
                ("Location" = String,
                 description = "\"The `Location` response header indicates the \
                                URL of the Template resource\" (ITS-REST \
                                `specifications/headers/Location_Template_adl2.yaml`, \
                                example \
                                `…/v1/definition/template/adl2/openEHR-EHR-COMPOSITION.t_clinical_info_ds_sf.v1.0.0`) \
                                — set here to \
                                `<base_path>/definition/template/adl2/<ARCHETYPE_HRID>` \
                                on every `Prefer` outcome \
                                (`Requests_and_responses.md` §Location: used \
                                \"in `201 Created` responses when a new resource \
                                is successfully created\")."),
                ("ETag" = String,
                 description = "\"The `ETag` (i.e. entity tag) response header is \
                                an identifier of Template\" (ITS-REST \
                                `specifications/headers/ETag_Template_adl2.yaml`), \
                                carrying the stored `ARCHETYPE_HRID` in the weak \
                                form the release requires — \"all `ETag` headers \
                                that hold a resource identifier MUST include a \
                                weakness indicator `W/`\" \
                                (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\"). The HRID carries the artefact's \
                                SEMVER `release_version`, so the tag changes \
                                whenever the artefact does. Shape: \
                                `W/\"openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0\"`."),
                ("Preference-Applied" = String,
                 description = "`return=minimal` | `return=identifier` | \
                                `return=representation` — the preference the \
                                service honoured. \"The service MAY include a \
                                `Preference-Applied` header in the response … to \
                                indicate that the client's preference has been \
                                honored\" (`Requests_and_responses.md` \
                                §\"Representation details negotiation\").")
            ),
            content(
                (String = "text/plain", example = r#"operational_template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">
...
"#),
                (serde_json::Value = "application/json", example = json!({
                    "template_id": "openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0"
                }))
            )
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here: \
                                      the body is not UTF-8 text, or the source \
                                      fails the ADL2 grammar (S-code syntax \
                                      errors — unparseable content, an empty \
                                      body, a missing mandatory section) — \
                                      \"syntactically invalid … content\" is \
                                      precisely that branch, the same split as \
                                      the ADL 1.4 sibling. AOM2 validation-phase \
                                      failures (V-codes) on a source that parses \
                                      are the semantic `422` below.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when a template \
                                      with same `template_id` already exists\" \
                                      (ITS-REST \
                                      `specifications/responses/409_template_already_exists.yaml`) \
                                      — here, an ADL2 artefact already stored \
                                      under the same `ARCHETYPE_HRID`. The \
                                      released wire wins over the SM \
                                      `upload_artefact`, which would \"replace\" \
                                      the existing artefact \
                                      (`SM/docs/UML/classes/i_definition_adl2.adoc`: \
                                      \"If an artefact with the same physical \
                                      identifier and namespace exists, replace \
                                      it\").",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a payload media type \
                                      the service cannot process as this \
                                      operation's `text/plain` ADL2 source: a \
                                      payload the service cannot process in the \
                                      declared format \"MUST respond with HTTP \
                                      status code `415 Unsupported Media Type`\" \
                                      (`Resources.md` §\"XML Format\" / §\"JSON \
                                      Format\" — the same cross-cutting rule that \
                                      grounds the ADL 1.4 guard). An absent \
                                      `Content-Type` declares nothing to refuse \
                                      and is accepted. The released operation \
                                      does not enumerate `415`; the MUST is \
                                      cross-cutting \
                                      (`Requests_and_responses.md` §\"HTTP status \
                                      codes\", the `415` row).",
         body = serde_json::Value),
        (status = 422, description = "OUR WIRE — the ADL2 source parses but fails \
                                      an AOM2 validation phase (V-codes, e.g. a \
                                      missing description block). The `Error` \
                                      body's `validationErrors` carry the rule-code \
                                      mnemonics with their detail \
                                      (`schemas/others/Error.yaml`). Grammar-level \
                                      S-code failures are the syntactic `400` \
                                      above. No released \
                                      response file covers the semantic branch on \
                                      this operation, so the assignment is the \
                                      overview status table's own `422` row — \
                                      \"the request was well-formed but was \
                                      unable to be followed due to semantic \
                                      errors\" (`Requests_and_responses.md` \
                                      §\"HTTP status codes\") — and \"additional \
                                      status codes MAY be used as long as they do \
                                      not conflict with the predefined codes\"; \
                                      it realizes the SM `invalid_artefact` \
                                      outcome of `upload_artefact`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl2_upload(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl2_upload",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve one ADL 2 template by id
/// (`GET /definition/template/adl2/{template_id}`).
///
/// Served as `text/plain` ADL2 source or the `application/json`
/// `OperationalTemplateV2` canonical-JSON projection, per `Accept`.
#[utoipa::path(
    get, path = "/definition/template/adl2/{template_id}", tag = "ADL2",
    params(
        ("template_id" = String, Path,
         description = "\"Template identifier or partial reference. A partial \
                        `template_id` will resolve to “latest” major version of \
                        that template\" (ITS-REST \
                        `specifications/parameters/path/template_id_adl2.yaml`, \
                        examples \
                        `org.highmed::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0` \
                        and the partial \
                        `openEHR-EHR-COMPOSITION.t_vital_signs.v1`). An ADL2 id \
                        is an `ARCHETYPE_HRID`, so the partial form resolves \
                        against its SEMVER `release_version` axis (AM \
                        `docs/Identification/master04-versioning.adoc`).",
         example = "openEHR-EHR-COMPOSITION.t_vital_signs.v1"),
        ("Accept" = Option<String>, Header,
         description = "`text/plain` (the ADL2 source; also the default for \
                        absent/`*/*`/`text/*`) or `application/json` (the \
                        `OperationalTemplateV2` canonical JSON) — the two \
                        representations \
                        `specifications/responses/200_Template_adl2_retrieved.yaml` \
                        declares content for. \
                        `specifications/parameters/header/Accept_Template_adl2.yaml` \
                        additionally enumerates `application/xml`, for which \
                        that `200` declares NO body, so an `Accept` naming only \
                        it is a `406`. A `;q=0` on a range rejects it (RFC 9110 \
                        §12.5.1); `text/plain` wins when both are acceptable.",
         example = "text/plain")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_Template_adl2_retrieved.yaml`), \
                                        for the operation that \"retrieves the \
                                        LATEST version of the ADL2 operational \
                                        template identified by `template_id`\" \
                                        (`operations/definition_template_adl2_get.yaml`). \
                                        The `text/plain` body is the stored ADL2 \
                                        source served back verbatim (the released \
                                        body is `oneOf: [OperationalTemplateV2, \
                                        string]` and its example IS ADL2 source); \
                                        the `application/json` body is the \
                                        `OperationalTemplateV2` canonical-JSON \
                                        projection (the released schema \
                                        `schemas/aom/OperationalTemplateV2.yaml` \
                                        is an opaque object, which the AOM2 \
                                        canonical JSON satisfies).",
            headers(
                ("ETag" = String,
                 description = "A SERVER EXTRA, not a declared header: \
                                `200_Template_adl2_retrieved.yaml` declares only \
                                `Content-Type`, and \"servers MAY add additional \
                                `ETag` response headers, consisting of an opaque \
                                quoted string, possibly prefixed by a weakness \
                                indicator\" (`Requests_and_responses.md` §\"ETag \
                                and Last-Modified\"). It carries the RESOLVED \
                                `ARCHETYPE_HRID` of the artefact actually served \
                                — never the partial id that addressed it — in \
                                the weak form (\"all `ETag` headers that hold a \
                                resource identifier MUST include a weakness \
                                indicator `W/`\", same section), and the HRID \
                                carries the artefact's SEMVER `release_version`, \
                                so the tag changes whenever the served artefact \
                                does. Identical across both representations, the \
                                `ETag` being \"independent of its resource \
                                serialization format\". No `Last-Modified`.")
            ),
            content(
                (String = "text/plain", example = r#"operational_template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">
...
"#),
                (serde_json::Value = "application/json")
            )
        ),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a template with \
                                      the specified `template_id` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_template_id.yaml`) \
                                      — including a partial id that resolves to \
                                      no stored version.",
         body = serde_json::Value),
        (status = 406, description = "OUR REASONED BRANCH — `Accept` names only \
                                      `application/xml`, which \
                                      `parameters/header/Accept_Template_adl2.yaml` \
                                      enumerates but \
                                      `200_Template_adl2_retrieved.yaml` declares \
                                      no body for. With no XML representation to \
                                      produce, the cross-cutting MUST applies: \
                                      \"If the service cannot fulfill this aspect \
                                      of the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\" \
                                      (`Resources.md` §\"XML Format\"). The \
                                      released operation enumerates no `406`; \
                                      that silence is what makes this ours.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl2_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl2_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Generate an example COMPOSITION for an ADL 2 template
/// (`GET /definition/template/adl2/{template_id}/example`).
///
/// The stored ADL2 template is compiled to its operational template and turned
/// into a Web Template (the `v2_4` front end), which the shared example generator
/// walks into a canonical example COMPOSITION — the same generator the ADL 1.4
/// example endpoint uses.
#[utoipa::path(
    get, path = "/definition/template/adl2/{template_id}/example", tag = "ADL2",
    params(
        ("template_id" = String, Path,
         description = "\"Template identifier or partial reference. A partial \
                        `template_id` will resolve to “latest” major version of \
                        that template\" (ITS-REST \
                        `specifications/parameters/path/template_id_adl2.yaml`) \
                        — an `ARCHETYPE_HRID`, resolved over its SEMVER \
                        `release_version` axis as on the sibling `_get`.",
         example = "openEHR-EHR-COMPOSITION.t_vital_signs.v1"),
        ("type" = Option<String>, Query,
         description = "\"Type of use for the data example\" (ITS-REST \
                        `specifications/parameters/query/example_type.yaml`, \
                        enum `[input, output]`, default `input`): `input` is \
                        ready to be submitted to the repository, `output` is as \
                        the instance would appear when retrieved from it.",
         example = "input"),
        ("detail_level" = Option<String>, Query,
         description = "\"The level of data points details of the example\" \
                        (ITS-REST \
                        `specifications/parameters/query/example_detail_level.yaml`, \
                        enum `[required, medium, complete]`, default \
                        `required`), which \"affects: Number and complexity of \
                        nested elements; Inclusion/exclusion of optional \
                        elements; Depth of data point details\" \
                        (`operations/definition_template_adl2_example_get.yaml`). \
                        The per-level MEANINGS and the fallback-or-`400` \
                        latitude are spelled out only on the ADL 1.4 sibling \
                        operation; this server applies them uniformly to both \
                        interfaces — OUR OWN reading, the released ADL2 text \
                        being silent.",
         example = "required"),
        ("Accept" = Option<String>, Header,
         description = "One of `application/json` (default), `application/xml`, \
                        `application/openehr.wt.flat+json`, or \
                        `application/openehr.wt.structured+json` — the four \
                        forms \
                        `specifications/parameters/header/Accept_LOCATABLE.yaml` \
                        enumerates and \
                        `responses/200_Template_example_retrieved.yaml` declares \
                        content for.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_Template_example_retrieved.yaml`) \
                                        — here the GENERATED example data \
                                        instance: the stored ADL2 template is \
                                        compiled to its operational template and \
                                        turned into a Web Template (the v2_4 \
                                        front end), which the shared example \
                                        generator walks into an RM COMPOSITION, \
                                        serialized per `Accept`. No `ETag`: the \
                                        example is derived output, not a stored \
                                        resource with an identifier.",
            content((serde_json::Value = "application/json"), (serde_json::Value = "application/xml"), (serde_json::Value = "application/openehr.wt.flat+json"), (serde_json::Value = "application/openehr.wt.structured+json"))
        ),
        (status = 400, description = "`type` or `detail_level` is outside its \
                                      enumerated set — the error arm of the \
                                      released either/or (\"fall back to the \
                                      closest supported level, or it may return \
                                      an error (typically `400 Bad Request`)\", \
                                      stated on the ADL 1.4 sibling and applied \
                                      uniformly here), and \"syntactically \
                                      invalid … parameter\" under \
                                      `responses/400.yaml`.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a template with \
                                      the specified `template_id` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_template_id.yaml`).",
         body = serde_json::Value),
        (status = 406, description = "The released trigger, verbatim: `406 Not \
                                      Acceptable` \"is returned when the service \
                                      cannot produce a response matching `Accept` \
                                      request header, i.e. content type or format \
                                      is not supported\" (ITS-REST \
                                      `specifications/responses/406.yaml`) — an \
                                      `Accept` outside the four \
                                      `Accept_LOCATABLE` representations.",
         body = serde_json::Value),
        (status = 422, description = "OUR WIRE — the stored ADL2 template cannot \
                                      be compiled into the operational template \
                                      the example generator needs. No released \
                                      response file covers this; the assignment \
                                      is the overview status table's `422` row, \
                                      \"the request was well-formed but was \
                                      unable to be followed due to semantic \
                                      errors\" (`Requests_and_responses.md` \
                                      §\"HTTP status codes\").",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_template_adl2_example_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl2_example_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve one ADL 2 template at a specific version
/// (`GET /definition/template/adl2/{template_id}/{version}`).
///
/// DEPRECATED in the released spec but served: the operation file carries
/// `deprecated: true`
/// (`specifications/operations/definition_template_adl2_version_get.yaml`) under
/// amendment SPECITS-87, "Deprecate ADL2 'Get template at version' and the
/// version query parameter from upload template"
/// (`specifications/docs/overview/Amendment_record.md`) — the only operation in
/// this group so marked, and the only declaration here that carries
/// `deprecated`. It is still released text, so it is still served; the release
/// defines no `Deprecation`/`Sunset` signalling for a deprecated operation, so
/// none is emitted (silence, registered as a conformance boundary).
///
/// It resolves `template_id` + the SEMVER `version` (exact or
/// `{major}[.{minor}]` prefix → highest match) and returns the same
/// representations as `_get` (`text/plain` source / `application/json`
/// `OperationalTemplateV2`).
#[utoipa::path(
    get, path = "/definition/template/adl2/{template_id}/{version}", tag = "ADL2",
    params(
        ("template_id" = String, Path,
         description = "\"Template identifier or partial reference\" (ITS-REST \
                        `specifications/parameters/path/template_id.yaml`), here \
                        resolved WITHIN the addressed `version`.",
         example = "openEHR-EHR-COMPOSITION.t_vital_signs"),
        ("version" = String, Path,
         description = "\"A SEMVER version number. This can be an exact version \
                        (e.g. `1.7.1`), or a pattern as partial prefix, in a form \
                        of `{major}` or `{major}.{minor}` (e.g. `1` or `1.0`), in \
                        which case the highest (latest) version matching the \
                        prefix will be considered\" (ITS-REST \
                        `specifications/parameters/path/version.yaml`). Matching \
                        runs over the NUMERIC `major.minor.patch` axis of the \
                        stored `ARCHETYPE_HRID` only: a pre-release/build suffix \
                        on a stored artefact (`…v2.0.0-rc.1`) is ignored when \
                        matching and ordering, and a `{version}` value that \
                        carries such a suffix therefore matches nothing. \"SEMVER \
                        version number\" is all the released text says, so that \
                        reading of the pre-release axis is OURS.",
         example = "1.0"),
        ("Accept" = Option<String>, Header,
         description = "`text/plain` (the ADL2 source; also the default for \
                        absent/`*/*`/`text/*`) or `application/json` (the \
                        `OperationalTemplateV2` canonical JSON) — the two \
                        representations \
                        `specifications/responses/200_Template_adl2_retrieved.yaml` \
                        declares content for. `application/xml`, which \
                        `parameters/header/Accept_Template_adl2.yaml` also \
                        enumerates, has no declared response body, so an `Accept` \
                        naming only it is a `406`.",
         example = "text/plain")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the template is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_Template_adl2_retrieved.yaml`), \
                                        here for the artefact at the addressed \
                                        `version`. Same two representations as \
                                        the version-less sibling: the stored ADL2 \
                                        source verbatim (`text/plain`) or the \
                                        `OperationalTemplateV2` canonical-JSON \
                                        projection (`application/json`).",
            headers(
                ("ETag" = String,
                 description = "A SERVER EXTRA, not a declared header: \
                                `200_Template_adl2_retrieved.yaml` declares only \
                                `Content-Type`, and \"servers MAY add additional \
                                `ETag` response headers, consisting of an opaque \
                                quoted string, possibly prefixed by a weakness \
                                indicator\" (`Requests_and_responses.md` §\"ETag \
                                and Last-Modified\"). It carries the RESOLVED \
                                `ARCHETYPE_HRID` of the artefact actually served \
                                — never the `{major}`/`{major}.{minor}` prefix \
                                that addressed it — in the required weak form \
                                (\"all `ETag` headers that hold a resource \
                                identifier MUST include a weakness indicator \
                                `W/`\"), so a client can see which concrete \
                                version a prefix resolved to. No \
                                `Last-Modified`.")
            ),
            content(
                (String = "text/plain", example = r#"operational_template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"unmanaged">
...
"#),
                (serde_json::Value = "application/json")
            )
        ),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a template with \
                                      the specified `template_id` at given \
                                      `version` does not exist\" (ITS-REST \
                                      `specifications/responses/404_unknown_template_id_or_version.yaml`) \
                                      — the template is unknown, or no stored \
                                      version matches the exact value or prefix.",
         body = serde_json::Value),
        (status = 406, description = "OUR REASONED BRANCH — `Accept` names only \
                                      `application/xml`, which \
                                      `parameters/header/Accept_Template_adl2.yaml` \
                                      enumerates but \
                                      `200_Template_adl2_retrieved.yaml` declares \
                                      no body for. With no XML representation to \
                                      produce, the cross-cutting MUST applies: \
                                      \"If the service cannot fulfill this aspect \
                                      of the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\" \
                                      (`Resources.md` §\"XML Format\"). The \
                                      released operation enumerates no `406`; \
                                      that silence is what makes this ours.",
         body = serde_json::Value)
    )
)]
// The Rust `#[deprecated]` attribute is what utoipa reflects into the served
// OpenAPI `deprecated: true` (utoipa 5 reads the attribute; its `path` macro has
// no `deprecated` argument), matching the released operation's own
// `deprecated: true` under SPECITS-87.
#[deprecated = "ITS-REST Release-1.1.0 marks this operation deprecated under \
                SPECITS-87 (definition_template_adl2_version_get.yaml); served \
                for compatibility"]
pub(crate) async fn definition_template_adl2_version_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_template_adl2_version_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// List every stored query (`GET /definition/query`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this bare form. The release
/// defines exactly one stored-query list operation,
/// `GET /definition/query/{qualified_query_name}`
/// (`specifications/operations/definition_query_list.yaml`), whose path
/// parameter is `required: true`
/// (`specifications/parameters/path/qualified_query_name.yaml`) — so that
/// operation's "when is empty, it will be treated as 'wildcard' in the search"
/// clause has no addressable form on the released wire. This route serves the
/// empty-prefix case as a convenience extension; it answers exactly what the
/// released list answers for the empty prefix.
#[utoipa::path(
    get, path = "/definition/query", tag = "Query",
    params(
        ("Accept" = Option<String>, Header,
         description = "`application/json` — the single representation of a \
                        stored-query list (the released sibling operation \
                        carries `parameters/header/Accept_JSON.yaml` and \
                        `responses/200_QueryList.yaml` declares JSON content \
                        only). Absent or `*/*` reads as JSON.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "Every stored query, all versions — the \
                                        empty-prefix case of the released list, \
                                        whose own trigger reads, verbatim: `200 \
                                        OK` \"is returned when the query \
                                        resources are successfully retrieved\" \
                                        (ITS-REST \
                                        `specifications/responses/200_QueryList.yaml`). \
                                        A bare `QueryList` array \
                                        (`schemas/definition/QueryList.yaml`), \
                                        empty when nothing is stored — never a \
                                        `404`. Rows are ordered by qualified \
                                        name and then ascending SEMVER; no \
                                        released text fixes an order for a \
                                        stored-query list, so that ordering is \
                                        OURS. This whole route is our own \
                                        extension — no openEHR spec governs it.",
            content((serde_json::Value = "application/json", example = json!([
                {
                    "name": "org.openehr::compositions",
                    "type": "AQL",
                    "version": "1.0.1",
                    "saved": "2017-07-16T19:20:30Z",
                    "q": "SELECT c FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c[$compositionid] WHERE c/name/value = 'Vitals'"
                },
                {
                    "name": "org.openehr::compositions",
                    "type": "AQL",
                    "version": "1.1.7",
                    "saved": "2018-06-13T09:37:20Z",
                    "q": "SELECT c FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c[$uid] WHERE c/name/value = 'Vitals'"
                }
            ])))
        ),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      stored-query list has the single \
                                      `application/json` representation the \
                                      released sibling response declares \
                                      (`200_QueryList.yaml`), so an \
                                      exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If the \
                                      service cannot fulfill this aspect of the \
                                      request, it MUST respond with HTTP status \
                                      code `406 Not Acceptable`\"). Neither this \
                                      extension nor its released sibling \
                                      enumerates `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_query_list_all(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_query_list_all",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// List the stored versions of a named stored query
/// (`GET /definition/query/{qualified_query_name}`).
///
/// The name is a prefix pattern (`[{namespace}::]{query-name}`), so an unmatched
/// name yields an empty `200` list, never `404` (see the module doc).
#[utoipa::path(
    get, path = "/definition/query/{qualified_query_name}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query …, in a format \
                        of `[{namespace}::]{query-name}`\" (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`), \
                        whose grammar is fixed by \
                        `specifications/docs/query/Qualified_query_name.md`: the \
                        `namespace` is optional and \"should be in a form of a \
                        reverse domain name\", and \"the `query-name` may include \
                        any combination of characters, matched by the pattern \
                        `[a-zA-Z0-9_.-]`\" — with the NOTE that \"the \
                        `query-name` value must not be `aql` (case-insensitive), \
                        as that is a reserved name\" (enforced at store time, \
                        not here: a read simply matches nothing). On THIS \
                        operation the value is a PREFIX — \"retrieves list of \
                        all stored queries on the system matched by \
                        `qualified_query_name` as pattern\" \
                        (`operations/definition_query_list.yaml`, whose own \
                        example lists \"all versions of all queries with names \
                        starting with `org.openehr`\"). The prefix match is \
                        case-insensitive, and a namespace-less prefix also \
                        matches its `misc::`-namespaced form, because that is \
                        the canonical key every surface stores under (SM \
                        `docs/openehr_platform/master04-definition_package.adoc` \
                        §Registered Queries: \"If no namespace is supplied, the \
                        namespace `misc` is assumed\").",
         example = "org.openehr::compositions"),
        ("Accept" = Option<String>, Header,
         description = "`application/json` — the single representation this \
                        operation declares \
                        (`specifications/parameters/header/Accept_JSON.yaml` on \
                        the operation, `responses/200_QueryList.yaml` for the \
                        body). Absent or `*/*` reads as JSON.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the query resources \
                                        are successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_QueryList.yaml`) \
                                        — retrieval, not matching: a prefix that \
                                        matches nothing is this `200` with an \
                                        empty array, never a `404`. The body is a \
                                        bare `QueryList` array \
                                        (`schemas/definition/QueryList.yaml`) of \
                                        `StoredQuery` objects, EVERY stored \
                                        version of every matching name, ordered \
                                        by qualified name and then ascending \
                                        SEMVER — no released text fixes an order, \
                                        so the ordering is OURS. `type` echoes \
                                        the stored formalism, which this build \
                                        records as `AQL`, the `default`/`example` \
                                        of `schemas/query/QueryType.yaml` (the \
                                        released response examples spell the same \
                                        value lowercase; casing is not fixed by \
                                        any released rule). `saved` is the store \
                                        timestamp, a `format: date-time` string \
                                        (`schemas/query/StoredQuery.yaml`), \
                                        rendered here as UTC extended ISO 8601 \
                                        where the released examples show a \
                                        `+01:00` offset form. `q` is returned \
                                        exactly as stored.",
            content((serde_json::Value = "application/json", example = json!([
                {
                    "name": "org.openehr::compositions",
                    "type": "AQL",
                    "version": "1.0.1",
                    "saved": "2017-07-16T19:20:30Z",
                    "q": "SELECT c FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c[$compositionid] WHERE c/name/value = 'Vitals'"
                },
                {
                    "name": "org.openehr::compositions",
                    "type": "AQL",
                    "version": "1.1.7",
                    "saved": "2018-06-13T09:37:20Z",
                    "q": "SELECT c FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c[$uid] WHERE c/name/value = 'Vitals'"
                }
            ])))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here \
                                      the only reachable trigger is malformed \
                                      request URL syntax: a \
                                      `qualified_query_name` segment whose \
                                      percent-encoding does not decode to valid \
                                      UTF-8, so no name can be read from the URL. \
                                      A syntactically fine name that matches \
                                      nothing is the empty `200` above. The \
                                      released list operation does not enumerate \
                                      `400`; the trigger is the cross-cutting \
                                      one.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      the stored-query list has the single \
                                      `application/json` representation the \
                                      released response declares \
                                      (`200_QueryList.yaml`, and \
                                      `parameters/header/Accept_JSON.yaml` on the \
                                      operation), so an exclusively-XML `Accept` \
                                      is refused (`Resources.md` §\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"). The released operation does \
                                      not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_query_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_query_list",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Store a named AQL query, server-assigned SEMVER
/// (`PUT /definition/query/{qualified_query_name}`).
///
/// The AQL text is the `text/plain` request body.
#[utoipa::path(
    put, path = "/definition/query/{qualified_query_name}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query …, in a format \
                        of `[{namespace}::]{query-name}`\" (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`). \
                        The grammar is \
                        `specifications/docs/query/Qualified_query_name.md`: the \
                        optional `namespace` \"should be in a form of a reverse \
                        domain name\", and \"the `query-name` may include any \
                        combination of characters, matched by the pattern \
                        `[a-zA-Z0-9_.-]`\". Its NOTE is enforced here: \"The \
                        `query-name` value must not be `aql` (case-insensitive), \
                        as that is a reserved name\" — such a name is the `400` \
                        below. An EXACT name on this operation (no prefix \
                        matching); identity is case-insensitive but stored \
                        case-preserving. A namespace-less name is keyed under \
                        the assumed `misc` namespace — SM \
                        `docs/openehr_platform/master04-definition_package.adoc` \
                        §Registered Queries: \"If no namespace is supplied, the \
                        namespace `misc` is assumed\" — the same canonical key \
                        every other stored-query surface uses.",
         example = "org.openehr::compositions"),
        ("query_type" = Option<String>, Query,
         description = "\"Parameter indicating the query language/type\" \
                        (ITS-REST \
                        `specifications/parameters/query/query_type.yaml`, \
                        `default: \"AQL\"`). Only AQL is supported: the store \
                        validates and persists AQL, so an unsupported non-AQL \
                        formalism is an honest unsupported-formalism `400` rather \
                        than a blanket \"invalid AQL\". Matching is \
                        case-insensitive and accepts the SM formalism-with-version \
                        spelling (`aql`, `AQL::1.0.3`) — SM \
                        `master04-definition_package.adoc` §Query Formalism.",
         example = "AQL"),
        ("Content-Type" = Option<String>, Header,
         description = "`text/plain` — the operation's only declared payload \
                        format \
                        (`specifications/operations/definition_query_store.yaml` \
                        carries `parameters/header/ContentType_text.yaml`, whose \
                        enum is the single value `text/plain`). The body is read \
                        as UTF-8 text; a body that is not UTF-8 is the `400` \
                        below.",
         example = "text/plain")
    ),
    request_body(content((String = "text/plain",
                          example = "SELECT c FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] CONTAINS OBSERVATION obs[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE obs/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= $systolic_bp")),
                 description = "\"The given AQL query\" \
                                (`schemas/query/AQL.yaml`) as `text/plain`; \
                                `required: true` on the released operation."),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the query was \
                                        successfully stored\" (ITS-REST \
                                        `specifications/responses/200_StoredQuery_stored.yaml`) \
                                        — for the operation that \"stores a new \
                                        query, or updates an existing query on \
                                        the system\" \
                                        (`operations/definition_query_store.yaml`). \
                                        The response is BODYLESS: that released \
                                        `200` declares a `Location` header and no \
                                        `content` at all. The version this \
                                        operation writes is the fixed `1.0.0` \
                                        slot, re-written on each call — the \
                                        released text names no version-minting \
                                        rule for the version-less store, so the \
                                        slot is OUR OWN design/extension, chosen \
                                        so that \"or updates an existing query\" \
                                        stays true while the VERSIONED store's \
                                        `(name, version)` pairs remain immutable \
                                        (`tags/StoredQuery_schema.md`).",
            headers(
                ("Location" = String,
                 description = "\"The `Location` response header indicates the \
                                URL of the Stored Query resource\" (ITS-REST \
                                `specifications/headers/Location_Query.yaml`, \
                                example \
                                `https://openEHRSys.example.com/v1/definition/query/org.openehr::compositions/1.0.1`) \
                                — set here to \
                                `<base_path>/definition/query/<qualified_query_name>/<version>`, \
                                naming the version THIS request stored, never \
                                some other stored version of the same name. \
                                Declaring `Location` on a `200` follows the \
                                operation's own released response, which is the \
                                specific text over the general rule that the \
                                header \"MUST ONLY be used for resource creation \
                                (e.g., `201 Created`) or redirect responses\" \
                                (`Requests_and_responses.md` §Location) — a \
                                released-vs-released conflict, reported \
                                upstream.")
            )
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). The \
                                      concrete triggers here: the query-name \
                                      segment is the reserved `aql` \
                                      (case-insensitive) — \"the `query-name` \
                                      value must not be `aql` (case-insensitive), \
                                      as that is a reserved name\" \
                                      (`docs/query/Qualified_query_name.md` \
                                      NOTE); the body is not syntactically valid \
                                      AQL (a store-time parse, realizing the SM \
                                      `Pre_valid_query` precondition of \
                                      `store_query` — SM \
                                      `docs/UML/classes/i_definition_query.adoc` \
                                      — syntactically, which is the only branch \
                                      the released status set grounds: this \
                                      operation declares `200` and `400` only, \
                                      and QUERY 1.1.0 states no store-time \
                                      validity requirement); an unsupported \
                                      non-AQL `query_type`; or a body that is not \
                                      UTF-8 text.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a payload media type \
                                      the service cannot process as this \
                                      operation's `text/plain` AQL body: a \
                                      payload the service cannot process in the \
                                      declared format \"MUST respond with HTTP \
                                      status code `415 Unsupported Media Type`\" \
                                      (`Resources.md` §format rules — the same \
                                      cross-cutting MUST that grounds the \
                                      template-upload guards). An absent \
                                      `Content-Type` declares nothing to refuse \
                                      and is accepted. The released operation \
                                      does not enumerate `415`; the MUST is \
                                      cross-cutting (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `415` row).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_query_store_yaml(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_query_store.yaml",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Retrieve a named stored query at a specific version
/// (`GET /definition/query/{qualified_query_name}/{version}`).
#[utoipa::path(
    get, path = "/definition/query/{qualified_query_name}/{version}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query …, in a format \
                        of `[{namespace}::]{query-name}`\" (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`; \
                        grammar in \
                        `specifications/docs/query/Qualified_query_name.md`, \
                        including its NOTE that \"the `query-name` value must not \
                        be `aql` (case-insensitive), as that is a reserved \
                        name\"). Matched EXACTLY here — case-insensitively, and \
                        with a namespace-less name resolving against the `misc` \
                        namespace the store keys it under (SM \
                        `docs/openehr_platform/master04-definition_package.adoc` \
                        §Registered Queries) — never as a prefix; that is the \
                        sibling list operation's semantic.",
         example = "org.openehr::compositions"),
        ("version" = String, Path,
         description = "\"A SEMVER version number. This can be an exact version \
                        (e.g. `1.7.1`), or a pattern as partial prefix, in a form \
                        of `{major}` or `{major}.{minor}` (e.g. `1` or `1.0`), in \
                        which case the highest (latest) version matching the \
                        prefix will be considered\" (ITS-REST \
                        `specifications/parameters/path/version.yaml`; the same \
                        rule in `docs/query/Qualified_query_name.md`: \"the \
                        system must use the latest `version` with the supplied \
                        prefix\"). Prefixes match on a dot boundary (`1` matches \
                        `1.x.y`, never `10.0.0`) and resolve over the numeric \
                        `major.minor.patch` axis. A value that matches no stored \
                        version — including a malformed one — is the `404` below; \
                        the read side never rejects the selector itself, which is \
                        where it differs from the versioned STORE (whose \
                        `version` must be an exact three-part number).",
         example = "1.0"),
        ("Accept" = Option<String>, Header,
         description = "`application/json` — the single representation this \
                        operation declares \
                        (`specifications/parameters/header/Accept_JSON.yaml` on \
                        the operation, `responses/200_StoredQuery_get.yaml` for \
                        the body). Absent or `*/*` reads as JSON.",
         example = "application/json")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the stored AQL is \
                                        successfully retrieved\" (ITS-REST \
                                        `specifications/responses/200_StoredQuery_get.yaml`) \
                                        — the operation \"retrieves the \
                                        definition of a particular stored query \
                                        (at specified version) and its associated \
                                        metadata\" \
                                        (`operations/definition_query_version_get.yaml`). \
                                        The body is one `StoredQuery` object \
                                        carrying all five members the schema \
                                        REQUIRES — `name`, `type`, `version`, \
                                        `saved`, `q` \
                                        (`schemas/query/StoredQuery.yaml`). \
                                        `version` is the CONCRETE stored version \
                                        a prefix resolved to, never the prefix \
                                        that addressed it; `type` echoes the \
                                        stored formalism, recorded as `AQL` (the \
                                        `default`/`example` of \
                                        `schemas/query/QueryType.yaml`; the \
                                        released response examples spell it \
                                        lowercase and no released rule fixes the \
                                        casing); `saved` is the store timestamp, \
                                        a `format: date-time` string rendered as \
                                        UTC extended ISO 8601; `q` is returned \
                                        verbatim as stored. No `ETag` and no \
                                        `Last-Modified`: the released response \
                                        declares only `Content-Type`, and this \
                                        build adds neither on a stored query.",
            content((serde_json::Value = "application/json", example = json!({
                "name": "org.openehr::compositions",
                "type": "AQL",
                "version": "1.0.1",
                "saved": "2017-07-16T19:20:30Z",
                "q": "SELECT c FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c[$compositionid] WHERE c/name/value = 'Vitals'"
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). Here \
                                      the only reachable trigger is malformed \
                                      request URL syntax: a path segment whose \
                                      percent-encoding does not decode to valid \
                                      UTF-8, so no name/version can be read from \
                                      the URL. A well-formed selector that \
                                      resolves to nothing is the `404` below. The \
                                      released operation does not enumerate \
                                      `400`; the trigger is the cross-cutting \
                                      one.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a stored query \
                                      with `qualified_query_name` and `version` \
                                      does not exist\" (ITS-REST \
                                      `specifications/responses/404_Query_version.yaml`) \
                                      — the name is unknown, or no stored version \
                                      matches the exact value or the prefix.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: a \
                                      stored query has the single \
                                      `application/json` representation the \
                                      released response declares \
                                      (`200_StoredQuery_get.yaml`, and \
                                      `parameters/header/Accept_JSON.yaml` on the \
                                      operation), so an exclusively-XML `Accept` \
                                      is refused (`Resources.md` §\"JSON \
                                      Format\": \"If the service cannot fulfill \
                                      this aspect of the request, it MUST respond \
                                      with HTTP status code `406 Not \
                                      Acceptable`\"). The released operation does \
                                      not enumerate `406`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_query_version_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_query_version_get",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Store a named AQL query at a specific version
/// (`PUT /definition/query/{qualified_query_name}/{version}`).
///
/// The AQL text is the `text/plain` request body; the `(name, version)` pair is
/// immutable, so a second store at the same pair is a `409`.
#[utoipa::path(
    put, path = "/definition/query/{qualified_query_name}/{version}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query …, in a format \
                        of `[{namespace}::]{query-name}`\" (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`; \
                        grammar in \
                        `specifications/docs/query/Qualified_query_name.md`). Its \
                        NOTE is enforced: \"The `query-name` value must not be \
                        `aql` (case-insensitive), as that is a reserved name\" — \
                        such a name is the `400` below. Identity is \
                        case-insensitive, stored case-preserving; a \
                        namespace-less name is keyed under the assumed `misc` \
                        namespace (SM \
                        `docs/openehr_platform/master04-definition_package.adoc` \
                        §Registered Queries).",
         example = "org.openehr::compositions"),
        ("version" = String, Path,
         description = "The SEMVER version to store the query under — \"in the \
                        format specified by SEMVER style (i.e. \
                        `major.minor.patch`)\" \
                        (`docs/query/Qualified_query_name.md`). On the WRITE it \
                        must be an EXACT three-part numeric version: the prefix \
                        grammar of \
                        `specifications/parameters/path/version.yaml` (\"a \
                        pattern as partial prefix … the highest (latest) version \
                        matching the prefix will be considered\") is a \
                        READ-resolution semantic with nothing to resolve on a \
                        store — it would either name an already-stored pair, \
                        which this operation answers `409`, or name nothing at \
                        all — and the released text never completes a prefix \
                        into a version to create. Anything that is not \
                        `major.minor.patch` — a prefix (`1`, `1.0`), a \
                        pre-release/build suffix (`1.0.0-rc.1`), a non-numeric \
                        segment — is therefore a `400`, the status the docs \
                        text assigns to exactly this case \
                        (`docs/overview/Requests_and_responses.md` §\"HTTP \
                        status codes\": \"Status code `400` … a generic \
                        client-side error, used when no other `4xx` error code \
                        is appropriate. The client SHOULD NOT repeat the \
                        request without modifications\"). Treating the released \
                        silence this way is OURS, and it keeps the whole \
                        stored-query surface coherent: every list and get \
                        resolves versions numerically, so a non-numeric stored \
                        version would break reads of unrelated queries.",
         example = "1.0.1"),
        ("query_type" = Option<String>, Query,
         description = "\"Parameter indicating the query language/type\" \
                        (ITS-REST \
                        `specifications/parameters/query/query_type.yaml`, \
                        `default: \"AQL\"`); only AQL is supported, so an \
                        unsupported non-AQL formalism is an honest \
                        unsupported-formalism `400`. Matching is case-insensitive \
                        and accepts the SM formalism-with-version spelling (SM \
                        `master04-definition_package.adoc` §Query Formalism).",
         example = "AQL")
    ),
    request_body(content((String = "text/plain",
                          example = "SELECT c FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] CONTAINS OBSERVATION obs[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE obs/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= $systolic_bp")),
                 description = "\"The given AQL query\" \
                                (`schemas/query/AQL.yaml`) as `text/plain` — the \
                                only request content type the released operation \
                                declares (`required: true`). Unlike the \
                                version-less sibling this operation carries no \
                                `Content-Type` header parameter of its own; the \
                                body is read as UTF-8 text."),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 OK` \
                                        \"is returned when the query was \
                                        successfully stored\" (ITS-REST \
                                        `specifications/responses/200_StoredQuery_stored.yaml`) \
                                        — for the operation that \"stores a \
                                        query, at a specified `version`, on the \
                                        system\" \
                                        (`operations/definition_query_version_store.yaml`). \
                                        The response is BODYLESS: that released \
                                        `200` declares a `Location` header and no \
                                        `content` at all. The `(name, version)` \
                                        pair is written once and never \
                                        overwritten — a stored query is \
                                        \"a reusable, immutable way to identify a \
                                        specific AQL statement\" \
                                        (`tags/StoredQuery_schema.md`) — so a \
                                        repeat store at the same pair is the \
                                        `409` below, not a second `200`.",
            headers(
                ("Location" = String,
                 description = "\"The `Location` response header indicates the \
                                URL of the Stored Query resource\" (ITS-REST \
                                `specifications/headers/Location_Query.yaml`, \
                                example \
                                `https://openEHRSys.example.com/v1/definition/query/org.openehr::compositions/1.0.1`) \
                                — set here to \
                                `<base_path>/definition/query/<qualified_query_name>/<version>`, \
                                the exact version this request stored. Declaring \
                                `Location` on a `200` follows the operation's own \
                                released response, which is the specific text \
                                over the general rule that the header \"MUST ONLY \
                                be used for resource creation (e.g., `201 \
                                Created`) or redirect responses\" \
                                (`Requests_and_responses.md` §Location) — a \
                                released-vs-released conflict, reported \
                                upstream.")
            )
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the request \
                                      could not be parsed or is invalid (e.g. \
                                      malformed request URL syntax, missing \
                                      required header or parameter, or \
                                      syntactically invalid header, parameter or \
                                      content)\" (ITS-REST \
                                      `specifications/responses/400.yaml`). The \
                                      concrete triggers here: `version` is not an \
                                      exact three-part numeric SEMVER (see that \
                                      parameter); the query-name segment is the \
                                      reserved `aql` (case-insensitive) — \"the \
                                      `query-name` value must not be `aql` \
                                      (case-insensitive), as that is a reserved \
                                      name\" (`docs/query/Qualified_query_name.md` \
                                      NOTE); the body is not syntactically valid \
                                      AQL (the store-time parse realizing the SM \
                                      `Pre_valid_query` precondition of \
                                      `store_query`, SM \
                                      `docs/UML/classes/i_definition_query.adoc`, \
                                      syntactically — the only branch the \
                                      released status set grounds); an \
                                      unsupported non-AQL `query_type`; or a body \
                                      that is not UTF-8 text.",
         body = serde_json::Value),
        (status = 409, description = "The released trigger, verbatim: `409 \
                                      Conflict` \"is returned when a query with \
                                      the given `qualified_query_name` and \
                                      `version` already exists on the server\" \
                                      (ITS-REST \
                                      `specifications/responses/409_StoredQuery_version.yaml`) \
                                      — the pair is compared case-insensitively \
                                      on the name (BASE \
                                      `docs/base_types/master05-identification_package.adoc` \
                                      §Composite Identifiers and Case) and the \
                                      insert is race-safe, so two concurrent \
                                      stores of the same pair yield one `200` and \
                                      one `409`.",
         body = serde_json::Value),
        (status = 415, description = "The request DECLARES a payload media type \
                                      the service cannot process as this \
                                      operation's `text/plain` AQL body: a \
                                      payload the service cannot process in the \
                                      declared format \"MUST respond with HTTP \
                                      status code `415 Unsupported Media Type`\" \
                                      (`Resources.md` §format rules — the same \
                                      cross-cutting MUST that grounds the \
                                      template-upload guards). An absent \
                                      `Content-Type` declares nothing to refuse \
                                      and is accepted. The released operation \
                                      does not enumerate `415`; the MUST is \
                                      cross-cutting (`Requests_and_responses.md` \
                                      §\"HTTP status codes\", the `415` row).",
         body = serde_json::Value)
    )
)]
pub(crate) async fn definition_query_version_store_yaml(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "definition_query_version_store.yaml",
        parts,
        super::dispatch::dispatch,
    )
    .await
}
