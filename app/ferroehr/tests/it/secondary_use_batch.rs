// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The batch surface a secondary-use loader reads, against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! `FerroBRIDGE` loads the OMOP CDM in batches over AQL: `VERSION` with a
//! `commit_audit/time_committed` predicate is a class expression of the query
//! language (QUERY master03 §FROM), the loader binds the instant of its last
//! run as `$since`, and it pages with `offset` and `fetch` (ITS-REST 1.1.0
//! query `execute_ad_hoc_query`). The batch answers a population query, so
//! the restriction of processing (GDPR Art. 18) and the research objection
//! (Art. 21(6)) take an EHR out of it (`docs/law/eu/gdpr/text.html`), and its
//! rows carry no identifier of the subject beyond the `ehr_id`. Which marks
//! leave a population query is our own design; the query shape is the
//! specification's.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::{composition, uv};

/// The loader's batch query: every latest composition version committed after
/// `$since`, oldest first, ties broken by the version uid so the order is
/// total and an offset window is stable.
const BATCH: &str = "SELECT e/ehr_id/value AS ehr_id, v/uid/value AS version_uid, \
     vo/uid/value AS versioned_object_uid, c AS composition \
     FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v[LATEST_VERSION] \
     CONTAINS COMPOSITION c \
     WHERE v/commit_audit/time_committed/value > $since \
     ORDER BY v/commit_audit/time_committed/value, v/uid/value";

/// The prefix of the synthetic subject identifier each EHR here is created for.
const SUBJECT: &str = "synthetic-subject-";

/// An EHR whose `EHR_STATUS.subject` names [`SUBJECT`] followed by `n` (RM ehr
/// master04 §EHR Status: `subject` 1..1 `PARTY_SELF`).
async fn ehr(svc: &FerroEhrService, n: u8) -> EhrId {
    let subject = format!("{SUBJECT}{n:04}");
    let status = openehr_its::json::from_canonical_value(&json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "urn:ferroehr:test-subjects",
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": subject, "scheme": "test" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    }))
    .expect("the subject-bearing EHR_STATUS decodes");
    Box::pin(svc.create_ehr_meta(Some(status), None))
        .await
        .map(|(id, _)| id)
        .expect("create the EHR")
}

/// Commit a composition named `name`; returns its version uid.
async fn commit(svc: &FerroEhrService, ehr_id: EhrId, name: &str) -> String {
    let uid = svc
        .create_composition(ehr_id, uv(&composition(name), "249", None))
        .await
        .expect("commit the composition")
        .version_uid();
    // Separates the commit instants, so `> $since` has an instant to split on.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    uid
}

/// Run the batch from `since` with the window `offset`/`fetch`; returns the
/// rows.
async fn batch(
    svc: &FerroEhrService,
    since: &str,
    offset: Option<i64>,
    fetch: Option<i64>,
) -> Vec<Value> {
    let result = svc
        .execute_aql(
            BATCH,
            None,
            &AqlQueryRequest {
                parameters: BTreeMap::from([("since".to_owned(), json!(since))]),
                offset,
                fetch,
                ..AqlQueryRequest::default()
            },
        )
        .await
        .expect("the batch query runs")
        .result_set;
    result["rows"].as_array().cloned().unwrap_or_default()
}

/// The `time_committed` of a version, read back through AQL.
async fn committed_at(svc: &FerroEhrService, version_uid: &str) -> String {
    let result = svc
        .execute_aql(
            "SELECT v/commit_audit/time_committed/value \
             FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
             WHERE v/uid/value = $uid",
            None,
            &AqlQueryRequest {
                parameters: BTreeMap::from([("uid".to_owned(), json!(version_uid))]),
                ..AqlQueryRequest::default()
            },
        )
        .await
        .expect("read the commit instant")
        .result_set;
    result["rows"][0][0]
        .as_str()
        .expect("time_committed is a string")
        .to_owned()
}

/// Every version committed after the instant is in the batch, oldest first,
/// whether it is a new composition or a new version of an older one; nothing
/// committed before it is; an EHR under a restriction of processing or a
/// research objection is not; the pages of a window concatenate to the whole
/// batch; and no row carries the subject's identifier.
#[tokio::test]
async fn the_batch_returns_every_version_after_the_instant_minus_the_marked_ehrs() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let open = ehr(&svc, 1).await;
    let restricted = ehr(&svc, 2).await;
    let objected = ehr(&svc, 3).await;

    let before = commit(&svc, open, "before the instant").await;
    let since = committed_at(&svc, &before).await;

    let fresh = commit(&svc, open, "after the instant").await;
    let vo = before.split("::").next().expect("object id").to_owned();
    let updated = svc
        .update_composition(
            open,
            vo.parse::<VoId>().expect("vo id"),
            uv(&composition("before, updated after"), "251", Some(&before)),
        )
        .await
        .expect("update the older composition")
        .version_uid();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    commit(&svc, restricted, "restricted").await;
    commit(&svc, objected, "objected").await;
    let last = commit(&svc, open, "last").await;

    svc.restrict_processing(&restricted.to_string(), None, "gdpr-18-1-d", None)
        .await
        .expect("restrict the EHR");
    svc.set_research_objection(&objected.to_string(), true, None)
        .await
        .expect("record the objection");

    let rows = batch(&svc, &since, None, None).await;
    let uids: Vec<&str> = rows
        .iter()
        .map(|row| row[1].as_str().expect("version uid"))
        .collect();
    assert_eq!(
        uids,
        vec![fresh.as_str(), updated.as_str(), last.as_str()],
        "the versions after the instant, oldest first, without the marked EHRs"
    );
    for row in &rows {
        assert_eq!(row[0], json!(open.to_string()), "the ehr_id column");
        assert_eq!(
            row[2].as_str().expect("vo uid"),
            row[1]
                .as_str()
                .expect("version uid")
                .split("::")
                .next()
                .expect("object id"),
            "the versioned object of the version"
        );
    }

    let mut paged = Vec::new();
    for offset in 0..4 {
        paged.extend(batch(&svc, &since, Some(offset), Some(1)).await);
    }
    assert_eq!(paged, rows, "the pages of the window make up the batch");

    let text = serde_json::to_string(&rows).expect("serialise the rows");
    assert!(
        !text.contains(SUBJECT),
        "no row carries a subject's identifier: {text}"
    );
}

/// A `VERSIONED_OBJECT` variable answers its `uid`, the `object_id` part of
/// its versions' uids, and its `owner_id`, the EHR that owns it (RM common
/// master06 §`VERSIONED_OBJECT`); `ALL_VERSIONS` below it answers one row per
/// version of the same object; a path it does not hold is refused, never
/// answered empty.
#[tokio::test]
async fn a_versioned_object_variable_answers_its_uid_and_owner() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let owner = ehr(&svc, 1).await;
    let first = commit(&svc, owner, "first").await;
    let vo = first.split("::").next().expect("object id").to_owned();
    let second = svc
        .update_composition(
            owner,
            vo.parse::<VoId>().expect("vo id"),
            uv(&composition("second"), "251", Some(&first)),
        )
        .await
        .expect("update the composition")
        .version_uid();

    let request = AqlQueryRequest::default();
    let run = |q: &'static str| svc.execute_aql(q, None, &request);
    let rows = run("SELECT vo/uid/value, vo/owner_id/id/value, v/uid/value \
         FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v[ALL_VERSIONS] \
         CONTAINS COMPOSITION c ORDER BY v/uid/value")
    .await
    .expect("the versioned object query runs")
    .result_set["rows"]
        .clone();
    assert_eq!(
        rows,
        json!([
            [vo, owner.to_string(), first],
            [vo, owner.to_string(), second]
        ])
    );

    let refused = run("SELECT vo/time_created/value \
         FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS COMPOSITION c")
    .await
    .expect_err("a path the versioned object does not answer is refused");
    assert!(
        refused
            .message()
            .contains("VERSIONED_OBJECT path `time_created/value`"),
        "{}",
        refused.message()
    );
}
