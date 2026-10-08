// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Structural completeness gate over the SERVED `OpenAPI` document — the only
//! `OpenAPI` we publish (owner hard rule: serve only what we generate). The
//! rules encode the ITS-REST conventions every declaration must document
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/
//! Requests_and_responses.md`: §"If-Match and accidental overwrites" — a
//! mismatch MUST be `412`; §Prefer; plus plain `OpenAPI` hygiene: every path
//! template parameter documented, every operation described, error outcomes
//! never omitted). A new endpoint that ships with a skeleton declaration
//! fails here — the completeness ratchet.

#![expect(
    clippy::expect_used,
    clippy::string_slice,
    clippy::missing_assert_message,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::Value;

use crate::common;

/// HTTP methods an `OpenAPI` path item may carry.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Fetch the full served document through the real router.
async fn served_document() -> Value {
    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let (_pg, app) = common::test_router().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/ferroehr/rest/api-docs/openapi.json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("openapi.json response");
    assert_eq!(resp.status(), http::StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("served openapi json")
}

/// The `{param}` names in a path template.
fn template_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push(rest[open + 1..open + close].to_owned());
        rest = &rest[open + close + 1..];
    }
    out
}

/// The documented parameter names of an operation, by location.
fn documented_params(op: &Value, location: &str) -> Vec<String> {
    op.get("parameters")
        .and_then(Value::as_array)
        .map(|params| {
            params
                .iter()
                .filter(|p| p.get("in").and_then(Value::as_str) == Some(location))
                .filter_map(|p| p.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Iterate `(path, method, operation)` over the whole document.
fn operations(doc: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            if let Some(item) = item.as_object() {
                for (method, op) in item {
                    if METHODS.contains(&method.as_str()) {
                        out.push((path.clone(), method.clone(), op.clone()));
                    }
                }
            }
        }
    }
    out
}

/// Rule 1: every `{param}` in a path template is a documented `Path`
/// parameter on every operation of that path.
#[tokio::test]
async fn every_path_template_parameter_is_documented() {
    let doc = served_document().await;
    let mut missing = Vec::new();
    for (path, method, op) in operations(&doc) {
        let documented = documented_params(&op, "path");
        for template in template_params(&path) {
            if !documented.contains(&template) {
                missing.push(format!("{} {path}: {{{template}}}", method.to_uppercase()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "path template parameters missing a documented Path parameter:\n{}",
        missing.join("\n")
    );
}

/// Rule 2: every operation carries a non-empty summary or description, and at
/// least one described NON-ERROR outcome — the thing that happens when the
/// request succeeds.
#[tokio::test]
async fn every_operation_is_described_with_a_success_outcome() {
    let doc = served_document().await;
    let mut findings = Vec::new();
    for (path, method, op) in operations(&doc) {
        let described = ["summary", "description"].iter().any(|k| {
            op.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        });
        if !described {
            findings.push(format!(
                "{} {path}: no summary/description",
                method.to_uppercase()
            ));
        }
        let responses = op.get("responses").and_then(Value::as_object);
        let has_success = responses.is_some_and(|r| {
            r.iter().any(|(code, body)| {
                // A 501-only operation (declared, not yet implemented — e.g.
                // the deferred ADL2 projections) documents its whole real
                // contract; a described 501 satisfies the rule. So does a 3xx:
                // for an operation whose whole contract IS a redirect (the
                // Swagger UI mount path), the redirect is the success outcome,
                // and requiring a 2xx there would only invite documenting a
                // response the route never sends.
                (code.starts_with('2') || code.starts_with('3') || code == "501")
                    && body
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some_and(|d| !d.trim().is_empty())
            })
        });
        if !has_success {
            findings.push(format!(
                "{} {path}: no described success outcome (2xx or 3xx)",
                method.to_uppercase()
            ));
        }
    }
    assert!(
        findings.is_empty(),
        "underdocumented operations:\n{}",
        findings.join("\n")
    );
}

/// Rule 3: every operation documents at least one error (4xx/5xx) outcome —
/// except the discovery/document endpoints that genuinely cannot fail at
/// the application layer (each exemption is deliberate and listed).
#[tokio::test]
async fn every_operation_documents_an_error_outcome() {
    // Discovery documents and static surfaces: a bare 200 is their whole
    // contract (no parameters, no state) — exempt by explicit decision.
    const EXEMPT: &[&str] = &[
        "/ferroehr/rest/api-docs/openapi.json",
        "/ferroehr/rest/.well-known/smart-configuration",
        "/ferroehr/rest/openehr/v1/definition/openapi.json",
        // Static/UI surface (config-gated at mount time, otherwise 200-only).
        "/ferroehr/rest/swagger-ui",
        // Probes and the status surface: 200-only by design (liveness never
        // errors at the application layer; readiness declares its 503).
        "/health",
        "/health/liveness",
        "/ferroehr/rest/status",
    ];
    let doc = served_document().await;
    let mut findings = Vec::new();
    for (path, method, op) in operations(&doc) {
        if EXEMPT.contains(&path.as_str()) || path.starts_with("/ferroehr/rest/api-docs/") {
            continue;
        }
        let has_error = op
            .get("responses")
            .and_then(Value::as_object)
            .is_some_and(|r| {
                r.keys()
                    .any(|code| code.starts_with('4') || code.starts_with('5'))
            });
        if !has_error {
            findings.push(format!("{} {path}", method.to_uppercase()));
        }
    }
    assert!(
        findings.is_empty(),
        "operations documenting no error outcome:\n{}",
        findings.join("\n")
    );
}

/// Rule 4: every PUT/DELETE on a change-controlled openEHR resource
/// documents the `If-Match` precondition AND its `412` outcome (overview
/// §"If-Match and accidental overwrites"). The exemptions are the surfaces
/// whose spec genuinely has no `If-Match` (item tags carry no version;
/// admin/extension deletes are not optimistic-concurrency controlled).
#[tokio::test]
async fn versioned_writes_document_if_match_and_412() {
    let doc = served_document().await;
    let mut findings = Vec::new();
    for (path, method, op) in operations(&doc) {
        if method != "put" && method != "delete" {
            continue;
        }
        let versioned = path.starts_with("/ferroehr/rest/openehr/v1/ehr/")
            && (path.ends_with("/directory")
                || path.ends_with("/ehr_status")
                || path.contains("/composition/"))
            || path.starts_with("/ferroehr/rest/openehr/v1/demographic/")
                && !path.contains("_tags");
        // Overview §"If-Match and accidental overwrites": If-Match is required
        // only "when the preceding_version_uid is not part of the endpoint
        // path segment" — a DELETE addressing {uid_based_id} carries the
        // preceding version IN THE PATH (stale → 409 with the latest ETag per
        // the delete response YAMLs), so it is the spec's own exemption.
        let preceding_in_path = method == "delete" && path.ends_with("{uid_based_id}");
        let exempt = path.contains("item_tag")
            || path.contains("/tags")
            || path.contains("/admin/")
            || path.contains("/versioned_")
            || preceding_in_path;
        if !versioned || exempt {
            continue;
        }
        let has_if_match = documented_params(&op, "header")
            .iter()
            .any(|n| n.eq_ignore_ascii_case("if-match"));
        let has_412 = op
            .get("responses")
            .and_then(Value::as_object)
            .is_some_and(|r| r.contains_key("412"));
        if !has_if_match || !has_412 {
            findings.push(format!(
                "{} {path}: if-match={has_if_match} 412={has_412}",
                method.to_uppercase()
            ));
        }
    }
    assert!(
        findings.is_empty(),
        "versioned writes missing If-Match/412 documentation (overview §If-Match):\n{}",
        findings.join("\n")
    );
}

/// The four EHR-resource operations are documented to full step-6
/// completeness: every real status branch, a `headers(...)` block on every
/// success response, the `Prefer`-conditional 201 body as a named example
/// pair, and a UUID-only `ehr_id`.
///
/// The branches are the released ITS-REST docs text (which wins every
/// conflict with the released OAS):
/// `Requests_and_responses.md` §"HTTP status codes" assigns `400` to
/// "malformed request syntax, syntactically invalid content", `409` to a
/// request that "might generate a duplicate or a conflict" and `422` to a
/// well-formed request "unable to be followed due to semantic errors";
/// `Resources.md` §"XML Format"/§"JSON Format"/§"Simplified Formats" make an
/// unprocessable request payload a `415` MUST and an unfulfillable `Accept` a
/// `406` MUST. `Requests_and_responses.md` §Location confines `Location` to
/// creation responses, so a read MUST NOT declare it.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one regression test per audited group keeps the pinned paths together"
)]
async fn ehr_resource_operations_are_fully_documented() {
    const EHR: &str = "/ferroehr/rest/openehr/v1/ehr";
    const EHR_BY_ID: &str = "/ferroehr/rest/openehr/v1/ehr/{ehr_id}";

    let doc = served_document().await;

    let codes = |op: &Value| -> Vec<String> {
        op["responses"]
            .as_object()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    };
    let header_names = |op: &Value, status: &str| -> Vec<String> {
        op["responses"][status]["headers"]
            .as_object()
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    };
    let param_description = |op: &Value, name: &str| -> String {
        op["parameters"]
            .as_array()
            .and_then(|params| {
                params
                    .iter()
                    .find(|p| p["name"].as_str() == Some(name))
                    .and_then(|p| p["description"].as_str())
            })
            .unwrap_or_default()
            .to_owned()
    };
    // The media-type entry of a `content` map, whatever the negotiated type is
    // keyed as (these operations declare one canonical media type).
    let first_content = |holder: &Value| -> Value {
        holder["content"]
            .as_object()
            .and_then(|c| c.values().next())
            .cloned()
            .unwrap_or_default()
    };

    // ── The two creates: every branch, every committal header, both bodies ──
    for (path, method) in [(EHR, "post"), (EHR_BY_ID, "put")] {
        let op = doc["paths"][path][method].clone();
        assert!(op.is_object(), "{method} {path} must be documented");
        let present = codes(&op);
        for expected in ["201", "400", "406", "409", "415", "422"] {
            assert!(
                present.iter().any(|c| c == expected),
                "{method} {path} must document {expected}; has {present:?}"
            );
        }
        let headers = header_names(&op, "201");
        for expected in ["ETag", "Location", "Last-Modified", "Preference-Applied"] {
            assert!(
                headers.iter().any(|h| h == expected),
                "{method} {path} 201 must document the {expected} header; has {headers:?}"
            );
        }
        // The Prefer-conditional body: utoipa cannot express a
        // request-header-conditional schema, so the two non-empty variants are
        // named examples on the 201 (§"Prefer minimal, identifier or full
        // representation response").
        let examples = first_content(&op["responses"]["201"])["examples"].clone();
        assert_eq!(
            examples["representation"]["value"]["_type"], "EHR",
            "{method} {path} 201 must carry the `representation` RM EHR example: {examples}"
        );
        assert!(
            examples["identifier"]["value"]["uid"].is_string(),
            "{method} {path} 201 must carry the `identifier` single-uid example: {examples}"
        );
        // The request body shows a real EHR_STATUS, archetype_details included.
        let body = first_content(&op["requestBody"])["example"].clone();
        assert_eq!(body["_type"], "EHR_STATUS", "{method} {path} body example");
        assert_eq!(
            body["archetype_details"]["_type"], "ARCHETYPED",
            "{method} {path} body example must carry archetype_details"
        );
        // All three Prefer tokens are enumerated on the header parameter.
        let prefer = param_description(&op, "Prefer");
        for token in [
            "return=minimal",
            "return=identifier",
            "return=representation",
        ] {
            assert!(
                prefer.contains(token),
                "{method} {path} Prefer description must enumerate {token}: {prefer}"
            );
        }
    }

    // ── The two reads: 400/406 branches, ETag documented, Location absent ───
    for path in [EHR, EHR_BY_ID] {
        let op = doc["paths"][path]["get"].clone();
        assert!(op.is_object(), "GET {path} must be documented");
        let present = codes(&op);
        for expected in ["200", "400", "404", "406"] {
            assert!(
                present.iter().any(|c| c == expected),
                "GET {path} must document {expected}; has {present:?}"
            );
        }
        let headers = header_names(&op, "200");
        assert!(
            headers.iter().any(|h| h == "ETag"),
            "GET {path} 200 must document the weak ETag; has {headers:?}"
        );
        assert!(
            !headers.iter().any(|h| h == "Location"),
            "GET {path} 200 must NOT declare Location (§Location: creation only)"
        );
        assert_eq!(
            first_content(&op["responses"]["200"])["example"]["_type"],
            "EHR",
            "GET {path} 200 must carry the real served EHR example"
        );
    }

    // ── `ehr_id` is UUID-only on the create-with-id path ────────────────────
    let put = doc["paths"][EHR_BY_ID]["put"].clone();
    let ehr_id = param_description(&put, "ehr_id");
    assert!(
        ehr_id.contains("UUID"),
        "PUT {EHR_BY_ID} must type ehr_id as a UUID: {ehr_id}"
    );
    assert!(
        !ehr_id.contains("strongly recommended"),
        "PUT {EHR_BY_ID} must not claim non-UUID HIER_OBJECT_IDs are accepted: {ehr_id}"
    );
}

/// The System API's one operation appears in the served document, fully
/// described (#418): a closure route mounted outside `OpenApiRouter` still
/// belongs to the composed `OpenAPI` — the served document describes the
/// whole served surface (ITS-REST System API, `operations/options.yaml`).
#[tokio::test]
async fn system_options_operation_is_documented() {
    let doc = served_document().await;
    let op = doc["paths"]["/ferroehr/rest/openehr/v1"]["options"].clone();
    assert!(
        op.is_object(),
        "OPTIONS on the API base path must be documented"
    );
    assert_eq!(op["operationId"], "options");
    let ok = &op["responses"]["200"];
    assert!(
        ok["headers"]["Allow"].is_object(),
        "the 200 documents the Allow header (200_options.yaml): {op}"
    );
    assert!(
        op["responses"]["406"].is_object(),
        "the exclusively-XML Accept branch is documented"
    );
    // The Options schema rode along into components.
    assert!(
        doc["components"]["schemas"]["Options"].is_object(),
        "the Options manifest schema is registered"
    );
}

/// The Demographic API group's 26 released operations are documented to full
/// step-6 completeness: every served status branch, a `headers(...)` block on
/// every success response that emits headers, released-shaped examples, and no
/// `Location` on a read or a delete.
///
/// The branches are the RELEASED ITS-REST docs text (the OAS is the
/// subordinate source). The five
/// party CRUD quintets are byte-identical on the wire
/// (`specifications/operations/{person,agent,group,organisation,role}_*.yaml`
/// and their `$ref`d responses), so the loop below asserts each kind
/// individually — a quintet that drifts apart fails here.
/// `Requests_and_responses.md` §Location confines `Location` to creation
/// responses and §"Deprecated headers" deprecates it on `GET`/`DELETE`;
/// §"If-Match and accidental overwrites" gives the update's `412` and its
/// missing-header `400`; `Resources.md` §"XML Format"/§"JSON Format"/
/// §"Simplified Formats" make an unprocessable payload a `415` and an
/// unfulfillable `Accept` a `406`.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one regression test per audited group keeps the pinned paths together"
)]
async fn demographic_operations_are_fully_documented() {
    const BASE: &str = "/ferroehr/rest/openehr/v1/demographic";
    const KINDS: &[(&str, &str)] = &[
        ("person", "PERSON"),
        ("agent", "AGENT"),
        ("group", "GROUP"),
        ("organisation", "ORGANISATION"),
        ("role", "ROLE"),
    ];

    let doc = served_document().await;

    let codes = |op: &Value| -> Vec<String> {
        op["responses"]
            .as_object()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    };
    let header_names = |op: &Value, status: &str| -> Vec<String> {
        op["responses"][status]["headers"]
            .as_object()
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    };
    let first_content = |holder: &Value| -> Value {
        holder["content"]
            .as_object()
            .and_then(|c| c.values().next())
            .cloned()
            .unwrap_or_default()
    };
    let require = |op: &Value, path: &str, method: &str, expected: &[&str]| {
        let present = codes(op);
        for want in expected {
            assert!(
                present.iter().any(|c| c == want),
                "{method} {path} must document {want}; has {present:?}"
            );
        }
    };

    for (segment, rm_type) in KINDS {
        // ── create: every branch, both Prefer bodies, the full header set ────
        let path = format!("{BASE}/{segment}");
        let op = doc["paths"][&path]["post"].clone();
        assert!(op.is_object(), "POST {path} must be documented");
        require(
            &op,
            &path,
            "POST",
            &["201", "400", "404", "406", "415", "422"],
        );
        let headers = header_names(&op, "201");
        for want in [
            "ETag",
            "Location",
            "Last-Modified",
            "Preference-Applied",
            "openehr-item-tag",
            "openehr-version-item-tag",
        ] {
            assert!(
                headers.iter().any(|h| h == want),
                "POST {path} 201 must document the {want} header; has {headers:?}"
            );
        }
        let examples = first_content(&op["responses"]["201"])["examples"].clone();
        assert_eq!(
            examples["representation"]["value"]["_type"], *rm_type,
            "POST {path} 201 must carry the `representation` RM {rm_type} example"
        );
        assert!(
            examples["identifier"]["value"]["uid"].is_string(),
            "POST {path} 201 must carry the `identifier` single-uid example"
        );
        let body = first_content(&op["requestBody"])["example"].clone();
        assert_eq!(body["_type"], *rm_type, "POST {path} request-body example");
        assert!(
            body["identities"].as_array().is_some_and(|i| !i.is_empty()),
            "POST {path} body example must satisfy PARTY.Identities_valid"
        );
        assert_eq!(
            body["name"]["value"], *rm_type,
            "POST {path} body example must satisfy PARTY.Type_valid (type = name)"
        );

        // ── get: the deleted-at-time 204, ETag + Last-Modified, no Location ──
        let path = format!("{BASE}/{segment}/{{uid_based_id}}");
        let op = doc["paths"][&path]["get"].clone();
        assert!(op.is_object(), "GET {path} must be documented");
        require(
            &op,
            &path,
            "GET",
            &["200", "204", "400", "404", "406", "415"],
        );
        let headers = header_names(&op, "200");
        for want in [
            "ETag",
            "Last-Modified",
            "openehr-item-tag",
            "openehr-version-item-tag",
        ] {
            assert!(
                headers.iter().any(|h| h == want),
                "GET {path} 200 must document the {want} header; has {headers:?}"
            );
        }
        assert!(
            !headers.iter().any(|h| h == "Location"),
            "GET {path} 200 must NOT declare Location (§Location: creation only)"
        );
        assert_eq!(
            first_content(&op["responses"]["200"])["example"]["_type"],
            *rm_type,
            "GET {path} 200 must carry the served RM {rm_type} example"
        );

        // ── update: If-Match + 412 with its ETag, both Prefer statuses ───────
        let op = doc["paths"][&path]["put"].clone();
        assert!(op.is_object(), "PUT {path} must be documented");
        require(
            &op,
            &path,
            "PUT",
            &["200", "204", "400", "404", "406", "412", "415", "422"],
        );
        assert!(
            documented_params(&op, "header")
                .iter()
                .any(|n| n.eq_ignore_ascii_case("if-match")),
            "PUT {path} must document the required If-Match precondition"
        );
        for status in ["200", "204"] {
            let headers = header_names(&op, status);
            for want in ["ETag", "Location", "Last-Modified", "Preference-Applied"] {
                assert!(
                    headers.iter().any(|h| h == want),
                    "PUT {path} {status} must document the {want} header; has {headers:?}"
                );
            }
        }
        let headers = header_names(&op, "412");
        assert!(
            headers.iter().any(|h| h == "ETag"),
            "PUT {path} 412 must echo the latest version_uid in ETag; has {headers:?}"
        );
        assert!(
            !headers.iter().any(|h| h == "Location"),
            "PUT {path} 412 must NOT declare Location (§Location: creation only)"
        );

        // ── delete: the deleted-version ETag, 409 with the latest ETag ───────
        let op = doc["paths"][&path]["delete"].clone();
        assert!(op.is_object(), "DELETE {path} must be documented");
        require(
            &op,
            &path,
            "DELETE",
            &["204", "400", "404", "406", "409", "415"],
        );
        let headers = header_names(&op, "204");
        assert!(
            headers.iter().any(|h| h == "ETag"),
            "DELETE {path} 204 must document the deleted version's ETag; has {headers:?}"
        );
        assert!(
            !headers.iter().any(|h| h == "Location"),
            "DELETE {path} 204 must NOT declare Location (§\"Deprecated headers\")"
        );
        assert!(
            header_names(&op, "409").iter().any(|h| h == "ETag"),
            "DELETE {path} 409 must echo the latest version_uid in ETag"
        );
    }

    // ── the four versioned_party reads: ETag, no Location, 400/404/406 ───────
    let versioned = [
        format!("{BASE}/versioned_party/{{versioned_object_uid}}"),
        format!("{BASE}/versioned_party/{{versioned_object_uid}}/revision_history"),
        format!("{BASE}/versioned_party/{{versioned_object_uid}}/version"),
        format!("{BASE}/versioned_party/{{versioned_object_uid}}/version/{{version_uid}}"),
    ];
    for path in &versioned {
        let op = doc["paths"][path]["get"].clone();
        assert!(op.is_object(), "GET {path} must be documented");
        require(&op, path, "GET", &["200", "400", "404", "406"]);
        let headers = header_names(&op, "200");
        assert!(
            headers.iter().any(|h| h == "ETag"),
            "GET {path} 200 must document the weak ETag; has {headers:?}"
        );
        assert!(
            !headers.iter().any(|h| h == "Location"),
            "GET {path} 200 must NOT declare Location (§Location: creation only)"
        );
    }
    // The container exposes no commit audit, the other three do.
    assert!(
        !header_names(&doc["paths"][&versioned[0]]["get"], "200")
            .iter()
            .any(|h| h == "Last-Modified"),
        "the VERSIONED_PARTY container has no commit audit to derive Last-Modified from"
    );
    // The container's 200 example carries `owner_id` in the shape the released
    // `VersionedParty` example shows — the plain `OBJECT_REF` the released
    // `ObjectRefOfHierObjectId` schema titles, `namespace: local`,
    // `type: SYSTEM` (vendored ITS-REST OAS
    // `crates/openehr-its/vendor/rest-oas/demographic-codegen.openapi.yaml`,
    // `components.schemas.VersionedParty.example`), not a
    // PARTY_REF and not a demographic-namespaced self-reference.
    let container_200 = &doc["paths"][&versioned[0]]["get"]["responses"]["200"];
    // The example sits in the media-typed `content` block, or bare on the
    // response when the declaration carries no schema.
    let container_example = container_200["content"]
        .as_object()
        .and_then(|c| c.values().next())
        .map_or_else(
            || container_200["example"].clone(),
            |media| media["example"].clone(),
        );
    assert_eq!(
        container_example["owner_id"]["_type"], "OBJECT_REF",
        "the VERSIONED_PARTY container example's owner_id is the plain \
         OBJECT_REF the released schema names"
    );
    assert_eq!(container_example["owner_id"]["namespace"], "local");
    assert_eq!(container_example["owner_id"]["type"], "SYSTEM");
    assert_eq!(
        container_example["owner_id"]["id"]["_type"], "HIER_OBJECT_ID",
        "the released example's owner_id id is a HIER_OBJECT_ID"
    );
    for path in &versioned[1..] {
        assert!(
            header_names(&doc["paths"][path]["get"], "200")
                .iter()
                .any(|h| h == "Last-Modified"),
            "GET {path} 200 must document Last-Modified (§\"ETag and Last-Modified\")"
        );
    }

    // ── the demographic CONTRIBUTION pair ────────────────────────────────────
    let path = format!("{BASE}/contribution");
    let op = doc["paths"][&path]["post"].clone();
    assert!(op.is_object(), "POST {path} must be documented");
    require(&op, &path, "POST", &["201", "400", "406", "409", "415"]);
    let headers = header_names(&op, "201");
    for want in ["ETag", "Location", "Preference-Applied"] {
        assert!(
            headers.iter().any(|h| h == want),
            "POST {path} 201 must document the {want} header; has {headers:?}"
        );
    }
    let body = first_content(&op["requestBody"])["example"].clone();
    assert!(
        body["versions"].as_array().is_some_and(|v| !v.is_empty()) && body["audit"].is_object(),
        "POST {path} body example must be a NewContribution (versions + audit)"
    );

    let path = format!("{BASE}/contribution/{{contribution_uid}}");
    let op = doc["paths"][&path]["get"].clone();
    assert!(op.is_object(), "GET {path} must be documented");
    require(&op, &path, "GET", &["200", "400", "404", "406"]);
    assert!(
        !header_names(&op, "200").iter().any(|h| h == "Location"),
        "GET {path} 200 must NOT declare Location (§Location: creation only)"
    );
    assert_eq!(
        first_content(&op["responses"]["200"])["example"]["_type"],
        "CONTRIBUTION",
        "GET {path} 200 must carry the served CONTRIBUTION example"
    );
}

/// The Demographic API group's sixteen released `ITEM_TAG` operations are
/// documented to full step-6 completeness: every served status branch, the
/// dual-form `uid_based_id` prose with its disjointness reading, the
/// `Prefer`/`Preference-Applied` pair on the five PUTs, the released-shaped
/// examples, and — because a tag collection is not change-controlled — NO
/// `ETag`, `Last-Modified` or `Location` anywhere on the family.
///
/// The five typed quintets are byte-identical on the released wire
/// (`specifications/operations/{person,agent,group,organisation,role}_tags_{get,update,delete}.yaml`
/// and their `$ref`d responses), so the loop asserts each kind individually — a
/// quintet that drifts apart fails here. The space-wide
/// `demographic_tags_get.yaml` is the delta: no scoping parameter at all, and
/// `200`/`400` as its only released branches (there is nothing to fail to
/// find, hence no `404`). `Resources.md` §"XML Format"/§"JSON Format" make the
/// unfulfillable `Accept` a `406` and the unprocessable payload a `415`;
/// `Requests_and_responses.md` §"Representation details negotiation" gives the
/// `Prefer` split and its `Preference-Applied` echo.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one regression test per audited group keeps the pinned paths together"
)]
async fn demographic_item_tag_operations_are_fully_documented() {
    const BASE: &str = "/ferroehr/rest/openehr/v1/demographic";
    const KINDS: &[(&str, &str)] = &[
        ("person", "PERSON"),
        ("agent", "AGENT"),
        ("group", "GROUP"),
        ("organisation", "ORGANISATION"),
        ("role", "ROLE"),
    ];

    let doc = served_document().await;

    let codes = |op: &Value| -> Vec<String> {
        op["responses"]
            .as_object()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    };
    let header_names = |op: &Value, status: &str| -> Vec<String> {
        op["responses"][status]["headers"]
            .as_object()
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    };
    let first_content = |holder: &Value| -> Value {
        holder["content"]
            .as_object()
            .and_then(|c| c.values().next())
            .cloned()
            .unwrap_or_default()
    };
    let require = |op: &Value, path: &str, method: &str, expected: &[&str]| {
        let present = codes(op);
        for want in expected {
            assert!(
                present.iter().any(|c| c == want),
                "{method} {path} must document {want}; has {present:?}"
            );
        }
    };
    // Whitespace-normalized (doc comments hard-wrap, so a released sentence
    // may span a line break in the served description).
    let text = |op: &Value| -> String {
        format!(
            "{} {}",
            op.get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            op.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    };
    let param_doc = |op: &Value, name: &str| -> String {
        op.get("parameters")
            .and_then(Value::as_array)
            .and_then(|params| {
                params
                    .iter()
                    .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
            })
            .and_then(|p| p.get("description").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    };
    // The served example of a response, wherever the declaration put it (a
    // media-typed `content(..)` block or the bare `example`).
    let response_example = |resp: &Value| -> Value {
        let via_content = first_content(resp)["example"].clone();
        if via_content.is_null() {
            resp["example"].clone()
        } else {
            via_content
        }
    };
    // A tag collection has no version and no uid, so none of the
    // change-control headers may be declared anywhere on this family.
    let no_versioning_headers = |op: &Value, path: &str, method: &str, status: &str| {
        for banned in ["ETag", "Last-Modified", "Location"] {
            assert!(
                !header_names(op, status).iter().any(|h| h == banned),
                "{method} {path} {status} must NOT declare {banned}: an ITEM_TAG \
                 collection is not change-controlled"
            );
        }
    };

    for (segment, _rm_type) in KINDS {
        // ── get: 200/400/404/406, the dual-form prose, the served row shape ──
        let path = format!("{BASE}/{segment}/{{uid_based_id}}/tags");
        let op = doc["paths"][&path]["get"].clone();
        assert!(op.is_object(), "GET {path} must be documented");
        require(&op, &path, "GET", &["200", "400", "404", "406"]);
        no_versioning_headers(&op, &path, "GET", "200");
        let uid = param_doc(&op, "uid_based_id");
        assert!(
            uid.contains("VERSIONED_PARTY") && uid.contains("version_uid"),
            "GET {path} must carry the released dual-form uid_based_id prose; has: {uid}"
        );
        assert!(
            uid.contains("DISJOINT"),
            "GET {path} must state that the version and container collections are disjoint"
        );
        assert!(
            documented_params(&op, "header")
                .iter()
                .any(|n| n == "Accept"),
            "GET {path} must document the canonical Accept negotiation"
        );
        let rows = response_example(&op["responses"]["200"]);
        assert!(
            rows.is_array(),
            "GET {path} 200 must carry a worked ITEM_TAG list example"
        );
        let first = rows[0].clone();
        assert_eq!(
            first["_type"], "ITEM_TAG",
            "GET {path} 200 example must be an ITEM_TAG list"
        );
        assert_eq!(
            first["target"]["_type"], "HIER_OBJECT_ID",
            "GET {path} 200 example's container target is the bare RM \
             UID_BASED_ID (item_tag.adoc), never an OBJECT_REF envelope"
        );
        assert_eq!(
            first["owner_id"]["type"], "SYSTEM",
            "GET {path} 200 example's owner_id follows the released \
             local/SYSTEM shape"
        );
        let retrieved = op["responses"]["200"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            retrieved.contains("empty list"),
            "GET {path} 200 must carry the released empty-collection sentence; has: {retrieved}"
        );

        // ── update: both Prefer branches with Preference-Applied, 415 + 422 ──
        let op = doc["paths"][&path]["put"].clone();
        assert!(op.is_object(), "PUT {path} must be documented");
        require(
            &op,
            &path,
            "PUT",
            &["200", "204", "400", "404", "406", "415", "422"],
        );
        assert!(
            documented_params(&op, "header")
                .iter()
                .any(|n| n == "Prefer"),
            "PUT {path} must document the Prefer negotiation"
        );
        for status in ["200", "204"] {
            assert!(
                header_names(&op, status)
                    .iter()
                    .any(|h| h == "Preference-Applied"),
                "PUT {path} {status} must echo the applied preference"
            );
            no_versioning_headers(&op, &path, "PUT", status);
        }
        assert!(
            !documented_params(&op, "header")
                .iter()
                .any(|n| n.eq_ignore_ascii_case("if-match")),
            "PUT {path} must NOT take If-Match: tags are not change-controlled"
        );
        let body = first_content(&op["requestBody"]).clone();
        assert!(
            body["example"][0]["key"].is_string(),
            "PUT {path} request body must be a bare UPDATE_ITEM_TAG array example"
        );
        let body_doc = op["requestBody"]["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(
            body_doc.contains("remove all ITEM_TAG"),
            "PUT {path} must carry the released empty-list clear-all sentence; has: {body_doc}"
        );
        assert!(
            body_doc.contains("REPLACE"),
            "PUT {path} must state the full-collection replace semantics"
        );

        // ── delete: 204/400/404, no headers, the plural set semantics ────────
        let path = format!("{BASE}/{segment}/{{uid_based_id}}/tags/{{key}}");
        let op = doc["paths"][&path]["delete"].clone();
        assert!(op.is_object(), "DELETE {path} must be documented");
        require(&op, &path, "DELETE", &["204", "400", "404"]);
        assert!(
            header_names(&op, "204").is_empty(),
            "DELETE {path} 204 carries no header at all; has {:?}",
            header_names(&op, "204")
        );
        assert!(
            text(&op).contains("resource(s)"),
            "DELETE {path} must carry the released plural 'resource(s)' semantics"
        );
        let key = param_doc(&op, "key");
        assert!(
            key.contains("target_path"),
            "DELETE {path} must explain that `key` alone selects every target_path"
        );
        let not_found = op["responses"]["404"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            not_found.contains("ITEM_TAG identified by the `key` does not exist"),
            "DELETE {path} 404 must carry the released two-trigger text; has: {not_found}"
        );
    }

    // ── the space-wide list: no scoping parameter, no 404 ────────────────────
    let path = format!("{BASE}/tags");
    let op = doc["paths"][&path]["get"].clone();
    assert!(op.is_object(), "GET {path} must be documented");
    require(&op, &path, "GET", &["200", "400", "406"]);
    assert!(
        !codes(&op).iter().any(|c| c == "404"),
        "GET {path} has no scoping parameter, so nothing can be not-found"
    );
    assert!(
        documented_params(&op, "path").is_empty(),
        "GET {path} is the one tag route with no scoping path parameter"
    );
    for filter in ["tag_key", "tag_value", "tag_target_path"] {
        assert!(
            documented_params(&op, "query").iter().any(|n| n == filter),
            "GET {path} must document the {filter} filter"
        );
        assert!(
            param_doc(&op, filter).contains("carries NO description"),
            "GET {path} must record that the released {filter} file is description-free"
        );
    }
    no_versioning_headers(&op, &path, "GET", "200");
    assert_eq!(
        response_example(&op["responses"]["200"])[0]["_type"],
        "ITEM_TAG",
        "GET {path} 200 must carry a served ITEM_TAG list example"
    );

    // ── the family is complete: sixteen released ITEM_TAG operations ─────────
    let tagged = operations(&doc)
        .into_iter()
        .filter(|(path, _, _)| path.starts_with(BASE) && path.contains("/tags"))
        .count();
    assert_eq!(
        tagged, 16,
        "the Demographic API serves sixteen released ITEM_TAG operations \
         (five typed quintets + the space-wide list)"
    );
}

/// The Admin API group's five served operations are documented to full step-6
/// completeness: every branch this server actually emits, the `Allow` header on
/// every config-gate `405`, worked examples, and NO `Location` anywhere.
///
/// The branches are the RELEASED ITS-REST docs text (the OAS is the
/// subordinate source). The two
/// released operations (`operations/admin_ehr_delete.yaml`,
/// `operations/admin_ehr_delete_all.yaml`) `$ref` only description-carrying
/// response files (`responses/{202,204_deleted_hard,404,404_unknown_ehr_id,405}.yaml`
/// declare no `headers:` at all), so the only response header this group may
/// declare is the `Allow` RFC 9110 §15.5.6 makes mandatory on a `405`.
/// `Requests_and_responses.md` §"Deprecated headers" deprecates `Location` on
/// `GET` and `DELETE` responses — which is every route here — and §"HTTP
/// Methods" is the ground for the `405` on every route EXCEPT
/// `admin_ehr_delete_all`, whose own NOTE covers it.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one regression test per audited group keeps the pinned paths together"
)]
async fn admin_operations_are_fully_documented() {
    const BASE: &str = "/ferroehr/rest/openehr/v1/admin";

    let doc = served_document().await;

    let codes = |op: &Value| -> Vec<String> {
        op["responses"]
            .as_object()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    };
    let header_names = |op: &Value, status: &str| -> Vec<String> {
        op["responses"][status]["headers"]
            .as_object()
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    };
    let require = |op: &Value, path: &str, method: &str, expected: &[&str]| {
        let present = codes(op);
        for want in expected {
            assert!(
                present.iter().any(|c| c == want),
                "{method} {path} must document {want}; has {present:?}"
            );
        }
    };
    // Whitespace-normalized (doc comments hard-wrap, so a released sentence
    // may span a line break in the served description).
    let text = |op: &Value| -> String {
        format!(
            "{} {}",
            op.get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            op.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    };
    let param_doc = |op: &Value, name: &str| -> String {
        op.get("parameters")
            .and_then(Value::as_array)
            .and_then(|params| {
                params
                    .iter()
                    .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
            })
            .and_then(|p| p.get("description").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned()
    };

    // ── every route: the RBAC pair, the gated 405 with its Allow, no Location ─
    let all: [(String, &str); 5] = [
        (format!("{BASE}/ehr/all"), "delete"),
        (format!("{BASE}/ehr/{{ehr_id}}"), "delete"),
        (format!("{BASE}/template/{{template_id}}"), "delete"),
        (
            format!("{BASE}/query/{{qualified_query_name}}/{{version}}"),
            "delete",
        ),
        (format!("{BASE}/config"), "get"),
    ];
    for (path, method) in &all {
        let op = doc["paths"][path][method].clone();
        assert!(op.is_object(), "{method} {path} must be documented");
        require(&op, path, method, &["401", "403", "405"]);
        // The one response header this group emits: `Allow` on the config-gate
        // 405 (RFC 9110 §15.5.6 makes it mandatory; the value is empty per
        // §10.2.1 — a resource "temporarily disabled by configuration").
        assert!(
            header_names(&op, "405").iter().any(|h| h == "Allow"),
            "{method} {path} 405 must document the mandatory Allow header; has {:?}",
            header_names(&op, "405")
        );
        // No response of this group may declare Location: §"Deprecated headers"
        // deprecates it on both GET and DELETE responses.
        for status in codes(&op) {
            assert!(
                !header_names(&op, &status).iter().any(|h| h == "Location"),
                "{method} {path} {status} must NOT declare Location \
                 (§\"Deprecated headers\": deprecated on GET and DELETE)"
            );
        }
    }

    // ── the two RELEASED operations ──────────────────────────────────────────
    // The single-EHR delete: its own 404 file, the UUID path parameter, the
    // GDPR cascade sentence, and the §"HTTP Methods" ground for its 405.
    let path = format!("{BASE}/ehr/{{ehr_id}}");
    let op = doc["paths"][&path]["delete"].clone();
    require(&op, &path, "DELETE", &["204", "400", "404"]);
    assert!(
        header_names(&op, "204").is_empty(),
        "DELETE {path} 204 carries no header at all (no released response file \
         declares one, and the EHR is gone); has {:?}",
        header_names(&op, "204")
    );
    let body = text(&op);
    assert!(
        body.contains("GDPR in the European Union"),
        "DELETE {path} must carry the released GDPR cascade sentence; has: {body}"
    );
    assert!(
        body.contains("202 Accepted") && body.contains("SYNCHRONOUS"),
        "DELETE {path} must document the released async 202 branch AND say this \
         server is synchronous; has: {body}"
    );
    assert!(
        !codes(&op).iter().any(|c| c == "202"),
        "DELETE {path} must NOT declare a 202 it never emits"
    );
    let not_found = op["responses"]["404"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        not_found.contains("`404 Not Found` is returned when an EHR with `ehr_id` does not exist."),
        "DELETE {path} 404 must carry the verbatim 404_unknown_ehr_id.yaml trigger; has: {not_found}"
    );
    let gate = op["responses"]["405"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        gate.contains("HTTP Methods"),
        "DELETE {path} 405 must cite overview §\"HTTP Methods\" — the bulk \
         route's NOTE does not govern it; has: {gate}"
    );
    let ehr_id = param_doc(&op, "ehr_id");
    assert!(
        ehr_id.contains("EHR.ehr_id.value") && ehr_id.contains("UUID"),
        "DELETE {path} must carry the released ehr_id prose and its uuid format; has: {ehr_id}"
    );

    // The bulk delete: the generic-404 binding, both query forms, the
    // dev/testing NOTE that is its own 405 provenance.
    let path = format!("{BASE}/ehr/all");
    let op = doc["paths"][&path]["delete"].clone();
    require(&op, &path, "DELETE", &["204", "400"]);
    assert!(
        header_names(&op, "204").is_empty(),
        "DELETE {path} 204 carries no header at all; has {:?}",
        header_names(&op, "204")
    );
    let body = text(&op);
    assert!(
        body.contains("development") && body.contains("testing"),
        "DELETE {path} must carry the released dev/testing NOTE; has: {body}"
    );
    assert!(
        body.contains("GDPR in the European Union"),
        "DELETE {path} must carry the released GDPR cascade sentence; has: {body}"
    );
    assert!(
        body.contains("responses/404.yaml") && body.contains("UNREACHABLE"),
        "DELETE {path} must record that it binds the GENERIC 404 and why that \
         branch is unreachable here (delete-what-exists); has: {body}"
    );
    assert!(
        !codes(&op).iter().any(|c| c == "404"),
        "DELETE {path} must NOT declare a 404 it never emits"
    );
    assert!(
        body.contains("202 Accepted") && body.contains("SYNCHRONOUS"),
        "DELETE {path} must document the released async 202 branch AND say this \
         server is synchronous; has: {body}"
    );
    assert!(
        !codes(&op).iter().any(|c| c == "202"),
        "DELETE {path} must NOT declare a 202 it never emits"
    );
    let gate = op["responses"]["405"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        gate.contains("responses/405.yaml"),
        "DELETE {path} 405 must cite its OWN released provenance (the NOTE + \
         responses/405.yaml); has: {gate}"
    );
    let ehr_id = param_doc(&op, "ehr_id");
    assert!(
        ehr_id.contains("?ehr_id=a&ehr_id=b") && ehr_id.contains("?ehr_id=a,b"),
        "DELETE {path} must document BOTH accepted query forms; has: {ehr_id}"
    );
    assert!(
        ehr_id.contains("OPTIONAL") && ehr_id.contains("ALL EHRs"),
        "DELETE {path} must state that an absent/empty list deletes ALL EHRs; has: {ehr_id}"
    );
    assert!(
        documented_params(&op, "path").is_empty(),
        "DELETE {path} takes no path parameter — the RFC 6570 `{{?ehr_id*}}` \
         suffix is normalized away, it is not a path segment"
    );

    // ── the template-delete extension: the in-use 409 guard ──────────────────
    let path = format!("{BASE}/template/{{template_id}}");
    let op = doc["paths"][&path]["delete"].clone();
    require(&op, &path, "DELETE", &["204", "404", "409"]);
    let conflict = op["responses"]["409"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        conflict.contains("orphan"),
        "DELETE {path} 409 must state the never-orphan-committed-data guard; has: {conflict}"
    );
    assert!(
        text(&op).contains("delete_opt"),
        "DELETE {path} must name the SM I_DEFINITION_ADL14.delete_opt relation"
    );

    // ── the stored-query-version delete: NOT SM delete_query ─────────────────
    let path = format!("{BASE}/query/{{qualified_query_name}}/{{version}}");
    let op = doc["paths"][&path]["delete"].clone();
    require(&op, &path, "DELETE", &["204", "404"]);
    let body = text(&op);
    assert!(
        body.contains("delete_query") && body.contains("NOT"),
        "DELETE {path} must state that it does NOT realize SM delete_query; has: {body}"
    );
    assert!(
        body.contains("(name, version)"),
        "DELETE {path} must state that it deletes exactly one (name, version) \
         row while SM delete_query deletes by name alone; has: {body}"
    );

    // ── the config read: 200 with a redacted tree, no versioning headers ─────
    let path = format!("{BASE}/config");
    let op = doc["paths"][&path]["get"].clone();
    require(&op, &path, "GET", &["200"]);
    for banned in ["ETag", "Last-Modified"] {
        assert!(
            !header_names(&op, "200").iter().any(|h| h == banned),
            "GET {path} 200 must NOT declare {banned}: the config tree is not a \
             versioned resource"
        );
    }
    assert!(
        op["responses"]["200"]["content"]
            .as_object()
            .and_then(|c| c.values().next())
            .map(|c| c["example"].clone())
            .is_some_and(|e| e["admin"]["enabled"].is_boolean()),
        "GET {path} 200 must carry a worked redacted-config example"
    );
}

/// The three admin extension routes are OUR OWN EXTENSION: the released Admin
/// API defines exactly two operations, both EHR deletes, so every other admin
/// route must say so in its description and none may be counted towards a
/// conformance-profile claim.
#[tokio::test]
async fn admin_extension_operations_are_flagged_as_an_extension() {
    const BASE: &str = "/ferroehr/rest/openehr/v1/admin";

    let doc = served_document().await;

    // The three routes of the ADMIN group that no released operation governs
    // (the other `/admin/*` surface — event subscriptions — belongs to its own
    // extension group and carries its own flag).
    let extensions: [(String, &str); 3] = [
        (format!("{BASE}/template/{{template_id}}"), "delete"),
        (
            format!("{BASE}/query/{{qualified_query_name}}/{{version}}"),
            "delete",
        ),
        (format!("{BASE}/config"), "get"),
    ];
    for (path, method) in &extensions {
        let op = doc["paths"][path][method].clone();
        assert!(op.is_object(), "{method} {path} must be documented");
        let text = format!(
            "{} {}",
            op.get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            op.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        assert!(
            text.contains("no ITS-REST operation governs this"),
            "{} {path} must carry the our-own-extension flag; has: {text}",
            method.to_uppercase()
        );
    }

    // …and the released half of the group is exactly two operations, both EHR
    // deletes (`specifications/admin.openapi.yaml`), so nothing else may claim
    // released status.
    let released = [format!("{BASE}/ehr/all"), format!("{BASE}/ehr/{{ehr_id}}")];
    for path in &released {
        let op = doc["paths"][path]["delete"].clone();
        assert!(op.is_object(), "DELETE {path} must be documented");
        let text = format!(
            "{} {}",
            op.get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            op.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        assert!(
            !text.contains("no ITS-REST operation governs this"),
            "DELETE {path} is a RELEASED operation and must not be flagged as an extension"
        );
    }
}

/// One family of served operations that **no released openEHR specification
/// governs**, and the honest-boundary flag every one of its operations must
/// carry in the SERVED document.
struct NonSpecFamily {
    /// What the family is (assertion messages only).
    label: &'static str,
    /// Served path prefixes; an operation under any of them is in the family.
    prefixes: &'static [&'static str],
    /// The flag phrase (lower-cased) the served summary+description must carry.
    flag: &'static str,
    /// How many operations the family serves.
    operations: usize,
}

/// Every non-spec path prefix the server serves, with its flag and its
/// operation count.
///
/// The load-bearing fact: nothing in the released ITS-REST text authorises
/// serving resources outside its own resource set, so **every** operation
/// outside the standardised groups is our own extension and MUST say so where
/// a reader of the served document can see it. A module-level `//!` comment
/// does not qualify — utoipa never serves those.
const NON_SPEC_FAMILIES: &[NonSpecFamily] = &[
    NonSpecFamily {
        label: "the ops-introspection management surface",
        prefixes: &["/management"],
        flag: "no openehr spec governs this",
        // 8 + /management/flamegraph (PR #1864, the on-demand CPU profiler)
        // + /management/health and /management/status (#3671, the detail the
        // public probes no longer carry).
        operations: 11,
    },
    NonSpecFamily {
        label: "the terminology extension wire",
        prefixes: &["/ferroehr/rest/openehr/v1/terminology"],
        flag: "no openehr spec governs this",
        operations: 6,
    },
    NonSpecFamily {
        label: "the event-subscription extension",
        prefixes: &["/ferroehr/rest/openehr/v1/admin/event_subscription"],
        flag: "no openehr spec governs this",
        operations: 5,
    },
    NonSpecFamily {
        // IHE ITI-81 is its own (non-openEHR) basis; the flag still has to say
        // that no openEHR spec governs the endpoint.
        label: "the ITI-81 ATNA audit retrieval",
        prefixes: &["/ferroehr/rest/openehr/v1/fhir/r4/AuditEvent"],
        flag: "no openehr spec governs this",
        operations: 1,
    },
    NonSpecFamily {
        label: "the PARTY_RELATIONSHIP demographic extension",
        prefixes: &[
            "/ferroehr/rest/openehr/v1/demographic/party_relationship",
            "/ferroehr/rest/openehr/v1/demographic/versioned_party_relationship",
        ],
        flag: "no its-rest operation governs this",
        operations: 8,
    },
    NonSpecFamily {
        // ITS-REST publishes an ad-hoc and a stored query and nothing else;
        // AQL has no demographic source at all.
        label: "the cohort-query extension",
        prefixes: &["/ferroehr/rest/openehr/v1/query/cohort"],
        flag: "no openehr spec governs this",
        operations: 1,
    },
    NonSpecFamily {
        label: "the ADMIN group's own-design routes",
        prefixes: &[
            "/ferroehr/rest/openehr/v1/admin/config",
            "/ferroehr/rest/openehr/v1/admin/template/",
            "/ferroehr/rest/openehr/v1/admin/query/",
        ],
        flag: "no its-rest operation governs this",
        operations: 3,
    },
    NonSpecFamily {
        // SM I_ADMIN_SERVICE's four statistics calls; the released Admin API
        // is the two EHR deletes alone.
        label: "the ADMIN activity-report extension",
        prefixes: &["/ferroehr/rest/openehr/v1/admin/report/"],
        flag: "no its-rest operation governs this",
        operations: 4,
    },
    NonSpecFamily {
        // SM I_ADMIN_ARCHIVE's two calls plus the two restore routes, which the
        // SM declares no operation for at all; nothing released moves a resource
        // to archival storage in either direction.
        label: "the ADMIN archive extension",
        prefixes: &["/ferroehr/rest/openehr/v1/admin/archive/"],
        flag: "no its-rest operation governs this",
        operations: 4,
    },
    NonSpecFamily {
        // The SM archetype/artefact operations of I_DEFINITION_ADL14 +
        // I_DEFINITION_ADL2; the released Definition API provisions
        // operational templates only.
        label: "the ADL 1.4 / ADL 2 archetype + artefact extension",
        prefixes: &[
            "/ferroehr/rest/openehr/v1/definition/archetype/",
            "/ferroehr/rest/openehr/v1/definition/artefact/",
        ],
        flag: "no its-rest operation governs this",
        operations: 9,
    },
    NonSpecFamily {
        // SM I_ADMIN_DUMP_LOAD's export/load pair; nothing released reads or
        // writes a file-system archive.
        label: "the ADMIN dump/load extension",
        prefixes: &[
            "/ferroehr/rest/openehr/v1/admin/dump",
            "/ferroehr/rest/openehr/v1/admin/load",
        ],
        flag: "no its-rest operation governs this",
        operations: 2,
    },
    NonSpecFamily {
        // The whole SM MESSAGE component (I_EHR_EXTRACT_SERVICE +
        // I_TDD_SERVICE); the release publishes no message API at all.
        label: "the MESSAGE extension group",
        prefixes: &["/ferroehr/rest/openehr/v1/message/"],
        flag: "no its-rest operation governs this",
        operations: 6,
    },
    NonSpecFamily {
        label: "the operational status document",
        prefixes: &["/ferroehr/rest/status"],
        flag: "no openehr spec governs an operational status endpoint",
        operations: 1,
    },
    NonSpecFamily {
        label: "the always-on public health family",
        prefixes: &["/health"],
        flag: "no openehr spec governs a health endpoint",
        operations: 3,
    },
    NonSpecFamily {
        label: "the OAS meta-endpoints",
        prefixes: &["/ferroehr/rest/api-docs", "/ferroehr/rest/swagger-ui"],
        flag: "no openehr spec governs",
        operations: 3,
    },
];

/// The honesty battery: EVERY operation the server serves outside the
/// standardised ITS-REST resource set carries its our-own-extension flag in the
/// served document, and each family serves exactly the operation count recorded
/// above (so a new unflagged endpoint cannot slip in, and a family cannot
/// silently shrink).
#[tokio::test]
async fn every_extension_operation_is_flagged_in_the_served_document() {
    let doc = served_document().await;
    // Whitespace-normalized + lower-cased: doc comments hard-wrap, so a flag
    // sentence may span a line break in the served description, and the
    // sentence-initial capital varies with where the flag sits.
    let text = |op: &Value| -> String {
        format!(
            "{} {}",
            op.get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            op.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
    };

    for family in NON_SPEC_FAMILIES {
        let mut seen = 0;
        for (path, method, op) in operations(&doc) {
            if !family.prefixes.iter().any(|p| path.starts_with(p)) {
                continue;
            }
            seen += 1;
            let body = text(&op);
            assert!(
                body.contains(family.flag),
                "{} {path} ({}) must carry the our-own-extension flag \
                 \"{}\" in its served summary/description; has: {body}",
                method.to_uppercase(),
                family.label,
                family.flag
            );
        }
        assert_eq!(
            seen, family.operations,
            "{} serves {} documented operation(s), found {seen} — update the \
             battery deliberately, never to make it pass",
            family.label, family.operations
        );
    }
}

/// Document-level ratchet: the served document declares the things a consumer
/// needs to use it — the OAS version, a `servers` entry, every tag its
/// operations use, the product version, and the openEHR ITS-REST contract
/// identity as a machine-readable `x-` extension (distinct from `info.version`,
/// which is the product `SemVer`).
#[tokio::test]
async fn the_document_declares_its_own_identity() {
    let doc = served_document().await;

    assert_eq!(
        doc["openapi"].as_str(),
        Some("3.1.0"),
        "utoipa emits OpenAPI 3.1.0; pin it so a version change is deliberate"
    );

    let version = doc["info"]["version"].as_str().unwrap_or_default();
    assert!(
        !version.is_empty(),
        "info.version must state the product version"
    );
    assert_eq!(
        version,
        env!("CARGO_PKG_VERSION"),
        "info.version is the PRODUCT SemVer, not an openEHR contract version"
    );

    let its_rest = doc["x-openehr-its-rest"].as_str().unwrap_or_default();
    assert!(
        !its_rest.is_empty(),
        "the document must publish the implemented ITS-REST contract version as \
         the x-openehr-its-rest extension"
    );

    let servers = doc["servers"].as_array().expect("a servers block");
    assert!(!servers.is_empty(), "the servers block must have an entry");
    // The paths are absolute from the server root and already carry the
    // configured base path, so the server URL must not repeat it.
    for server in servers {
        let url = server["url"].as_str().unwrap_or_default();
        assert!(
            !url.contains("/openehr/v1"),
            "server url {url} repeats the base path the paths already carry"
        );
    }

    let external = doc["externalDocs"]["url"].as_str().unwrap_or_default();
    assert!(
        external.contains("specifications.openehr.org")
            && external.contains("ITS-REST/Release-1.1.0"),
        "externalDocs must point at the implemented released ITS-REST spec; has: {external}"
    );

    let declared: std::collections::BTreeSet<String> = doc["tags"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .filter_map(|t| t["name"].as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut undeclared = Vec::new();
    for (path, method, op) in operations(&doc) {
        for tag in op["tags"].as_array().into_iter().flatten() {
            let Some(tag) = tag.as_str() else { continue };
            if !declared.contains(tag) {
                undeclared.push(format!("{} {path}: {tag}", method.to_uppercase()));
            }
        }
    }
    assert!(
        undeclared.is_empty(),
        "every tag an operation uses must be declared (with a description) at \
         document level:\n{}",
        undeclared.join("\n")
    );
}

/// `expand_multimedia` is declared on exactly the reads whose handlers honour
/// it — no more, no less.
///
/// The defect this pins was live: externalization is applied by the generic
/// versioning path, so a `DV_MULTIMEDIA` leaves the database from a
/// COMPOSITION, an `EHR_STATUS` or a FOLDER alike, while re-inlining was wired to
/// the COMPOSITION read alone. Content committed inside an `EHR_STATUS` therefore
/// sat in the object store with no API that returned it, and the parameter that
/// should have fetched it was silently ignored as undeclared.
///
/// A declaration list is the only half a document can check. It catches the
/// direction that actually bit — a handler honouring a parameter nobody
/// documented, or a document promising one no handler reads.
#[tokio::test]
async fn expand_multimedia_is_declared_on_every_read_that_can_serve_externalized_media() {
    // Every read that can return externalized content: the bare resources and
    // the VERSION envelopes that wrap them.
    let expected: Vec<&str> = vec![
        "/ehr/{ehr_id}/composition/{uid_based_id}",
        "/ehr/{ehr_id}/directory",
        "/ehr/{ehr_id}/directory/{version_uid}",
        "/ehr/{ehr_id}/ehr_status",
        "/ehr/{ehr_id}/ehr_status/{version_uid}",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}",
        "/ehr/{ehr_id}/versioned_ehr_status/version",
        "/ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}",
    ];

    let doc = served_document().await;
    let mut declared: Vec<String> = operations(&doc)
        .into_iter()
        .filter(|(_, method, op)| {
            method == "get"
                && documented_params(op, "query")
                    .iter()
                    .any(|p| p == "expand_multimedia")
        })
        .map(|(path, _, _)| path)
        .collect();
    declared.sort();
    declared.dedup();

    // The served document carries the deployment's base path on every route.
    let expected: Vec<String> = {
        let mut e: Vec<String> = expected
            .into_iter()
            .map(|p| format!("/ferroehr/rest/openehr/v1{p}"))
            .collect();
        e.sort();
        e
    };
    assert_eq!(
        declared, expected,
        "the reads declaring `expand_multimedia` drifted from the reads that honour it"
    );
}
