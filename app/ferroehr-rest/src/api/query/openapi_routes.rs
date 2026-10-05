// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native `utoipa-axum` routing for the Query API group (ad-hoc + stored AQL).
//!
//! Operation semantics are the ITS-REST Query API (`docs/specs/openehr/ITS-REST`;
//! AQL 1.1); no openEHR spec governs the OAS layout. Each handler forwards to the
//! group dispatcher through [`guarded_dispatch`].
//!
//! The Query API is one of the few groups with dedicated released prose
//! (`docs/query/Request.md`, `Response.md`, `Query_types.md`,
//! `Qualified_query_name.md`, all STABLE in Release 1.1.0), so the declarations
//! below follow that text wherever it conflicts with the released OAS, which
//! grounds only what the docs text leaves silent.
//!
//! ## Prose-vs-OAS reconciliations (documented real wire)
//!
//! - **`ehr_id` scope** (`docs/query/Request.md` §About the `ehr_id`
//!   parameter): every operation — `GET` and `POST` alike — accepts the EHR
//!   scope as the `ehr_id` query parameter OR the `openehr-ehr-id` request
//!   header. Supplying both is only accepted when they name the same EHR; a
//!   conflict is a `400` (the released text is silent on precedence, so that
//!   is our own handling). A well-formed-but-absent `ehr_id` is a `404`, a malformed UUID
//!   a `400`.
//! - **URL parameters on the `POST`s**: `Request.md`'s SHOULD-list is headed
//!   "All query execution requests" and draws no `GET`/`POST` distinction, so
//!   the three `POST`s accept `offset`/`fetch`/named `$parameter` binds from the
//!   query string as well as from the body. Precedence between the two carriers
//!   is unassigned, so the same rule applies: equal values are accepted,
//!   conflicting values are a `400`.
//! - **JSON-only response** (`200_Query.yaml` declares `application/json`; the
//!   `RESULT_SET` has no canonical-XML shape): an exclusively-XML `Accept`
//!   negotiates to `406` on every operation — documented below as our real wire
//!   though the OAS does not enumerate `406`. The `POST`s answer a non-JSON
//!   request `Content-Type` with `415`, likewise OAS-undeclared and likewise a
//!   cross-cutting MUST (`Resources.md` §"JSON Format").
//! - **`ETag`** (`200_Query.yaml` + `headers/ETag_RESULT_SET.yaml`): the `200`
//!   carries a weak `W/"…"` `ETag` — a deterministic content digest of the
//!   assembled `RESULT_SET` (the schema carries no `id`), set only on success.
//! - **`408`** (`408_Query.yaml`): a query that overruns the configured
//!   execution budget (`FERROEHR__QUERY__TIMEOUT_MS`) is a `408 Request Timeout`.
//! - **POST body** (`AdhocQueryExecute` / `Query` schemas): the POST forms carry
//!   `offset`/`fetch`/`query_parameters` (and `q` for ad-hoc) in the JSON body;
//!   the `ehr_id` scope still comes from the query string or header. Every
//!   `Query` member is OPTIONAL on the wire — the released schema's
//!   `required: [offset, fetch, query_parameters]` cannot stand against the
//!   docs text that gives `offset` a default of `0` and leaves `fetch`
//!   implementation-defined (a required member cannot default), so `{}`
//!   executes a parameterless stored query.

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

/// The Query API group as a native `utoipa-axum` router (group-relative paths).
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            query_execute_adhoc_query,
            query_execute_adhoc_query_body
        ))
        .routes(routes!(
            query_execute_stored_query,
            query_execute_stored_query_body
        ))
        .routes(routes!(
            query_execute_stored_query_version,
            query_execute_stored_query_version_body
        ))
        .routes(routes!(query_execute_cohort))
}

/// Execute an ad-hoc AQL query from the query string (`GET /query/aql`).
///
/// An ad-hoc query carries its own definition: it has no stored identifier and
/// is "executed as-is, as part of the request body or as a query parameter"
/// (`docs/query/Query_types.md` §Ad-hoc queries). This is the query-string form;
/// the request-body form is the sibling `POST` on the same path, which
/// `Request.md` §"GET vs POST" recommends for anything long: "Requests based on
/// the `GET` method have URI length restriction, or some characters might not be
/// allowed and have to be encoded. Long queries in the `q` parameter and having
/// a long list of `query_parameters` may add up to reach that limit, thus we
/// recommend clients using the `POST` method instead of `GET`." That is a
/// CLIENT-side recommendation about URI limits — this server imposes no
/// preference and both forms execute identically.
#[utoipa::path(
    get, path = "/query/aql", tag = "Query",
    params(
        ("q" = String, Query,
         description = "\"The AQL query to be executed\" (ITS-REST \
                        `specifications/parameters/query/q.yaml`), REQUIRED on \
                        this form. An absent `q` is the released `400` \
                        trigger's own first example — \"the server was unable \
                        to execute the query due to invalid input, e.g. a \
                        required parameter is missing\" \
                        (`specifications/responses/400_Query.yaml`) — and so is \
                        AQL that does not parse or type-check.",
         example = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c"),
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter). The pre-1.1.0 MixedCase spelling \
                        `openEHR-EHR-id` is the DEPRECATED form of this same \
                        header (`Requests_and_responses.md` §\"Deprecated \
                        headers\": they \"remain available for backward \
                        compatibility\") and resolves identically, HTTP field \
                        names being case-insensitive (RFC 9110 §5.1). \
                        Accepted alongside the `ehr_id` query parameter only \
                        when both name the same EHR; a conflict is a `400` — \
                        the released text says \"or alternatively\" and never \
                        assigns a precedence, so that handling is OURS \
                        (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`), \
                        an EHR identifier \"taken from EHR.ehr_id.value\". It \
                        \"SHOULD be supplied by clients when executing single \
                        EHR queries and MAY be used by the underlying backend \
                        to perform routing, optimizations or similar. It MUST \
                        NOT be supplied for 'population queries' and similar \
                        multi-patient queries\" (`Request.md` §About the \
                        `ehr_id` parameter) — omitting it is exactly the \
                        population-query case (`Query_types.md` §Population \
                        queries). May instead be supplied as the \
                        `openehr-ehr-id` header (both together only when they \
                        agree; a conflict is a `400`). What the scope DOES to \
                        the executed query is never stated by the released \
                        text — it is worded only as a routing hint — so this \
                        server filters the executed set to that EHR, which \
                        means an in-AQL predicate naming a DIFFERENT EHR simply \
                        yields no rows; that reading is OURS. A malformed \
                        (non-UUID) value is a `400`, a well-formed one naming \
                        no EHR a `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). A \
                        negative value is rejected `400`.",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`) — the \
                        page size. The one released PROHIBITION: `fetch` \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters), so `fetch` \
                        alongside the deprecated AQL `TOP` modifier is a `400`. \
                        Its relation to AQL `LIMIT`/`OFFSET` is left totally \
                        unlegislated (the exclusion clause names only `TOP`): \
                        this server COMPOSES the REST page OVER the AQL-shaped \
                        result set — the window is cut out of the rows the AQL \
                        `LIMIT`/`OFFSET` already selected — which is OUR OWN \
                        reading of that silence. A negative value is rejected \
                        `400`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds. The released form is one query-string \
                        entry PER PARAMETER under its own name: they are \
                        \"generically named `query_parameters` in this \
                        specification, but in the real request they will have \
                        specific names (e.g. `uid`, `systolic_bp`, etc.) \
                        according to their names in the query definition\", and \
                        \"provided query parameters SHOULD NOT be prefixed with \
                        `$` sign. Instead, the server will (whenever necessary) \
                        add the prefix or format queries as valid AQL queries\" \
                        (`Request.md` §Query parameters; worked example \
                        `?temperature_from=36&temperature_unit=Cel`). This \
                        server binds every un-prefixed query-string key that is \
                        not a request control, tolerates and strips a `$` \
                        prefix, and reads values as JSON first (`36` → number, \
                        `true` → boolean) falling back to text. The literal \
                        `query_parameters=<JSON object>` entry declared here is \
                        an accepted SUPERSET; on a name collision the named \
                        form wins. The released text never says which names are \
                        reserved — and its own `QueryParameters` example binds \
                        `ehr_id`, which collides with the request control of \
                        that name — so this server reserves `q`, `ehr_id`, \
                        `offset`, `fetch` and `query_parameters` from binding: \
                        OUR OWN handling of that silence.",
         example = "{\"temperature\":38.5}")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        query that matches nothing is this \
                                        `200` with `rows: []`, never a `404` \
                                        (`rows` is the only required member of \
                                        the released `ResultSet` schema). \
                                        `columns[]` pairs each projection's \
                                        `name` with its AQL `path`, and \
                                        \"when column alias is not present in \
                                        the AQL, a `0`-based column index is \
                                        used prefixed by a hash sign (i.e. \
                                        `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive for a primitive \
                                        projection, a canonical `_type`-tagged \
                                        RM object for an RM one, as the example \
                                        shows both. `meta` is \"a set of \
                                        optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        this server emits `_type`, \
                                        `_schema_version`, `_created` — \
                                        \"result creation time (in the extended \
                                        ISO 8601 format)\" — and \
                                        `_executed_aql`, \"the actual AQL query \
                                        that was executed by the server, after \
                                        replacing the query parameters\", while \
                                        `q` keeps the query exactly as \
                                        submitted. `meta._href` (\"URL of the \
                                        executed query (only for GET \
                                        endpoint)\", \
                                        `schemas/query/ResultSetMetadata.yaml`) \
                                        is not emitted — the whole `meta` set \
                                        is optional. `id` is ADDITIVE: the SM \
                                        RESULT_SET declares `id [1..1]`, \
                                        \"unique identifier of this result \
                                        set\" (SM `result_set.adoc`), which the \
                                        released ITS-REST schema omits, so it \
                                        is emitted alongside without breaking \
                                        the released shape. NO `Location` (it \
                                        \"MUST NOT be used to indicate an \
                                        alternate representation of an existing \
                                        resource\" and is for creation or \
                                        redirects only — \
                                        `Requests_and_responses.md` §Location) \
                                        and NO `Preference-Applied`: `Prefer` \
                                        negotiation is scoped to \"using \
                                        `POST`, `PUT`, or `DELETE` to create, \
                                        update, or delete a resource\" \
                                        (§\"Representation details \
                                        negotiation\") and executing a query \
                                        creates nothing.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", whose own \
                                example is already the weak `W/\"…\"` form, \
                                required since Release 1.1.0 — \"all `ETag` \
                                headers that hold a resource identifier MUST \
                                include a weakness indicator `W/`\" \
                                (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\"). The released `ResultSet` \
                                schema carries no identifier field, so there is \
                                no stored uid to publish: this tag is a \
                                DETERMINISTIC CONTENT DIGEST of the assembled \
                                RESULT_SET rendered in that weak form — \
                                re-executing the same query over unchanged data \
                                yields the same tag, any change to the result \
                                changes it. That derivation is OURS; no \
                                released text says what the identifier is \
                                computed from. Emitted only on this success \
                                path (a negotiated `406` is an error body and \
                                carries no RESULT_SET identifier). Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude AS temperature, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.body_temperature.v2] WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > 38.5"
                },
                "q": "SELECT e/ehr_id/value, o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude AS temperature, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.body_temperature.v2] WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > $temperature",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "temperature", "path": "/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude" },
                    { "name": "#2", "path": "/name" }
                ],
                "rows": [
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398", 38.9,
                      { "_type": "DV_TEXT", "value": "Body temperature" } ],
                    [ "347a5490-55ee-4da9-b91a-9bba710f730e", 39.4,
                      { "_type": "DV_TEXT", "value": "Body temperature" } ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: `q` is missing (the released \
                                      \"required parameter is missing\" case); \
                                      the AQL does not parse, uses an \
                                      unsupported construct, or fails path/type \
                                      analysis; a `$parameter` the query \
                                      references is unbound; `ehr_id` is not a \
                                      UUID; `offset`/`fetch` are not \
                                      non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP` \
                                      (the one released prohibition); or the \
                                      `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs.",
         body = serde_json::Value),
        (status = 404, description = "A well-formed `ehr_id` scope names no \
                                      existing EHR. This branch is OUR OWN \
                                      assignment: the released ad-hoc \
                                      operations declare no `404` at all, and \
                                      the SM `ehr_id_does_not_exist` error \
                                      (`i_query_service.adoc`) has no released \
                                      wire realization — this server raises it, \
                                      mapping to `404 Not Found`, so a client \
                                      is told its scope is wrong instead of \
                                      reading an empty result as \"no matching \
                                      data\". A malformed (non-UUID) `ehr_id` \
                                      is a `400`, and an EXISTING EHR with no \
                                      matching data is `200` with `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only \
                                      (`200_Query.yaml` declares that one \
                                      representation, and the canonical XML ITS \
                                      defines no RESULT_SET wire shape), so an \
                                      exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\"). The \
                                      released operation does not enumerate \
                                      `406`; the MUST is cross-cutting.",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`). \
                                      The maximum here is the configured \
                                      per-query execution budget \
                                      (`FERROEHR__QUERY__TIMEOUT_MS`); with no \
                                      budget configured only the global request \
                                      timeout applies.",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_adhoc_query(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_adhoc_query",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute an ad-hoc AQL query from the request body (`POST /query/aql`).
///
/// The body form of the same ad-hoc operation, and the one `Request.md`
/// §"GET vs POST" recommends to clients: "Requests based on the `GET` method
/// have URI length restriction, or some characters might not be allowed and
/// have to be encoded. Long queries in the `q` parameter and having a long list
/// of `query_parameters` may add up to reach that limit, thus we recommend
/// clients using the `POST` method instead of `GET`." That is a client-side
/// recommendation about URI limits only — the two forms execute identically
/// here.
///
/// `Request.md`'s parameter list is headed "All query execution requests SHOULD
/// support at least the following parameters" and draws no `GET`/`POST`
/// distinction, so this operation ALSO accepts `offset`, `fetch` and named
/// `$parameter` binds from the query string. When a value arrives both in the
/// URL and in the body, equal values are accepted and conflicting ones are a
/// `400` — the released text assigns no precedence, so that is OUR OWN handling,
/// the same rule the two `ehr_id` carriers follow.
#[utoipa::path(
    post, path = "/query/aql", tag = "Query",
    params(
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter) — the `AdhocQueryExecute` body has no \
                        `ehr_id` member, so the scope always comes from one of \
                        these two. The deprecated MixedCase spelling \
                        `openEHR-EHR-id` resolves identically \
                        (`Requests_and_responses.md` §\"Deprecated headers\"; \
                        RFC 9110 §5.1). Accepted alongside the `ehr_id` query \
                        parameter only when both name the same EHR; a conflict \
                        is a `400` (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`); \
                        it \"MUST NOT be supplied for 'population queries' and \
                        similar multi-patient queries\" (`Request.md` §About \
                        the `ehr_id` parameter). May instead be the \
                        `openehr-ehr-id` header (both only when they agree; a \
                        conflict is a `400`). Malformed → `400`; well-formed \
                        but naming no EHR → `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). \
                        Accepted here as well as in the body, because the \
                        released parameter list covers \"all query execution \
                        requests\"; a value in both carriers must agree or the \
                        request is a `400` (OUR OWN precedence handling).",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); it \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters) — that pairing is \
                        a `400`. Its relation to AQL `LIMIT`/`OFFSET` is \
                        unlegislated; this server composes the REST page OVER \
                        the AQL-shaped result set (OUR OWN reading). Accepted \
                        here as well as in the body, with the same must-agree \
                        rule as `offset`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds supplied in the URL — the named form \
                        of `Request.md` §Query parameters (\"provided query \
                        parameters SHOULD NOT be prefixed with `$` sign. \
                        Instead, the server will (whenever necessary) add the \
                        prefix or format queries as valid AQL queries\"), \
                        accepted on this `POST` for the same \"all query \
                        execution requests\" reason. The request controls `q`, \
                        `ehr_id`, `offset`, `fetch` and `query_parameters` \
                        never bind as AQL parameters (OUR OWN reservation — the \
                        released text names no reserved set, and its own \
                        `QueryParameters` example binds `ehr_id`). A name \
                        supplied both in the URL and in the body must carry the \
                        same value, else `400`.",
         example = "{\"temperature\":38.5}")
    ),
    request_body(
        content((serde_json::Value = "application/json", example = json!({
            "q": "SELECT e/ehr_id/value, o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude AS temperature, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.body_temperature.v2] WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > $temperature ORDER BY temperature DESC",
            "offset": 0,
            "fetch": 10,
            "query_parameters": { "temperature": 38.5 }
        }))),
        description = "An `AdhocQueryExecute` \
                       (`schemas/query/AdhocQueryExecute.yaml`): `q` REQUIRED, \
                       plus the optional `offset`, `fetch` and \
                       `query_parameters`. `query_parameters` is \"a set of \
                       query parameters\" whose members are JSON-TYPED — the \
                       released `Request.md` body example binds \
                       `\"temperature\": 38.5` (a number) beside \
                       `\"chills\": \"at0.64\"` (a string) — and the names are \
                       un-prefixed: \"provided query parameters SHOULD NOT be \
                       prefixed with `$` sign\" (§Query parameters). The EHR \
                       scope is NOT a body member: it is the `ehr_id` query \
                       parameter or the `openehr-ehr-id` header."
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        query that matches nothing is this \
                                        `200` with `rows: []`, never a `404` \
                                        (`rows` is the only required member of \
                                        the released `ResultSet` schema). \
                                        `columns[]` pairs each projection's \
                                        `name` with its AQL `path`, and \
                                        \"when column alias is not present in \
                                        the AQL, a `0`-based column index is \
                                        used prefixed by a hash sign (i.e. \
                                        `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive or a canonical \
                                        `_type`-tagged RM object, as the \
                                        example shows both. `meta` is \"a set \
                                        of optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        `_type`, `_schema_version`, `_created` \
                                        (\"in the extended ISO 8601 format\") \
                                        and `_executed_aql`, \"the actual AQL \
                                        query that was executed by the server, \
                                        after replacing the query parameters\" \
                                        — the example's `38.5` in place of \
                                        `$temperature` — while `q` keeps the \
                                        query as submitted. `meta._href` is \
                                        defined \"only for GET endpoint\" \
                                        (`schemas/query/ResultSetMetadata.yaml`) \
                                        and is therefore absent here by \
                                        definition. `id` is ADDITIVE (SM \
                                        `result_set.adoc` `id [1..1]`, omitted \
                                        by the released schema). NO `Location` \
                                        (creation/redirect only — \
                                        §Location) and NO `Preference-Applied`: \
                                        `Prefer` negotiation is scoped to \
                                        `POST`/`PUT`/`DELETE` that \"create, \
                                        update, or delete a resource\" \
                                        (§\"Representation details \
                                        negotiation\"), and this `POST` \
                                        executes a query rather than creating \
                                        anything.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", in the \
                                weak `W/\"…\"` form required since Release \
                                1.1.0 (\"all `ETag` headers that hold a \
                                resource identifier MUST include a weakness \
                                indicator `W/`\", `Requests_and_responses.md` \
                                §\"ETag and Last-Modified\"). The released \
                                `ResultSet` schema carries no identifier field, \
                                so ours is a DETERMINISTIC CONTENT DIGEST of \
                                the assembled RESULT_SET — same result, same \
                                tag — which is OUR OWN derivation. Emitted only \
                                on this success path. Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude AS temperature, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.body_temperature.v2] WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > 38.5 ORDER BY temperature DESC"
                },
                "q": "SELECT e/ehr_id/value, o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude AS temperature, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.body_temperature.v2] WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > $temperature ORDER BY temperature DESC",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "temperature", "path": "/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude" },
                    { "name": "#2", "path": "/name" }
                ],
                "rows": [
                    [ "347a5490-55ee-4da9-b91a-9bba710f730e", 39.4,
                      { "_type": "DV_TEXT", "value": "Body temperature" } ],
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398", 38.9,
                      { "_type": "DV_TEXT", "value": "Body temperature" } ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: the body is not parseable JSON or \
                                      is not an `AdhocQueryExecute`; `q` is \
                                      missing (the released \"required \
                                      parameter is missing\" case); the AQL \
                                      does not parse, uses an unsupported \
                                      construct, or fails path/type analysis; a \
                                      referenced `$parameter` is unbound; \
                                      `ehr_id` is not a UUID; `offset`/`fetch` \
                                      are not non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP`; \
                                      the `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs; or the URL and the body \
                                      carry CONFLICTING values for the same \
                                      `offset`/`fetch`/named parameter (OUR OWN \
                                      handling of the unassigned precedence — \
                                      equal values are accepted).",
         body = serde_json::Value),
        (status = 404, description = "A well-formed `ehr_id` scope names no \
                                      existing EHR. OUR OWN assignment: the \
                                      released ad-hoc operations declare no \
                                      `404`, and the SM `ehr_id_does_not_exist` \
                                      error (`i_query_service.adoc`) has no \
                                      released wire realization. A malformed \
                                      `ehr_id` is a `400`; an EXISTING EHR with \
                                      no matching data is `200` with \
                                      `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no RESULT_SET shape), so \
                                      an exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`) \
                                      — here the configured per-query execution \
                                      budget (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one \
                                      this operation can process: the body is \
                                      an `AdhocQueryExecute` in \
                                      `application/json` only, so any other \
                                      media type — `application/xml` included, \
                                      there being no canonical-XML request \
                                      shape for it — is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot process the request \
                                      payload as JSON format, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\"). The released operation \
                                      does not enumerate `415`; the MUST is \
                                      cross-cutting.",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_adhoc_query_body(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_adhoc_query_body",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute a named stored query, latest version, from the query string
/// (`GET /query/{qualified_query_name}`).
///
/// A stored query "has its definition stored (registered) on the server" and is
/// "identified by their qualified name and version" (`docs/query/Query_types.md`
/// §Stored queries) — the client sends no AQL, only the scope, the paging window
/// and the `$parameter` binds. With no `{version}` segment the resolution rule is
/// the prefix rule's limit case: "when only a partial `version` pattern is
/// supplied, or when `version` is not supplied at all, the system must use the
/// latest `version` with the supplied prefix" (`Qualified_query_name.md`), i.e.
/// the highest stored version of that name.
#[utoipa::path(
    get, path = "/query/{qualified_query_name}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query to be \
                        executed, in a format of `[{namespace}::]{query-name}`\" \
                        (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`). \
                        \"The `namespace` is optional, and when used it should \
                        be in a form of a reverse domain name, which allows for \
                        separation of use of stored queries by teams, \
                        companies, etc. The `query-name` may include any \
                        combination of characters, matched by the pattern \
                        `[a-zA-Z0-9_.-]`\", and \"the `query-name` value must \
                        not be `aql` (case-insensitive), as that is a reserved \
                        name\" (`docs/query/Qualified_query_name.md`; SPECITS-46) \
                        — this server refuses to STORE such a name, so \
                        `org.openehr::aql` and friends can never resolve and \
                        always land in the `404`. Executed at its latest \
                        version.",
         example = "org.openehr::compositions"),
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter). The deprecated MixedCase spelling \
                        `openEHR-EHR-id` resolves identically \
                        (`Requests_and_responses.md` §\"Deprecated headers\"; \
                        RFC 9110 §5.1). Accepted alongside the `ehr_id` query \
                        parameter only when both name the same EHR; a conflict \
                        is a `400` (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`); \
                        it \"MUST NOT be supplied for 'population queries' and \
                        similar multi-patient queries\" (`Request.md` §About \
                        the `ehr_id` parameter). May instead be the \
                        `openehr-ehr-id` header (both only when they agree; a \
                        conflict is a `400`). Malformed → `400`; well-formed \
                        but naming no EHR → `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). A \
                        negative value is rejected `400`.",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); it \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters) — the one \
                        released prohibition, a `400` when the stored AQL uses \
                        the deprecated `TOP` modifier. Its relation to AQL \
                        `LIMIT`/`OFFSET` is unlegislated; this server composes \
                        the REST page OVER the AQL-shaped result set (OUR OWN \
                        reading). A negative value is rejected `400`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds for the stored definition — one \
                        query-string entry per parameter, under the name the \
                        query definition uses: they \"will have specific names \
                        (e.g. `uid`, `systolic_bp`, etc.) according to their \
                        names in the query definition\", and \"provided query \
                        parameters SHOULD NOT be prefixed with `$` sign. \
                        Instead, the server will (whenever necessary) add the \
                        prefix or format queries as valid AQL queries\" \
                        (`Request.md` §Query parameters — whose worked examples \
                        are exactly this stored form, \
                        `GET …/query/myQuery?uid=…` and \
                        `…/query/com.vendor::compositions?temperature_from=36&temperature_unit=Cel`). \
                        A `$` prefix is tolerated and stripped; values are read \
                        as JSON first and fall back to text. The literal \
                        `query_parameters=<JSON object>` entry declared here is \
                        an accepted SUPERSET, the named form winning a \
                        collision. The request controls `q`, `ehr_id`, \
                        `offset`, `fetch` and `query_parameters` never bind \
                        (OUR OWN reservation — the released text names no \
                        reserved set, and its own `QueryParameters` example \
                        binds `ehr_id`).",
         example = "{\"temperature_from\":36}")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        stored query that matches nothing is \
                                        this `200` with `rows: []` (`rows` is \
                                        the only required member of the \
                                        released `ResultSet` schema); the \
                                        `404` is reserved for the QUERY not \
                                        existing. `name` carries the qualified \
                                        name that was executed. `columns[]` \
                                        pairs each projection's `name` with its \
                                        AQL `path`, and \"when column alias is \
                                        not present in the AQL, a `0`-based \
                                        column index is used prefixed by a hash \
                                        sign (i.e. `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive or a canonical \
                                        `_type`-tagged RM object, as the \
                                        example shows both. `meta` is \"a set \
                                        of optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        `_type`, `_schema_version`, `_created` \
                                        (\"in the extended ISO 8601 format\") \
                                        and `_executed_aql`, \"the actual AQL \
                                        query that was executed by the server, \
                                        after replacing the query parameters\" \
                                        — for a stored query, the STORED text \
                                        with the binds substituted, which is \
                                        also the only place the client sees \
                                        what it actually ran. `meta._href` \
                                        (\"URL of the executed query (only for \
                                        GET endpoint)\", \
                                        `schemas/query/ResultSetMetadata.yaml`) \
                                        is not emitted — the whole `meta` set \
                                        is optional. `id` is ADDITIVE (SM \
                                        `result_set.adoc` `id [1..1]`, omitted \
                                        by the released schema). NO `Location` \
                                        (creation/redirect only — §Location) \
                                        and NO `Preference-Applied` (`Prefer` \
                                        negotiation is scoped to create/update/ \
                                        delete — §\"Representation details \
                                        negotiation\").",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", in the \
                                weak `W/\"…\"` form required since Release \
                                1.1.0 (\"all `ETag` headers that hold a \
                                resource identifier MUST include a weakness \
                                indicator `W/`\", `Requests_and_responses.md` \
                                §\"ETag and Last-Modified\"). The released \
                                `ResultSet` schema carries no identifier field, \
                                so ours is a DETERMINISTIC CONTENT DIGEST of \
                                the assembled RESULT_SET — same result, same \
                                tag — which is OUR OWN derivation. It \
                                identifies the RESULT, not the stored query \
                                version. Emitted only on this success path. \
                                Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, c/context/start_time/value AS startTime, c/uid/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] WHERE c/context/start_time/value >= '2026-01-01'"
                },
                "name": "org.openehr::compositions",
                "q": "SELECT e/ehr_id/value, c/context/start_time/value AS startTime, c/uid/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] WHERE c/context/start_time/value >= $from",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "startTime", "path": "/context/start_time/value" },
                    { "name": "#2", "path": "/uid/value" }
                ],
                "rows": [
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398",
                      "2026-02-16T13:50:11.308+01:00",
                      "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: the STORED AQL fails path/type \
                                      analysis or uses an unsupported \
                                      construct; a `$parameter` the stored \
                                      query references is unbound; `ehr_id` is \
                                      not a UUID; `offset`/`fetch` are not \
                                      non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP`; \
                                      or the `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a stored query \
                                      with `qualified_query_name` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_Query.yaml`) \
                                      — resolution being name-bound here, since \
                                      this route carries no `{version}`. A name \
                                      whose `query-name` is the reserved `aql` \
                                      always lands here: such a name cannot be \
                                      stored (`Qualified_query_name.md`), so it \
                                      can never resolve. This server ALSO \
                                      answers `404` when a well-formed `ehr_id` \
                                      scope names no existing EHR — OUR OWN \
                                      assignment (the SM \
                                      `ehr_id_does_not_exist` error has no \
                                      released wire realization on the query \
                                      operations); a malformed `ehr_id` is a \
                                      `400`, and an existing EHR with no \
                                      matching data is `200` with `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no RESULT_SET shape), so \
                                      an exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`) \
                                      — here the configured per-query execution \
                                      budget (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_stored_query(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_stored_query",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute a named stored query, latest version, from the request body
/// (`POST /query/{qualified_query_name}`).
///
/// The body form of the latest-version stored execution: the client sends no
/// AQL (the definition is stored — `docs/query/Query_types.md` §Stored queries),
/// only the paging window and the `$parameter` binds. Every `Query` member is
/// OPTIONAL on this wire: the released schema's
/// `required: [offset, fetch, query_parameters]` cannot stand against the docs
/// text that gives `offset` a default of `0` and leaves `fetch`
/// implementation-defined (`Request.md` §Common Headers and Query Parameters) —
/// a required member cannot default — so `{}` is a valid body and executes a
/// parameterless stored query. `Request.md`'s parameter list is likewise headed
/// "All query execution requests" with no `GET`/`POST` distinction, so this
/// operation also accepts `offset`/`fetch`/named binds from the query string;
/// a value supplied in both carriers must agree, else `400` (OUR OWN handling of
/// the unassigned precedence, the same rule `ehr_id` follows).
#[utoipa::path(
    post, path = "/query/{qualified_query_name}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query to be \
                        executed, in a format of `[{namespace}::]{query-name}`\" \
                        (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`); \
                        the optional `namespace` \"should be in a form of a \
                        reverse domain name\" and \"the `query-name` may \
                        include any combination of characters, matched by the \
                        pattern `[a-zA-Z0-9_.-]`\", except that \"the \
                        `query-name` value must not be `aql` \
                        (case-insensitive), as that is a reserved name\" \
                        (`docs/query/Qualified_query_name.md`; SPECITS-46). \
                        Executed at its latest version.",
         example = "org.openehr::compositions"),
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter) — the `Query` body has no `ehr_id` member. \
                        The deprecated MixedCase spelling `openEHR-EHR-id` \
                        resolves identically (`Requests_and_responses.md` \
                        §\"Deprecated headers\"; RFC 9110 §5.1). Accepted \
                        alongside the `ehr_id` query parameter only when both \
                        name the same EHR; a conflict is a `400` \
                        (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`); \
                        it \"MUST NOT be supplied for 'population queries' and \
                        similar multi-patient queries\" (`Request.md` §About \
                        the `ehr_id` parameter). May instead be the \
                        `openehr-ehr-id` header (both only when they agree; a \
                        conflict is a `400`). Malformed → `400`; well-formed \
                        but naming no EHR → `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). \
                        Accepted here as well as in the body, because the \
                        released parameter list covers \"all query execution \
                        requests\"; a value in both carriers must agree or the \
                        request is a `400` (OUR OWN precedence handling).",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); it \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters) — a `400` when \
                        the stored AQL uses the deprecated `TOP` modifier. Its \
                        relation to AQL `LIMIT`/`OFFSET` is unlegislated; this \
                        server composes the REST page OVER the AQL-shaped \
                        result set (OUR OWN reading). Accepted here as well as \
                        in the body, with the same must-agree rule as `offset`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds supplied in the URL — the named form \
                        of `Request.md` §Query parameters, whose worked \
                        examples are exactly this stored shape \
                        (`…/query/com.vendor::compositions?temperature_from=36&temperature_unit=Cel`), \
                        and whose rule is \"provided query parameters SHOULD \
                        NOT be prefixed with `$` sign. Instead, the server will \
                        (whenever necessary) add the prefix or format queries \
                        as valid AQL queries\". Accepted on this `POST` for the \
                        \"all query execution requests\" reason. The request \
                        controls `q`, `ehr_id`, `offset`, `fetch` and \
                        `query_parameters` never bind as AQL parameters (OUR \
                        OWN reservation). A name supplied both in the URL and \
                        in the body must carry the same value, else `400`.",
         example = "{\"temperature_from\":36}")
    ),
    request_body(
        content((serde_json::Value = "application/json", example = json!({
            "query_parameters": { "temperature_from": 36, "temperature_unit": "Cel" }
        }))),
        description = "A `Query` (`schemas/query/Query.yaml`): `offset`, \
                       `fetch` and `query_parameters`, all OPTIONAL, and no \
                       `q` — the AQL is the stored definition. The released \
                       schema lists all three as `required`, but the docs text \
                       wins this real conflict with the released OAS: \
                       `offset` has \"default is `0`\" and `fetch`'s \"default \
                       depends on the implementation\" (`Request.md` §Common \
                       Headers and Query Parameters), and a required member \
                       cannot have a default — so an empty body `{}` is valid \
                       and runs the stored query with no binds and the default \
                       window. `query_parameters` members are JSON-TYPED and \
                       un-prefixed (\"provided query parameters SHOULD NOT be \
                       prefixed with `$` sign\", §Query parameters); the \
                       example binds `temperature_from` as a number and \
                       `temperature_unit` as a string. The EHR scope is not a \
                       body member: it is the `ehr_id` query parameter or the \
                       `openehr-ehr-id` header."
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        stored query that matches nothing is \
                                        this `200` with `rows: []` (`rows` is \
                                        the only required member of the \
                                        released `ResultSet` schema); the `404` \
                                        is reserved for the QUERY not existing. \
                                        `name` carries the qualified name that \
                                        was executed. `columns[]` pairs each \
                                        projection's `name` with its AQL \
                                        `path`, and \"when column alias is not \
                                        present in the AQL, a `0`-based column \
                                        index is used prefixed by a hash sign \
                                        (i.e. `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive or a canonical \
                                        `_type`-tagged RM object, as the \
                                        example shows both. `meta` is \"a set \
                                        of optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        `_type`, `_schema_version`, `_created` \
                                        (\"in the extended ISO 8601 format\") \
                                        and `_executed_aql`, \"the actual AQL \
                                        query that was executed by the server, \
                                        after replacing the query parameters\" \
                                        — the STORED text with the binds \
                                        substituted. `meta._href` is defined \
                                        \"only for GET endpoint\" \
                                        (`schemas/query/ResultSetMetadata.yaml`) \
                                        and is therefore absent here by \
                                        definition. `id` is ADDITIVE (SM \
                                        `result_set.adoc` `id [1..1]`, omitted \
                                        by the released schema). NO `Location` \
                                        (creation/redirect only — §Location) \
                                        and NO `Preference-Applied`: `Prefer` \
                                        negotiation is scoped to \
                                        `POST`/`PUT`/`DELETE` that \"create, \
                                        update, or delete a resource\" \
                                        (§\"Representation details \
                                        negotiation\"), and this `POST` \
                                        executes a query.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", in the \
                                weak `W/\"…\"` form required since Release \
                                1.1.0 (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\": \"all `ETag` headers that hold \
                                a resource identifier MUST include a weakness \
                                indicator `W/`\"). The released `ResultSet` \
                                schema carries no identifier field, so ours is \
                                a DETERMINISTIC CONTENT DIGEST of the assembled \
                                RESULT_SET — OUR OWN derivation. It identifies \
                                the RESULT, not the stored query version. \
                                Emitted only on this success path. Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= 36 AND o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/units = 'Cel'"
                },
                "name": "com.vendor::compositions",
                "q": "SELECT e/ehr_id/value, o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= $temperature_from AND o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/units = $temperature_unit",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "systolic", "path": "/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude" },
                    { "name": "#2", "path": "/name" }
                ],
                "rows": [
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398", 140,
                      { "_type": "DV_TEXT", "value": "Blood pressure" } ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: the body is not parseable JSON or \
                                      is not a `Query`; the STORED AQL fails \
                                      path/type analysis or uses an unsupported \
                                      construct; a `$parameter` the stored \
                                      query references is unbound; `ehr_id` is \
                                      not a UUID; `offset`/`fetch` are not \
                                      non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP`; \
                                      the `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs; or the URL and the body \
                                      carry CONFLICTING values for the same \
                                      `offset`/`fetch`/named parameter (OUR OWN \
                                      handling of the unassigned precedence — \
                                      equal values are accepted). An EMPTY body \
                                      `{}` is NOT an error.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a stored query \
                                      with `qualified_query_name` does not \
                                      exist\" (ITS-REST \
                                      `specifications/responses/404_Query.yaml`) \
                                      — name-bound, this route carrying no \
                                      `{version}`. A `query-name` of the \
                                      reserved `aql` always lands here (such a \
                                      name cannot be stored). This server ALSO \
                                      answers `404` when a well-formed `ehr_id` \
                                      scope names no existing EHR — OUR OWN \
                                      assignment (the SM \
                                      `ehr_id_does_not_exist` error has no \
                                      released wire realization on the query \
                                      operations); a malformed `ehr_id` is a \
                                      `400`, and an existing EHR with no \
                                      matching data is `200` with `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no RESULT_SET shape), so \
                                      an exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`) \
                                      — here the configured per-query execution \
                                      budget (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one \
                                      this operation can process: the body is a \
                                      `Query` in `application/json` only, so \
                                      any other media type — `application/xml` \
                                      included, there being no canonical-XML \
                                      request shape for it — is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot process the request \
                                      payload as JSON format, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\").",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_stored_query_body(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_stored_query_body",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute a named stored query at a specific version, from the query string
/// (`GET /query/{qualified_query_name}/{version}`).
///
/// The version-addressed form of the stored execution. `{version}` is matched
/// either exactly or as a prefix: "the `version` identifier is in the format
/// specified by SEMVER style (i.e. `major.minor.patch`). When only a partial
/// `version` pattern is supplied, or when `version` is not supplied at all, the
/// system must use the latest `version` with the supplied prefix - i.e. if only
/// `major` or `major.minor` is used, then the latest query version matching
/// supplied prefix will be used" (`docs/query/Qualified_query_name.md`).
#[utoipa::path(
    get, path = "/query/{qualified_query_name}/{version}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query to be \
                        executed, in a format of `[{namespace}::]{query-name}`\" \
                        (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`); \
                        the optional `namespace` \"should be in a form of a \
                        reverse domain name\" and \"the `query-name` may \
                        include any combination of characters, matched by the \
                        pattern `[a-zA-Z0-9_.-]`\", except that \"the \
                        `query-name` value must not be `aql` \
                        (case-insensitive), as that is a reserved name\" \
                        (`docs/query/Qualified_query_name.md`; SPECITS-46).",
         example = "org.openehr::compositions"),
        ("version" = String, Path,
         description = "\"A SEMVER version number. This can be an exact version \
                        (e.g. `1.7.1`), or a pattern as partial prefix, in a \
                        form of `{major}` or `{major}.{minor}` (e.g. `1` or \
                        `1.0`), in which case the highest (latest) version \
                        matching the prefix will be considered\" (ITS-REST \
                        `specifications/parameters/path/version.yaml`), the \
                        governing rule being \"when only a partial `version` \
                        pattern is supplied … the system must use the latest \
                        `version` with the supplied prefix\" \
                        (`docs/query/Qualified_query_name.md`). Matching is on \
                        a dot boundary, so `1` selects the highest `1.x.y` and \
                        `1.2` the highest `1.2.x`. A `{version}` that is \
                        neither an exact SEMVER nor such a numeric prefix \
                        matches no stored version and takes the `404` — no \
                        released text assigns a malformed `{version}` its own \
                        code, and the released `404` trigger is worded exactly \
                        as \"a stored query with `qualified_query_name` and \
                        `version` does not exist\", so that is where it lands.",
         example = "1.0"),
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter). The deprecated MixedCase spelling \
                        `openEHR-EHR-id` resolves identically \
                        (`Requests_and_responses.md` §\"Deprecated headers\"; \
                        RFC 9110 §5.1). Accepted alongside the `ehr_id` query \
                        parameter only when both name the same EHR; a conflict \
                        is a `400` (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`); \
                        it \"MUST NOT be supplied for 'population queries' and \
                        similar multi-patient queries\" (`Request.md` §About \
                        the `ehr_id` parameter). May instead be the \
                        `openehr-ehr-id` header (both only when they agree; a \
                        conflict is a `400`). Malformed → `400`; well-formed \
                        but naming no EHR → `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). A \
                        negative value is rejected `400`.",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); it \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters) — a `400` when \
                        the stored AQL uses the deprecated `TOP` modifier. Its \
                        relation to AQL `LIMIT`/`OFFSET` is unlegislated; this \
                        server composes the REST page OVER the AQL-shaped \
                        result set (OUR OWN reading). A negative value is \
                        rejected `400`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds for the stored definition — one \
                        query-string entry per parameter, under the name the \
                        query definition uses (`Request.md` §Query parameters: \
                        they \"will have specific names (e.g. `uid`, \
                        `systolic_bp`, etc.) according to their names in the \
                        query definition\", and \"provided query parameters \
                        SHOULD NOT be prefixed with `$` sign. Instead, the \
                        server will (whenever necessary) add the prefix or \
                        format queries as valid AQL queries\"). A `$` prefix is \
                        tolerated and stripped; values are read as JSON first \
                        and fall back to text. The literal \
                        `query_parameters=<JSON object>` entry declared here is \
                        an accepted SUPERSET, the named form winning a \
                        collision. The request controls `q`, `ehr_id`, \
                        `offset`, `fetch` and `query_parameters` never bind \
                        (OUR OWN reservation).",
         example = "{\"temperature_from\":36}")
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        stored query that matches nothing is \
                                        this `200` with `rows: []` (`rows` is \
                                        the only required member of the \
                                        released `ResultSet` schema); the `404` \
                                        is reserved for the QUERY VERSION not \
                                        existing. `name` carries the qualified \
                                        name that was executed — note that the \
                                        released `ResultSet` has no member for \
                                        the resolved VERSION, so which stored \
                                        version a prefix selected is not \
                                        observable on the wire. `columns[]` \
                                        pairs each projection's `name` with its \
                                        AQL `path`, and \"when column alias is \
                                        not present in the AQL, a `0`-based \
                                        column index is used prefixed by a hash \
                                        sign (i.e. `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive or a canonical \
                                        `_type`-tagged RM object, as the \
                                        example shows both. `meta` is \"a set \
                                        of optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        `_type`, `_schema_version`, `_created` \
                                        (\"in the extended ISO 8601 format\") \
                                        and `_executed_aql`, \"the actual AQL \
                                        query that was executed by the server, \
                                        after replacing the query parameters\" \
                                        — the STORED text with the binds \
                                        substituted. `meta._href` (\"URL of the \
                                        executed query (only for GET \
                                        endpoint)\", \
                                        `schemas/query/ResultSetMetadata.yaml`) \
                                        is not emitted — the whole `meta` set \
                                        is optional. `id` is ADDITIVE (SM \
                                        `result_set.adoc` `id [1..1]`, omitted \
                                        by the released schema). NO `Location` \
                                        (creation/redirect only — §Location) \
                                        and NO `Preference-Applied` (`Prefer` \
                                        negotiation is scoped to create/update/ \
                                        delete — §\"Representation details \
                                        negotiation\").",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", in the \
                                weak `W/\"…\"` form required since Release \
                                1.1.0 (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\": \"all `ETag` headers that hold \
                                a resource identifier MUST include a weakness \
                                indicator `W/`\"). The released `ResultSet` \
                                schema carries no identifier field, so ours is \
                                a DETERMINISTIC CONTENT DIGEST of the assembled \
                                RESULT_SET — OUR OWN derivation. It identifies \
                                the RESULT, not the stored query version. \
                                Emitted only on this success path. Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, c/context/start_time/value AS startTime, c/uid/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] WHERE c/context/start_time/value >= '2026-01-01'"
                },
                "name": "org.openehr::compositions",
                "q": "SELECT e/ehr_id/value, c/context/start_time/value AS startTime, c/uid/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] WHERE c/context/start_time/value >= $from",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "startTime", "path": "/context/start_time/value" },
                    { "name": "#2", "path": "/uid/value" }
                ],
                "rows": [
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398",
                      "2026-02-16T13:50:11.308+01:00",
                      "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1" ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: the STORED AQL fails path/type \
                                      analysis or uses an unsupported \
                                      construct; a `$parameter` the stored \
                                      query references is unbound; `ehr_id` is \
                                      not a UUID; `offset`/`fetch` are not \
                                      non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP`; \
                                      or the `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a stored query \
                                      with `qualified_query_name` and `version` \
                                      does not exist\" (ITS-REST \
                                      `specifications/responses/404_Query_version.yaml`) \
                                      — the identity is name AND version \
                                      together: an existing name with no \
                                      version matching the exact value or the \
                                      `{major}`/`{major}.{minor}` prefix is \
                                      this branch, as is a `{version}` that is \
                                      not a SEMVER value at all (no released \
                                      text gives a malformed `{version}` its \
                                      own code) and a `query-name` of the \
                                      reserved `aql` (such a name cannot be \
                                      stored). This server ALSO answers `404` \
                                      when a well-formed `ehr_id` scope names \
                                      no existing EHR — OUR OWN assignment (the \
                                      SM `ehr_id_does_not_exist` error has no \
                                      released wire realization on the query \
                                      operations); a malformed `ehr_id` is a \
                                      `400`, and an existing EHR with no \
                                      matching data is `200` with `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no RESULT_SET shape), so \
                                      an exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`) \
                                      — here the configured per-query execution \
                                      budget (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_stored_query_version(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_stored_query_version",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute a named stored query at a specific version, from the request body
/// (`POST /query/{qualified_query_name}/{version}`).
///
/// The body form of the version-addressed stored execution. `{version}` matches
/// exactly or as a prefix — "when only a partial `version` pattern is supplied
/// … the system must use the latest `version` with the supplied prefix - i.e.
/// if only `major` or `major.minor` is used, then the latest query version
/// matching supplied prefix will be used" (`docs/query/Qualified_query_name.md`).
/// As on the sibling `POST`, every `Query` body member is OPTIONAL (the docs
/// text's `offset` default of `0` and implementation-defined `fetch` cannot
/// coexist with the released schema's `required` list, and the docs text wins),
/// so `{}` executes the addressed version with no binds; and `offset`, `fetch`
/// and named `$parameter` binds are equally accepted from the query string,
/// conflicting values across the two carriers being a `400` (OUR OWN handling of
/// the unassigned precedence).
#[utoipa::path(
    post, path = "/query/{qualified_query_name}/{version}", tag = "Query",
    params(
        ("qualified_query_name" = String, Path,
         description = "\"The (fully qualified) name of the query to be \
                        executed, in a format of `[{namespace}::]{query-name}`\" \
                        (ITS-REST \
                        `specifications/parameters/path/qualified_query_name.yaml`); \
                        the optional `namespace` \"should be in a form of a \
                        reverse domain name\" and \"the `query-name` may \
                        include any combination of characters, matched by the \
                        pattern `[a-zA-Z0-9_.-]`\", except that \"the \
                        `query-name` value must not be `aql` \
                        (case-insensitive), as that is a reserved name\" \
                        (`docs/query/Qualified_query_name.md`; SPECITS-46).",
         example = "org.openehr::compositions"),
        ("version" = String, Path,
         description = "\"A SEMVER version number. This can be an exact version \
                        (e.g. `1.7.1`), or a pattern as partial prefix, in a \
                        form of `{major}` or `{major}.{minor}` (e.g. `1` or \
                        `1.0`), in which case the highest (latest) version \
                        matching the prefix will be considered\" (ITS-REST \
                        `specifications/parameters/path/version.yaml`; the \
                        governing rule is `Qualified_query_name.md`: \"the \
                        system must use the latest `version` with the supplied \
                        prefix\"). Matching is on a dot boundary — `1` selects \
                        the highest `1.x.y`, `1.2` the highest `1.2.x`. A \
                        `{version}` that is neither an exact SEMVER nor such a \
                        numeric prefix matches no stored version and takes the \
                        `404`; no released text assigns a malformed \
                        `{version}` its own code.",
         example = "1.0"),
        ("openehr-ehr-id" = Option<String>, Header,
         description = "The EHR scope in its header form: clients \"MAY supply \
                        it as a query parameter `ehr_id` or alternatively as a \
                        request header named `openehr-ehr-id`\" \
                        (`docs/query/Request.md` §About the `ehr_id` \
                        parameter) — the `Query` body has no `ehr_id` member. \
                        The deprecated MixedCase spelling `openEHR-EHR-id` \
                        resolves identically (`Requests_and_responses.md` \
                        §\"Deprecated headers\"; RFC 9110 §5.1). Accepted \
                        alongside the `ehr_id` query parameter only when both \
                        name the same EHR; a conflict is a `400` \
                        (no released text assigns a precedence).",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("ehr_id" = Option<String>, Query,
         description = "\"An optional parameter to execute the query within an \
                        EHR context\" (ITS-REST \
                        `specifications/parameters/query/ehr_id_Query.yaml`); \
                        it \"MUST NOT be supplied for 'population queries' and \
                        similar multi-patient queries\" (`Request.md` §About \
                        the `ehr_id` parameter). May instead be the \
                        `openehr-ehr-id` header (both only when they agree; a \
                        conflict is a `400`). Malformed → `400`; well-formed \
                        but naming no EHR → `404`.",
         example = "7d44b88c-4199-4bad-97dc-d78268e01398"),
        ("offset" = Option<i64>, Query,
         description = "\"The row number in result-set to start result-set from \
                        (`0`-based), default is `0`\" (ITS-REST \
                        `specifications/parameters/query/offset.yaml`). \
                        Accepted here as well as in the body, because the \
                        released parameter list covers \"all query execution \
                        requests\"; a value in both carriers must agree or the \
                        request is a `400` (OUR OWN precedence handling).",
         example = 10),
        ("fetch" = Option<i64>, Query,
         description = "\"Number of rows to fetch (the default depends on the \
                        implementation)\" (ITS-REST \
                        `specifications/parameters/query/fetch.yaml`); it \
                        \"cannot be combined with AQL-top\" (`Request.md` \
                        §Common Headers and Query Parameters) — a `400` when \
                        the stored AQL uses the deprecated `TOP` modifier. Its \
                        relation to AQL `LIMIT`/`OFFSET` is unlegislated; this \
                        server composes the REST page OVER the AQL-shaped \
                        result set (OUR OWN reading). Accepted here as well as \
                        in the body, with the same must-agree rule as `offset`.",
         example = 10),
        ("query_parameters" = Option<String>, Query,
         description = "AQL `$name` binds supplied in the URL — the named form \
                        of `Request.md` §Query parameters (\"provided query \
                        parameters SHOULD NOT be prefixed with `$` sign. \
                        Instead, the server will (whenever necessary) add the \
                        prefix or format queries as valid AQL queries\"), \
                        accepted on this `POST` for the \"all query execution \
                        requests\" reason. The request controls `q`, `ehr_id`, \
                        `offset`, `fetch` and `query_parameters` never bind as \
                        AQL parameters (OUR OWN reservation). A name supplied \
                        both in the URL and in the body must carry the same \
                        value, else `400`.",
         example = "{\"temperature_from\":36}")
    ),
    request_body(
        content((serde_json::Value = "application/json", example = json!({
            "query_parameters": { "temperature_from": 36, "temperature_unit": "Cel" }
        }))),
        description = "A `Query` (`schemas/query/Query.yaml`): `offset`, \
                       `fetch` and `query_parameters`, all OPTIONAL, and no \
                       `q` — the AQL is the stored definition at the addressed \
                       version. The released schema lists all three as \
                       `required`, but the docs text wins: `offset` has \
                       \"default is `0`\" and `fetch`'s \"default depends on \
                       the implementation\" (`Request.md` §Common Headers and \
                       Query Parameters), and a required member cannot have a \
                       default — so an empty body `{}` is valid and runs the \
                       addressed version with no binds and the default window. \
                       `query_parameters` members are JSON-TYPED and \
                       un-prefixed (\"provided query parameters SHOULD NOT be \
                       prefixed with `$` sign\", §Query parameters). The EHR \
                       scope is not a body member: it is the `ehr_id` query \
                       parameter or the `openehr-ehr-id` header."
    ),
    responses(
        (
            status = 200, description = "The released trigger, verbatim: `200 \
                                        OK` \"is returned when the server is \
                                        able to execute the query\" (ITS-REST \
                                        `specifications/responses/200_Query.yaml`) \
                                        — ability to EXECUTE, not to match: a \
                                        stored query that matches nothing is \
                                        this `200` with `rows: []` (`rows` is \
                                        the only required member of the \
                                        released `ResultSet` schema); the `404` \
                                        is reserved for the QUERY VERSION not \
                                        existing. `name` carries the qualified \
                                        name that was executed; the released \
                                        `ResultSet` has no member for the \
                                        RESOLVED version, so which stored \
                                        version a prefix selected is not \
                                        observable on the wire. `columns[]` \
                                        pairs each projection's `name` with its \
                                        AQL `path`, and \"when column alias is \
                                        not present in the AQL, a `0`-based \
                                        column index is used prefixed by a hash \
                                        sign (i.e. `#0`, `#1`...)\" \
                                        (`schemas/query/ResultSetColumn.yaml`). \
                                        A row is \"a set of cells representing \
                                        a RESULT_SET row, one cell for each \
                                        column. Content of a cell is `ANY`\" \
                                        (`schemas/query/ResultSetRow.yaml`) — a \
                                        JSON primitive or a canonical \
                                        `_type`-tagged RM object, as the \
                                        example shows both. `meta` is \"a set \
                                        of optional (implementation dependent) \
                                        attributes, useful for debugging\" \
                                        (`docs/query/Response.md` §Metadata): \
                                        `_type`, `_schema_version`, `_created` \
                                        (\"in the extended ISO 8601 format\") \
                                        and `_executed_aql`, \"the actual AQL \
                                        query that was executed by the server, \
                                        after replacing the query parameters\" \
                                        — the STORED text with the binds \
                                        substituted. `meta._href` is defined \
                                        \"only for GET endpoint\" \
                                        (`schemas/query/ResultSetMetadata.yaml`) \
                                        and is therefore absent here by \
                                        definition. `id` is ADDITIVE (SM \
                                        `result_set.adoc` `id [1..1]`, omitted \
                                        by the released schema). NO `Location` \
                                        (creation/redirect only — §Location) \
                                        and NO `Preference-Applied`: `Prefer` \
                                        negotiation is scoped to \
                                        `POST`/`PUT`/`DELETE` that \"create, \
                                        update, or delete a resource\" \
                                        (§\"Representation details \
                                        negotiation\"), and this `POST` \
                                        executes a query.",
            headers(
                ("ETag" = String,
                 description = "The weak entity tag of this result set — \
                                ITS-REST \
                                `specifications/headers/ETag_RESULT_SET.yaml`: \
                                \"the `ETag` (i.e. entity tag) response header \
                                is an identifier of the RESULT_SET\", in the \
                                weak `W/\"…\"` form required since Release \
                                1.1.0 (`Requests_and_responses.md` §\"ETag and \
                                Last-Modified\": \"all `ETag` headers that hold \
                                a resource identifier MUST include a weakness \
                                indicator `W/`\"). The released `ResultSet` \
                                schema carries no identifier field, so ours is \
                                a DETERMINISTIC CONTENT DIGEST of the assembled \
                                RESULT_SET — OUR OWN derivation. It identifies \
                                the RESULT, not the stored query version. \
                                Emitted only on this success path. Shape: \
                                `W/\"cdbb5db1-e466-4429-a9e5-bf80a54e120b\"`.")
            ),
            content((serde_json::Value = "application/json", example = json!({
                "id": "0826851c-c4c2-4d61-92b9-410fb8275ff0",
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": "1.1.0",
                    "_created": "2026-07-26T09:12:44.512331Z",
                    "_executed_aql": "SELECT e/ehr_id/value, o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= 36 AND o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/units = 'Cel'"
                },
                "name": "com.vendor::compositions",
                "q": "SELECT e/ehr_id/value, o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic, o/name FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v1] WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= $temperature_from AND o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/units = $temperature_unit",
                "columns": [
                    { "name": "#0", "path": "/ehr_id/value" },
                    { "name": "systolic", "path": "/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude" },
                    { "name": "#2", "path": "/name" }
                ],
                "rows": [
                    [ "7d44b88c-4199-4bad-97dc-d78268e01398", 140,
                      { "_type": "DV_TEXT", "value": "Blood pressure" } ]
                ]
            })))
        ),
        (status = 400, description = "The released trigger, verbatim: `400 Bad \
                                      Request` \"is returned when the server \
                                      was unable to execute the query due to \
                                      invalid input, e.g. a required parameter \
                                      is missing, or at least one of the \
                                      parameters has an invalid syntax\" \
                                      (ITS-REST \
                                      `specifications/responses/400_Query.yaml`). \
                                      Here: the body is not parseable JSON or \
                                      is not a `Query`; the STORED AQL fails \
                                      path/type analysis or uses an unsupported \
                                      construct; a `$parameter` the stored \
                                      query references is unbound; `ehr_id` is \
                                      not a UUID; `offset`/`fetch` are not \
                                      non-negative integers; `fetch` is \
                                      combined with the deprecated AQL `TOP`; \
                                      the `ehr_id` query parameter and the \
                                      `openehr-ehr-id` header name different \
                                      EHRs; or the URL and the body \
                                      carry CONFLICTING values for the same \
                                      `offset`/`fetch`/named parameter (OUR OWN \
                                      handling of the unassigned precedence — \
                                      equal values are accepted). An EMPTY body \
                                      `{}` is NOT an error.",
         body = serde_json::Value),
        (status = 404, description = "The released trigger, verbatim: `404 Not \
                                      Found` \"is returned when a stored query \
                                      with `qualified_query_name` and `version` \
                                      does not exist\" (ITS-REST \
                                      `specifications/responses/404_Query_version.yaml`) \
                                      — the identity is name AND version \
                                      together: an existing name with no \
                                      version matching the exact value or the \
                                      `{major}`/`{major}.{minor}` prefix is \
                                      this branch, as is a `{version}` that is \
                                      not a SEMVER value at all and a \
                                      `query-name` of the reserved `aql` (such \
                                      a name cannot be stored). This server \
                                      ALSO answers `404` when a well-formed \
                                      `ehr_id` scope names no existing EHR — \
                                      OUR OWN assignment (the SM \
                                      `ehr_id_does_not_exist` error has no \
                                      released wire realization on the query \
                                      operations); a malformed `ehr_id` is a \
                                      `400`, and an existing EHR with no \
                                      matching data is `200` with `rows: []`.",
         body = serde_json::Value),
        (status = 406, description = "The `Accept` header cannot be satisfied: \
                                      a RESULT_SET is served as canonical \
                                      `application/json` only (the canonical \
                                      XML ITS defines no RESULT_SET shape), so \
                                      an exclusively-XML `Accept` is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot fulfill this aspect of \
                                      the request, it MUST respond with HTTP \
                                      status code `406 Not Acceptable`\").",
         body = serde_json::Value),
        (status = 408, description = "The released trigger, verbatim: `408 \
                                      Request Timeout` \"is returned when there \
                                      is a query execution timeout (i.e. \
                                      maximum query execution time reached, \
                                      therefore the server aborted the \
                                      execution of the query)\" (ITS-REST \
                                      `specifications/responses/408_Query.yaml`) \
                                      — here the configured per-query execution \
                                      budget (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not one \
                                      this operation can process: the body is a \
                                      `Query` in `application/json` only, so \
                                      any other media type — `application/xml` \
                                      included, there being no canonical-XML \
                                      request shape for it — is refused \
                                      (`Resources.md` §\"JSON Format\": \"If \
                                      the service cannot process the request \
                                      payload as JSON format, it MUST respond \
                                      with HTTP status code `415 Unsupported \
                                      Media Type`\").",
         body = serde_json::Value),
        (status = 409, description = "OUR OWN EXTENSION — no openEHR spec \
                                      governs runtime \
                                      specification-generation selection. A \
                                      whole-object projection (`SELECT c FROM \
                                      EHR e CONTAINS COMPOSITION c`) serves a \
                                      stored version BODY, so it takes the \
                                      same generation gate the version reads \
                                      take: under `spec_profile = \"stable\"` \
                                      a projected version the released \
                                      generations cannot express refuses the \
                                      query — never elided from the rows, \
                                      never down-converted — naming the \
                                      version and the remedy. Leaf/scalar \
                                      projections over the same rows serve \
                                      data values rather than version bodies \
                                      and are NOT gated. Unreachable under the \
                                      default `development` profile.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_stored_query_version_body(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_stored_query_version_body",
        parts,
        super::dispatch::dispatch,
    )
    .await
}

/// Execute an AQL query over a cohort selected in the demographic domain
/// (`POST /query/cohort`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this operation. ITS-REST 1.1.0
/// publishes an ad-hoc and a stored query and nothing else, and AQL is defined
/// over the clinical domain alone (QUERY `master03-syntax.adoc`), with no
/// demographic source. The operation runs in three steps, each on its own
/// database credential: a predicate from the deployment's `[cohort.predicates]`
/// allow-list selects parties in the demographic domain, the linkage domain
/// resolves those parties to the EHRs they are the subject of, and the AQL runs
/// over exactly those EHRs. Only identifiers cross between the steps — no name,
/// address or national identifier leaves the demographic domain, and the caller
/// never receives a party id.
///
/// The result set carries a `meta.cohort` object with the cohort size, the
/// number of EHRs served, whether small-cell suppression withheld the rows, and
/// a SHA-256 digest of the predicate list, which is also how the one access
/// event this execution records names the cohort.
///
/// The route answers `404` until the deployment binds at least one predicate.
#[utoipa::path(
    post, path = "/query/cohort", tag = "Query",
    request_body(
        content((serde_json::Value = "application/json", example = json!({
            "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
            "cohort": [{ "name": "city", "value": "Groningen" }],
            "purpose": "secondary-use",
            "offset": 0,
            "fetch": 100
        }))),
        description = "`q` REQUIRED (the AQL), `cohort` REQUIRED (the \
                       predicates, intersected), plus the OPTIONAL `purpose`, \
                       `query_parameters`, `offset` and `fetch`. The last four \
                       are spelled as on the released `AdhocQueryExecute` body, \
                       so a client that already speaks the query API needs no \
                       second vocabulary. There is no `ehr_id`: the cohort IS \
                       the scope. Each `cohort` entry names a key the \
                       deployment bound in `[cohort.predicates]`; several \
                       entries INTERSECT. An undeclared member is refused \
                       `400`."
    ),
    responses(
        (status = 200, description = "The `RESULT_SET`, with a `meta.cohort` \
                                      object carrying `size` (the EHRs the \
                                      cohort resolved to), `served_ehrs` (how \
                                      many the rows came from, `0` when \
                                      suppressed), `suppressed`, and \
                                      `definition` (the predicate digest). \
                                      Suppression empties `rows` and says so \
                                      rather than pretending the cohort was \
                                      empty: a caller who cannot tell the two \
                                      apart widens the predicate until the \
                                      boundary leaks.",
         body = serde_json::Value,
         content_type = "application/json"),
        (status = 400, description = "The request names no predicate, names one \
                                      the deployment did not bind, carries a \
                                      value the binding's kind cannot read (an \
                                      age band that is not `<min>-<max>`), \
                                      pre-scopes the query with `ehr_ids`, or \
                                      carries AQL that does not parse or \
                                      type-check.",
         body = serde_json::Value),
        (status = 404, description = "The deployment binds no cohort predicate \
                                      (`[cohort.predicates]` is empty), so the \
                                      surface serves nothing. With \
                                      authentication enabled, an \
                                      unauthenticated request is answered `401` \
                                      first — the gate sits behind \
                                      authentication.",
         body = serde_json::Value),
        (status = 406, description = "The `RESULT_SET` has no canonical-XML \
                                      shape, so an exclusively-XML `Accept` \
                                      negotiates to `406` (`Resources.md` \
                                      §\"JSON Format\").",
         body = serde_json::Value),
        (status = 408, description = "The inner AQL execution overran the \
                                      configured per-query execution budget \
                                      (`FERROEHR__QUERY__TIMEOUT_MS`).",
         body = serde_json::Value),
        (status = 415, description = "The body is JSON only; any other request \
                                      `Content-Type` is refused \
                                      (`Resources.md` §\"JSON Format\").",
         body = serde_json::Value),
        (status = 422, description = "The predicate matched more parties than \
                                      `FERROEHR__COHORT__MAX_COHORT_SIZE` \
                                      allows. Refused rather than truncated: a \
                                      silently shortened cohort is a wrong \
                                      denominator with nothing on the wire to \
                                      say so.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn query_execute_cohort(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "query_execute_cohort",
        parts,
        super::dispatch::dispatch,
    )
    .await
}
