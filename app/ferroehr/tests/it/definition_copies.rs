// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The definition copies of two replicas over one database (#5948): a
//! change on one replica decides the next request of the other, which reads
//! the definition revision in its read of the policy heads and hears no
//! notification. No openEHR spec governs the caches: our own design.

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::adl2_fixture::adl2_source;
use crate::fixtures::{base_composition, uv};

/// The template id both uploads store.
const TEMPLATE_ID: &str = "IDCR Allergies List.v0";

/// A replica over `pool`, with no listener, as a replica whose
/// notifications never arrive.
fn replica(pool: &sqlx::PgPool) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
}

/// The OPT 1.4 fixture at `name` under the service knowledge corpus.
fn opt(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/service/knowledge")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The allergies template, and a template of other content stored under the
/// same id.
fn templates() -> (String, String) {
    let first = opt("IDCR Allergies List.v0.opt");
    let other = opt("IDCR Problem List.v1.opt").replace(
        "<value>IDCR Problem List.v1</value>",
        &format!("<value>{TEMPLATE_ID}</value>"),
    );
    (first, other)
}

/// What a request of `svc` answers as the Web Template of [`TEMPLATE_ID`],
/// after the read every request makes first.
async fn served(svc: &FerroEhrService) -> Result<Value, ServiceError> {
    svc.confirm_policy(true, true)
        .await
        .expect("the policy heads");
    let wt = svc.web_template(TEMPLATE_ID).await?;
    Ok(serde_json::to_value(&*wt).expect("the Web Template serializes"))
}

#[tokio::test]
async fn a_template_deleted_and_replaced_on_another_replica_is_not_served_from_the_old_copy() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    let (first, other) = templates();

    writer
        .template_adl14_upload(&first)
        .await
        .expect("the first upload");
    let before = served(&reader)
        .await
        .expect("the reader builds the template");

    writer
        .admin_template_delete(TEMPLATE_ID)
        .await
        .expect("the delete");
    let deleted = served(&reader).await;
    assert!(
        matches!(deleted, Err(ServiceError::Unprocessable { .. })),
        "a deleted template is unknown to the next request of the other replica: {deleted:?}"
    );

    writer
        .template_adl14_upload(&other)
        .await
        .expect("the upload of other content under the same id");
    let after = served(&reader).await.expect("the reader builds it again");
    let stored = served(&writer).await.expect("the writer's own build");
    assert_ne!(before, after, "the replaced content is served");
    assert_eq!(after, stored, "both replicas serve what the store holds");
}

/// A COMPOSITION in `ehr` whose OBSERVATION was created with `archetype`.
async fn commit_under(svc: &FerroEhrService, ehr: ferroehr::ids::EhrId, archetype: &str) {
    let mut c = base_composition();
    if let Some(details) = c
        .get_mut("archetype_details")
        .and_then(Value::as_object_mut)
    {
        details.remove("template_id");
    }
    if let Some(obj) = c.as_object_mut() {
        obj.remove("uid");
    }
    c["content"][0]["archetype_node_id"] = json!(archetype);
    c["content"][0]["archetype_details"] = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
        "rm_version": "1.1.0",
    });
    svc.create_composition(ehr, uv(&c, "249", None))
        .await
        .unwrap_or_else(|e| panic!("the composition under {archetype}: {e:?}"));
}

/// How many OBSERVATIONs of `ehr` a request of `svc` matches by `archetype`.
async fn matched(
    svc: &FerroEhrService,
    ehr: ferroehr::ids::EhrId,
    archetype: &str,
) -> Result<i64, ServiceError> {
    svc.confirm_policy(true, true)
        .await
        .expect("the policy heads");
    let aql = format!("SELECT COUNT(*) FROM EHR e CONTAINS OBSERVATION o[{archetype}]");
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        ..AqlQueryRequest::default()
    };
    let result = svc.execute_aql(&aql, None, &request).await?;
    let rows = result.result_set.to_value().expect("the result set");
    Ok(rows["rows"][0][0].as_i64().expect("a count"))
}

const PARENT: &str = "openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0";
const CHILD: &str = "openEHR-EHR-OBSERVATION.hdl_result.v1.0.0";

#[tokio::test]
async fn an_artefact_written_on_another_replica_decides_the_next_query() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    writer
        .template_adl2_upload(&adl2_source("archetype", PARENT, None))
        .await
        .expect("the parent archetype");
    let ehr = crate::fixtures::create_ehr(&writer).await;
    commit_under(&writer, ehr, "openEHR-EHR-OBSERVATION.hdl_result.v1").await;

    assert_eq!(
        matched(&reader, ehr, PARENT).await.expect("the query"),
        0,
        "no stored artefact makes the data a specialisation of the parent yet"
    );
    writer
        .template_adl2_upload(&adl2_source(
            "archetype",
            CHILD,
            Some("openEHR-EHR-OBSERVATION.lipid_panel.v1"),
        ))
        .await
        .expect("the specialised archetype");
    assert_eq!(
        matched(&reader, ehr, PARENT).await.expect("the query"),
        1,
        "the next query of the other replica widens the parent with the stored child"
    );

    writer
        .delete_artefact(CHILD)
        .await
        .expect("the child is deleted");
    assert_eq!(
        matched(&reader, ehr, PARENT).await.expect("the query"),
        0,
        "a deleted artefact no longer widens the next query of the other replica"
    );
}

#[tokio::test]
async fn a_lineage_read_that_fails_refuses_the_query() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool());
    let ehr = crate::fixtures::create_ehr(&svc).await;
    sqlx::query("ALTER TABLE adl2_artefact RENAME TO adl2_artefact_away")
        .execute(&db.pool())
        .await
        .expect("the artefact store is taken away");
    let refused = matched(&svc, ehr, PARENT).await;
    assert!(
        matches!(refused, Err(ServiceError::Overloaded(_))),
        "an unreadable lineage refuses the query with 503: {refused:?}"
    );
}
