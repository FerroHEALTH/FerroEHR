// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Simplified-Formats payload adapter (ITS-REST
//! `docs/specs/openehr/ITS-REST/docs/simplified_formats/`, STABLE).
//!
//! The wire seam between the negotiation core
//! ([`crate::overview::negotiate`], which classifies the media type) and the
//! `openehr_sdt::flat` conversion engine, in both directions for the FLAT and
//! STRUCTURED representations of a versioned object.
//!
//! On a request it parses the simplified body per its media type, resolves the
//! template id from the `openehr-template-id` request header — the mechanism for
//! a simplified COMPOSITION commit, since the payload carries no
//! `archetype_details.template_id` (`Requests_and_responses.md`
//! §openehr-template-id) — and builds the canonical-JSON COMPOSITION. On a
//! response it serializes a stored canonical COMPOSITION into the negotiated
//! form, reading its template id from `archetype_details/template_id`.
//! `WebTemplate` resolution is the service's concern, reached through
//! `state.backend().web_template(..)`.
//!
//! CONTRIBUTION keeps the envelope canonical (`contribution_create.yaml`
//! §Simplified Formats); only each `versions[i].data` COMPOSITION is simplified.
//! Non-templated resources have no Simplified-Formats mapping and are rejected
//! uniformly (`guard_non_templated`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — one byte-to-JSON \
              step per route, consumed by the typed decode"
)]

use axum::response::Response;
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde_json::{Map, Value};

use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::runtime::ApiError;

use crate::negotiate;
use crate::negotiate::WireFormat;
use crate::overview::error::RestError;
use crate::state::AppState;

/// Reads the `openehr-template-id` request header
/// (`Requests_and_responses.md` §openehr-template-id).
///
/// HTTP header names are case-insensitive, and the deprecated
/// `openEHR-TEMPLATE_ID` spelling is accepted as a fallback. No query parameter
/// is read: the spec defines only the header.
pub(crate) fn header_template_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("openehr-template-id")
        .or_else(|| headers.get("openEHR-TEMPLATE_ID"))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// A UTC ISO 8601 timestamp for the `ctx/time` default (Simplified Formats
/// `master04 §Context`: "defaults to the current server time (`now()`)").
fn now() -> String {
    jiff::Timestamp::now().to_string()
}

/// A server-side fault raised while converting between the canonical and
/// simplified representations. `detail` (a serde diagnostic, a missing stored
/// attribute, an unreachable branch) is server-internal, so it goes to the
/// trace record and the `500` body carries the curated opaque message
/// ([`crate::overview::error::internal_fault`]).
fn internal(detail: impl std::fmt::Display) -> RestError {
    RestError(crate::overview::error::internal_fault(
        "convert between the canonical and Simplified Formats",
        &detail,
    ))
}

/// The `422` for a simplified COMPOSITION commit that supplies no template id.
/// `Requests_and_responses.md §openehr-template-id` makes the header the
/// mechanism; without it the payload cannot be resolved to a template — a
/// well-formed-but-unprocessable request (`Requests_and_responses.md §HTTP
/// status codes`, row `422`).
fn missing_template_id() -> RestError {
    RestError(ApiError::Unprocessable(
        "a Simplified-Format COMPOSITION commit requires the target template id \
         in the `openehr-template-id` request header (Requests_and_responses \
         §openehr-template-id)"
            .to_owned(),
    ))
}

/// A Simplified-Format **input** conversion failure: the body parsed as JSON
/// but does not conform to the target template's simplified-data-template shape
/// — well-formed-but-semantically-invalid client content → `422`
/// (`Requests_and_responses.md §HTTP status codes`, row `422`).
fn flat_input_err(e: &openehr_sdt::flat::error::FlatError) -> RestError {
    // NOTE: TEMPLATE-INDEPENDENT FLAT violations are the 400 row, everything
    // template-/RM-mediated the 422 row ("could be converted to a resource",
    // `responses/{400,422}.yaml`) — no released text splits the two.
    use openehr_sdt::flat::error::FlatError;
    let syntactic = matches!(
        e,
        FlatError::MalformedPath { .. }
            | FlatError::UnknownContext(_)
            | FlatError::OtherSuffixConflict(_)
    );
    let msg = format!("Simplified-Format conversion failed: {e}");
    if syntactic {
        RestError(ApiError::BadRequest(msg))
    } else {
        RestError(ApiError::Unprocessable(msg))
    }
}

/// A Simplified-Format **output** conversion failure: the server failed to
/// render its own stored canonical COMPOSITION into the requested simplified
/// form. Stored data is the server's own and should always convert, so this is
/// a server fault → `500` (`Requests_and_responses.md §HTTP status codes`, row
/// `500`).
fn flat_output_err(e: &openehr_sdt::flat::error::FlatError) -> RestError {
    internal(e)
}

// ── COMPOSITION: request side ──────────────────────────────────────────────

/// Parse a FLAT request body into a canonical-JSON COMPOSITION, driven by the
/// template named in the `openehr-template-id` header.
pub(crate) async fn composition_from_flat(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Value, RestError> {
    let template_id = header_template_id(headers).ok_or_else(missing_template_id)?;
    let flat: Map<String, Value> = serde_json::from_slice(body)
        .map_err(|e| RestError(ApiError::BadRequest(format!("invalid FLAT JSON: {e}"))))?;
    let wt = state
        .backend()
        .web_template(&template_id)
        .await
        .map_err(RestError::from)?;
    openehr_sdt::flat::convert::submitted_composition_from_flat(&flat, &wt, &now())
        .map_err(|e| flat_input_err(&e))
}

/// Parse a STRUCTURED request body into a canonical-JSON COMPOSITION.
pub(crate) async fn composition_from_structured(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Value, RestError> {
    let template_id = header_template_id(headers).ok_or_else(missing_template_id)?;
    let structured: Value = serde_json::from_slice(body).map_err(|e| {
        RestError(ApiError::BadRequest(format!(
            "invalid STRUCTURED JSON: {e}"
        )))
    })?;
    let wt = state
        .backend()
        .web_template(&template_id)
        .await
        .map_err(RestError::from)?;
    openehr_sdt::flat::convert::submitted_composition_from_structured(&structured, &wt, &now())
        .map_err(|e| flat_input_err(&e))
}

// ── COMPOSITION: response side ─────────────────────────────────────────────

/// The template id a stored COMPOSITION declares
/// (`archetype_details/template_id/value`).
fn composition_template_id(comp: &Value) -> Result<String, RestError> {
    comp.pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| internal("composition has no archetype_details/template_id"))
}

/// Render a canonical-JSON COMPOSITION as a FLAT
/// `application/openehr.wt.flat+json` response.
pub(crate) async fn composition_flat_response(
    state: &AppState,
    status: StatusCode,
    comp: &Value,
) -> Result<Response, RestError> {
    let wt = state
        .backend()
        .web_template(&composition_template_id(comp)?)
        .await
        .map_err(RestError::from)?;
    composition_flat_response_with(status, comp, &wt)
}

/// Render a COMPOSITION as a FLAT response using an **already-resolved**
/// `WebTemplate` — the seam for surfaces whose template is not in the ADL 1.4
/// store (e.g. an ADL2 example, whose Web Template the `v2_4` front end built).
pub(crate) fn composition_flat_response_with(
    status: StatusCode,
    comp: &Value,
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
) -> Result<Response, RestError> {
    let flat = openehr_sdt::flat::convert::composition_to_flat(comp, wt)
        .map_err(|e| flat_output_err(&e))?;
    let json =
        serde_json::to_string(&flat).map_err(|e| internal(format!("FLAT serialization: {e}")))?;
    Ok(negotiate::flat_json_body(status, json))
}

/// Render a canonical-JSON COMPOSITION as a STRUCTURED
/// `application/openehr.wt.structured+json` response.
pub(crate) async fn composition_structured_response(
    state: &AppState,
    status: StatusCode,
    comp: &Value,
) -> Result<Response, RestError> {
    let wt = state
        .backend()
        .web_template(&composition_template_id(comp)?)
        .await
        .map_err(RestError::from)?;
    composition_structured_response_with(status, comp, &wt)
}

/// Render a COMPOSITION as a STRUCTURED response using an **already-resolved**
/// `WebTemplate` (the ADL2-example seam; see [`composition_flat_response_with`]).
pub(crate) fn composition_structured_response_with(
    status: StatusCode,
    comp: &Value,
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
) -> Result<Response, RestError> {
    let structured = openehr_sdt::flat::convert::composition_to_structured(comp, wt)
        .map_err(|e| flat_output_err(&e))?;
    let json = serde_json::to_string(&structured)
        .map_err(|e| internal(format!("STRUCTURED serialization: {e}")))?;
    Ok(negotiate::structured_json_body(status, json))
}

// ── CONTRIBUTION: envelope canonical, inner payload simplified ─────────────

// NOTE: master05-rm_mapping §scope maps only COMPOSITION (master02
// §Relationship to Other Specifications), so only COMPOSITION inner payloads
// are simplifiable — others refuse (create `422`; get `406` naming COMPOSITION).

/// Convert a simplified CONTRIBUTION request into a canonical envelope: the
/// envelope stays canonical JSON and reads as the generated
/// `NewContribution<Value>`; each present `versions[i].data` is rebuilt from the
/// simplified `format` into a canonical COMPOSITION using the
/// `openehr-template-id` header. A version with no `data` is a logical deletion
/// (RM common master06 §Logical Deletion) and passes through. Missing header →
/// `422`.
pub(crate) async fn contribution_from_simplified(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
    format: WireFormat,
) -> Result<Value, RestError> {
    let template_id = header_template_id(headers).ok_or_else(missing_template_id)?;
    let mut envelope: NewContribution<Value> = serde_json::from_slice(body).map_err(|e| {
        RestError(ApiError::BadRequest(format!(
            "invalid CONTRIBUTION envelope: {e}"
        )))
    })?;
    let wt = state
        .backend()
        .web_template(&template_id)
        .await
        .map_err(RestError::from)?;
    let now = now();
    for version in &mut envelope.versions {
        let Some(data) = version.data.as_mut() else {
            continue;
        };
        let comp = match format {
            WireFormat::Flat => {
                let map = data.as_object().ok_or_else(|| {
                    RestError(ApiError::Unprocessable(
                        "a FLAT CONTRIBUTION versions[].data must be a JSON object".to_owned(),
                    ))
                })?;
                openehr_sdt::flat::convert::submitted_composition_from_flat(map, &wt, &now)
            }
            WireFormat::Structured => {
                openehr_sdt::flat::convert::submitted_composition_from_structured(data, &wt, &now)
            }
            _ => {
                return Err(internal(
                    "non-simplified format routed to CONTRIBUTION converter",
                ));
            }
        }
        .map_err(|e| flat_input_err(&e))?;
        *data = comp;
    }
    serde_json::to_value(&envelope).map_err(|e| {
        internal(format!(
            "the canonical CONTRIBUTION envelope does not encode: {e}"
        ))
    })
}

/// Render a stored CONTRIBUTION body in a simplified `format`: the envelope
/// stays canonical JSON; each present `versions[i].data` COMPOSITION is
/// serialized into the simplified form. A present non-COMPOSITION inner payload
/// → `406` naming COMPOSITION as the only simplifiable kind.
pub(crate) async fn contribution_to_simplified(
    state: &AppState,
    status: StatusCode,
    body: &Value,
    format: WireFormat,
) -> Result<Response, RestError> {
    let mut envelope = body.clone();
    if let Some(versions) = envelope.get_mut("versions").and_then(Value::as_array_mut) {
        for version in versions.iter_mut() {
            let Some(obj) = version.as_object_mut() else {
                continue;
            };
            // Clone the inner payload so no borrow of `obj` is held across the
            // async WebTemplate resolution or the final re-insert.
            let Some(data) = obj.get("data").cloned() else {
                continue; // an OBJECT_REF version (unresolved) has no inner payload
            };
            let kind = data.pointer("/_type").and_then(Value::as_str);
            if kind != Some("COMPOSITION") {
                return Err(RestError(ApiError::NotAcceptable(
                    "only COMPOSITION version payloads can be serialized in a Simplified \
                     Format; request application/json for this CONTRIBUTION"
                        .to_owned(),
                )));
            }
            let wt = state
                .backend()
                .web_template(&composition_template_id(&data)?)
                .await
                .map_err(RestError::from)?;
            let simplified = match format {
                WireFormat::Flat => {
                    openehr_sdt::flat::convert::composition_to_flat(&data, &wt).map(Value::Object)
                }
                WireFormat::Structured => {
                    openehr_sdt::flat::convert::composition_to_structured(&data, &wt)
                }
                _ => {
                    return Err(internal(
                        "non-simplified format routed to CONTRIBUTION renderer",
                    ));
                }
            }
            .map_err(|e| flat_output_err(&e))?;
            obj.insert("data".to_owned(), simplified);
        }
    }
    let json = serde_json::to_string(&envelope)
        .map_err(|e| internal(format!("CONTRIBUTION serialization: {e}")))?;
    Ok(match format {
        WireFormat::Structured => negotiate::structured_json_body(status, json),
        _ => negotiate::flat_json_body(status, json),
    })
}

// ── Non-templated resources: uniform reject ────────────────────────────────

/// Reject Simplified-Formats negotiation on a resource that has no
/// Simplified-Formats mapping — EHR, `EHR_STATUS`, FOLDER, and the demographic
/// PARTY types. `415` when the request `Content-Type` is a simplified type
/// (input), `406` when the `Accept` cannot be satisfied by canonical JSON/XML
/// (output). Canonical requests pass through untouched.
///
/// NOTE (`ITS-REST/docs/simplified_formats/master05-rm_mapping.adoc`
/// §scope + `master02-overview.adoc` §Relationship to Other Specifications):
/// simplified field identifiers are generated from an Operational Template, and
/// master05 defines mappings only for COMPOSITION and the classes it contains.
/// The OAS declares the simplified media types on these endpoints via
/// `Accept_LOCATABLE`/`ContentType_LOCATABLE`, but no spec governs their
/// simplified serialization — so we reject rather than guess. If a future
/// ITS-REST release defines these mappings, this reject branch is replaced.
pub(crate) fn guard_non_templated(headers: &HeaderMap) -> Result<(), RestError> {
    if let Some(WireFormat::Flat | WireFormat::Structured | WireFormat::WebTemplate) =
        negotiate::content_type_format(headers)
    {
        return Err(RestError(ApiError::UnsupportedMediaType(
            "Simplified Formats are not defined for this resource; supported request \
             Content-Type: application/json, application/xml"
                .to_owned(),
        )));
    }
    if negotiate::resolve_accept(headers, negotiate::CANONICAL, WireFormat::CanonicalJson).is_none()
    {
        return Err(RestError(ApiError::NotAcceptable(
            "Simplified Formats are not defined for this resource; supported response \
             formats: application/json, application/xml"
                .to_owned(),
        )));
    }
    Ok(())
}

/// Whether the request body arrived in a simplified data-instance format
/// (`application/openehr.wt.flat+json` / `…structured+json`) — used by callers
/// that must resolve the template id from the header rather than the canonical
/// payload (e.g. the authz PEP's composition-template extraction).
pub(crate) fn is_simplified_body(headers: &HeaderMap) -> bool {
    matches!(
        negotiate::content_type_format(headers),
        Some(WireFormat::Flat | WireFormat::Structured)
    )
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use http::{HeaderMap, HeaderValue, StatusCode, header};

    use super::{flat_input_err, flat_output_err, guard_non_templated, header_template_id};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    /// An input conversion failure is client data → `422` (`Requests_and_responses`
    /// §HTTP status codes).
    #[test]
    fn input_conversion_failure_maps_to_422() {
        let e = openehr_sdt::flat::error::FlatError::Conversion("bad leaf".to_owned());
        let status = flat_input_err(&e).into_response().status();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// An output conversion failure (rendering stored server data) stays `500`.
    #[test]
    fn output_conversion_failure_stays_500() {
        let e = openehr_sdt::flat::error::FlatError::Conversion("bad leaf".to_owned());
        let status = flat_output_err(&e).into_response().status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// The template id is read from the `openehr-template-id` header
    /// (case-insensitive) — never a query parameter (`Requests_and_responses`
    /// §openehr-template-id).
    #[test]
    fn template_id_from_header_case_insensitive() {
        assert_eq!(
            header_template_id(&headers(&[("openehr-template-id", "T1")])).as_deref(),
            Some("T1")
        );
        assert_eq!(
            header_template_id(&headers(&[("OPENEHR-TEMPLATE-ID", "T2")])).as_deref(),
            Some("T2")
        );
        assert!(header_template_id(&HeaderMap::new()).is_none());
    }

    /// A simplified `Content-Type` on a non-templated resource → `415`; a
    /// simplified `Accept` → `406`; canonical requests pass.
    #[test]
    fn guard_non_templated_rejects_simplified() {
        assert_eq!(
            guard_non_templated(&headers(&[(
                "content-type",
                "application/openehr.wt.flat+json"
            )]))
            .unwrap_err()
            .0
            .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            guard_non_templated(&headers(&[(
                "accept",
                "application/openehr.wt.structured+json"
            )]))
            .unwrap_err()
            .0
            .status(),
            StatusCode::NOT_ACCEPTABLE
        );
        assert!(guard_non_templated(&headers(&[("content-type", "application/json")])).is_ok());
        assert!(guard_non_templated(&headers(&[("accept", "application/xml")])).is_ok());
    }
}
