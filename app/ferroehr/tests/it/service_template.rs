// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for OPT 1.4 operational-template ingestion against a
//! real `PostgreSQL` 18 (shared testkit harness): upload a corpus `.opt` template, list it,
//! retrieve its XML, and re-upload (idempotent replace) — driven through the
//! `FerroEhrService` definition methods the REST layer calls.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::Value;

use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::types::TemplateListFilter;
use ferroehr::service::list::Page;
use ferroehr::service::status::{ServiceStatus, StatusError};

/// Fixed `ctx/time` default for the FLAT rebuild directions (ITS-REST
/// `simplified_formats` master04 §Context) so round-trips stay deterministic.
const NOW: &str = "2024-01-01T00:00:00Z";

/// A representative corpus template (Ocean Template Designer OPT 1.4 XML).
const TEMPLATE_REL: &str = "knowledge/IDCR Allergies List.v0.opt";
const TEMPLATE_ID: &str = "IDCR Allergies List.v0";

fn corpus_opt(rel: &str) -> String {
    let path = format!(
        "{}/../../corpus/fixtures/service/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[tokio::test]
async fn template_upload_list_get_roundtrip() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let xml = corpus_opt(TEMPLATE_REL);

    // Upload the OPT XML (arrives as a JSON string, as the lenient body reader hands it over).
    let desc = svc
        .template_adl14_upload(xml.clone())
        .await
        .expect("upload");
    assert_eq!(desc["template_id"], TEMPLATE_ID, "descriptor template_id");
    assert!(
        desc["archetype_id"]
            .as_str()
            .is_some_and(|a| a.contains("COMPOSITION")),
        "root archetype extracted: {desc}"
    );

    // List includes the uploaded template.
    let list = svc
        .template_adl14_list(TemplateListFilter::default(), Page::all())
        .await
        .expect("list");
    assert!(
        list.iter().any(|t| t["template_id"] == TEMPLATE_ID),
        "list contains the template: {list:?}"
    );

    // Retrieve returns the stored OPT XML verbatim.
    let got = svc
        .template_adl14_get(TEMPLATE_ID.to_owned())
        .await
        .expect("get");
    assert_eq!(
        got, xml,
        "retrieved OPT XML is byte-identical to the upload"
    );

    // Re-uploading an existing template_id is a 409 Conflict, not a silent
    // overwrite: OPTs are immutable on the adl1.4 endpoint (ITS-REST
    // `409_template_already_exists.yaml`; CNF `upload_opt-valid_opt_twice_conflict`).
    let conflict = svc
        .template_adl14_upload(xml.clone())
        .await
        .expect_err("re-upload of an existing template_id must conflict");
    assert!(
        matches!(
            conflict,
            StatusError {
                status: ServiceStatus::Conflict,
                ..
            }
        ),
        "got {conflict:?}"
    );

    // The original template is untouched and there is still exactly one row.
    let list2 = svc
        .template_adl14_list(TemplateListFilter::default(), Page::all())
        .await
        .expect("list after conflicting re-upload");
    assert_eq!(
        list2
            .iter()
            .filter(|t| t["template_id"] == TEMPLATE_ID)
            .count(),
        1,
        "conflicting re-upload did not duplicate"
    );
    let still = svc
        .template_adl14_get(TEMPLATE_ID.to_owned())
        .await
        .expect("get after conflict");
    assert_eq!(
        still, xml,
        "conflicting re-upload did not overwrite the stored OPT"
    );
}

#[tokio::test]
async fn get_unknown_template_is_not_found() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    // An unknown template_id → 404 (`template_does_not_exist`).
    let err = svc
        .template_adl14_get("unknown template.v0".to_owned())
        .await
        .expect_err("expected not-found");
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn invalid_opt_xml_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let err = svc
        .template_adl14_upload("<not-a-template/>".to_owned())
        .await
        .expect_err("expected rejection of a non-OPT body");
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::InvalidTemplate,
                ..
            }
        ),
        "got {err:?}"
    );
}

// ── example generation (`adl1.4/{id}/example`) ───────────────────────────────

/// Varied real templates: an OBSERVATION with a history of events, an
/// EVALUATION-list, and one carrying an ACTION/INSTRUCTION structure.
const EXAMPLE_TEMPLATES: &[&str] = &[
    "knowledge/Vital Signs Encounter (Composition).opt",
    "knowledge/IDCR Allergies List.v0.opt",
    "knowledge/IDCR - Immunisation summary.v0.opt",
];

/// The (cached) `WebTemplate` built from an OPT file, as the service builds it.
fn web_template_of(rel: &str) -> openehr_sdt::flat::webtemplate::model::WebTemplate {
    let opt = openehr_its::opt14::from_xml(&corpus_opt(rel)).expect("parse OPT");
    openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).expect("build web template")
}

/// The generated `required` example is committable (passes the validator)
/// and survives FLAT round-trip + canonical-XML serialization for real
/// templates. The example is fetched through the service definition methods
/// exactly as the REST layer calls it; validation/conversion use the same
/// `WebTemplate` the service caches.
#[tokio::test]
async fn required_example_validates_and_converts() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    for (i, rel) in EXAMPLE_TEMPLATES.iter().enumerate() {
        let xml = corpus_opt(rel);
        let desc = svc
            .template_adl14_upload(xml)
            .await
            .unwrap_or_else(|e| panic!("upload {rel}: {e:?}"));
        let template_id = desc["template_id"]
            .as_str()
            .unwrap_or_else(|| panic!("template_id for {rel}"))
            .to_owned();
        let wt = web_template_of(rel);

        // `required` example, via the generated trait (as the REST layer calls it).
        let comp = svc
            .template_adl14_example(template_id.clone(), Some("required".to_owned()), None)
            .await
            .unwrap_or_else(|e| panic!("example for {rel}: {e:?}"));
        assert_eq!(
            comp.get("_type").and_then(Value::as_str),
            Some("COMPOSITION"),
            "{rel}: example is a COMPOSITION"
        );

        // Acceptance bar: the required example is committable — it passes the
        // full validator (RM invariants + terminology + archetype
        // conformance) with no violations.
        let violations = openehr_sdt::rm_instance::validate_composition(&comp, &wt);
        assert!(
            violations.is_empty(),
            "{rel}: required example must validate clean, got {} violation(s): {violations:?}",
            violations.len()
        );

        // FLAT round-trip is stable (canonical → FLAT → canonical → FLAT). The
        // rebuild's ctx/time default is a fixed instant (ITS-REST
        // simplified_formats master04 §Context) so the round-trip is deterministic.
        let flat1 = openehr_sdt::flat::convert::composition_to_flat(&comp, &wt)
            .unwrap_or_else(|e| panic!("{rel} to_flat: {e}"));
        let flat1_map: serde_json::Map<String, Value> = flat1.clone().into_iter().collect();
        let comp2 = openehr_sdt::flat::convert::composition_from_flat(&flat1_map, &wt, NOW)
            .unwrap_or_else(|e| panic!("{rel} from_flat: {e}"));
        let flat2 = openehr_sdt::flat::convert::composition_to_flat(&comp2, &wt)
            .unwrap_or_else(|e| panic!("{rel} to_flat2: {e}"));
        assert_eq!(flat1, flat2, "{rel}: FLAT round-trip is stable");

        // Canonical-XML serialization succeeds (deserialises as an RM COMPOSITION
        // then emits canonical XML — the XML `Accept` path in the dispatcher).
        let typed: openehr_rm::prelude::Composition =
            openehr_its::json::from_canonical_value(&comp)
                .unwrap_or_else(|e| panic!("{rel}: example deserialises as Composition: {e}"));
        let xml_out = openehr_its::xml::runtime::to_xml(
            &typed,
            "composition",
            openehr_its::xml::runtime::Namespace::V2,
        )
        .unwrap_or_else(|e| panic!("{rel}: canonical XML: {e}"));
        assert!(
            xml_out.contains("<composition"),
            "{rel}: XML has a composition root"
        );

        // The `output` form carries a deterministic uid; `input` does not.
        let output = svc
            .template_adl14_example(template_id.clone(), None, Some("output".to_owned()))
            .await
            .unwrap_or_else(|e| panic!("output example for {rel}: {e:?}"));
        assert!(
            output.pointer("/uid/value").is_some(),
            "{rel}: output example carries a uid"
        );
        assert!(comp.get("uid").is_none(), "{rel}: input example has no uid");

        // Distinct databases per iteration are unnecessary; template ids differ.
        let _ = i;
    }
}

#[tokio::test]
async fn example_for_unknown_template_is_not_found() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let err = svc
        .template_adl14_example("does.not.exist.v0".to_owned(), None, None)
        .await
        .expect_err("expected not-found for an unknown template");
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn example_with_invalid_detail_level_is_bad_request() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    // Upload a template so the failure is the detail_level, not a missing id.
    let xml = corpus_opt(TEMPLATE_REL);
    svc.template_adl14_upload(xml).await.expect("upload");
    let err = svc
        .template_adl14_example(TEMPLATE_ID.to_owned(), Some("exhaustive".to_owned()), None)
        .await
        .expect_err("expected bad-request for an invalid detail_level");
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::PreconditionViolation,
                ..
            }
        ),
        "got {err:?}"
    );
}

/// The OPT-1.4 → ADL2 conversion capability (service-only, no wire): a stored
/// operational template is loaded by `template_id`, decomposed into one
/// 1.4-shaped source per embedded archetype root, and each is converted to ADL2 source text. No
/// openEHR spec governs 1.4 → 2 conversion — our own design/extension.
#[tokio::test]
async fn opt_converts_to_adl2_sources() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    // A minimal template: a COMPOSITION root with one embedded OBSERVATION root.
    let xml = corpus_opt("knowledge/opt/minimal_observation.opt");
    let summary = svc.template_adl14_upload(xml).await.expect("upload opt");
    let template_id = summary["template_id"]
        .as_str()
        .expect("the upload names its template_id")
        .to_owned();

    // The service loads the stored OPT by template_id, decomposes + converts it, and
    // returns one ADL2 source per embedded archetype root (>= 2 here: the
    // COMPOSITION root and the embedded OBSERVATION).
    let conversion = svc
        .adl14_convert_opt_to_adl2(template_id)
        .await
        .expect("convert stored OPT to ADL2");
    assert!(
        !conversion.structure.is_empty(),
        "the embedded OBSERVATION fills a slot of the COMPOSITION"
    );
    let sources: Vec<&str> = conversion.roots.iter().map(|r| r.adl2.as_str()).collect();
    assert!(
        sources.len() >= 2,
        "expected >= 2 converted sources, got {}",
        sources.len()
    );
    for src in sources {
        assert!(
            src.contains("archetype"),
            "each converted source is ADL2 text: {}",
            src.lines().next().unwrap_or_default()
        );
    }

    // An unknown OPT id is a 404 (`template_does_not_exist`).
    let missing = svc
        .adl14_convert_opt_to_adl2("unknown template.v0".to_owned())
        .await
        .expect_err("unknown OPT id must 404");
    assert!(
        matches!(
            missing,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {missing:?}"
    );
}
