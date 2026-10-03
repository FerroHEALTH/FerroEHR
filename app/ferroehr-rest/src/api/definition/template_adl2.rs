// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ITS-REST **ADL2 template** resource (`tags: ADL2`).
//!
//! Operations (`docs/specs/openehr/ITS-REST/specifications/operations/`):
//! `definition_template_adl2_list` / `_upload` / `_get` / `_example_get` /
//! `_version_get`. Governing spec text:
//! `docs/specs/openehr/ITS-REST/specifications/docs/definition/`.
//!
//! ADL2 artefacts are validated + compiled by the `openehr-adl` engine through
//! the service layer (`FerroEhrService::adl2_*`). The `_get`/`_version_get`
//! operations serve two representations of the stored operational template:
//! `text/plain` — the stored ADL2 **source** verbatim
//! (`200_Template_adl2_retrieved.yaml` body `oneOf: [OperationalTemplateV2,
//! string]`, whose example is ADL2 source) — and `application/json` — the
//! `OperationalTemplateV2` canonical-JSON projection (the OAS declares that
//! schema as an opaque `type: object`, `schemas/aom/OperationalTemplateV2.yaml`,
//! so the AOM2 canonical JSON of the OPT satisfies it). `application/xml` is
//! enumerated by the `Accept` header but the response declares **no**
//! `application/xml` body content, so an `Accept` naming only XML is a `406`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};

use openehr_its::rest::generated::definition::{
    DefinitionTemplateAdl2ExampleGetParams, DefinitionTemplateAdl2GetParams,
    DefinitionTemplateAdl2ListParams, DefinitionTemplateAdl2UploadParams,
    DefinitionTemplateAdl2VersionGetParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::Composition;

use crate::api::RequestParts;
use crate::negotiate;
use crate::negotiate::{AppliedPreference, WireFormat};
use crate::overview::error::RestError;
use crate::state::AppState;

use super::dispatch::list_filter_and_page;

/// The four `Accept_LOCATABLE` representations of a generated example
/// COMPOSITION: canonical JSON/XML + FLAT/STRUCTURED (`200_Template_example_
/// retrieved.yaml` + `Accept_LOCATABLE.yaml`).
const EXAMPLE_FORMATS: &[WireFormat] = &[
    WireFormat::CanonicalJson,
    WireFormat::CanonicalXml,
    WireFormat::Flat,
    WireFormat::Structured,
];

/// The representation a `GET …/adl2/{template_id}[/{version}]` request resolves
/// to, per its `Accept` header (`parameters/header/Accept_Template_adl2.yaml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adl2Repr {
    /// `text/plain` — the stored ADL2 source verbatim (also the default for
    /// absent / `*/*` / `text/*`).
    Source,
    /// `application/json` — the `OperationalTemplateV2` canonical-JSON
    /// projection.
    Json,
}

/// `GET …/definition/template/adl2` — list the stored ADL2 templates, with the
/// `template_id`/`concept`/`version`/`offset`/`fetch` filter + pagination the
/// wire decodes (`operations/definition_template_adl2_list.yaml`) threaded to
/// the adapter.
pub(super) async fn list(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p =
        DefinitionTemplateAdl2ListParams::from_request(parts.route()?, parts.query.as_deref(), h)?;
    let (filter, page) =
        list_filter_and_page(p.template_id, p.concept, p.version, p.offset, p.fetch);
    Ok(negotiate::respond(
        h,
        StatusCode::OK,
        &state.backend().template_adl2_list(filter, page).await?,
    ))
}

/// `POST …/definition/template/adl2` — ingest an ADL2 operational-template
/// `text/plain` source (`operations/definition_template_adl2_upload.yaml`),
/// validated by the `openehr-adl` engine.
///
/// NOTE: the `at_version` (`version`) query parameter is `deprecated: true`
/// (`parameters/query/at_version.yaml`); dropping it is spec-permitted, so only
/// `Prefer` is read. Recorded as residue, not a defect.
pub(super) async fn upload(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    // Built for its parameter validation only: `Prefer` is read off the header
    // map through the shared negotiation predicates, like every write route.
    DefinitionTemplateAdl2UploadParams::from_request(parts.route()?, parts.query.as_deref(), h)?;
    // ADL2 arrives as `text/plain` source, the operation's single declared body
    // type, so a payload declaring another media type is refused `415` before
    // parsing (`Resources.md` §format rules). An absent `Content-Type` declares
    // nothing to refuse.
    negotiate::require_text_plain(h)?;
    let source = negotiate::text_body(&parts.body)?;
    // An unparseable source is a `400`, an AOM2-invalid one a `422` carrying the
    // rule codes, and a duplicate HRID a `409` — all through `ServiceError`, so
    // the `422` renders the `Error` object with per-code `validationErrors`.
    let hrid = state.backend().template_adl2_upload(source.clone()).await?;
    let hrid = hrid.as_str();
    let location = format!(
        "{}/definition/template/adl2/{hrid}",
        state.config().server.base_path
    );
    // 201_Template_adl2_upload: the body follows `Prefer` — the OPT source on
    // representation, `{template_id}` on identifier, empty on minimal — with
    // `Location` and the weak `ETag` in every case.
    let mut resp = upload_response(h, &location, hrid, source);
    set_template_etag(&mut resp, hrid);
    Ok(resp)
}

/// `GET …/definition/template/adl2/{template_id}` — the stored operational
/// template.
///
/// Resolves `template_id` (full HRID, or a partial that selects the latest
/// matching version) and serves the representation `Accept` negotiates:
/// `text/plain` source, `application/json` `OperationalTemplateV2`, or `406`
/// when only `application/xml` (which the response declares no body for) is
/// acceptable. Unknown `template_id` → `404`.
pub(super) async fn get(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p =
        DefinitionTemplateAdl2GetParams::from_request(parts.route()?, parts.query.as_deref(), h)?;
    render(state, h, p.template_id, None).await
}

/// `GET …/definition/template/adl2/{template_id}/{version}` — the stored
/// operational template at an explicit SEMVER `version`
/// (`operations/definition_template_adl2_version_get.yaml`; `deprecated: true`
/// in the OAS but spec-declared, so served). `version` is an exact version or a
/// `{major}`/`{major}.{minor}` prefix resolving to the highest match; a missing
/// template/version → `404`.
pub(super) async fn version_get(
    state: &AppState,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = DefinitionTemplateAdl2VersionGetParams::from_request(
        parts.route()?,
        parts.query.as_deref(),
        h,
    )?;
    render(state, h, p.template_id, Some(p.version)).await
}

/// `GET …/definition/template/adl2/{template_id}/example` — a generated example
/// COMPOSITION from the ADL2 template, negotiated across the four
/// `Accept_LOCATABLE` forms (canonical JSON/XML + FLAT/STRUCTURED), exactly as
/// the ADL 1.4 example endpoint (`200_Template_example_retrieved.yaml` +
/// `Accept_LOCATABLE.yaml`). `type` ∈ {input, output} (default input),
/// `detail_level` ∈ {required, medium, complete} (default required).
pub(super) async fn example_get(
    state: &AppState,
    parts: &RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = DefinitionTemplateAdl2ExampleGetParams::from_request(
        parts.route()?,
        parts.query.as_deref(),
        h,
    )?;
    // An unknown template is a 404, an invalid `type`/`detail_level` a 400, and
    // an uncompilable template a 422.
    let comp = state
        .backend()
        .template_adl2_example(p.template_id.clone(), p.detail_level, p.r#type)
        .await?;
    // The four representations `Accept_LOCATABLE` enumerates; any other media
    // type is a `406`.
    match negotiate::resolve_accept(h, EXAMPLE_FORMATS, WireFormat::CanonicalJson) {
        Some(WireFormat::Flat) => {
            // The ADL2 template's WebTemplate is not in the ADL 1.4 store, so
            // the `v2_4` front end resolves it.
            let wt = state.backend().web_template_adl2(&p.template_id).await?;
            crate::formats::dispatch::composition_flat_response_with(StatusCode::OK, &comp, &wt)
        }
        Some(WireFormat::Structured) => {
            let wt = state.backend().web_template_adl2(&p.template_id).await?;
            crate::formats::dispatch::composition_structured_response_with(
                StatusCode::OK,
                &comp,
                &wt,
            )
        }
        Some(WireFormat::CanonicalJson | WireFormat::CanonicalXml) => {
            Ok(negotiate::respond_rm::<Composition>(
                h,
                StatusCode::OK,
                &comp,
                "composition",
            ))
        }
        _ => Err(RestError(ApiError::NotAcceptable(
            "the template example is available as application/json, application/xml, \
             application/openehr.wt.flat+json, or application/openehr.wt.structured+json"
                .to_owned(),
        ))),
    }
}

/// Set the weak `ETag` of a served ADL2 template. The value is the artefact's
/// **resolved** `ARCHETYPE_HRID` (AM `v2_4` — the HRID carries the artefact's
/// SEMVER `release_version`, so it changes whenever the served artefact does),
/// mirroring the ADL 1.4 sibling's `template_id` `ETag`.
///
/// ITS-REST `Requests_and_responses.md` §"`ETag` and Last-Modified": both
/// headers "SHOULD be included in responses for VERSION, `VERSIONED_OBJECT`, or
/// other resources that have versioning or unique state identifiers", and the
/// `ETag` "is considered to be of weak-type and should have a weakness
/// indicator `W/` prefix" — constructed by the shared
/// [`negotiate::set_etag`] helper so upload and GET share one
/// implementation.
fn set_template_etag(resp: &mut Response, hrid: &str) {
    negotiate::set_etag(resp, hrid);
}

/// Resolve + render one ADL2 template in the `Accept`-negotiated representation.
async fn render(
    state: &AppState,
    headers: &HeaderMap,
    template_id: String,
    version: Option<String>,
) -> Result<Response, RestError> {
    match negotiate_get(headers) {
        Some(Adl2Repr::Source) => {
            let template = state
                .backend()
                .template_adl2_source(template_id, version)
                .await?;
            let mut resp = text_response(StatusCode::OK, None, template.payload);
            set_template_etag(&mut resp, &template.hrid);
            Ok(resp)
        }
        Some(Adl2Repr::Json) => {
            let template = state
                .backend()
                .template_adl2_opt_json(template_id, version)
                .await?;
            let mut resp = json_response(StatusCode::OK, template.payload);
            set_template_etag(&mut resp, &template.hrid);
            Ok(resp)
        }
        None => Err(RestError(ApiError::NotAcceptable(
            "the ADL2 template is served as text/plain source or application/json \
             OperationalTemplateV2; application/xml has no declared response body"
                .to_owned(),
        ))),
    }
}

/// Choose the `GET` representation from `Accept`
/// (`parameters/header/Accept_Template_adl2.yaml` enumerates `text/plain`,
/// `application/json`, `application/xml`). Absent / `*/*` / `text/*` /
/// `text/plain` → [`Adl2Repr::Source`]; `application/json` (or `application/*`)
/// with text not acceptable → [`Adl2Repr::Json`]; only `application/xml` → `None`
/// (→ `406`, the response declares no XML body).
fn negotiate_get(headers: &HeaderMap) -> Option<Adl2Repr> {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) else {
        return Some(Adl2Repr::Source); // absent → text/plain source
    };
    if accept.trim().is_empty() {
        return Some(Adl2Repr::Source);
    }
    let mut source_ok = false;
    let mut json_ok = false;
    for range in accept.split(',') {
        // A `;q=0` explicitly rejects the range (RFC 9110 §12.5.1).
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or(range).trim();
        if parts.any(|p| p.trim().eq_ignore_ascii_case("q=0")) {
            continue;
        }
        match media {
            "" | "*/*" | "text/*" | "text/plain" => source_ok = true,
            "application/json" | "application/*" => json_ok = true,
            _ => {}
        }
    }
    // `text/plain` is the canonical ADL2 interchange form, so it wins when both
    // are acceptable.
    if source_ok {
        Some(Adl2Repr::Source)
    } else if json_ok {
        Some(Adl2Repr::Json)
    } else {
        None
    }
}

/// A `text/plain` response for ADL2 source (the artefact interchange form), with
/// an optional `Location` header.
fn text_response(status: StatusCode, location: Option<&str>, source: String) -> Response {
    // axum's `String` responder already sets `Content-Type: text/plain; charset=utf-8`.
    let mut resp = (status, source).into_response();
    if let Some(loc) = location
        && let Ok(value) = HeaderValue::from_str(loc)
    {
        resp.headers_mut().insert(header::LOCATION, value);
    }
    resp
}

/// An `application/json` response carrying a pre-serialized
/// `OperationalTemplateV2` canonical-JSON body.
fn json_response(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

/// Render the `201 Created` for an ADL2 template upload per `Prefer`
/// (`201_Template_adl2_upload`): `return=representation` → the OPT source
/// (text/plain); `return=identifier` → a `{template_id}` JSON body
/// (`schemas/others/TemplateIdentifier.yaml`); missing/`return=minimal` → an
/// empty body. `Location` is set on every case, and the applied preference is
/// declared through the shared [`negotiate::set_preference_applied`] seam
/// (`Requests_and_responses.md` §Representation details negotiation).
///
/// A template is not `uid`-versioned, so the identifier body is the
/// `template_id` object rather than the generic `{uid}` of
/// [`negotiate::write_negotiated`]; every upload outcome is `201 Created`, so
/// the identifier variant's never-`204` rule (§"Prefer only identifier") holds
/// trivially.
fn upload_response(headers: &HeaderMap, location: &str, hrid: &str, source: String) -> Response {
    let applied = if negotiate::prefers_representation(headers) {
        AppliedPreference::Representation
    } else if negotiate::prefers_identifier(headers) {
        AppliedPreference::Identifier(hrid)
    } else {
        AppliedPreference::Minimal
    };
    let mut resp = match applied {
        AppliedPreference::Representation => {
            text_response(StatusCode::CREATED, Some(location), source)
        }
        AppliedPreference::Identifier(id) => {
            let body = serde_json::json!({ "template_id": id });
            let mut resp = (StatusCode::CREATED, axum::Json(body)).into_response();
            if let Ok(value) = HeaderValue::from_str(location) {
                resp.headers_mut().insert(header::LOCATION, value);
            }
            resp
        }
        AppliedPreference::Minimal => negotiate::empty_with_location(StatusCode::CREATED, location),
    };
    negotiate::set_preference_applied(&mut resp, applied);
    resp
}
