// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 2 template operations of the Definition API group: list, upload,
//! get, example and the deprecated version get.

use crate::api::definition::template_adl2;
use crate::api::guarded_handler;

guarded_handler! {
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
    definition_template_adl2_list
        => |state, parts| async move { template_adl2::list(&state, &parts).await }
}

guarded_handler! {
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
                                          the body is not UTF-8 text, the source \
                                          fails the ADL2 grammar (S-code syntax \
                                          errors: unparseable content, an empty \
                                          body, a missing mandatory section), or the \
                                          source parses but fails an AOM2 validation \
                                          phase (V-codes such as VCORM or VCARM, or \
                                          VARXR for a constituent archetype the CDR \
                                          does not hold). The operation declares only \
                                          `201`, `400` and `409`, so every refusal of \
                                          an invalid template is this `400`; a \
                                          request with `Prefer: \
                                          return=representation` receives each rule \
                                          code and its detail in the problem's \
                                          `errors`.",
             body = serde_json::Value),
            (status = 409, description = "The released trigger, verbatim: `409 \
                                          Conflict` \"is returned when a template \
                                          with same `template_id` already exists\" \
                                          (ITS-REST \
                                          `specifications/responses/409_template_already_exists.yaml`) \
                                          — here, an ADL2 artefact already stored \
                                          under the same `ARCHETYPE_HRID`.",
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
             body = serde_json::Value)
        )
    )]
    definition_template_adl2_upload
        => |state, parts| async move { template_adl2::upload(&state, &parts).await }
}

guarded_handler! {
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
                            against its SEMVER `release_version` axis (BASE \
                            `docs/artefact_identification/master04-versioning.adoc`).",
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
    definition_template_adl2_get
        => |state, parts| async move { template_adl2::get(&state, &parts).await }
}

guarded_handler! {
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
                                            generator walks into an RM COMPOSITION \
                                            (an RM party for a demographic template), \
                                            serialized per `Accept`. No `ETag`: the \
                                            example is derived output, not a stored \
                                            resource with an identifier.",
                content((serde_json::Value = "application/json"), (serde_json::Value = "application/xml"), (serde_json::Value = "application/openehr.wt.flat+json"), (serde_json::Value = "application/openehr.wt.structured+json"))
            ),
            (status = 400, description = "`type` or `detail_level` is outside its enumerated set \
                                          — the error arm of the released either/or (\"fall back \
                                          to the closest supported level, or it may return an \
                                          error (typically `400 Bad Request`)\", stated on the ADL \
                                          1.4 sibling and applied uniformly here), and \
                                          \"syntactically invalid … parameter\" under \
                                          `responses/400.yaml`. It also answers a template whose \
                                          root is neither a COMPOSITION nor a party class, and a \
                                          party template that constrains no `PARTY_IDENTITY` (RM \
                                          demographic §PARTY `Identities_valid`).",
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
    definition_template_adl2_example_get
        => |state, parts| async move { template_adl2::example_get(&state, &parts).await }
}

guarded_handler! {
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
                            stored `ARCHETYPE_HRID` only (OUR reading): a stored \
                            pre-release/build suffix is ignored, and a `{version}` \
                            carrying one matches nothing (`404`). A value of neither \
                            form (`v1.x`, a leading zero) is the `400`.",
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
            (status = 400, description = "`version` is no SEMVER version and no \
                                          `{major}` or `{major}.{minor}` prefix \
                                          (`parameters/path/version.yaml`): a \
                                          \"syntactically invalid … parameter\" \
                                          (`specifications/responses/400.yaml`).",
             body = serde_json::Value),
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
    definition_template_adl2_version_get
        => |state, parts| async move { template_adl2::version_get(&state, &parts).await }
}
