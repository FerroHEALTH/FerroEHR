// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Request body decoders: the `Content-Type` gate and the typed reads of a
//! JSON, canonical-RM or plain-text body (ITS-REST `Resources.md` §Data
//! representation).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — one byte-to-JSON \
              step per route, consumed by the typed decode"
)]

use bytes::Bytes;
use http::{HeaderMap, header};
use openehr_its::json::JsonParseError;
use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::FromXml;
use serde::de::DeserializeOwned;

use super::{
    WireFormat, classify_media, content_type_format, header_str, media_token,
    require_known_xml_lineage,
};

/// Decode a required JSON body into a `serde_json::Value`.
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not JSON;
/// [`ApiError::BadRequest`] if the bytes are not valid JSON.
pub(crate) fn json_value(headers: &HeaderMap, body: &Bytes) -> Result<serde_json::Value, ApiError> {
    require_json(headers)?;
    parse_json(body)
}

/// Decode a required JSON body into the typed DTO `T`, strictly, with a
/// refusal that names the offending member by its JSON path.
///
/// The generated DTOs carry `#[serde(deny_unknown_fields)]` wherever the OAS
/// schema declares `additionalProperties: false`, so a member the schema does
/// not define is REFUSED here rather than silently dropped, and a member of the
/// wrong JSON type is refused rather than silently read as absent. The refusal
/// names the member by its JSON PATH (`[0].value`): `serde_json` alone reports
/// only a line/column, which tells a client nothing about which member it must
/// fix. `what` names the body in that refusal (`JSON body`, `JSON array body`).
///
/// The `ITEM_TAG` PUT (`schemas/common/UpdateItemTag.yaml`: required `key`,
/// optional `value`/`target_path`, `additionalProperties: false`) decodes a
/// `Vec` of its DTO here. The ITS-REST docs text is silent on that body's
/// member set, and where the docs text is silent the released OAS grounds the
/// expectation: the release presents its OAS files as its computable
/// specification artifacts (ITS-REST overview `Specifications.md`:
/// "Specifications can be downloaded as YAML files in OpenAPI Specification
/// 3.0 format").
///
/// # Errors
/// [`ApiError::UnsupportedMediaType`] if the `Content-Type` is not JSON;
/// [`ApiError::BadRequest`] if the bytes do not decode as `T`.
pub(crate) fn typed_json<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
    what: &str,
) -> Result<T, ApiError> {
    require_json(headers)?;
    let mut de = serde_json::Deserializer::from_slice(body);
    serde_path_to_error::deserialize::<_, T>(&mut de).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        if path == "." {
            ApiError::BadRequest(format!("invalid {what}: {inner}"))
        } else {
            ApiError::BadRequest(format!("invalid {what} at {path}: {inner}"))
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
/// list and a malformed identifier are all refused there, path-named. A
/// refused body the published ITS-JSON schema refuses too could not be parsed
/// (**400**, ITS-REST `responses/400.yaml`); one the schema admits parses and
/// breaks an RM rule, such as a mandatory member ITS-JSON leaves optional
/// (**422**, `responses/422.yaml`, [`ferroehr::error::its_json_admits`]).
///
/// # Errors
/// [`ApiError::BadRequest`] if the body cannot be parsed in the declared
/// format; [`ApiError::Unprocessable`] for canonical JSON the ITS-JSON schema
/// admits and the RM refuses; [`ApiError::UnsupportedMediaType`] for any `Content-Type` other than
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
                .map_err(|e| canonical_json_refusal(json, &e))
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

/// The refusal of a canonical-JSON body the strict typed reader could not
/// decode: `422` when it parses and the ITS-JSON schema admits it, else `400`.
pub(crate) fn canonical_json_refusal(json: &str, error: &JsonParseError) -> ApiError {
    let admitted = serde_json::from_str::<serde_json::Value>(json)
        .is_ok_and(|value| ferroehr::error::its_json_admits(&value));
    if admitted {
        ApiError::Unprocessable(format!("content does not satisfy the RM: {error}"))
    } else {
        ApiError::BadRequest(format!("invalid canonical JSON body: {error}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use openehr_its::xml::runtime::Namespace;

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

        let xml = openehr_its::xml::runtime::to_xml(&dv, "value", Namespace::V2).expect("to xml");
        let mut xml_headers = HeaderMap::new();
        xml_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/xml"),
        );
        let from_xml = rm_value::<DvText>(&xml_headers, &Bytes::from(xml)).expect("xml decode");
        assert_eq!(ferroehr::versioning::audit::text_value(&from_xml), "hello");

        let json = openehr_its::json::to_canonical_json(&dv).into_bytes();
        let mut json_headers = HeaderMap::new();
        json_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let from_json = rm_value::<DvText>(&json_headers, &Bytes::from(json)).expect("json decode");
        assert_eq!(ferroehr::versioning::audit::text_value(&from_json), "hello");
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
            let xml = openehr_its::xml::runtime::to_xml(&dv, "value", ns).expect("to xml");
            let decoded = rm_value::<DvText>(
                &headers(&[("content-type", declared)]),
                &Bytes::from(xml.clone()),
            )
            .unwrap_or_else(|e| panic!("{declared} must decode: {e:?} ({xml})"));
            assert_eq!(ferroehr::versioning::audit::text_value(&decoded), "hello");
        }

        let xml = openehr_its::xml::runtime::to_xml(&dv, "value", Namespace::V2).expect("to xml");
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
}
