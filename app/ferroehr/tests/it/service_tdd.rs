// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for the SM `I_TDD_SERVICE.import_tdd` /
//! `import_tdds` TDD (Template Data Document) import path against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: SM `docs/specs/openehr/SM/docs/UML/classes/i_tdd_service.adoc`
//! (included by `SM/docs/openehr_platform/master09-message_service.adoc`).
//! Fixtures are
//! the vendored CNF corpus TDD instances
//! (`docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/compositions/TDD/`)
//! and their matching OPT
//! (`.../valid_templates/minimal_persistent/persistent_minimal.opt`).
//!
//! These tests assert the typed envelope rejections (malformed payload, wrong
//! namespace, unknown EHR, unknown template) and the full happy path: a
//! well-formed TDD for a provisioned template is converted (`openehr_sdt::flat::tdd::from_tdd`)
//! and committed through the validated `create_composition` path, then read back
//! via the composition surface.

#![expect(
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::status::CallStatusType;

const CORPUS: &str = "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";
const TEMPLATE_ID: &str = "persistent_minimal.en.v1";

fn read_fixture(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn tdd(name: &str) -> String {
    read_fixture(&format!("{CORPUS}/compositions/TDD/{name}"))
}

fn persistent_minimal_opt() -> String {
    read_fixture(&format!(
        "{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt"
    ))
}

/// A malformed (non-XML) payload is rejected `content_invalid`, not a 500.
#[tokio::test]
async fn tdd_import_rejects_malformed_payload() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    let err = svc
        .import_tdd(ehr, "this is not a TDD at all".to_owned())
        .await
        .expect_err("a non-XML payload must be rejected");
    assert_eq!(
        err.status,
        CallStatusType::ContentInvalid,
        "malformed payload → content_invalid: {err:?}"
    );
}

/// A payload that is not in the Ocean templates namespace is not a TDD →
/// `precondition_violation`.
#[tokio::test]
async fn tdd_import_rejects_non_tdd_xml() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    // Well-formed XML, but a canonical-openEHR (not templates) namespace.
    let not_tdd = r#"<composition xmlns="http://schemas.openehr.org/v1"><name/></composition>"#;
    let err = svc
        .import_tdd(ehr, not_tdd.to_owned())
        .await
        .expect_err("a non-TDD XML document must be rejected");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "wrong namespace → precondition_violation: {err:?}"
    );
}

/// A valid TDD targeting an EHR that does not exist is `ehr_id_does_not_exist`
/// (the design-filled `has_ehr` precondition).
#[tokio::test]
async fn tdd_import_rejects_unknown_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let err = svc
        .import_tdd(
            ferroehr::ids::EhrId(uuid::Uuid::now_v7()),
            tdd("persistent_minimal.en.v1__full.xml"),
        )
        .await
        .expect_err("a TDD for a non-existent EHR must be rejected");
    assert_eq!(
        err.status,
        CallStatusType::EhrIdDoesNotExist,
        "unknown EHR → ehr_id_does_not_exist: {err:?}"
    );
}

/// A valid TDD whose `template_id` names an unprovisioned template is
/// `template_does_not_exist` — the corpus `..__invalid_opt_doesnt_exist` case
/// (its root carries `template_id="not_exist"`).
#[tokio::test]
async fn tdd_import_rejects_unknown_template() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    let err = svc
        .import_tdd(ehr, tdd("nested.en.v1__invalid_opt_doesnt_exist.xml"))
        .await
        .expect_err("a TDD for an unprovisioned template must be rejected");
    assert_eq!(
        err.status,
        CallStatusType::TemplateDoesNotExist,
        "unknown template → template_does_not_exist: {err:?}"
    );
}

/// A well-formed TDD for a **provisioned** template is converted (OPT-guided
/// body walk, `openehr_sdt::flat::tdd::from_tdd`) and committed through the validated
/// `create_composition` path, then read back via the composition surface.
///
/// This upgrades the former `..._body_deferred` expectation: the OPT-guided
/// TDD-body → COMPOSITION converter (B3 task 2's remaining sub-item) has landed,
/// so a provisioned-template TDD now commits a COMPOSITION rather than being
/// rejected with a `precondition_violation`.
#[tokio::test]
async fn tdd_import_commits_composition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    // Provision the operational template the TDD instantiates.
    let desc = svc
        .template_adl14_upload(persistent_minimal_opt())
        .await
        .expect("opt upload");
    assert_eq!(desc["template_id"], TEMPLATE_ID, "opt template_id");

    let ovid = svc
        .import_tdd(ehr, tdd("persistent_minimal.en.v1__full.xml"))
        .await
        .expect("provisioned-template TDD imports and commits");
    assert!(
        ovid.contains("::"),
        "import returns an OBJECT_VERSION_ID: {ovid}"
    );

    // Exactly one COMPOSITION committed.
    let comps: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM version WHERE ehr_id = $1 AND kind = 'COMPOSITION'",
    )
    .bind(ehr)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(comps, 1, "one COMPOSITION committed on TDD import");

    // Readable via the composition surface, with the instance data carried from
    // the TDD and the template-supplied archetype identity.
    let vo_uuid = ovid
        .split("::")
        .next()
        .unwrap()
        .parse::<ferroehr::ids::VoId>()
        .expect("vo uuid");
    let comp = svc
        .get_composition_latest(ehr, vo_uuid)
        .await
        .expect("read committed COMPOSITION");
    assert_eq!(comp["_type"], "COMPOSITION");
    assert_eq!(comp["name"]["value"], "Persistent minimal");
    assert_eq!(
        comp["archetype_details"]["template_id"]["value"],
        TEMPLATE_ID
    );
    assert_eq!(comp["territory"]["code_string"], "US");
    // The compacted HISTORY/EVENT/ITEM_TREE/ELEMENT chain, with the TDD leaf.
    assert_eq!(
        comp["content"][0]["data"]["events"][0]["data"]["items"][0]["value"]["value"],
        "value 1"
    );
}

/// A batch import commits every TDD (all-or-nothing prepare-then-commit).
#[tokio::test]
async fn tdd_import_tdds_batch_commits_all() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("ehr");
    svc.template_adl14_upload(persistent_minimal_opt())
        .await
        .expect("opt upload");

    // Two event (non-persistent) TDDs would collide on the persistent-uniqueness
    // rule; this template is persistent, so import the single persistent TDD in a
    // one-element batch to exercise the prepare-then-commit path end to end.
    let ids = svc
        .import_tdds(ehr, vec![tdd("persistent_minimal.en.v1__full.xml")])
        .await
        .expect("batch import commits");
    assert_eq!(ids.len(), 1);

    let comps: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM version WHERE ehr_id = $1 AND kind = 'COMPOSITION'",
    )
    .bind(ehr)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(comps, 1, "batch committed the COMPOSITION");
}

/// `import_tdds` is fail-fast: a batch containing an invalid TDD rejects with a
/// typed error and commits nothing.
#[tokio::test]
async fn tdd_import_tdds_batch_fail_fast() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = svc.create_ehr(None).await.expect("ehr");

    let err = svc
        .import_tdds(ehr, vec![tdd("nested.en.v1__invalid_opt_doesnt_exist.xml")])
        .await
        .expect_err("a batch with an unprovisioned-template TDD must be rejected");
    assert_eq!(
        err.status,
        CallStatusType::TemplateDoesNotExist,
        "batch fail-fast surfaces the item's typed error: {err:?}"
    );
}
