// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored-query operations of the Definition API group: list, store,
//! version get and version store.

use crate::api::definition::stored_query;
use crate::api::guarded_handler;

guarded_handler! {
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
    definition_query_list_all
        => |state, parts| async move { stored_query::list_all(&state, &parts).await }
}

guarded_handler! {
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
                            the canonical key every surface stores under (our own \
                            naming design).",
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
    definition_query_list
        => |state, parts| async move { stored_query::list(&state, &parts).await }
}

guarded_handler! {
    /// Store a named AQL query, server-assigned SEMVER
    /// (`PUT /definition/query/{qualified_query_name}`).
    ///
    /// The AQL text is the `text/plain` request body.
    #[utoipa::path(
        put, path = "/definition/query/{qualified_query_name}", tag = "Query",
        operation_id = "definition_query_store.yaml",
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
                            the assumed `misc` namespace (our own naming design), \
                            the same canonical key \
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
                            case-insensitive and accepts the formalism-with-version \
                            spelling (`aql`, `AQL::1.0.3`).",
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
                                          AQL (a store-time parse, syntactically, \
                                          which is the only branch \
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
    definition_query_store_yaml as "definition_query_store.yaml"
        => |state, parts| async move { stored_query::store(&state, &parts).await }
}

guarded_handler! {
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
                            namespace the store keys it under — never as a prefix; that is the \
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
                            `major.minor.patch` axis. A well-formed value matching \
                            no stored version is the `404`, a malformed one (`v1.x`, \
                            a leading zero) the `400`; the versioned STORE takes an \
                            exact three-part number only.",
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
                                          the triggers are a path segment whose \
                                          percent-encoding is not valid UTF-8 and a \
                                          `version` that is no SEMVER version or \
                                          `{major}[.{minor}]` prefix. The released \
                                          operation does not enumerate `400`; the \
                                          trigger is the cross-cutting one.",
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
    definition_query_version_get
        => |state, parts| async move { stored_query::version_get(&state, &parts).await }
}

guarded_handler! {
    /// Store a named AQL query at a specific version
    /// (`PUT /definition/query/{qualified_query_name}/{version}`).
    ///
    /// The AQL text is the `text/plain` request body; the `(name, version)` pair is
    /// immutable, so a second store at the same pair is a `409`.
    #[utoipa::path(
        put, path = "/definition/query/{qualified_query_name}/{version}", tag = "Query",
        operation_id = "definition_query_version_store.yaml",
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
                            namespace (our own \
                            naming design).",
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
                            and accepts the formalism-with-version spelling \
                            (`aql`, `AQL::1.0.3`).",
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
                                          AQL (the store-time parse, \
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
    definition_query_version_store_yaml as "definition_query_version_store.yaml"
        => |state, parts| async move { stored_query::version_store(&state, &parts).await }
}
