// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Response bodies: the one builder of a pre-formed body under its media
//! type, and the JSON and canonical-RM renderers that honour `Accept`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — one byte-to-JSON \
              step per route, consumed by the typed decode"
)]

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use openehr_its::rest::runtime::ApiError;
use openehr_its::xml::runtime::{Namespace, ToXml};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{
    APPLICATION_JSON, APPLICATION_XML, APPLICATION_XML_V1, CANONICAL, WireFormat,
    accept_xml_namespace, resolve_accept,
};

/// A response whose body is the pre-formed `text` under the media type
/// `media`: the one place a response body and its `Content-Type` are paired.
pub(crate) fn body(status: StatusCode, media: &'static str, text: String) -> Response {
    let mut resp = (status, text).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(media));
    resp
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
            Ok(json) => body(status, APPLICATION_JSON, json),
            Err(e) => crate::overview::error::internal_fault("serialize the JSON response", &e)
                .into_response_body(),
        },
        None => ApiError::NotAcceptable(
            "this response is available as application/json only".to_owned(),
        )
        .into_response_body(),
    }
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
        Some(WireFormat::CanonicalJson) => body(
            status,
            APPLICATION_JSON,
            openehr_its::json::to_canonical_json(value),
        ),
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
            let serialized = match openehr_its::xml::declared_abstract_root_type(root_tag) {
                Some(declared) => {
                    openehr_its::xml::runtime::to_xml_declared(&typed, root_tag, declared, ns)
                }
                None => openehr_its::xml::runtime::to_xml(&typed, root_tag, ns),
            };
            match serialized {
                Ok(xml) => body(status, xml_media(ns), xml),
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

/// The media type of a canonical-XML RM document, labelling the ITS-XML
/// lineage it carries.
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
fn xml_media(ns: Namespace) -> &'static str {
    match ns {
        Namespace::V1 => APPLICATION_XML_V1,
        Namespace::V2 => APPLICATION_XML,
    }
}

/// Small helper so error rendering here reuses the crate's `RestError` body.
trait IntoErrorResponse {
    fn into_response_body(self) -> Response;
}

impl IntoErrorResponse for ApiError {
    fn into_response_body(self) -> Response {
        crate::overview::error::RestError::from(self).into_response()
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
        assert_eq!(
            openehr_its::xml::declared_abstract_root_type("version"),
            Some("VERSION")
        );
        assert_eq!(
            openehr_its::xml::declared_abstract_root_type("items"),
            Some("LOCATABLE")
        );
        for concrete in ["composition", "template", "archetype", "versioned_object"] {
            assert_eq!(
                openehr_its::xml::declared_abstract_root_type(concrete),
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
                openehr_its::xml::declared_abstract_root_type(undeclared),
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
}
