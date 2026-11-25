// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A PDF/A original stored as `DV_MULTIMEDIA` reads back over ITS-REST byte
//! for byte, with its `media_type`, `size` and `integrity_check` intact. The
//! draft implementing act on the European exchange format (Art. 3(1)(a),
//! Annex II) exchanges a document as PDF/A-1a or PDF/A-1b, which holds only
//! when the bytes served are the bytes committed. RM 1.2.0 `DV_MULTIMEDIA`: `data` holds the media inline,
//! `uri` references it, and `integrity_check` is its digest under
//! `integrity_check_algorithm`.

use axum::Router;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::{StatusCode, header};
use openehr_rm::prelude::{Composition, OriginalVersion};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common;
use crate::common::BASE;

/// A one-page PDF/A-1b document, which veraPDF passes for flavour 1b.
const PDF: &[u8] = include_bytes!("../fixtures/pdfa-1b-original.pdf");

/// The template whose ACTION holds an ELEMENT that admits `application/pdf`.
const TEMPLATE_ID: &str = "obs_act.en.v1";

/// Where the referenced original lives; FerroEHR stores the reference only.
const ORIGINAL_URI: &str = "https://documents.example.org/originals/pdfa-1b-original.pdf";

fn opt_xml() -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/fixtures/opt/valid/minimal_all_entries.opt"),
    )
    .expect("minimal_all_entries.opt in the corpus")
}

fn sha256() -> Vec<u8> {
    Sha256::digest(PDF).to_vec()
}

fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
        "code_string": code,
    })
}

/// The original with its media inline.
fn inline_pdf() -> Value {
    json!({
        "_type": "DV_MULTIMEDIA",
        "media_type": code_phrase("IANA_media-types", "application/pdf"),
        "data": STANDARD.encode(PDF),
        "size": PDF.len(),
        "integrity_check": STANDARD.encode(sha256()),
        "integrity_check_algorithm": code_phrase("openehr_integrity_check_algorithms", "SHA-256"),
    })
}

/// The original by reference, with the size and digest of the bytes it names.
fn referenced_pdf() -> Value {
    json!({
        "_type": "DV_MULTIMEDIA",
        "media_type": code_phrase("IANA_media-types", "application/pdf"),
        "uri": { "_type": "DV_URI", "value": ORIGINAL_URI },
        "size": PDF.len(),
        "integrity_check": STANDARD.encode(sha256()),
        "integrity_check_algorithm": code_phrase("openehr_integrity_check_algorithms", "SHA-256"),
    })
}

/// A router with the template uploaded, and a fresh EHR.
async fn app_with_template() -> (testkit::TestDb, Router, String) {
    let (pg, app) = common::app(common::api_config(false)).await;
    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(opt_xml())
        .build();
    let (status, _headers, body) = common::send(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let ehr_id = common::create_ehr(&app).await;
    (pg, app, ehr_id)
}

/// The template's complete example with every `DV_MULTIMEDIA` replaced by
/// `media`.
async fn composition_with(app: &Router, media: &Value) -> Value {
    let uri =
        format!("{BASE}/definition/template/adl1.4/{TEMPLATE_ID}/example?detail_level=complete");
    let (status, mut example) = common::send_json(app, common::req("GET", uri).build()).await;
    assert_eq!(status, StatusCode::OK, "example: {example}");
    assert!(
        replace_media(&mut example, media) > 0,
        "a DV_MULTIMEDIA in the example"
    );
    example
}

fn replace_media(node: &mut Value, media: &Value) -> usize {
    match node {
        Value::Object(map) if map.get("_type").and_then(Value::as_str) == Some("DV_MULTIMEDIA") => {
            *node = media.clone();
            1
        }
        Value::Object(map) => map.values_mut().map(|v| replace_media(v, media)).sum(),
        Value::Array(items) => items.iter_mut().map(|v| replace_media(v, media)).sum(),
        _ => 0,
    }
}

/// Commits `body` with `content_type` and returns the version uid.
async fn commit(app: &Router, ehr_id: &str, content_type: &str, body: String) -> String {
    let req = common::req("POST", format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .build();
    let (status, headers, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    common::etag_uid(&headers)
}

/// `GET` `uri` as `accept`; `read` names the read in a failure.
async fn get(app: &Router, uri: &str, accept: &str, read: &str) -> String {
    let (status, headers, body) =
        common::send(app, common::req("GET", uri).accept(accept).build()).await;
    assert_eq!(status, StatusCode::OK, "{read}: {body}");
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), accept, "{read}");
    body
}

/// The composition and its version, each read in canonical JSON and XML, as
/// canonical JSON values named by the read.
async fn reads(app: &Router, ehr_id: &str, version_uid: &str) -> Vec<(&'static str, Value)> {
    const JSON: &str = "application/json";
    const XML: &str = "application/xml";
    let vo = version_uid.split("::").next().unwrap();
    let composition = format!("{BASE}/ehr/{ehr_id}/composition/{version_uid}");
    let version = format!("{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{version_uid}");
    let composition_xml: Composition = openehr_its::xml::from_canonical_xml(
        &get(app, &composition, XML, "the composition as XML").await,
    )
    .expect("the composition XML decodes");
    let version_xml: OriginalVersion<Composition> =
        openehr_its::xml::from_canonical_xml(&get(app, &version, XML, "the version as XML").await)
            .expect("the version XML decodes");
    let composition_json = get(app, &composition, JSON, "the composition as JSON").await;
    let version_json = get(app, &version, JSON, "the version as JSON").await;
    vec![
        (
            "the composition as JSON",
            serde_json::from_str(&composition_json).unwrap(),
        ),
        (
            "the composition as XML",
            openehr_its::json::to_canonical_value(&composition_xml),
        ),
        (
            "the version as JSON",
            serde_json::from_str(&version_json).unwrap(),
        ),
        (
            "the version as XML",
            openehr_its::json::to_canonical_value(&version_xml),
        ),
    ]
}

fn media_of<'a>(node: &'a Value, found: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) if map.get("_type").and_then(Value::as_str) == Some("DV_MULTIMEDIA") => {
            found.push(node);
        }
        Value::Object(map) => map.values().for_each(|v| media_of(v, found)),
        Value::Array(items) => items.iter().for_each(|v| media_of(v, found)),
        _ => {}
    }
}

/// Every `DV_MULTIMEDIA` of one read.
fn all_media<'a>(value: &'a Value, read: &str) -> Vec<&'a Value> {
    let mut found = Vec::new();
    media_of(value, &mut found);
    assert!(!found.is_empty(), "{read}: no DV_MULTIMEDIA");
    found
}

fn octets(media: &Value, key: &str, read: &str) -> Vec<u8> {
    let text = media[key]
        .as_str()
        .unwrap_or_else(|| panic!("{read}: no {key}"));
    STANDARD
        .decode(text)
        .unwrap_or_else(|e| panic!("{read}: {key} is not base64: {e}"))
}

/// The attributes both forms keep: the media type, the size and the digest.
fn assert_described(media: &Value, read: &str) {
    assert_eq!(
        media["media_type"]["code_string"], "application/pdf",
        "{read}"
    );
    assert_eq!(
        media["media_type"]["terminology_id"]["value"], "IANA_media-types",
        "{read}"
    );
    assert_eq!(media["size"], json!(PDF.len()), "{read}");
    assert_eq!(octets(media, "integrity_check", read), sha256(), "{read}");
    assert_eq!(
        media["integrity_check_algorithm"]["code_string"], "SHA-256",
        "{read}"
    );
}

async fn assert_inline_original(app: &Router, ehr_id: &str, version_uid: &str) {
    for (read, value) in reads(app, ehr_id, version_uid).await {
        for media in all_media(&value, read) {
            assert_described(media, read);
            let served = octets(media, "data", read);
            assert!(
                served == PDF,
                "{read}: the served bytes differ from the committed PDF"
            );
            assert_eq!(Sha256::digest(&served).to_vec(), sha256(), "{read}");
        }
    }
}

#[tokio::test]
async fn an_inline_pdf_a_original_committed_as_json_reads_back_byte_for_byte() {
    let (_pg, app, ehr_id) = app_with_template().await;
    let body = composition_with(&app, &inline_pdf()).await;
    let version_uid = commit(&app, &ehr_id, "application/json", body.to_string()).await;
    assert_inline_original(&app, &ehr_id, &version_uid).await;
}

#[tokio::test]
async fn an_inline_pdf_a_original_committed_as_xml_reads_back_byte_for_byte() {
    let (_pg, app, ehr_id) = app_with_template().await;
    let body = composition_with(&app, &inline_pdf()).await;
    let typed: Composition = openehr_its::json::from_canonical_value(&body).unwrap();
    let xml = openehr_its::xml::runtime::to_xml(
        &typed,
        "composition",
        openehr_its::xml::runtime::Namespace::V2,
    )
    .unwrap();
    let version_uid = commit(&app, &ehr_id, "application/xml", xml).await;
    assert_inline_original(&app, &ehr_id, &version_uid).await;
}

#[tokio::test]
async fn a_referenced_pdf_a_original_keeps_its_uri_size_and_integrity_check() {
    let (_pg, app, ehr_id) = app_with_template().await;
    let body = composition_with(&app, &referenced_pdf()).await;
    let version_uid = commit(&app, &ehr_id, "application/json", body.to_string()).await;
    for (read, value) in reads(&app, &ehr_id, &version_uid).await {
        for media in all_media(&value, read) {
            assert_described(media, read);
            assert_eq!(media["uri"]["value"], ORIGINAL_URI, "{read}");
            assert!(
                media.get("data").is_none(),
                "{read}: a reference gains no inline data"
            );
        }
    }
}
