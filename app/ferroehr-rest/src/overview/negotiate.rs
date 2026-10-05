// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Content negotiation core (ITS-REST `Resources.md §Data representation` +
//! §Simplified Formats).
//!
//! One `WireFormat` enum names every representation the server negotiates —
//! canonical JSON/XML plus the Simplified Formats (FLAT, STRUCTURED, and the
//! Web Template document). Two resolvers, both parameterized by the set of
//! formats an endpoint allows, are the single negotiation seam every endpoint
//! dispatches through:
//!
//! - `content_type_format` classifies a request `Content-Type` (unknown →
//!   `None` → the caller answers `415`, Resources.md §Simplified Formats MUST).
//! - `resolve_accept` parses `Accept` with RFC 9110 §12.5.1 quality values
//!   and returns the highest-q allowed format (`None` → the caller answers
//!   `406`, same MUST rule).
//!
//! The recognized media types are matched EXACTLY (`application/json`,
//! `application/xml`, `application/openehr.wt.flat+json`,
//! `application/openehr.wt.structured+json`, `application/openehr.wt+json`).
//! The deprecated `…wt.flat.schema+json`/`…wt.structured.schema+json` names and
//! the legacy `application/openehr.nc.flat+json` / `…tds2+xml` types are NOT
//! recognized (Resources.md §Simplified Formats NOTE + §Alternative data
//! formats) — they fall out as `406`/`415` like any other unsupported type.
//!
//! Canonical bodies and responses are (de)serialized via `openehr-its`. The
//! generated server traits exchange `serde_json::Value` at the boundary, so an
//! XML body is decoded into its concrete `openehr-rm` type and re-emitted as
//! canonical JSON (`rm_value`), and an XML response re-types the canonical JSON
//! so the generated `ToXml` runs (`respond_rm`). The Simplified-Formats payload
//! conversion is the sibling `crate::formats` adapter.
//!
//! ## The `version` media-type parameter (ITS-XML lineage selection)
//!
//! openEHR publishes canonical XML in two wire lineages that differ only by
//! the document's root namespace — `http://schemas.openehr.org/v1`
//! (`Release-1.0.2v2`, STABLE) and `http://schemas.openehr.org/v2`
//! (`Release-2.0.0v2`, TRIAL upstream); see
//! `docs/specs/openehr/ITS-XML/README.adoc` §"Releases and IM Versions".
//! A client selects one per request with a media-type parameter on the XML
//! type — `Accept: application/xml; version=1` for the response,
//! `Content-Type: application/xml; version=1` to declare a request payload.
//! Absent (or `version=2`) means the v2 default — the only published lineage
//! whose schemas model the RM 1.2.0 this server serves (owner ruling
//! 2026-08-03, #1666; the v1 bundle lacks 50 concrete RM classes), so the
//! default a schema-validating client receives actually
//! validates.
//!
//! NOTE: no openEHR spec governs the parameter — our own design/extension:
//! `Resources.md` §XML Format requires conformance to "the [published XSDs]"
//! without naming a lineage and says nothing about media-type parameters.
//!
//! What the released text does fix is the refusal shape, and both branches here
//! are exactly those MUSTs: an unrecognized `version` on `Accept` is an aspect
//! of the request the service cannot fulfill ("it MUST respond with HTTP status
//! code `406 Not Acceptable`") and an unrecognized `version` on `Content-Type`
//! is a payload it cannot process as XML ("it MUST respond with HTTP status code
//! `415 Unsupported Media Type`"). Selection applies to canonical RM documents
//! only; the OPT 1.4 template representation is always v1 (`openehr_its::opt14`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — one byte-to-JSON \
              step per route, consumed by the typed decode"
)]

use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, StatusCode, header};

use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::{FromXml, Namespace, ToXml};
use serde::Serialize;
use serde::de::DeserializeOwned;

use ferroehr::service::response::{ResourceMeta, ServiceResponse};

/// A negotiated wire representation of a resource (ITS-REST
/// `Resources.md §Data representation` + §Simplified Formats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireFormat {
    /// `application/json` — canonical openEHR JSON.
    CanonicalJson,
    /// `application/xml` (or `text/xml`) — canonical openEHR XML.
    CanonicalXml,
    /// `application/openehr.wt.flat+json` — Simplified FLAT (simSDT) JSON.
    Flat,
    /// `application/openehr.wt.structured+json` — Simplified STRUCTURED
    /// (structSDT) JSON.
    Structured,
    /// `application/openehr.wt+json` — the Operational Template rendered as a
    /// Web Template document (template resource only).
    WebTemplate,
}

const APPLICATION_JSON: &str = "application/json";
const APPLICATION_XML: &str = "application/xml";
const TEXT_XML: &str = "text/xml";
/// Web Template document media type (`master02 §MIME Types`, template only).
const APPLICATION_WT_JSON: &str = "application/openehr.wt+json";
/// Simplified FLAT (simSDT) media type.
const APPLICATION_WT_FLAT_JSON: &str = "application/openehr.wt.flat+json";
/// Simplified STRUCTURED (structSDT) media type.
const APPLICATION_WT_STRUCTURED_JSON: &str = "application/openehr.wt.structured+json";

/// Line-delimited JSON, the media type our own long-running operational routes
/// stream their findings as. Not a [`WireFormat`] — see [`accepts_ndjson`].
pub(crate) const APPLICATION_NDJSON: &str = "application/x-ndjson";

/// The canonical-only allowed set (`Accept_LOCATABLE` minus the simplified
/// types) — the negotiation set for every canonical RM object endpoint.
pub(crate) const CANONICAL: &[WireFormat] = &[WireFormat::CanonicalJson, WireFormat::CanonicalXml];

// ── The negotiation core ─────────────────────────────────────────────────

/// The media type token of a header range (parameters after `;` stripped).
fn media_token(range: &str) -> &str {
    range.split(';').next().unwrap_or(range).trim()
}

/// Classify one media type (parameters stripped) into a [`WireFormat`],
/// comparing case-insensitively in place (RFC 9110 §8.3.1 — media types are
/// case-insensitive; no lowercased copy is allocated). Exact-match only — the
/// deprecated `…schema+json` and legacy `…nc.flat+json`/`…tds2+xml` types are
/// deliberately unrecognized (`Resources.md §Simplified Formats` NOTE +
/// §Alternative data formats), so they return `None`.
fn classify_media(media: &str) -> Option<WireFormat> {
    if media.eq_ignore_ascii_case(APPLICATION_JSON) {
        Some(WireFormat::CanonicalJson)
    } else if media.eq_ignore_ascii_case(APPLICATION_XML) || media.eq_ignore_ascii_case(TEXT_XML) {
        Some(WireFormat::CanonicalXml)
    } else if media.eq_ignore_ascii_case(APPLICATION_WT_FLAT_JSON) {
        Some(WireFormat::Flat)
    } else if media.eq_ignore_ascii_case(APPLICATION_WT_STRUCTURED_JSON) {
        Some(WireFormat::Structured)
    } else if media.eq_ignore_ascii_case(APPLICATION_WT_JSON) {
        Some(WireFormat::WebTemplate)
    } else {
        None
    }
}

/// The [`WireFormat`] a request `Content-Type` declares, or `None` when the
/// media type is not one this server recognizes (caller → `415`, Resources.md
/// §Simplified Formats MUST). An absent `Content-Type` defaults to canonical
/// JSON (`Resources.md §JSON Format`).
pub(crate) fn content_type_format(headers: &HeaderMap) -> Option<WireFormat> {
    match header_str(headers, header::CONTENT_TYPE) {
        None => Some(WireFormat::CanonicalJson),
        Some(ct) => classify_media(media_token(ct)),
    }
}

/// Resolve the response [`WireFormat`] from `Accept` against `allowed`, per
/// RFC 9110 §12.5.1 quality-value negotiation (`Resources.md §Data
/// representation`): the highest-q media range that maps to an allowed format
/// wins; a more specific range beats a wildcard at equal q; the endpoint
/// `default` breaks any remaining tie and answers an absent (or empty)
/// `Accept`. Returns `None` when no allowed format is acceptable (caller →
/// `406`, Resources.md §Simplified Formats MUST).
pub(crate) fn resolve_accept(
    headers: &HeaderMap,
    allowed: &[WireFormat],
    default: WireFormat,
) -> Option<WireFormat> {
    let Some(accept) = header_str(headers, header::ACCEPT) else {
        return Some(default);
    };
    if accept.trim().is_empty() {
        return Some(default);
    }
    let per_format = rank_allowed_formats(accept, allowed);
    let mut best: Option<(WireFormat, f64, u8)> = None;
    for (slot, &fmt) in per_format.iter().zip(allowed) {
        let Some((q, spec)) = *slot else {
            continue;
        };
        if q <= 0.0 {
            // `;q=0` explicitly rejects the format (RFC 9110 §12.5.1).
            continue;
        }
        let candidate = (fmt, q, spec);
        best = Some(match best {
            None => candidate,
            Some(current) => choose(current, candidate, default),
        });
    }
    best.map(|(fmt, _, _)| fmt)
}

/// The best `(quality, specificity)` each allowed format reaches across the
/// `Accept` ranges, positionally aligned to `allowed`.
///
/// ONE pass over the ranges: each is tokenized and q-parsed once, and every
/// allowed format aggregates its best pair from that same pass — highest q,
/// then specificity.
fn rank_allowed_formats(accept: &str, allowed: &[WireFormat]) -> Vec<Option<(f64, u8)>> {
    let mut per_format: Vec<Option<(f64, u8)>> = vec![None; allowed.len()];
    for range in accept.split(',') {
        let range = range.trim();
        if range.is_empty() {
            continue;
        }
        let token = media_token(range);
        let q = quality_of(range);
        for (slot, &fmt) in per_format.iter_mut().zip(allowed) {
            let Some(spec) = specificity_for(token, fmt) else {
                continue;
            };
            *slot = Some(match *slot {
                None => (q, spec),
                Some((bq, bs)) if q > bq || (q >= bq && spec > bs) => (q, spec),
                Some(current) => current,
            });
        }
    }
    per_format
}

/// The specificity with which `token` matches `fmt` (compared
/// case-insensitively in place), or `None` for no match. specificity: `2` =
/// exact type/subtype, `1` = a type wildcard (`application/*`, `text/*`),
/// `0` = `*/*`.
fn specificity_for(token: &str, fmt: WireFormat) -> Option<u8> {
    if token == "*/*" {
        Some(0)
    } else if token.eq_ignore_ascii_case("application/*") {
        // Every negotiated format has an `application/*` media type.
        Some(1)
    } else if token.eq_ignore_ascii_case("text/*") {
        (fmt == WireFormat::CanonicalXml).then_some(1)
    } else {
        (classify_media(token) == Some(fmt)).then_some(2)
    }
}

/// The quality value of a media range (`;q=` weight; default `1.0`, clamped to
/// `[0, 1]`, RFC 9110 §12.5.1).
fn quality_of(range: &str) -> f64 {
    for param in range.split(';').skip(1) {
        let param = param.trim();
        if let Some(v) = param
            .strip_prefix("q=")
            .or_else(|| param.strip_prefix("Q="))
        {
            return v.trim().parse::<f64>().map_or(1.0, |q| q.clamp(0.0, 1.0));
        }
    }
    1.0
}

/// Pick the winner of two candidates: higher q, then higher specificity, then
/// the endpoint `default`, then a fixed preference order (canonical JSON
/// first).
fn choose(
    a: (WireFormat, f64, u8),
    b: (WireFormat, f64, u8),
    default: WireFormat,
) -> (WireFormat, f64, u8) {
    if a.1 > b.1 {
        return a;
    }
    if b.1 > a.1 {
        return b;
    }
    if a.2 > b.2 {
        return a;
    }
    if b.2 > a.2 {
        return b;
    }
    if a.0 == default {
        return a;
    }
    if b.0 == default {
        return b;
    }
    if pref_rank(a.0) <= pref_rank(b.0) {
        a
    } else {
        b
    }
}

// ── ITS-XML lineage selection (the `version` media-type parameter) ─────────
//
// NOTE: no openEHR spec governs this — our own design/extension (module docs
// above carry the full reasoning and the two released refusal MUSTs it reuses).

/// The name of the media-type parameter that selects the ITS-XML lineage.
/// Parameter names are case-insensitive (RFC 9110 §8.3.1).
const XML_VERSION_PARAM: &str = "version";

/// `application/xml` labelled with the non-default v1 lineage — the response
/// `Content-Type` when v1 was negotiated, so a client is told which lineage it
/// received rather than having to sniff the root `xmlns`.
const APPLICATION_XML_V1: &str = "application/xml; version=1";

/// What the `version` parameter of ONE media range says about the lineage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XmlLineage {
    /// No `version` parameter: the default (v2) lineage.
    Default,
    /// A recognized lineage selector.
    Selected(Namespace),
    /// A `version` value this server does not serve — the caller answers
    /// `406` (response side) or `415` (request side).
    Unrecognized,
}

/// Read the `version` parameter off one media range (`application/xml;
/// version=2`). Quoted and bare forms are both accepted (RFC 9110 §5.6.6
/// makes a parameter value a token OR a quoted-string).
fn xml_lineage_of(range: &str) -> XmlLineage {
    for param in range.split(';').skip(1) {
        let Some((name, value)) = param.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(XML_VERSION_PARAM) {
            continue;
        }
        return match value.trim().trim_matches('"') {
            "1" => XmlLineage::Selected(Namespace::V1),
            "2" => XmlLineage::Selected(Namespace::V2),
            _ => XmlLineage::Unrecognized,
        };
    }
    XmlLineage::Default
}

/// The XML media range an `Accept` header offers with the highest
/// `(quality, specificity)` — the range whose `version` parameter governs the
/// response lineage. Mirrors [`resolve_accept`]'s per-range ordering so the
/// parameter is read off the very range that won the negotiation.
fn best_xml_range(accept: &str) -> Option<&str> {
    let mut best: Option<(&str, f64, u8)> = None;
    for range in accept.split(',') {
        let range = range.trim();
        if range.is_empty() {
            continue;
        }
        let token = media_token(range);
        let Some(spec) = specificity_for(token, WireFormat::CanonicalXml) else {
            continue;
        };
        let q = quality_of(range);
        if q <= 0.0 {
            continue;
        }
        best = Some(match best {
            None => (range, q, spec),
            Some((_, bq, bs)) if q > bq || (q >= bq && spec > bs) => (range, q, spec),
            Some(current) => current,
        });
    }
    best.map(|(range, _, _)| range)
}

/// The ITS-XML lineage an XML RESPONSE must be serialized in, resolved from
/// `Accept`. `None` means the client asked for a lineage this server does not
/// serve — the caller answers `406` (`Resources.md` §XML Format: "If the
/// service cannot fulfill this aspect of the request, it MUST respond with
/// HTTP status code `406 Not Acceptable`").
///
/// NOTE: the lineage is a second gate after the format, not a condition folded
/// into [`resolve_accept`], so an `Accept` naming an unserved lineage beside an
/// otherwise acceptable format (`application/json;q=0.5, application/xml;version=3`)
/// answers `406` rather than quietly serving a format the client ranked lower.
/// No openEHR spec governs the parameter — our own design/extension.
pub(crate) fn accept_xml_namespace(headers: &HeaderMap) -> Option<Namespace> {
    let Some(accept) = header_str(headers, header::ACCEPT) else {
        return Some(Namespace::V2);
    };
    let Some(range) = best_xml_range(accept) else {
        return Some(Namespace::V2);
    };
    match xml_lineage_of(range) {
        XmlLineage::Default => Some(Namespace::V2),
        XmlLineage::Selected(ns) => Some(ns),
        XmlLineage::Unrecognized => None,
    }
}

/// Whether the request explicitly asks for a line-delimited JSON stream.
///
/// Line-delimited JSON is not a [`WireFormat`]: no openEHR spec defines it, no
/// RM resource is served that way, and no ITS-REST operation offers it. It is
/// the response SHAPE two of our own operational routes can take when the work
/// behind them runs longer than one response may take to produce. It is
/// therefore asked for BY NAME and never matched by a wildcard — a client
/// sending `*/*`, or no `Accept` at all, keeps the aggregated document, which
/// is what stops the shape changing under an existing caller.
///
/// NOTE: no openEHR spec governs this media type — our own design/extension.
pub(crate) fn accepts_ndjson(headers: &HeaderMap) -> bool {
    let Some(accept) = header_str(headers, header::ACCEPT) else {
        return false;
    };
    accept.split(',').any(|range| {
        media_token(range).eq_ignore_ascii_case(APPLICATION_NDJSON) && quality_of(range) > 0.0
    })
}

/// Refuse a request whose `Content-Type` declares canonical XML in a lineage
/// this server cannot read.
///
/// A declared lineage is otherwise inert: the `openehr-its` reader dispatches
/// on local element names and `xsi:type` and never inspects the root `xmlns`,
/// so a v1 and a v2 payload parse identically. This guard exists so an
/// unrecognized `version` is refused loudly instead of being parsed as
/// whatever the body happens to contain.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] for an unrecognized `version` value
/// (`Resources.md` §XML Format: "If the service cannot process the request
/// payload as XML format, it MUST respond with HTTP status code `415
/// Unsupported Media Type`").
pub(crate) fn require_known_xml_lineage(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(declared) = header_str(headers, header::CONTENT_TYPE) else {
        return Ok(());
    };
    if classify_media(media_token(declared)) != Some(WireFormat::CanonicalXml) {
        return Ok(());
    }
    match xml_lineage_of(declared) {
        XmlLineage::Default | XmlLineage::Selected(_) => Ok(()),
        XmlLineage::Unrecognized => Err(ApiError::UnsupportedMediaType(format!(
            "canonical XML is served in the openEHR ITS-XML lineages \
             `version=1` and `version=2` (default), got {declared}"
        ))),
    }
}

/// Fixed tie-break order: canonical JSON is the server's preferred default.
fn pref_rank(fmt: WireFormat) -> u8 {
    match fmt {
        WireFormat::CanonicalJson => 0,
        WireFormat::CanonicalXml => 1,
        WireFormat::Flat => 2,
        WireFormat::Structured => 3,
        WireFormat::WebTemplate => 4,
    }
}

// ── Simplified-Formats + Web Template body builders ────────────────────────

/// Serve a pre-serialized STRUCTURED (structSDT) document as
/// `application/openehr.wt.structured+json`.
pub(crate) fn structured_json_body(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_WT_STRUCTURED_JSON),
    );
    resp
}

/// Serve a pre-serialized JSON document as `application/json`.
///
/// The `application/json` sibling of [`wt_json_body`] / [`flat_json_body`] /
/// [`structured_json_body`]: the same body under the canonical JSON media
/// type, for an endpoint whose negotiated `Accept` was `application/json`
/// (`Resources.md` §JSON Format: "Proper header `Content-Type:
/// application/json` MUST be present in the response of the service unless the
/// response has no content body"). Payloads that are serialized here rather
/// than by the RM codec use [`respond`]; this builder takes the already-formed
/// document.
pub(crate) fn json_body(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_JSON),
    );
    resp
}

/// Serve a pre-serialized Web Template document as `application/openehr.wt+json`.
pub(crate) fn wt_json_body(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_WT_JSON),
    );
    resp
}

/// Serve a pre-serialized FLAT (simSDT) document as
/// `application/openehr.wt.flat+json`.
pub(crate) fn flat_json_body(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_WT_FLAT_JSON),
    );
    resp
}

// ── Body decoders ──────────────────────────────────────────────────────────

/// Format an HTTP-date (RFC 7231 IMF-fixdate, always GMT) for `Last-Modified`.
pub(crate) fn http_date(at: jiff::Timestamp) -> String {
    at.strftime("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Decode a required JSON body into a `serde_json::Value`.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not JSON;
/// [`ApiError::BadRequest`] if the bytes are not valid JSON.
pub(crate) fn json_value(headers: &HeaderMap, body: &Bytes) -> Result<serde_json::Value, ApiError> {
    require_json(headers)?;
    parse_json(body)
}

/// Decode a required JSON array body into untyped values (the TDD item list,
/// which the release binds to no component schema).
pub(crate) fn json_vec(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Vec<serde_json::Value>, ApiError> {
    require_json(headers)?;
    serde_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON array body: {e}")))
}

/// Decode a required JSON **array** body against the generated DTO of the
/// component schema the release binds to it — strictly.
///
/// The generated DTOs carry `#[serde(deny_unknown_fields)]` wherever the OAS
/// schema declares `additionalProperties: false`, so a member the schema does
/// not define is REFUSED here rather than silently dropped, and a member of the
/// wrong JSON type is refused rather than silently read as absent. The refusal
/// names the offending member by its JSON PATH (`[0].value`) — `serde_json`
/// alone reports only a line/column, which tells a client nothing about which
/// array element it must fix.
///
/// The one caller family today is the `ITEM_TAG` PUT
/// (`schemas/common/UpdateItemTag.yaml`: required `key`, optional
/// `value`/`target_path`, `additionalProperties: false`). The ITS-REST docs
/// text is silent on the write body's member set, and where the docs text is
/// silent the released OAS grounds the expectation — the release presents its
/// OAS files as its computable specification artifacts (ITS-REST overview
/// `Specifications.md`: "Specifications can be downloaded as YAML files in
/// OpenAPI Specification 3.0 format") — so the released schema is the ground
/// here.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not canonical
/// JSON; [`ApiError::BadRequest`] if the bytes are not a JSON array, or if any
/// element violates the declared schema.
pub(crate) fn typed_json_vec<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Vec<T>, ApiError> {
    require_json(headers)?;
    let mut de = serde_json::Deserializer::from_slice(body);
    serde_path_to_error::deserialize::<_, Vec<T>>(&mut de).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        if path == "." {
            ApiError::BadRequest(format!("invalid JSON array body: {inner}"))
        } else {
            ApiError::BadRequest(format!("invalid JSON array body at {path}: {inner}"))
        }
    })
}

/// Decode a required JSON **object** body into the typed DTO `T` — the scalar
/// sibling of [`typed_json_vec`], with the same path-named refusal (`400`)
/// so a client learns which member it must fix.
///
/// Only the event-subscription routes decode a scalar body today, so the
/// helper carries their feature gate: a slim build without `events` has no
/// caller and would otherwise fail the dead-code lane.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not JSON;
/// [`ApiError::BadRequest`] if the bytes do not decode as `T`.
#[cfg(feature = "events")]
pub(crate) fn typed_json<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<T, ApiError> {
    require_json(headers)?;
    let mut de = serde_json::Deserializer::from_slice(body);
    serde_path_to_error::deserialize::<_, T>(&mut de).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        if path == "." {
            ApiError::BadRequest(format!("invalid JSON body: {inner}"))
        } else {
            ApiError::BadRequest(format!("invalid JSON body at {path}: {inner}"))
        }
    })
}

/// Decode a plain-text body (e.g. a stored-query YAML document).
pub(crate) fn text_body(body: &Bytes) -> Result<String, ApiError> {
    String::from_utf8(body.to_vec())
        .map_err(|e| ApiError::BadRequest(format!("body is not UTF-8: {e}")))
}

/// Decode an RM-typed body from canonical JSON or XML into the concrete
/// `openehr-rm` type `T` — the typed commit seam.
///
/// **Both branches are typed.** The JSON branch reads through
/// `openehr_its::json::from_canonical_json`, whose emitted `Deserialize` impls
/// ARE the strict canonical reader: an undeclared key, a repeated key, an
/// absent mandatory attribute, a present-but-wrong `_type`, an empty `1..*`
/// list and a malformed identifier are all refused there, path-named. That is
/// the PARSE class, so every one of them answers **400**: ITS-REST overview
/// `Requests_and_responses.md` §HTTP status codes assigns 400 to a request
/// whose content "could not be parsed or is invalid" and reserves 422 for a
/// body that is "well-formed but was unable to be followed due to semantic
/// errors" — a structurally impossible RM instance never becomes an RM value
/// at all, so it cannot reach the semantic pass.
///
/// # Errors
/// [`ApiError::BadRequest`] if the body cannot be parsed in the declared
/// format; [`ApiError::UnsupportedMediaType`] for any `Content-Type` other than
/// canonical JSON or XML (a Simplified-Formats or unknown type on a canonical
/// RM endpoint, Resources.md §Simplified Formats MUST), and for canonical XML
/// declared in an unrecognized ITS-XML lineage (see
/// [`require_known_xml_lineage`]).
pub(crate) fn rm_value<T>(headers: &HeaderMap, body: &Bytes) -> Result<T, ApiError>
where
    T: FromXml + DeserializeOwned,
{
    match content_type_format(headers) {
        Some(WireFormat::CanonicalJson) => {
            let json = std::str::from_utf8(body)
                .map_err(|e| ApiError::BadRequest(format!("body is not UTF-8: {e}")))?;
            openehr_its::json::from_canonical_json::<T>(json)
                .map_err(|e| ApiError::BadRequest(format!("invalid canonical JSON body: {e}")))
        }
        Some(WireFormat::CanonicalXml) => {
            require_known_xml_lineage(headers)?;
            let xml = text_body(body)?;
            openehr_its::xml::from_canonical_xml(&xml)
                .map_err(|e| ApiError::BadRequest(format!("invalid canonical XML body: {e}")))
        }
        _ => Err(ApiError::UnsupportedMediaType(format!(
            "this operation accepts application/json or application/xml only, got {}",
            header_str(headers, header::CONTENT_TYPE).unwrap_or("<none>")
        ))),
    }
}

/// Optional RM-typed body (empty → `None`).
///
/// # Errors
/// As [`rm_value`], for a non-empty body.
pub(crate) fn optional_rm_value<T>(headers: &HeaderMap, body: &Bytes) -> Result<Option<T>, ApiError>
where
    T: FromXml + DeserializeOwned,
{
    if body.is_empty() {
        return Ok(None);
    }
    Ok(Some(rm_value::<T>(headers, body)?))
}

/// A JSON-only write op accepts only canonical JSON on the wire.
fn require_json(headers: &HeaderMap) -> Result<(), ApiError> {
    match content_type_format(headers) {
        Some(WireFormat::CanonicalJson) => Ok(()),
        _ => Err(ApiError::UnsupportedMediaType(format!(
            "this operation accepts application/json only, got {}",
            header_str(headers, header::CONTENT_TYPE).unwrap_or("<none>")
        ))),
    }
}

/// Refuse a request whose `Content-Type` DECLARES a media type outside
/// `allowed`. `expected` names the accepted type(s) in the error message.
///
/// An ABSENT `Content-Type` is accepted: `Resources.md` §XML Format and §JSON
/// Format both make the header a client MAY ("A client MAY use the header
/// `Content-Type: application/xml` in the requests to specify the XML payload
/// format"), so its absence declares nothing to refuse and the operation's own
/// single body type applies.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] when the declared media type is not one
/// of `allowed` (`Resources.md` §XML Format: "If the service cannot process
/// the request payload as XML format, it MUST respond with HTTP status code
/// `415 Unsupported Media Type`" — and the symmetric §JSON Format sentence).
pub(crate) fn require_content_type(
    headers: &HeaderMap,
    allowed: &[WireFormat],
    expected: &str,
) -> Result<(), ApiError> {
    let Some(declared) = header_str(headers, header::CONTENT_TYPE) else {
        return Ok(());
    };
    match classify_media(media_token(declared)) {
        Some(fmt) if allowed.contains(&fmt) => Ok(()),
        _ => Err(ApiError::UnsupportedMediaType(format!(
            "this operation accepts {expected} only, got {declared}"
        ))),
    }
}

/// Refuse a request whose `Content-Type` DECLARES a media type other than
/// `text/plain` — the single body type of the ADL2 template upload
/// (`docs/specs/openehr/ITS-REST/specifications/operations/
/// definition_template_adl2_upload.yaml` declares `text/plain` as the only
/// request content type). The mirror of [`require_content_type`] for the one
/// route whose body type is outside the [`WireFormat`] vocabulary; an ABSENT
/// `Content-Type` is accepted for the same Resources.md client-MAY reason.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] when the declared media type is not
/// `text/plain` (`Resources.md` §format rules: a payload the service cannot
/// process as the operation's format "MUST respond with HTTP status code
/// `415 Unsupported Media Type`").
pub(crate) fn require_text_plain(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(declared) = header_str(headers, header::CONTENT_TYPE) else {
        return Ok(());
    };
    if media_token(declared).eq_ignore_ascii_case("text/plain") {
        return Ok(());
    }
    Err(ApiError::UnsupportedMediaType(format!(
        "this operation accepts text/plain only, got {declared}"
    )))
}

fn parse_json(body: &Bytes) -> Result<serde_json::Value, ApiError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
    // The strict reader at the door: a wire key the RM class does not declare
    // is a refusal, and a document the reader cannot READ never converts — so
    // it is a `400`, not the convertible-but-semantically-invalid `422`
    // (ITS-REST overview `Requests_and_responses.md` §HTTP status codes: 400 =
    // content that "could not be parsed or is invalid"; 422 = "well-formed but
    // was unable to be followed due to semantic errors").
    openehr_its::json::reject_undeclared_keys(&value)
        .map_err(|e| ApiError::BadRequest(format!("invalid canonical JSON body: {e}")))?;
    Ok(value)
}

/// Render a serializable payload as a JSON response. Used for responses that
/// are not a spec-typed RM value (`serde_json::Value` collections and DTOs:
/// item tags, terminology/query results). If the client's `Accept` cannot be
/// satisfied by canonical JSON, this returns `406` (those payloads have no
/// spec-defined canonical-XML shape). Spec-typed RM objects use [`respond_rm`].
pub(crate) fn respond<T: Serialize>(
    headers: &HeaderMap,
    status: StatusCode,
    value: &T,
) -> Response {
    // `respond` serves JSON-only, non-RM payloads: a `serde_json::Value` the
    // service already produced (already canonical), or an application DTO with
    // its own serde. These are not RM spec types, so they serialize via serde
    // (not the RM canonical codec — that path is `respond_rm`/`json_response`).
    match resolve_accept(
        headers,
        &[WireFormat::CanonicalJson],
        WireFormat::CanonicalJson,
    ) {
        Some(_) => match serde_json::to_string(value) {
            Ok(json) => {
                let mut resp = (status, json).into_response();
                resp.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(APPLICATION_JSON),
                );
                resp
            }
            Err(e) => crate::overview::error::internal_fault("serialize the JSON response", &e)
                .into_response_body(),
        },
        None => ApiError::NotAcceptable(
            "this response is available as application/json only".to_owned(),
        )
        .into_response_body(),
    }
}

/// The abstract XSD type declared for a published document element, when the
/// element's declared type is abstract and the instance must therefore carry
/// `xsi:type` — the published-element fact is stated ONCE, in the crate that
/// owns the schemas ([`openehr_its::xml::PUBLISHED_ROOTS`]).
fn declared_root_type(root_tag: &str) -> Option<&'static str> {
    openehr_its::xml::declared_abstract_root_type(root_tag)
}

/// Render a canonical-JSON `Value` that IS a single spec-typed RM object,
/// honouring `Accept` for canonical JSON or XML. `T` is the concrete
/// `openehr-rm` type the value encodes; `root_tag` is the XML root element
/// name. A JSON `null` value renders as a bodyless response.
///
/// A `root_tag` whose published XSD type is abstract
/// ([`openehr_its::xml::declared_abstract_root_type`]) is serialized through
/// the declared-type entry point, so the root names its concrete class with
/// `xsi:type` as the schema requires.
pub(crate) fn respond_rm<T>(
    headers: &HeaderMap,
    status: StatusCode,
    value: &serde_json::Value,
    root_tag: &str,
) -> Response
where
    T: DeserializeOwned + ToXml,
{
    match resolve_accept(headers, CANONICAL, WireFormat::CanonicalJson) {
        Some(WireFormat::CanonicalJson) => json_response(status, value),
        Some(WireFormat::CanonicalXml) => {
            if value.is_null() {
                return empty(status);
            }
            // The lineage is read off the winning XML media range; an
            // unrecognized `version` is an aspect of the request the service
            // cannot fulfill → 406 (Resources.md §XML Format).
            let Some(ns) = accept_xml_namespace(headers) else {
                return ApiError::NotAcceptable(
                    "canonical XML is served in the openEHR ITS-XML lineages \
                     `version=1` and `version=2` (default)"
                        .to_owned(),
                )
                .into_response_body();
            };
            let typed: T = match openehr_its::json::from_canonical_value(value) {
                Ok(t) => t,
                Err(e) => {
                    return crate::overview::error::internal_fault(
                        "re-type the canonical JSON for the XML response",
                        &format!("<{root_tag}>: {e}"),
                    )
                    .into_response_body();
                }
            };
            let serialized = match declared_root_type(root_tag) {
                Some(declared) => {
                    openehr_its::xml::to_canonical_xml_declared(&typed, root_tag, declared, ns)
                }
                None => openehr_its::xml::to_canonical_xml_ns(&typed, root_tag, ns),
            };
            match serialized {
                Ok(xml) => xml_body_ns(status, xml, ns),
                Err(e) => crate::overview::error::internal_fault("serialize the XML response", &e)
                    .into_response_body(),
            }
        }
        // Any other allowed format is impossible for `CANONICAL`; an
        // unsatisfiable `Accept` is a `406` (Resources.md §Simplified Formats).
        _ => ApiError::NotAcceptable(
            "this resource is available as application/json or application/xml".to_owned(),
        )
        .into_response_body(),
    }
}

/// A bodyless success response (204/200 for deletes).
pub(crate) fn empty(status: StatusCode) -> Response {
    status.into_response()
}

/// A bodyless success response carrying a `Location` header.
pub(crate) fn empty_with_location(status: StatusCode, location: &str) -> Response {
    let mut resp = status.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        resp.headers_mut().insert(header::LOCATION, value);
    }
    resp
}

// ── ITS-REST response-header + `Prefer` handling ─────────────────────

/// Whether the client asked for the full representation on `Prefer`
/// (`return=representation`). The ITS-REST default is `return=minimal`.
pub(crate) fn prefers_representation(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| {
            p.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("return=representation"))
        })
}

/// Whether the client asked for `OBJECT_REF` resolution on `Prefer`
/// (`resolve_refs`).
pub(crate) fn prefers_resolve_refs(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| {
            p.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("resolve_refs"))
        })
}

/// Whether the client asked for an identifier-only response on `Prefer`
/// (`return=identifier`).
pub(crate) fn prefers_identifier(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| {
            p.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("return=identifier"))
        })
}

/// The return preference a write response ACTUALLY applied — the value the
/// `Preference-Applied` response header declares.
///
/// ITS-REST overview `Requests_and_responses.md` §Representation details
/// negotiation: "The service MAY include a `Preference-Applied` header in the
/// response, such as `Preference-Applied: return=minimal` or
/// `Preference-Applied: return=representation`, to indicate that the client's
/// preference has been honored" — so the header states what the response DID,
/// never what the client merely asked for (RFC 7240 §3, the field
/// definition this section builds on).
///
/// [`Identifier`](AppliedPreference::Identifier) carries the identifier it
/// renders, which makes the identifier branch unreachable without one: that is
/// the structural guarantee behind §"Prefer only identifier" — "a variant of
/// preference that implies minimal response semantics, but with a non-empty
/// response body (i.e. the status will be `201 Created` or `200 OK`, never
/// `204 No Content`)".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedPreference<'a> {
    /// `return=minimal` — the default when no `Prefer` is sent: no body, and
    /// "if no response body is returned, the service SHOULD use `204 No
    /// Content`" (§"Prefer minimal, identifier or full representation
    /// response").
    Minimal,
    /// `return=identifier` — a body carrying only the affected resource's
    /// identifier (the `&str` is that identifier).
    Identifier(&'a str),
    /// `return=representation` — the full resource representation.
    Representation,
}

impl AppliedPreference<'_> {
    /// The `Preference-Applied` field value this outcome declares.
    fn token(self) -> &'static str {
        match self {
            AppliedPreference::Minimal => "return=minimal",
            AppliedPreference::Identifier(_) => "return=identifier",
            AppliedPreference::Representation => "return=representation",
        }
    }
}

/// Declare the applied return preference on a write response — the ONE place
/// `Preference-Applied` is written, so every write path (canonical RM,
/// JSON-only, demographic, template upload, item-tag collection, and the
/// Simplified-Formats commit) states its outcome the same way.
///
/// NOTE: the header is emitted on every write, including requests that carry
/// no `Prefer` at all — the applied preference is then the spec default,
/// `return=minimal` ("If no `Prefer` header is provided, the default behavior
/// is assumed to be `return=minimal`", §Representation details negotiation).
/// RFC 7240 §3 permits an unsolicited `Preference-Applied`, and stating the
/// applied default is what makes the behaviour uniform across write paths.
pub(crate) fn set_preference_applied(resp: &mut Response, applied: AppliedPreference<'_>) {
    resp.headers_mut().insert(
        header::HeaderName::from_static("preference-applied"),
        HeaderValue::from_static(applied.token()),
    );
}

/// The return preference a write can actually apply, given the identifier (if
/// any) the write produced.
///
/// `return=identifier` is honoured only when there IS an identifier to return:
/// its whole contract is a non-empty identifier body with a `201`/`200` status
/// (§"Prefer only identifier"), so a write that produced no resource metadata
/// cannot honour it. Rather than emit an empty (possibly `204`) body while
/// claiming `Preference-Applied: return=identifier`, the server applies — and
/// declares — the default `return=minimal`.
fn resolve_write_preference<'a>(
    headers: &HeaderMap,
    uid: Option<&'a str>,
) -> AppliedPreference<'a> {
    if prefers_representation(headers) {
        return AppliedPreference::Representation;
    }
    match (prefers_identifier(headers), uid) {
        (true, Some(uid)) => AppliedPreference::Identifier(uid),
        _ => AppliedPreference::Minimal,
    }
}

/// The status of an applied `return=identifier` write: the minimal status
/// unless that is `204 No Content`, which the identifier variant forbids —
/// "the status will be `201 Created` or `200 OK`, never `204 No Content`"
/// (§"Prefer only identifier") — in which case the representation status
/// (`200`/`201`) carries the identifier body.
fn identifier_status(minimal_status: StatusCode, repr_status: StatusCode) -> StatusCode {
    if minimal_status == StatusCode::NO_CONTENT {
        repr_status
    } else {
        minimal_status
    }
}

/// The single `Prefer` seam for a create/update response: resolve the
/// preference the write can apply, render its body, and declare it.
///
/// `representation` renders the full body at the status it is handed; it is
/// only called when `return=representation` is applied. The identifier body is
/// rendered by [`identifier_response`] at [`identifier_status`] — structurally
/// never `204`, because the branch is reachable only with an identifier in
/// hand. Resource headers (`ETag`/`Last-Modified`/`Location`) are the caller's
/// business; this seam owns the body + `Preference-Applied` only.
pub(crate) fn write_negotiated(
    headers: &HeaderMap,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    uid: Option<&str>,
    representation: impl FnOnce(StatusCode) -> Response,
) -> Response {
    let applied = resolve_write_preference(headers, uid);
    let mut out = match applied {
        AppliedPreference::Representation => representation(repr_status),
        AppliedPreference::Identifier(uid) => {
            identifier_response(headers, identifier_status(minimal_status, repr_status), uid)
        }
        AppliedPreference::Minimal => empty(minimal_status),
    };
    set_preference_applied(&mut out, applied);
    out
}

/// Render a `return=identifier` response body: `{ "uid": "<uid>" }` in JSON, or
/// the `<uid>` element when XML is negotiated.
pub(crate) fn identifier_response(headers: &HeaderMap, status: StatusCode, uid: &str) -> Response {
    match resolve_accept(headers, CANONICAL, WireFormat::CanonicalJson) {
        Some(WireFormat::CanonicalXml) => {
            // The OAS defines the identifier body only for JSON; the spec is
            // silent on an XML shape, so we emit a minimal `<uid>` element as
            // the direct XML equivalent of the JSON `{uid}`.
            xml_body(status, format!("<uid>{}</uid>", xml_escape(uid)))
        }
        _ => json_response(status, &serde_json::json!({ "uid": uid })),
    }
}

/// Escape the XML text-content special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build the (path-absolute) `Location` URL for an EHR sub-resource.
pub(crate) fn location(base_path: &str, ehr_id: &str, segment: Option<&str>, uid: &str) -> String {
    match segment {
        Some(seg) => format!("{base_path}/ehr/{ehr_id}/{seg}/{uid}"),
        None => format!("{base_path}/ehr/{ehr_id}"),
    }
}

/// The `ETag` header value for a resource identifier: the weak form `W/"{uid}"`
/// (overview §"`ETag` and Last-Modified" — the `W/` weakness indicator is a MUST).
pub(crate) fn resource_etag(uid: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!("W/\"{uid}\"")).ok()
}

/// Set the weak `ETag` of a resource identifier on a response (overview
/// §"`ETag` and Last-Modified" — the value "is usually taken from e.g.
/// `VERSIONED_OBJECT.uid.value`, `VERSION.uid.value`, `EHR.ehr_id.value`").
/// The single place the `W/"…"` header is written.
pub(crate) fn set_etag(resp: &mut Response, uid: &str) {
    if let Some(etag) = resource_etag(uid) {
        resp.headers_mut().insert(header::ETAG, etag);
    }
}

/// Set the versioning headers on a response: the weak `ETag` and — when the
/// metadata carries a commit time — `Last-Modified`. No `Location`.
pub(crate) fn set_versioning_headers(resp: &mut Response, meta: &ResourceMeta) {
    set_etag(resp, &meta.uid);
    if let Some(at) = meta.last_modified
        && let Ok(lm) = HeaderValue::from_str(&http_date(at))
    {
        resp.headers_mut().insert(header::LAST_MODIFIED, lm);
    }
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditObject {
            ehr_id: Some(meta.ehr_id.clone()),
            uid: Some(meta.uid.clone()),
            result_count: None,
            domain: None,
            origins: meta.origins.clone(),
            origin_count: (!meta.origins.is_empty())
                .then(|| u64::try_from(meta.origins.len()).unwrap_or(u64::MAX)),
        });
}

/// Set the `Location` header for a newly created/updated resource
/// (overview §Location — creation/redirect only).
pub(crate) fn set_location(
    resp: &mut Response,
    base_path: &str,
    segment: Option<&str>,
    meta: &ResourceMeta,
) {
    if let Ok(loc) = HeaderValue::from_str(&location(base_path, &meta.ehr_id, segment, &meta.uid)) {
        resp.headers_mut().insert(header::LOCATION, loc);
    }
}

/// Set the full create/update response headers: the versioning headers plus
/// `Location`.
pub(crate) fn set_resource_headers(
    resp: &mut Response,
    base_path: &str,
    segment: Option<&str>,
    meta: &ResourceMeta,
) {
    set_versioning_headers(resp, meta);
    set_location(resp, base_path, segment, meta);
}

/// Render a create/update response honouring `Prefer` and setting the
/// versioning + `Location` headers.
pub(crate) fn write_rm<T>(
    headers: &HeaderMap,
    base_path: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    segment: Option<&str>,
    resp: &ServiceResponse,
    root_tag: &str,
) -> Response
where
    T: DeserializeOwned + ToXml,
{
    let mut out = write_negotiated(
        headers,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| respond_rm::<T>(headers, status, &resp.body, root_tag),
    );
    if let Some(meta) = &resp.meta {
        set_resource_headers(&mut out, base_path, segment, meta);
    }
    out
}

/// As [`write_rm`] but for a JSON-only payload (no canonical-XML shape, e.g. a
/// CONTRIBUTION wrapper).
pub(crate) fn write_json(
    headers: &HeaderMap,
    base_path: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    segment: Option<&str>,
    resp: &ServiceResponse,
) -> Response {
    let mut out = write_negotiated(
        headers,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| respond(headers, status, &resp.body),
    );
    if let Some(meta) = &resp.meta {
        set_resource_headers(&mut out, base_path, segment, meta);
    }
    out
}

/// Render a sub-collection write (the `ITEM_TAG` list of a target): the stored
/// collection on `Prefer: return=representation`, otherwise the empty
/// `minimal_status` body — and the applied preference declared either way.
///
/// The collection is not a `uid`-versioned resource of its own, so there is no
/// identifier to return: `return=identifier` cannot be honoured here and
/// resolves to the applied default `return=minimal` (§Representation details
/// negotiation: "If no `Prefer` header is provided, the default behavior is
/// assumed to be `return=minimal`"). No `ETag`/`Location` — the collection
/// write mints no new resource.
pub(crate) fn write_collection(
    headers: &HeaderMap,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    body: &serde_json::Value,
) -> Response {
    write_negotiated(headers, minimal_status, repr_status, None, |status| {
        respond(headers, status, body)
    })
}

/// Render a `200 OK` read of a single spec-typed RM object, additionally
/// setting the weak `ETag`/`Last-Modified`. No `Location` (overview §Location).
pub(crate) fn read_rm<T>(
    headers: &HeaderMap,
    base_path: &str,
    segment: Option<&str>,
    resp: &ServiceResponse,
    root_tag: &str,
) -> Response
where
    T: DeserializeOwned + ToXml,
{
    let _ = (base_path, segment);
    let mut out = respond_rm::<T>(headers, StatusCode::OK, &resp.body, root_tag);
    if let Some(meta) = &resp.meta {
        set_versioning_headers(&mut out, meta);
    }
    out
}

/// A `204 No Content` delete outcome carrying the deleted version's weak
/// `ETag`/`Last-Modified`. No `Location` (overview §Location).
pub(crate) fn deleted_with_headers(
    base_path: &str,
    segment: Option<&str>,
    resp: &ServiceResponse,
) -> Response {
    let _ = (base_path, segment);
    let mut out = empty(StatusCode::NO_CONTENT);
    if let Some(meta) = &resp.meta {
        set_versioning_headers(&mut out, meta);
    }
    out
}

/// Render an error response, additionally setting the latest-version `ETag` the
/// spec requires on a `409`/`412`. No `Location` on the error path.
pub(crate) fn error_with_meta(
    error: ApiError,
    base_path: &str,
    segment: Option<&str>,
    meta: Option<&ResourceMeta>,
) -> Response {
    let _ = (base_path, segment);
    let mut out = crate::overview::error::RestError(error).into_response();
    if let Some(meta) = meta {
        set_versioning_headers(&mut out, meta);
    }
    out
}

/// Serve pre-formed canonical JSON verbatim as `application/json` — the
/// stored-body passthrough (the text is the database's own jsonb rendering of
/// the canonical body, uid-stamped at commit; no parse → serialize round
/// trip).
pub(crate) fn raw_json_body(status: StatusCode, json: String) -> Response {
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_JSON),
    );
    resp
}

/// Serve a pre-formed XML document verbatim as `application/xml`.
///
/// The lineage-agnostic builder: the OPT 1.4 template representation (always
/// v1) and the `return=identifier` `<uid>` body use it. A canonical RM
/// document goes through [`xml_body_ns`], which labels the lineage it carries.
pub(crate) fn xml_body(status: StatusCode, xml: String) -> Response {
    let mut resp = (status, xml).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_XML),
    );
    resp
}

/// Serve a canonical-XML RM document, labelling the ITS-XML lineage it carries.
///
/// The default (v2) response `Content-Type` is exactly `application/xml`, and
/// only the negotiated non-default v1 response adds the `version=1` parameter,
/// so the client that asked for the non-default lineage is told it got it
/// (owner ruling 2026-08-03, #1666: v2 is the served default — the only
/// published lineage whose schemas model the RM 1.2.0 this server serves). Either way the media
/// type itself is `application/xml`, which is what `Resources.md` §XML Format
/// makes a MUST ("Proper header `Content-Type: application/xml` MUST be
/// present in the response of the service unless the response has no content
/// body").
fn xml_body_ns(status: StatusCode, xml: String, ns: Namespace) -> Response {
    let mut resp = (status, xml).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(match ns {
            Namespace::V1 => APPLICATION_XML_V1,
            Namespace::V2 => APPLICATION_XML,
        }),
    );
    resp
}

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response {
    // The native codec serializes canonical JSON infallibly.
    let json = openehr_its::json::to_canonical_json(value);
    let mut resp = (status, json).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(APPLICATION_JSON),
    );
    resp
}

/// Small helper so error rendering here reuses the crate's `RestError` body.
trait IntoErrorResponse {
    fn into_response_body(self) -> Response;
}

impl IntoErrorResponse for ApiError {
    fn into_response_body(self) -> Response {
        crate::overview::error::RestError(self).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The prior `accept_selection` assertions, re-expressed against the
    /// [`resolve_accept`] core (RFC 9110 §12.5.1): the same canonical
    /// json-preferred-over-xml behaviour, plus `text/xml` → XML.
    #[test]
    fn accept_selection() {
        let json = Some(WireFormat::CanonicalJson);
        let xml = Some(WireFormat::CanonicalXml);
        assert_eq!(
            resolve_accept(&HeaderMap::new(), CANONICAL, WireFormat::CanonicalJson),
            json
        );
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "*/*")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            json
        );
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/xml")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            xml
        );
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/json, application/xml")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            json
        );
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "text/xml")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            xml
        );
    }

    /// q-values pick the highest-weight acceptable format even when it is not
    /// the server default (RFC 9110 §12.5.1).
    #[test]
    fn accept_qvalue_prefers_highest_weight() {
        // XML at q=1 beats JSON at q=0.5.
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/json;q=0.5, application/xml;q=1.0")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            Some(WireFormat::CanonicalXml)
        );
        // A specific type beats a `*/*` wildcard at equal q.
        let allowed = &[
            WireFormat::CanonicalJson,
            WireFormat::CanonicalXml,
            WireFormat::Flat,
            WireFormat::Structured,
        ];
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/openehr.wt.flat+json, */*")]),
                allowed,
                WireFormat::CanonicalJson
            ),
            Some(WireFormat::Flat)
        );
        // `;q=0` explicitly rejects a format.
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/json;q=0, application/xml")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            Some(WireFormat::CanonicalXml)
        );
    }

    /// The Simplified data-instance types resolve only where the endpoint
    /// allows them; on a canonical-only endpoint they are unacceptable (`406`).
    #[test]
    fn accept_simplified_only_where_allowed() {
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/openehr.wt.flat+json")]),
                CANONICAL,
                WireFormat::CanonicalJson
            ),
            None
        );
        let with_flat = &[WireFormat::CanonicalJson, WireFormat::Flat];
        assert_eq!(
            resolve_accept(
                &headers(&[("accept", "application/openehr.wt.flat+json")]),
                with_flat,
                WireFormat::CanonicalJson
            ),
            Some(WireFormat::Flat)
        );
    }

    /// The deprecated `…schema+json` and legacy `…nc.flat+json` types are not
    /// recognized (Resources.md §Simplified Formats NOTE + §Alternative data
    /// formats): unacceptable on `Accept`, unsupported on `Content-Type`.
    #[test]
    fn deprecated_and_legacy_types_unrecognized() {
        let all = &[
            WireFormat::CanonicalJson,
            WireFormat::CanonicalXml,
            WireFormat::Flat,
            WireFormat::Structured,
            WireFormat::WebTemplate,
        ];
        for t in [
            "application/openehr.wt.flat.schema+json",
            "application/openehr.wt.structured.schema+json",
            "application/openehr.nc.flat+json",
            "application/openehr.tds2+xml",
        ] {
            assert_eq!(
                resolve_accept(&headers(&[("accept", t)]), all, WireFormat::CanonicalJson),
                None,
                "{t} must not be an acceptable Accept"
            );
            assert_eq!(
                content_type_format(&headers(&[("content-type", t)])),
                None,
                "{t} must not be a recognized Content-Type"
            );
        }
    }

    #[test]
    fn content_type_selection() {
        assert_eq!(
            content_type_format(&HeaderMap::new()),
            Some(WireFormat::CanonicalJson)
        );
        assert_eq!(
            content_type_format(&headers(&[(
                "content-type",
                "application/xml; charset=utf-8"
            )])),
            Some(WireFormat::CanonicalXml)
        );
        assert_eq!(
            content_type_format(&headers(&[("content-type", "application/json")])),
            Some(WireFormat::CanonicalJson)
        );
        assert_eq!(
            content_type_format(&headers(&[(
                "content-type",
                "application/openehr.wt.flat+json"
            )])),
            Some(WireFormat::Flat)
        );
    }

    #[test]
    fn json_body_decodes() {
        let h = headers(&[("content-type", "application/json")]);
        let v = json_value(&h, &Bytes::from_static(br#"{"a":1}"#)).expect("json");
        assert_eq!(v, serde_json::json!({"a": 1}));
    }

    #[test]
    fn xml_content_type_rejected_for_json_only_op() {
        let h = headers(&[("content-type", "application/xml")]);
        let err = json_value(&h, &Bytes::from_static(b"<x/>")).expect_err("reject");
        assert!(
            matches!(err, ApiError::UnsupportedMediaType(_)),
            "got {err:?}"
        );
    }

    /// `Resources.md` §XML Format: a request payload the service cannot
    /// process as XML MUST be answered `415`; the header itself is a client
    /// MAY, so an absent `Content-Type` is not a refusal.
    #[test]
    fn require_content_type_refuses_only_a_declared_foreign_type() {
        let xml_only = &[WireFormat::CanonicalXml];
        assert!(
            require_content_type(&HeaderMap::new(), xml_only, "application/xml").is_ok(),
            "an absent Content-Type declares nothing to refuse (Resources.md \
             §XML Format: a client MAY use the header)"
        );
        for accepted in [
            "application/xml",
            "text/xml",
            "application/xml; charset=utf-8",
        ] {
            assert!(
                require_content_type(
                    &headers(&[("content-type", accepted)]),
                    xml_only,
                    "application/xml"
                )
                .is_ok(),
                "{accepted} is the XML payload format"
            );
        }
        for refused in [
            "application/json",
            "text/plain",
            "application/openehr.wt+json",
        ] {
            let err = require_content_type(
                &headers(&[("content-type", refused)]),
                xml_only,
                "application/xml",
            )
            .expect_err("refused");
            assert!(
                matches!(err, ApiError::UnsupportedMediaType(_)),
                "Resources.md §XML Format: a payload the service cannot process as \
                 XML MUST be 415, got {err:?} for {refused}"
            );
        }
    }

    /// The `text/plain` mirror of [`require_content_type`] for the ADL2
    /// upload: absent header accepted (client MAY), parameters ignored,
    /// any other declared type → `415` (Resources.md §format rules).
    #[test]
    fn require_text_plain_refuses_only_a_declared_foreign_type() {
        assert!(
            require_text_plain(&HeaderMap::new()).is_ok(),
            "an absent Content-Type declares nothing to refuse"
        );
        for accepted in ["text/plain", "text/plain; charset=utf-8", "Text/Plain"] {
            assert!(
                require_text_plain(&headers(&[("content-type", accepted)])).is_ok(),
                "{accepted} declares the operation's single body type"
            );
        }
        for refused in ["application/xml", "application/json", "text/html"] {
            let err =
                require_text_plain(&headers(&[("content-type", refused)])).expect_err("refused");
            assert!(
                matches!(err, ApiError::UnsupportedMediaType(_)),
                "a payload the service cannot process as text/plain MUST be \
                 415, got {err:?} for {refused}"
            );
        }
    }

    #[test]
    fn rm_value_rejects_simplified_content_type() {
        use openehr_rm::prelude::DvText;
        let h = headers(&[("content-type", "application/openehr.wt.flat+json")]);
        let err = rm_value::<DvText>(&h, &Bytes::from_static(b"{}")).expect_err("reject");
        assert!(
            matches!(err, ApiError::UnsupportedMediaType(_)),
            "a simplified Content-Type on a canonical RM op is 415: {err:?}"
        );
    }

    #[test]
    fn rm_body_decodes_from_both_json_and_xml() {
        use openehr_rm::prelude::DvText;

        let dv: DvText = openehr_its::json::from_canonical_value(
            &serde_json::json!({"_type": "DV_TEXT", "value": "hello"}),
        )
        .expect("dv_text");

        let xml = openehr_its::xml::to_canonical_xml(&dv, "value").expect("to xml");
        let mut xml_headers = HeaderMap::new();
        xml_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/xml"),
        );
        let from_xml = rm_value::<DvText>(&xml_headers, &Bytes::from(xml)).expect("xml decode");
        assert_eq!(
            ferroehr::service::version_update::text_value(&from_xml),
            "hello"
        );

        let json = openehr_its::json::to_canonical_json(&dv).into_bytes();
        let mut json_headers = HeaderMap::new();
        json_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let from_json = rm_value::<DvText>(&json_headers, &Bytes::from(json)).expect("json decode");
        assert_eq!(
            ferroehr::service::version_update::text_value(&from_json),
            "hello"
        );
    }

    /// The OPT 1.4 upload reads its XML body verbatim; the retired
    /// `lenient_value` JSON-or-string wrapper (which let a JSON-declared
    /// payload reach the parser and fail `400` instead of `415`) is gone.
    #[test]
    fn text_body_reads_an_xml_payload_verbatim() {
        assert_eq!(
            text_body(&Bytes::from_static(b"<template/>")).unwrap(),
            "<template/>"
        );
    }

    fn content_type(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn respond_rm_serves_json_by_default() {
        use openehr_rm::prelude::DvText;
        let value = serde_json::json!({"_type": "DV_TEXT", "value": "hello"});
        let resp = respond_rm::<DvText>(&HeaderMap::new(), StatusCode::OK, &value, "value");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp).as_deref(), Some(APPLICATION_JSON));
    }

    #[test]
    fn respond_rm_rejects_simplified_accept_with_406() {
        use openehr_rm::prelude::DvText;
        let h = headers(&[("accept", "application/openehr.wt.flat+json")]);
        let value = serde_json::json!({"_type": "DV_TEXT", "value": "hello"});
        let resp = respond_rm::<DvText>(&h, StatusCode::OK, &value, "value");
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[test]
    fn respond_rm_null_is_bodyless() {
        use openehr_rm::prelude::DvText;
        let h = headers(&[("accept", "application/xml")]);
        let resp = respond_rm::<DvText>(
            &h,
            StatusCode::NO_CONTENT,
            &serde_json::Value::Null,
            "value",
        );
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_ne!(content_type(&resp).as_deref(), Some(APPLICATION_XML));
    }

    #[tokio::test]
    async fn respond_rm_renders_canonical_xml_for_xml_accept() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::DvText;

        let value = serde_json::json!({"_type": "DV_TEXT", "value": "hello"});
        let h = headers(&[("accept", "application/xml")]);
        let resp = respond_rm::<DvText>(&h, StatusCode::OK, &value, "value");

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp).as_deref(), Some(APPLICATION_XML));

        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let xml = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(xml.contains("<value"), "root element present: {xml}");
        assert!(xml.contains("hello"), "leaf value present: {xml}");
        assert!(!xml.contains("_type"), "not a serialized JSON blob: {xml}");
    }

    // ── the `version` media-type parameter (our own extension) ────────────

    const V1_NS: &str = "xmlns=\"http://schemas.openehr.org/v1\"";
    const V2_NS: &str = "xmlns=\"http://schemas.openehr.org/v2\"";

    /// The parameter grammar: absent → default, `1`/`2` → that lineage
    /// (quoted or bare, name case-insensitive per RFC 9110 §8.3.1), anything
    /// else → unrecognized. Reading is done off the winning XML media range,
    /// so a q-value contest picks the parameter with the range.
    #[test]
    fn xml_lineage_reads_the_winning_accept_range() {
        let ns = |accept: &str| accept_xml_namespace(&headers(&[("accept", accept)]));

        assert_eq!(
            accept_xml_namespace(&HeaderMap::new()),
            Some(Namespace::V2),
            "no Accept at all is the v2 default (#1666)"
        );
        assert_eq!(ns("application/xml"), Some(Namespace::V2));
        assert_eq!(ns("application/xml; version=1"), Some(Namespace::V1));
        assert_eq!(ns("application/xml; version=2"), Some(Namespace::V2));
        assert_eq!(ns("application/xml;version=2"), Some(Namespace::V2));
        assert_eq!(ns("application/xml; Version=\"2\""), Some(Namespace::V2));
        assert_eq!(ns("text/xml; version=2"), Some(Namespace::V2));
        assert_eq!(
            ns("*/*"),
            Some(Namespace::V2),
            "a wildcard names no lineage, so the default stands"
        );
        assert_eq!(
            ns("application/json"),
            Some(Namespace::V2),
            "a JSON-only Accept leaves the XML default in place"
        );
        // The highest-q XML range carries the governing parameter.
        assert_eq!(
            ns("application/xml;q=0.5, application/xml;version=2;q=0.9"),
            Some(Namespace::V2)
        );
        assert_eq!(
            ns("application/xml;version=1;q=0.2, application/xml;q=0.9"),
            Some(Namespace::V2)
        );
        // …and a q=0 rejection does not get to choose the lineage.
        assert_eq!(
            ns("application/xml;version=1;q=0, application/xml"),
            Some(Namespace::V2)
        );
        for unknown in [
            "application/xml; version=3",
            "application/xml; version=0",
            "application/xml; version=v2",
            "application/xml; version=",
        ] {
            assert_eq!(
                ns(unknown),
                None,
                "{unknown} names a lineage this server does not serve"
            );
        }
    }

    /// A bare `application/xml` serves the v2 default (owner ruling
    /// 2026-08-03, #1666); `Accept: application/xml; version=1` selects the
    /// non-default v1 lineage and labels it.
    #[tokio::test]
    async fn respond_rm_serves_the_negotiated_xml_lineage() {
        async fn body_of(accept: &str) -> (Option<String>, StatusCode, String) {
            use http_body_util::BodyExt;
            use openehr_rm::prelude::DvText;

            let value = serde_json::json!({"_type": "DV_TEXT", "value": "hello"});
            let resp = respond_rm::<DvText>(
                &headers(&[("accept", accept)]),
                StatusCode::OK,
                &value,
                "value",
            );
            let ct = content_type(&resp);
            let status = resp.status();
            let bytes = resp.into_body().collect().await.expect("body").to_bytes();
            (ct, status, String::from_utf8(bytes.to_vec()).expect("utf8"))
        }

        let (ct, status, xml) = body_of("application/xml").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            ct.as_deref(),
            Some(APPLICATION_XML),
            "the default response Content-Type carries no version parameter"
        );
        assert!(xml.contains(V2_NS), "default lineage is v2 (#1666): {xml}");
        assert!(!xml.contains(V1_NS), "v1 namespace not declared: {xml}");

        let (ct, status, xml) = body_of("application/xml; version=1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            ct.as_deref(),
            Some(APPLICATION_XML_V1),
            "a v1 response names the non-default lineage it carries"
        );
        assert!(xml.contains(V1_NS), "negotiated lineage is v1: {xml}");

        let (ct, status, xml) = body_of("application/xml; version=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some(APPLICATION_XML));
        assert!(xml.contains(V2_NS), "explicit v2 is the default: {xml}");
    }

    /// An `Accept` naming a lineage this server does not serve is an aspect of
    /// the request it cannot fulfill → `406` (`Resources.md` §XML Format).
    #[test]
    fn respond_rm_refuses_an_unknown_xml_lineage_with_406() {
        use openehr_rm::prelude::DvText;
        let value = serde_json::json!({"_type": "DV_TEXT", "value": "hello"});
        let resp = respond_rm::<DvText>(
            &headers(&[("accept", "application/xml; version=3")]),
            StatusCode::OK,
            &value,
            "value",
        );
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
    }

    /// A request payload declared in either lineage is read (the reader is
    /// namespace-agnostic); an unrecognized `version` is a payload the service
    /// cannot process as XML → `415` (`Resources.md` §XML Format).
    #[test]
    fn rm_value_accepts_both_lineages_and_refuses_an_unknown_one() {
        use openehr_rm::prelude::DvText;

        let dv: DvText = openehr_its::json::from_canonical_value(
            &serde_json::json!({"_type": "DV_TEXT", "value": "hello"}),
        )
        .expect("dv_text");

        for (declared, ns) in [
            ("application/xml", Namespace::V1),
            ("application/xml; version=1", Namespace::V1),
            ("application/xml; version=2", Namespace::V2),
        ] {
            let xml = openehr_its::xml::to_canonical_xml_ns(&dv, "value", ns).expect("to xml");
            let decoded = rm_value::<DvText>(
                &headers(&[("content-type", declared)]),
                &Bytes::from(xml.clone()),
            )
            .unwrap_or_else(|e| panic!("{declared} must decode: {e:?} ({xml})"));
            assert_eq!(
                ferroehr::service::version_update::text_value(&decoded),
                "hello"
            );
        }

        let xml = openehr_its::xml::to_canonical_xml(&dv, "value").expect("to xml");
        let err = rm_value::<DvText>(
            &headers(&[("content-type", "application/xml; version=3")]),
            &Bytes::from(xml),
        )
        .expect_err("an unserved lineage is refused");
        assert!(
            matches!(err, ApiError::UnsupportedMediaType(_)),
            "an XML payload in a lineage the service cannot process is 415: {err:?}"
        );
    }

    /// The guard is scoped to canonical XML: a `version` parameter on any
    /// other declared media type is not this parameter and is left alone.
    #[test]
    fn xml_lineage_guard_ignores_non_xml_content_types() {
        for declared in [
            "application/json; version=3",
            "application/openehr.wt.flat+json; version=3",
            "text/plain; version=3",
        ] {
            assert!(
                require_known_xml_lineage(&headers(&[("content-type", declared)])).is_ok(),
                "{declared} is not canonical XML — the lineage guard must not fire"
            );
        }
        assert!(
            require_known_xml_lineage(&HeaderMap::new()).is_ok(),
            "an absent Content-Type declares no lineage"
        );
    }

    /// An `ORIGINAL_VERSION` canonical-JSON envelope, as the version reads
    /// serve it (`data` is a `DV_TEXT` here — `ORIGINAL_VERSION.data` is
    /// `xs:anyType` in the XSD, so the payload type is unconstrained).
    fn original_version_envelope() -> serde_json::Value {
        serde_json::json!({
            "_type": "ORIGINAL_VERSION",
            "contribution": {
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "CONTRIBUTION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "c1" }
            },
            "commit_audit": {
                "_type": "AUDIT_DETAILS",
                "system_id": "ferroehr",
                "time_committed": { "_type": "DV_DATE_TIME", "value": "2024-01-01T00:00:00Z" },
                "change_type": {
                    "_type": "DV_CODED_TEXT", "value": "creation",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249" }
                },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "clinician" }
            },
            "signature": "-----BEGIN PGP SIGNATURE-----\nDEADBEEF\n-----END PGP SIGNATURE-----",
            "uid": { "_type": "OBJECT_VERSION_ID", "value": "v1::openEHRSys::1" },
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" }
            },
            "data": { "_type": "DV_TEXT", "value": "hello" }
        })
    }

    /// The published document element for a `VERSION` resource is
    /// `<xs:element name="version" type="VERSION"/>` over the ABSTRACT
    /// `<xs:complexType name="VERSION" abstract="true">`
    /// (`crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/ALL/Version.xsd`;
    /// the 2.0.0 lineage repeats it in `RM/latest/documents/Version.xsd` +
    /// `RM/latest/Common.xsd`), so a served `ORIGINAL_VERSION` is that root
    /// PLUS the `xsi:type` XML Schema Part 1 §2.6.1/§3.4.6 requires of an
    /// instance of an abstract type
    /// (<https://www.w3.org/TR/xmlschema-1/#xsi_type>). ITS-REST overview
    /// `Resources.md` §"XML Format" is what binds the schemas to the wire:
    /// "both request payloads and responses MUST conform to the [published
    /// XSDs]". `original_version` is a document element NEITHER published
    /// lineage declares.
    #[tokio::test]
    async fn respond_rm_renders_original_version_under_the_published_version_root() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::{DvText, Version};

        let h = headers(&[("accept", "application/xml")]);
        let resp = respond_rm::<Version<DvText>>(
            &h,
            StatusCode::OK,
            &original_version_envelope(),
            "version",
        );

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp).as_deref(), Some(APPLICATION_XML));
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let xml = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(
            xml.starts_with("<version "),
            "the published document element is the root: {xml}"
        );
        assert!(
            xml.contains(r#"xsi:type="ORIGINAL_VERSION""#),
            "an instance of the abstract VERSION type names its concrete class: {xml}"
        );
        assert!(
            !xml.contains("<original_version"),
            "no undeclared per-subtype root: {xml}"
        );
        assert!(
            xml.contains("<signature"),
            "signature element present: {xml}"
        );
        assert!(xml.contains("DEADBEEF"), "signature value present: {xml}");
        assert!(!xml.contains("\"_type\""), "not a JSON blob: {xml}");
    }

    /// The `IMPORTED_VERSION` twin of the row above: the same published
    /// `<version>` root, discriminated by `xsi:type="IMPORTED_VERSION"` —
    /// `ALL/Version.xsd` derives BOTH concrete classes from the abstract
    /// `VERSION` and declares no element for either, so the subtype reaches
    /// the wire through the attribute (RM common master06 §Version and its
    /// Subtypes).
    #[tokio::test]
    async fn respond_rm_renders_imported_version_under_the_published_version_root() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::{DvText, Version};

        let value = serde_json::json!({
            "_type": "IMPORTED_VERSION",
            "contribution": {
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "CONTRIBUTION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "c2" }
            },
            "commit_audit": {
                "_type": "AUDIT_DETAILS",
                "system_id": "ferroehr",
                "time_committed": { "_type": "DV_DATE_TIME", "value": "2024-02-02T00:00:00Z" },
                "change_type": {
                    "_type": "DV_CODED_TEXT", "value": "creation",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249" }
                },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "importer" }
            },
            "item": original_version_envelope()
        });
        let h = headers(&[("accept", "application/xml")]);
        let resp = respond_rm::<Version<DvText>>(&h, StatusCode::OK, &value, "version");

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp).as_deref(), Some(APPLICATION_XML));
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let xml = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(
            xml.starts_with("<version "),
            "the published document element is the root: {xml}"
        );
        assert!(
            xml.contains(r#"xsi:type="IMPORTED_VERSION""#),
            "the wrapper names its own concrete class: {xml}"
        );
        assert!(
            !xml.contains("<imported_version"),
            "no undeclared per-subtype root: {xml}"
        );
        assert!(
            xml.contains("<item"),
            "IMPORTED_VERSION extends VERSION with `item` (ALL/Version.xsd): {xml}"
        );
    }

    /// The abstract-root table IS the published-schema fact: the only two
    /// document elements either ITS-XML lineage declares over an abstract type
    /// are `version` (type `VERSION`) and `items` (type `LOCATABLE`) — both
    /// `abstract="true"` in `ALL/Version.xsd` / `ALL/Structure.xsd` (nsv1) and
    /// `RM/latest/Common.xsd` (nsv2). A concretely-typed element
    /// (`composition`, `template`, …) takes no `xsi:type`, and a root name the
    /// schemas publish no element for is absent by construction.
    #[test]
    fn declared_root_type_names_only_the_published_abstract_elements() {
        assert_eq!(declared_root_type("version"), Some("VERSION"));
        assert_eq!(declared_root_type("items"), Some("LOCATABLE"));
        for concrete in ["composition", "template", "archetype", "versioned_object"] {
            assert_eq!(
                declared_root_type(concrete),
                None,
                "{concrete} is declared with a concrete type — no xsi:type at the root"
            );
        }
        for undeclared in [
            "original_version",
            "imported_version",
            "ehr_status",
            "folder",
        ] {
            assert_eq!(
                declared_root_type(undeclared),
                None,
                "{undeclared} is not a published document element at all"
            );
        }
    }

    #[tokio::test]
    async fn respond_rm_renders_versioned_object_xml() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::VersionedComposition;

        let value = serde_json::json!({
            "_type": "VERSIONED_COMPOSITION",
            "uid": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" },
            "owner_id": {
                "_type": "OBJECT_REF", "namespace": "local", "type": "EHR",
                "id": { "_type": "HIER_OBJECT_ID", "value": "e1" }
            },
            "time_created": { "_type": "DV_DATE_TIME", "value": "2024-01-01T00:00:00Z" }
        });
        let h = headers(&[("accept", "application/xml")]);
        let resp =
            respond_rm::<VersionedComposition>(&h, StatusCode::OK, &value, "versioned_composition");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp).as_deref(), Some(APPLICATION_XML));
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let xml = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(
            xml.contains("<versioned_composition"),
            "root element: {xml}"
        );
        assert!(
            xml.contains("8849182c-82ad-4088-a07f-48ead4180515"),
            "uid present: {xml}"
        );
    }

    // ── header + `Prefer` handling ──────────────────────────────────────────

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    fn etag(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    fn loc(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    fn preference_applied(resp: &Response) -> Option<String> {
        resp.headers()
            .get("preference-applied")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn prefer_default_is_minimal() {
        assert!(!prefers_representation(&HeaderMap::new()));
        assert!(!prefers_representation(&headers(&[(
            "prefer",
            "return=minimal"
        )])));
        assert!(prefers_representation(&headers(&[(
            "prefer",
            "return=representation"
        )])));
        assert!(prefers_representation(&headers(&[(
            "prefer",
            "RETURN=REPRESENTATION"
        )])));
    }

    #[test]
    fn location_builds_per_segment() {
        assert_eq!(location(BASE, "e1", None, "e1"), format!("{BASE}/ehr/e1"));
        assert_eq!(
            location(BASE, "e1", Some("composition"), "v::s::1"),
            format!("{BASE}/ehr/e1/composition/v::s::1")
        );
    }

    fn meta(ehr: &str, uid: &str) -> ResourceMeta {
        ResourceMeta::new(ehr.to_owned(), uid.to_owned())
    }

    #[test]
    fn write_rm_minimal_is_headers_only() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::1"));
        let out = write_rm::<Composition>(
            &HeaderMap::new(),
            BASE,
            StatusCode::CREATED,
            StatusCode::CREATED,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::CREATED);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::1\""));
        assert_eq!(
            loc(&out).as_deref(),
            Some(&*format!("{BASE}/ehr/e1/composition/v::s::1"))
        );
        assert_eq!(content_type(&out), None);
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
    }

    #[test]
    fn write_rm_representation_returns_body() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::2"));
        let h = headers(&[("prefer", "return=representation")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(content_type(&out).as_deref(), Some(APPLICATION_JSON));
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::2\""));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=representation")
        );
    }

    #[test]
    fn deleted_with_headers_is_204_with_weak_etag_no_location() {
        let resp = ServiceResponse::deleted(meta("e1", "v::s::3"));
        let out = deleted_with_headers(BASE, Some("composition"), &resp);
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::3\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn error_with_meta_sets_latest_version_etag_only() {
        let out = error_with_meta(
            ApiError::PreconditionFailed("stale".to_owned()),
            BASE,
            Some("ehr_status"),
            Some(&meta("e1", "v::s::5")),
        );
        assert_eq!(out.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::5\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn read_rm_emits_weak_etag_and_no_location() {
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::7"));
        let out = read_rm::<Composition>(
            &HeaderMap::new(),
            BASE,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::7\""));
        assert!(loc(&out).is_none());
    }

    #[test]
    fn resource_etag_is_weak() {
        let v = resource_etag("8849182c::openEHRSys::2").expect("etag");
        assert_eq!(v.to_str().unwrap(), "W/\"8849182c::openEHRSys::2\"");
    }

    #[test]
    fn prefers_identifier_detects_return_identifier() {
        assert!(prefers_identifier(&headers(&[(
            "prefer",
            "return=identifier"
        )])));
        assert!(prefers_identifier(&headers(&[(
            "prefer",
            "RETURN=IDENTIFIER"
        )])));
        assert!(!prefers_identifier(&headers(&[(
            "prefer",
            "return=minimal"
        )])));
        assert!(!prefers_identifier(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn write_rm_identifier_returns_uid_body() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::9"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(content_type(&out).as_deref(), Some(APPLICATION_JSON));
        assert_eq!(etag(&out).as_deref(), Some("W/\"v::s::9\""));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "v::s::9" }));
    }

    /// `Requests_and_responses.md` §"Prefer only identifier": the identifier
    /// variant "implies minimal response semantics, but with a non-empty
    /// response body (i.e. the status will be `201 Created` or `200 OK`, never
    /// `204 No Content`)" — across every (minimal, representation) status pair
    /// a write route uses.
    #[test]
    fn identifier_status_is_never_204() {
        let pairs = [
            (StatusCode::NO_CONTENT, StatusCode::OK),
            (StatusCode::CREATED, StatusCode::CREATED),
            (StatusCode::OK, StatusCode::OK),
            (StatusCode::NO_CONTENT, StatusCode::CREATED),
        ];
        for (minimal, repr) in pairs {
            let got = identifier_status(minimal, repr);
            assert_ne!(
                got,
                StatusCode::NO_CONTENT,
                "return=identifier is `201 Created` or `200 OK`, never `204 No Content` \
                 (overview §Prefer only identifier); minimal={minimal}, repr={repr}"
            );
            assert!(
                got == StatusCode::OK || got == StatusCode::CREATED,
                "return=identifier status must be 200 or 201, got {got}"
            );
        }
    }

    /// The identifier branch is unreachable without an identifier to render,
    /// so an applied `return=identifier` can never degrade into the empty
    /// (possibly `204`) minimal body.
    #[test]
    fn resolve_write_preference_needs_a_uid_for_identifier() {
        let h = headers(&[("prefer", "return=identifier")]);
        assert_eq!(
            resolve_write_preference(&h, Some("v::s::1")),
            AppliedPreference::Identifier("v::s::1")
        );
        assert_eq!(
            resolve_write_preference(&h, None),
            AppliedPreference::Minimal,
            "an unhonourable return=identifier applies the default return=minimal \
             (overview §Representation details negotiation)"
        );
        assert_eq!(
            resolve_write_preference(&HeaderMap::new(), Some("v::s::1")),
            AppliedPreference::Minimal
        );
        assert_eq!(
            resolve_write_preference(
                &headers(&[("prefer", "return=representation")]),
                Some("v::s::1")
            ),
            AppliedPreference::Representation
        );
    }

    #[tokio::test]
    async fn write_rm_identifier_on_create_is_201_with_uid_body() {
        use http_body_util::BodyExt;
        use openehr_rm::prelude::Composition;
        let value = serde_json::json!({"_type": "COMPOSITION"});
        let resp = ServiceResponse::new(value, meta("e1", "v::s::1"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::CREATED,
            StatusCode::CREATED,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(out.status(), StatusCode::CREATED);
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "v::s::1" }));
    }

    /// A write that yields no resource metadata cannot produce the identifier
    /// body the preference is defined by, so the server applies AND declares
    /// the default `return=minimal` — never `Preference-Applied:
    /// return=identifier` over an empty body (overview §Representation details
    /// negotiation: the header indicates the preference that "has been
    /// honored").
    #[test]
    fn write_rm_identifier_without_meta_declares_minimal() {
        use openehr_rm::prelude::Composition;
        let resp = ServiceResponse::plain(serde_json::Value::Null);
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_rm::<Composition>(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("composition"),
            &resp,
            "composition",
        );
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=minimal"),
            "an unapplied preference is never claimed"
        );
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        assert!(etag(&out).is_none());
    }

    #[tokio::test]
    async fn write_json_identifier_is_never_204() {
        use http_body_util::BodyExt;
        let value = serde_json::json!({"_type": "CONTRIBUTION"});
        let resp = ServiceResponse::new(value, meta("e1", "c::s::1"));
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_json(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("contribution"),
            &resp,
        );
        assert_eq!(
            out.status(),
            StatusCode::OK,
            "the identifier variant is 200/201, never 204 \
             (overview §Prefer only identifier)"
        );
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        let bytes = out.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body, serde_json::json!({ "uid": "c::s::1" }));
    }

    #[test]
    fn write_json_identifier_without_meta_declares_minimal() {
        let resp = ServiceResponse::plain(serde_json::Value::Null);
        let h = headers(&[("prefer", "return=identifier")]);
        let out = write_json(
            &h,
            BASE,
            StatusCode::NO_CONTENT,
            StatusCode::OK,
            Some("contribution"),
            &resp,
        );
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
    }

    /// `Preference-Applied` on the identifier path is the same header the
    /// shared setter writes for every other write route.
    #[test]
    fn set_preference_applied_renders_each_token() {
        let mut out = empty(StatusCode::OK);
        set_preference_applied(&mut out, AppliedPreference::Minimal);
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
        set_preference_applied(&mut out, AppliedPreference::Identifier("v::s::1"));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        set_preference_applied(&mut out, AppliedPreference::Representation);
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=representation")
        );
    }

    #[test]
    fn http_date_is_imf_fixdate() {
        let ts: jiff::Timestamp = "2009-07-22T19:15:56Z".parse().unwrap();
        assert_eq!(http_date(ts), "Wed, 22 Jul 2009 19:15:56 GMT");
    }

    #[test]
    fn set_resource_headers_emits_last_modified() {
        let ts: jiff::Timestamp = "2024-03-04T05:06:07Z".parse().unwrap();
        let m = ResourceMeta::new("e1".to_owned(), "v::s::1".to_owned()).with_last_modified(ts);
        let mut resp = empty(StatusCode::OK);
        set_resource_headers(&mut resp, BASE, Some("composition"), &m);
        let lm = resp
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok());
        assert_eq!(lm, Some("Mon, 04 Mar 2024 05:06:07 GMT"));
    }

    #[test]
    fn set_resource_headers_omits_last_modified_when_absent() {
        let m = ResourceMeta::new("e1".to_owned(), "v::s::1".to_owned());
        let mut resp = empty(StatusCode::OK);
        set_resource_headers(&mut resp, BASE, Some("composition"), &m);
        assert!(resp.headers().get(header::LAST_MODIFIED).is_none());
    }
}
