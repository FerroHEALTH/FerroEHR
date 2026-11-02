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
//! The submodules hold the rest, one concern each: `decode` reads request
//! bodies, `body` renders response bodies, `prefer` reads `Prefer` and
//! declares `Preference-Applied`, and `resource` sets the versioning headers
//! of a read, write or delete.
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

use http::{HeaderMap, header};

use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::Namespace;

pub(crate) mod body;
pub(crate) mod decode;
pub(crate) mod prefer;
pub(crate) mod resource;

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

pub(crate) const APPLICATION_JSON: &str = "application/json";
pub(crate) const APPLICATION_XML: &str = "application/xml";
const TEXT_XML: &str = "text/xml";
/// Web Template document media type (`master02 §MIME Types`, template only).
pub(crate) const APPLICATION_WT_JSON: &str = "application/openehr.wt+json";
/// Simplified FLAT (simSDT) media type.
pub(crate) const APPLICATION_WT_FLAT_JSON: &str = "application/openehr.wt.flat+json";
/// Simplified STRUCTURED (structSDT) media type.
pub(crate) const APPLICATION_WT_STRUCTURED_JSON: &str = "application/openehr.wt.structured+json";

/// Line-delimited JSON, the media type our own long-running operational routes
/// stream their findings as. Not a [`WireFormat`] — see [`accepts_ndjson`].
pub(crate) const APPLICATION_NDJSON: &str = "application/x-ndjson";

/// The canonical-only allowed set (`Accept_LOCATABLE` minus the simplified
/// types) — the negotiation set for every canonical RM object endpoint.
pub(crate) const CANONICAL: &[WireFormat] = &[WireFormat::CanonicalJson, WireFormat::CanonicalXml];

// ── The negotiation core ─────────────────────────────────────────────────

/// The text of the header `name`, when present and valid header text.
fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

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
}
