// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 template operations of the Definition API group: list,
//! upload, get and example.

use crate::api::definition::template_adl14;
use crate::api::guarded_handler;

guarded_handler! {
    /// List the stored ADL 1.4 operational templates
    /// (`GET /definition/template/adl1.4`).
    #[utoipa::path(
        get, path = "/definition/template/adl1.4", tag = "ADL1.4",
        operation_id = "definition_template_adl1.4_list",
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
    definition_template_adl1_4_list as "definition_template_adl1.4_list"
        => |state, parts| async move { template_adl14::list(&state, &parts).await }
}

guarded_handler! {
    /// Upload an ADL 1.4 operational template
    /// (`POST /definition/template/adl1.4`).
    ///
    /// The template arrives as canonical OPT XML (`Content-Type: application/xml`);
    /// a request declaring another payload type is `415`
    /// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
    /// §XML Format).
    #[utoipa::path(
        post, path = "/definition/template/adl1.4", tag = "ADL1.4",
        operation_id = "definition_template_adl1.4_upload",
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
             example = "return=representation"),
            ("Accept" = Option<String>, Header,
             description = "The media type of the `return=representation` body \
                            (`specifications/parameters/header/Accept_Template.yaml`): \
                            `application/xml` (the OPT; also the default for \
                            absent/`*/*`), or `application/openehr.wt+json` / \
                            `application/json` (the Web Template document), as \
                            `responses/201_Template_adl1_4_upload.yaml` declares. \
                            Any other `Accept` with `return=representation` is \
                            `406`, before the template is stored.",
             example = "application/openehr.wt+json")
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
                    (serde_json::Value = "application/openehr.wt+json"),
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
                                          the body is not UTF-8 text, is not \
                                          well-formed XML (mismatched tags, an empty \
                                          document with no root element), or is not \
                                          a usable operational template: it does not \
                                          decode as an OPT document, carries a \
                                          foreign or duplicated top-level element, a \
                                          blank `template_id` or an empty `concept`, \
                                          or breaks an AOM2 validity rule. The \
                                          operation declares only `201`, `400` and \
                                          `409`, so every refusal of an invalid \
                                          template is this `400`; a request with \
                                          `Prefer: return=representation` receives \
                                          each violation in the problem's `errors`.",
             body = serde_json::Value),
            (status = 409, description = "The released trigger, verbatim: `409 \
                                          Conflict` \"is returned when a template \
                                          with same `template_id` already exists\" \
                                          (ITS-REST \
                                          `specifications/responses/409_template_already_exists.yaml`).",
             body = serde_json::Value),
            (status = 406, description = "`Prefer: return=representation` with an \
                                          `Accept` outside \
                                          `parameters/header/Accept_Template.yaml`: \
                                          \"If the service cannot fulfill this aspect \
                                          of the request, it MUST respond with HTTP \
                                          status code `406 Not Acceptable`\" \
                                          (`Resources.md` §Simplified Formats). \
                                          Nothing is stored.",
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
             body = serde_json::Value)
        )
    )]
    definition_template_adl1_4_upload as "definition_template_adl1.4_upload"
        => |state, parts| async move { template_adl14::upload(&state, &parts).await }
}

guarded_handler! {
    /// Retrieve one ADL 1.4 template by id
    /// (`GET /definition/template/adl1.4/{template_id}`).
    #[utoipa::path(
        get, path = "/definition/template/adl1.4/{template_id}", tag = "ADL1.4",
        operation_id = "definition_template_adl1.4_get",
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
    definition_template_adl1_4_get as "definition_template_adl1.4_get"
        => |state, parts| async move { template_adl14::get(&state, &parts).await }
}

guarded_handler! {
    /// Generate an example COMPOSITION for an ADL 1.4 template
    /// (`GET /definition/template/adl1.4/{template_id}/example`).
    #[utoipa::path(
        get, path = "/definition/template/adl1.4/{template_id}/example", tag = "ADL1.4",
        operation_id = "definition_template_adl1.4_example_get",
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
                                            of the two Simplified Formats); for a \
                                            demographic template, an RM party of \
                                            the root class (PERSON, ORGANISATION, \
                                            GROUP, AGENT or ROLE) in canonical \
                                            JSON/XML. No \
                                            `ETag`: the example is derived output, \
                                            not a stored resource with an identifier \
                                            (`Requests_and_responses.md` §\"ETag and \
                                            Last-Modified\" scopes the header to \
                                            \"resources that have versioning or \
                                            unique state identifiers\").",
                content((serde_json::Value = "application/json"), (serde_json::Value = "application/xml"), (serde_json::Value = "application/openehr.wt.flat+json"), (serde_json::Value = "application/openehr.wt.structured+json"))
            ),
            (status = 400, description = "`type` or `detail_level` is outside its enumerated set. \
                                          The released operation leaves the handling open — when \
                                          the server \"does not support the requested `type` or \
                                          `detail_level`, it will fall back to the closest \
                                          supported level, or it may return an error (typically \
                                          `400 Bad Request`)\" — and this server takes the error \
                                          arm for an out-of-enum value (an out-of-enum value is \
                                          also \"syntactically invalid … parameter\" under \
                                          `responses/400.yaml`). Which arm of that either/or \
                                          applies is OURS. The same `400` answers a template whose \
                                          root is neither a COMPOSITION nor a party class (an \
                                          ENTRY or a SECTION): the `200` body is one of the \
                                          versioned-object classes, none of which such a template \
                                          admits, and a party template that constrains no \
                                          `PARTY_IDENTITY` (RM demographic §PARTY \
                                          `Identities_valid`).",
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
                                          `Accept_LOCATABLE` representations, or a \
                                          Simplified Format for a demographic \
                                          template's example party (the Simplified \
                                          Formats RM mapping, \
                                          `simplified_formats/master05-rm_mapping.adoc`, \
                                          maps no demographic party class).",
             body = serde_json::Value)
        )
    )]
    definition_template_adl1_4_example_get as "definition_template_adl1.4_example_get"
        => |state, parts| async move { template_adl14::example_get(&state, &parts).await }
}
