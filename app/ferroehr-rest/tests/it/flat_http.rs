// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the FLAT (simSDT) COMPOSITION endpoints, driven
//! through the assembled router over a **real** `FerroEhrService` on a real
//! `PostgreSQL`.
//!
//! The IPS OPT + its canonical composition are the pair driven end-to-end
//! through the real service in `app/ferroehr/tests/service_validation.rs`, so
//! they upload + commit cleanly here (the Demo Vitals corpus composition the
//! former Mock served fails the real template value-set validation; the
//! generated `Medium` example fails a proportion constraint — neither is
//! actually committable). The FLAT glue is exercised through the router:
//!
//! * GET with `Accept: application/openehr.wt.flat+json` → the stored canonical
//!   composition is returned as a flat map;
//! * POST with `Content-Type: application/openehr.wt.flat+json` + `?template_id`
//!   → the flat body is rebuilt into a canonical composition before the service
//!   commits it (verified by reading the stored composition back);
//! * POST flat without a template id → 400, and a `|raw` without `_type` → 400;
//! * a full flat → RM → flat round-trip through the two endpoints is stable
//!   (modulo the server-assigned version `_uid`s the Mock never produced).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_assert_message,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::Router;
use http::{StatusCode, header};
use serde_json::{Value, json};

use crate::common;
use crate::common::{etag_uid, ips_opt_xml, send};

const BASE: &str = "/ferroehr/rest/openehr/v1";
const FLAT_MIME: &str = "application/openehr.wt.flat+json";
const STRUCTURED_MIME: &str = "application/openehr.wt.structured+json";
/// The IPS template id, supplied through the `openehr-template-id` request
/// header on a simplified commit (`Requests_and_responses` §openehr-template-id —
/// the header, not a query parameter, is the mechanism).
const TEMPLATE_ID: &str = "International Patient Summary";
const TEMPLATE_ID_HEADER: &str = "openehr-template-id";

/// The IPS canonical composition (with its stored `uid` removed — a create
/// assigns a fresh one).
fn canonical_composition() -> Value {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
        ),
    )
    .expect("ips_canonical.json vendored in openehr-its");
    let mut v: Value = serde_json::from_str(&text).expect("valid canonical composition");
    v.as_object_mut().unwrap().remove("uid");
    v
}

/// The IPS `WebTemplate` (built from the vendored OPT).
fn web_template() -> openehr_sdt::flat::webtemplate::model::WebTemplate {
    let opt = openehr_its::opt14::from_xml(&ips_opt_xml()).expect("parse OPT");
    openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).expect("build web template")
}

/// The full `OBJECT_VERSION_ID` carried in the weak `ETag` (`W/"<uid>"`).
fn etag_full(h: &header::HeaderMap) -> String {
    h.get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

fn location_of(h: &header::HeaderMap) -> String {
    h.get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location present")
        .to_owned()
}

fn vo_of(ovid: &str) -> String {
    ovid.split("::").next().expect("vo uuid").to_owned()
}

/// Drop the `.../_uid` leaves (server-assigned version ids) so a flat→RM→flat
/// comparison isolates the data — the real service assigns version `_uid`s the
/// former in-memory Mock never produced.
fn without_uids(map: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    map.into_iter()
        .filter(|(k, _)| !k.ends_with("/_uid"))
        .collect()
}

/// A router over a fresh real service with the IPS OPT uploaded; returns the
/// router and a created EHR id.
async fn app_with_ehr() -> (testkit::TestDb, Router, String) {
    let (pg, app) = common::app(common::api_config(false)).await;
    let (status, _h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(ips_opt_xml())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let (status, h, _b) = send(&app, common::req("POST", format!("{BASE}/ehr")).build()).await;
    assert_eq!(status, StatusCode::CREATED);
    (pg, app, etag_uid(&h))
}

/// Commit the canonical `comp` into `ehr_id`; return the new versioned-object uuid.
async fn commit_canonical(app: &Router, ehr_id: &str, comp: &Value) -> String {
    let (status, h, body) = send(
        app,
        common::req("POST", format!("{BASE}/ehr/{ehr_id}/composition"))
            .json(comp.to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "canonical commit: {body}");
    vo_of(&etag_uid(&h))
}

#[tokio::test]
async fn get_composition_as_flat() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let vo = commit_canonical(&app, &ehr, &canonical_composition()).await;

    let req = common::req("GET", format!("{BASE}/ehr/{ehr}/composition/{vo}"))
        .accept(FLAT_MIME)
        .build();
    let (status, headers, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK, "flat get: {body}");
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), FLAT_MIME);
    let flat: serde_json::Map<String, Value> = serde_json::from_str(&body).unwrap();
    assert!(flat.contains_key("ctx/language"), "flat has ctx keys");
    assert!(
        flat.keys().any(|k| k.ends_with("|magnitude")),
        "flat has a |magnitude leaf: {:?}",
        flat.keys().collect::<Vec<_>>()
    );
    assert!(
        !flat.keys().any(|k| k.ends_with("|units")),
        "|unit is singular"
    );
}

#[tokio::test]
async fn post_flat_composition_is_rebuilt_to_canonical() {
    let (_pg, app, ehr) = app_with_ehr().await;

    // Derive a real flat body from the canonical composition + its template.
    let wt = web_template();
    let flat =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();
    let flat_body = serde_json::to_string(&flat_map).unwrap();

    let (status, h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(flat_body)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "flat commit: {body}");
    let vo = vo_of(&etag_uid(&h));

    // The service received (and stored) a canonical COMPOSITION, not the flat map.
    let (status, _h, body) = send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "canonical read: {body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        stored.get("_type").and_then(Value::as_str),
        Some("COMPOSITION")
    );
    assert!(stored.get("content").is_some(), "rebuilt content present");
    assert!(
        stored.pointer("/context/start_time/value").is_some(),
        "rebuilt context from ctx/"
    );
}

/// A simplified COMPOSITION commit with no `openehr-template-id` header is a
/// `400`: `Requests_and_responses.md` §openehr-template-id, "The
/// `openehr-template-id` request header MUST be used whenever committing
/// COMPOSITION ... using a Simplified Format", and `responses/400.yaml`
/// answers a "missing required header or parameter".
#[tokio::test]
async fn post_flat_without_template_id_is_400() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let req = common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
        .header(header::CONTENT_TYPE, FLAT_MIME)
        .body("{\"ctx/language\":\"en\"}")
        .build();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// A `|raw` value without `_type` cannot be read as an RM object (master04
/// §Raw canonical JSON: "The raw JSON value must include the `_type`
/// property"), so the commit is syntactically invalid content, a `400`.
#[tokio::test]
async fn post_flat_raw_without_type_is_400() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let wt = web_template();
    let mut flat: BTreeMap<String, Value> =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt)
            .unwrap()
            .into_iter()
            .collect();
    let leaf = flat
        .keys()
        .find(|k| k.ends_with("|magnitude"))
        .map(|k| k.trim_end_matches("|magnitude").to_owned())
        .expect("a quantity leaf");
    flat.retain(|k, _| !k.starts_with(&format!("{leaf}|")));
    flat.insert(format!("{leaf}|raw"), json!({ "magnitude": 120 }));
    let req = common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
        .header(header::CONTENT_TYPE, FLAT_MIME)
        .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
        .body(serde_json::to_string(&flat).unwrap())
        .build();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// The deprecated and legacy simplified-format media types the release does
/// not require a server to support: the `.schema+json` twins ("now deprecated
/// and will be removed", Resources.md §Simplified Formats NOTE) and the
/// legacy/experimental names (§Alternative data formats: "Some of these
/// formats might not be supported").
const BANNED_SIMPLIFIED_TYPES: &[&str] = &[
    "application/openehr.wt.flat.schema+json",
    "application/openehr.wt.structured.schema+json",
    "application/openehr.nc.flat+json",
    "application/openehr.tds2+xml",
];

/// A COMPOSITION commit under any deprecated/legacy simplified `Content-Type`
/// is a `415` — "If the service cannot process the request payload as the
/// simplified format is not supported, it MUST respond with HTTP status code
/// 415 Unsupported Media Type" (Resources.md §Simplified Formats).
#[tokio::test]
async fn post_composition_deprecated_or_legacy_content_type_is_415() {
    let (_pg, app, ehr) = app_with_ehr().await;
    for mime in BANNED_SIMPLIFIED_TYPES {
        let req = common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, *mime)
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body("{}")
            .build();
        let (status, _h, _b) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{mime} must refuse as 415"
        );
    }
}

/// A COMPOSITION GET whose `Accept` names only a deprecated/legacy simplified
/// type is a `406` — "If the service cannot fulfill this aspect of the
/// request, it MUST respond with HTTP status code 406 Not Acceptable"
/// (Resources.md §Simplified Formats).
#[tokio::test]
async fn get_composition_deprecated_or_legacy_accept_is_406() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let vo = commit_canonical(&app, &ehr, &canonical_composition()).await;
    for mime in BANNED_SIMPLIFIED_TYPES {
        let req = common::req("GET", format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .accept(*mime)
            .build();
        let (status, _h, _b) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::NOT_ACCEPTABLE,
            "{mime} must refuse as 406"
        );
    }
}

/// `EHR_STATUS` is not templated → a simplified `Accept` on its retrieval is a
/// `406`, and a simplified `Content-Type` on its update a `415`
/// (`formats::dispatch::guard_non_templated`; master05 defines no mapping for
/// non-templated resources).
#[tokio::test]
async fn ehr_status_simplified_is_rejected() {
    let (_pg, app, ehr) = app_with_ehr().await;

    // Simplified Accept on the EHR_STATUS retrieval → 406.
    let (status, _h, _b) = send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/ehr_status"))
            .accept(FLAT_MIME)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE);

    // Simplified Content-Type on the EHR_STATUS update → 415.
    let (status, _h, _b) = send(
        &app,
        common::req("PUT", format!("{BASE}/ehr/{ehr}/ehr_status"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .if_match("does-not-matter")
            .body("{}")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn flat_round_trips_through_http() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let wt = web_template();
    let flat_in =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_in_map: serde_json::Map<String, Value> = flat_in.clone().into_iter().collect();

    // POST the flat body → the service stores the rebuilt canonical composition.
    // The template id is supplied via the `openehr-template-id` header.
    let (status, h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(serde_json::to_string(&flat_in_map).unwrap())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "flat commit: {body}");
    let vo = vo_of(&etag_uid(&h));

    // GET it back as flat.
    let (status, _h, body) = send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .accept(FLAT_MIME)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "flat get: {body}");
    let flat_out: BTreeMap<String, Value> = serde_json::from_str(&body).unwrap();

    let flat_in_sorted: BTreeMap<String, Value> = flat_in.into_iter().collect();
    assert_eq!(
        without_uids(flat_in_sorted),
        without_uids(flat_out),
        "flat → RM → flat stable through the HTTP endpoints (modulo server uids)"
    );
}

/// Regression for #229: a `201 Created` whose negotiated representation is FLAT
/// (`Accept: application/openehr.wt.flat+json`, `Prefer: return=representation`)
/// MUST still carry the committed-resource `ETag` (new version uid) and
/// `Location` headers — they are representation-independent (RFC 7231 §6.3.2
/// requires `Location` on a `201` regardless of body form; the ITS-REST 201
/// response declares both headers unconditionally,
/// `docs/specs/openehr/ITS-REST/specifications/responses/201_COMPOSITION.yaml`).
/// Previously the FLAT response path dropped both headers.
#[tokio::test]
async fn post_flat_representation_carries_etag_and_location() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let wt = web_template();
    let flat =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();

    let (status, h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .accept(FLAT_MIME)
            .prefer_representation()
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(serde_json::to_string(&flat_map).unwrap())
            .build(),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "flat commit: {body}");
    // The body is the FLAT representation…
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), FLAT_MIME);
    assert!(
        serde_json::from_str::<serde_json::Map<String, Value>>(&body).is_ok(),
        "FLAT body present: {body}"
    );
    // …and the version-id headers are the same set the canonical path sets.
    let uid = etag_full(&h);
    assert!(
        uid.contains("::"),
        "ETag carries the full version uid: {uid}"
    );
    assert_eq!(
        location_of(&h),
        format!("{BASE}/ehr/{ehr}/composition/{uid}"),
        "Location points at the new COMPOSITION version"
    );
}

/// The STRUCTURED representation of the same commit likewise carries `ETag` +
/// `Location` (same spec basis as the FLAT case above; #229).
#[tokio::test]
async fn post_structured_representation_carries_etag_and_location() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let wt = web_template();
    let flat =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();

    // Commit in FLAT, negotiate the STRUCTURED representation back.
    let (status, h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .accept(STRUCTURED_MIME)
            .prefer_representation()
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(serde_json::to_string(&flat_map).unwrap())
            .build(),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "structured commit: {body}");
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), STRUCTURED_MIME);
    let uid = etag_full(&h);
    assert!(
        uid.contains("::"),
        "ETag carries the full version uid: {uid}"
    );
    assert_eq!(
        location_of(&h),
        format!("{BASE}/ehr/{ehr}/composition/{uid}")
    );
}

/// The `200 OK` from an update whose negotiated representation is FLAT carries
/// the new version's `ETag` + `Location`, matching the canonical update path
/// (`docs/specs/openehr/ITS-REST/specifications/responses/
/// 200_COMPOSITION_updated.yaml` declares both headers unconditionally; #229).
#[tokio::test]
async fn update_flat_representation_carries_etag_and_location() {
    let (_pg, app, ehr) = app_with_ehr().await;

    // Seed a first version canonically to obtain its full version uid for If-Match.
    let (status, seed_h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .json(canonical_composition().to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "seed commit: {body}");
    let prior_uid = etag_full(&seed_h);
    let vo = vo_of(&prior_uid);

    // Update with a FLAT body, negotiating the FLAT representation back.
    let wt = web_template();
    let flat =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();
    let (status, h, body) = send(
        &app,
        common::req("PUT", format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .accept(FLAT_MIME)
            .if_match(&prior_uid)
            .prefer_representation()
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(serde_json::to_string(&flat_map).unwrap())
            .build(),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "flat update: {body}");
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), FLAT_MIME);
    let new_uid = etag_full(&h);
    assert!(
        new_uid.contains("::"),
        "ETag carries the version uid: {new_uid}"
    );
    assert_ne!(new_uid, prior_uid, "the update produced a new version");
    assert_eq!(
        location_of(&h),
        format!("{BASE}/ehr/{ehr}/composition/{new_uid}")
    );
}

/// A Simplified-Formats commit answers with the committed COMPOSITION in the
/// negotiated FLAT form — the `Accept` decides the body here, so the applied
/// preference is `return=representation`, and the response declares it through
/// the same seam as the canonical path ("The service MAY include a
/// `Preference-Applied` header in the response … to indicate that the client's
/// preference has been honored", `Requests_and_responses.md` §Representation
/// details negotiation).
#[tokio::test]
async fn post_flat_declares_the_applied_preference() {
    let (_pg, app, ehr) = app_with_ehr().await;
    let wt = web_template();
    let flat =
        openehr_sdt::flat::convert::composition_to_flat(&canonical_composition(), &wt).unwrap();
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();

    let (status, h, body) = send(
        &app,
        common::req("POST", format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, FLAT_MIME)
            .accept(FLAT_MIME)
            .prefer_representation()
            .header(TEMPLATE_ID_HEADER, TEMPLATE_ID)
            .body(serde_json::to_string(&flat_map).unwrap())
            .build(),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "flat commit: {body}");
    assert_eq!(
        h.get("preference-applied").and_then(|v| v.to_str().ok()),
        Some("return=representation"),
        "the Simplified-Formats commit declares the preference it applied"
    );
}
