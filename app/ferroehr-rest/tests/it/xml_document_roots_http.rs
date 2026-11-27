// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-XML root of the six documents no published ITS-XML schema
//! gives an element: `EHR`, `CONTRIBUTION`, `VERSIONED_COMPOSITION`,
//! `VERSIONED_EHR_STATUS`, `VERSIONED_PARTY` and `REVISION_HISTORY`.
//!
//! The ITS-XML bundles declare document elements only for `archetype`,
//! `composition`, `extract`, `extract_request`, `items`, `template`, `version`
//! and `versioned_object` (plus `query_request` and `result_set` in 2.0.0), and
//! none of the six classes is a `LOCATABLE` or an `X_VERSIONED_OBJECT`. No
//! specification governs these roots: FerroEHR's own design names each after
//! its class and puts no `xsi:type` on it, pending the upstream report #4652.
use axum::Router;
use http::{StatusCode, header};
use serde_json::{Value, json};

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
    let req = common::req("GET", uri).accept("application/xml").build();
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
    let (_pg, app) = common::app(common::api_config(false)).await;

    let ehr_id = common::create_ehr(&app).await;

    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(common::ips_opt_xml())
        .build();
    let (status, _h, body) = common::send(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let commit = common::req("POST", format!("{BASE}/ehr/{ehr_id}/composition"))
        .json(common::ips_canonical_composition().to_string())
        .build();
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

/// A CONTRIBUTION holding one creation version of `data`.
fn contribution_of(data: &Value) -> Value {
    let creation = json!({
        "_type": "DV_CODED_TEXT",
        "value": "creation",
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "249" }
    });
    let audit = json!({
        "_type": "UPDATE_AUDIT",
        "change_type": creation,
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "committer" }
    });
    json!({
        "versions": [{
            "data": data,
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT",
                "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" }
            },
            "commit_audit": audit
        }],
        "audit": audit
    })
}

/// Commits `body` to `uri` asking for the CONTRIBUTION in XML; returns its uid
/// and the root's start tag.
async fn commit_xml(app: &Router, uri: &str, body: &Value) -> (String, String) {
    let req = common::req("POST", uri.to_owned())
        .accept("application/xml")
        .prefer_representation()
        .json(body.to_string())
        .build();
    let (status, headers, text) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "{uri}: {text}");
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml",
        "{uri}"
    );
    (common::etag_uid(&headers), root_start_tag(&text).to_owned())
}

/// The CONTRIBUTION reads and creates of both trees and the `VERSIONED_PARTY`
/// reads answer an XML `Accept` (`Resources.md` §XML Format): the CONTRIBUTION,
/// the `VERSIONED_PARTY` and its `REVISION_HISTORY` under class-named roots, a
/// party version under the published `version` root.
#[tokio::test]
async fn contribution_and_versioned_party_reads_answer_in_xml() {
    let (_pg, app) = common::app(common::api_config(false)).await;

    let ehr_id = common::create_ehr(&app).await;
    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(common::ips_opt_xml())
        .build();
    let (status, _h, body) = common::send(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let ehr_uri = format!("{BASE}/ehr/{ehr_id}/contribution");
    let (cid, root) = commit_xml(
        &app,
        &ehr_uri,
        &contribution_of(&common::ips_canonical_composition()),
    )
    .await;
    assert_class_named_root(&ehr_uri, "contribution", &root);
    let read = format!("{ehr_uri}/{cid}");
    assert_class_named_root(&read, "contribution", &xml_root(&app, &read).await);

    // Resolved versions are no OBJECT_REFs, so that body has no ITS-XML type.
    let resolved = common::req("GET", read.clone())
        .accept("application/xml")
        .header("prefer", "resolve_refs")
        .build();
    let (status, _h, body) = common::send(&app, resolved).await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE, "{body}");

    let person = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Person" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Person" }
                }]
            }
        }]
    });
    let demographic_uri = format!("{BASE}/demographic/contribution");
    let (cid, root) = commit_xml(&app, &demographic_uri, &contribution_of(&person)).await;
    assert_class_named_root(&demographic_uri, "contribution", &root);
    let read = format!("{demographic_uri}/{cid}");
    let json_read = common::req("GET", read.clone())
        .accept("application/json")
        .build();
    let (status, _h, text) = common::send(&app, json_read).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_class_named_root(&read, "contribution", &xml_root(&app, &read).await);
    let contribution: Value = serde_json::from_str(&text).unwrap();
    let ovid = contribution["versions"][0]["id"]["value"]
        .as_str()
        .expect("the version's OBJECT_VERSION_ID")
        .to_owned();
    let vo = ovid.split("::").next().expect("versioned object uid");

    let party = format!("{BASE}/demographic/versioned_party/{vo}");
    for (uri, tag) in [
        (party.clone(), "versioned_party"),
        (format!("{party}/revision_history"), "revision_history"),
    ] {
        assert_class_named_root(&uri, tag, &xml_root(&app, &uri).await);
    }
    for uri in [
        format!("{party}/version"),
        format!("{party}/version/{ovid}"),
    ] {
        let root = xml_root(&app, &uri).await;
        assert!(root.starts_with("<version "), "{uri}: {root}");
        assert!(root.contains("ORIGINAL_VERSION"), "{uri}: {root}");
    }
}
