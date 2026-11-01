// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-XML root of the four documents no published ITS-XML schema
//! gives an element: `EHR`, `VERSIONED_COMPOSITION`, `VERSIONED_EHR_STATUS`
//! and `REVISION_HISTORY`.
//!
//! The ITS-XML bundles declare document elements only for `archetype`,
//! `composition`, `extract`, `extract_request`, `items`, `template`, `version`
//! and `versioned_object` (plus `query_request` and `result_set` in 2.0.0), and
//! none of the four classes is a `LOCATABLE` or an `X_VERSIONED_OBJECT`. No
//! specification governs these roots: FerroEHR's own design names each after
//! its class and puts no `xsi:type` on it, pending the upstream report #4652.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use crate::common;
use crate::common::BASE;

/// The start tag of the document element: the first tag that is not the XML
/// declaration.
fn root_start_tag(xml: &str) -> &str {
    let start = xml
        .match_indices('<')
        .map(|(at, _)| at)
        .find(|&at| !xml.get(at..).unwrap().starts_with("<?"))
        .expect("a document element");
    let rest = xml.get(start..).unwrap();
    let end = rest.find('>').expect("the start tag closes");
    rest.get(..=end).unwrap()
}

/// `GET` `uri` as canonical XML and return the root's start tag.
async fn xml_root(app: &Router, uri: &str) -> String {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/xml")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml",
        "{uri}"
    );
    root_start_tag(&body).to_owned()
}

fn assert_class_named_root(uri: &str, tag: &str, root: &str) {
    let open = format!("<{tag} ");
    assert!(
        root.starts_with(&open) || root == format!("<{tag}>"),
        "{uri}: the root is `{tag}`, got {root}"
    );
    assert!(
        !root.contains("xsi:type"),
        "{uri}: the class-named root carries no xsi:type, got {root}"
    );
}

#[tokio::test]
async fn documents_without_a_published_element_keep_their_class_named_roots() {
    let (_pg, service) = common::test_service().await;
    let app = common::router_with(common::api_config(false), service).await;

    let ehr_id = common::create_ehr(&app).await;

    let upload = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .unwrap();
    let (status, _h, body) = common::send(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let commit = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(common::ips_canonical_composition().to_string()))
        .unwrap();
    let (status, headers, body) = common::send(&app, commit).await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    let version_uid = common::etag_uid(&headers);
    let vo = version_uid
        .split("::")
        .next()
        .expect("versioned object uid");

    let routes = [
        (format!("{BASE}/ehr/{ehr_id}"), "ehr"),
        (
            format!("{BASE}/ehr/{ehr_id}/versioned_composition/{vo}"),
            "versioned_composition",
        ),
        (
            format!("{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/revision_history"),
            "revision_history",
        ),
        (
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status"),
            "versioned_ehr_status",
        ),
        (
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/revision_history"),
            "revision_history",
        ),
    ];
    for (uri, tag) in &routes {
        let root = xml_root(&app, uri).await;
        assert_class_named_root(uri, tag, &root);
    }
}
