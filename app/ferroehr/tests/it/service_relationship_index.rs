// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests against a real PostgreSQL 18 (shared testkit harness): the
//! `PARTY_RELATIONSHIP` CRUD + versioning + `VERSIONED_OBJECT` + revision
//! history + error cases (driven through the `PartyRelationshipService` seam).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixtures::uv;
use crate::fixtures::{uid, vo_of};
use crate::typed_body::typed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::refusal::{Missing, Refusal};

/// The bare versioned-object UUID of a versioned-object body.
fn vo_uuid(v: &Value) -> String {
    vo_of(uid(v)).to_owned()
}

/// The DB's current instant (`SELECT now()`) as a `jiff::Timestamp`. Time-travel
/// probes MUST anchor their reference instant on this server clock — the same
/// clock that stamps `version.committed_at` — never on the test-process wall
/// clock: a client/DB clock skew under parallel test load would race
/// the at-time read against the version validity intervals.
async fn db_now(pool: &PgPool) -> jiff::Timestamp {
    sqlx::query_scalar::<_, jiff_sqlx::Timestamp>("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("db now()")
        .to_jiff()
}

fn party_ref(id: &str) -> Value {
    json!({
        "_type": "PARTY_REF",
        "namespace": "demographic",
        "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

fn relationship(name: &str, source: &str, target: &str) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "name": { "_type": "DV_TEXT", "value": name },
        "source": party_ref(source),
        "target": party_ref(target)
    })
}

// ─── PARTY_RELATIONSHIP ───────────────────────────────────────────────────────

/// Whether a live relationship is served under `vo_id`: the read answers a
/// body, where a deleted current version reads empty.
async fn relationship_is_live(svc: &FerroEhrService, vo_id: ferroehr::ids::VoId) -> bool {
    !svc.party_relationship_get(vo_id.to_string(), None)
        .await
        .expect("the relationship reads")
        .is_empty()
}

/// The relationship round trip through the service calls the REST routes
/// make: create → read (current, by version) → update → delete, checking
/// that a deleted relationship is no longer present.
#[tokio::test]
async fn relationship_typed_calls_round_trip() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let src = "11111111-1111-4111-8111-111111111111";
    let tgt = "22222222-2222-4222-8222-222222222222";

    // The create → the new VERSIONED_OBJECT's id.
    let vo_id = Box::pin(crate::fixtures::create_relationship(
        &svc,
        uv(&relationship("parent-of", src, tgt), "249", None),
    ))
    .await
    .expect("create_party_relationship");

    assert!(relationship_is_live(&svc, vo_id).await, "live relationship");
    let got = svc
        .party_relationship_get(vo_id.to_string(), None)
        .await
        .expect("get_party_relationship")
        .body;
    assert_eq!(got["_type"], "PARTY_RELATIONSHIP");
    assert_eq!(got["name"]["value"], "parent-of");

    let v1 = got["uid"]["value"].as_str().expect("uid.value").to_owned();
    let by_ver = svc
        .party_relationship_version_get_by_id(vo_id.to_string(), v1.clone())
        .await
        .expect("get_party_relationship_at_version")
        .body;
    assert_eq!(by_ver["_type"], "ORIGINAL_VERSION");

    // The update under `If-Match` = the current version → the new version uid.
    let update: openehr_its::rest::generated::common::UpdateVersion<
        openehr_rm::prelude::PartyRelationship,
    > = uv(&relationship("guardian-of", src, tgt), "251", Some(&v1));
    let committal = crate::fixtures::envelope_committal(&update);
    let v2 = svc
        .party_relationship_update(
            vo_id.to_string(),
            v1.clone(),
            update.data.expect("a PARTY_RELATIONSHIP"),
            Some(committal),
        )
        .await
        .expect("update_party_relationship")
        .meta
        .expect("the update answers its version")
        .uid;
    assert!(v2.ends_with("::2"), "second version, got {v2}");
    let got2 = svc
        .party_relationship_get(vo_id.to_string(), None)
        .await
        .expect("get after update")
        .body;
    assert_eq!(got2["name"]["value"], "guardian-of");

    // The delete of the latest version → the relationship is no longer present.
    let del = svc
        .party_relationship_delete(v2.clone(), None, None)
        .await
        .expect("delete_party_relationship")
        .meta
        .map(|meta| meta.uid)
        .unwrap_or_default();
    assert!(!del.is_empty(), "delete returns the deleted version uid");
    assert!(
        !relationship_is_live(&svc, vo_id).await,
        "a deleted relationship is no longer present"
    );
    // A deleted relationship reads empty, served as `204`.
    let after = svc
        .party_relationship_get(vo_id.to_string(), None)
        .await
        .expect("a deleted relationship reads");
    assert!(
        after.is_empty(),
        "get after delete serves no body, got {after:?}"
    );
}

#[tokio::test]
async fn relationship_lifecycle_end_to_end() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let src = "11111111-1111-4111-8111-111111111111";
    let tgt = "22222222-2222-4222-8222-222222222222";

    // create → v1
    let created = svc
        .party_relationship_create(typed(&relationship("parent-of", src, tgt)), None)
        .await
        .expect("create relationship");
    assert_eq!(created.body["_type"], "PARTY_RELATIONSHIP");
    assert_eq!(created.body["source"]["id"]["value"], src);
    assert_eq!(created.body["target"]["id"]["value"], tgt);
    let ovid_v1 = uid(&created.body).to_owned();
    let vo = vo_uuid(&created.body);
    assert!(ovid_v1.ends_with("::1"), "first version, got {ovid_v1}");
    assert_eq!(
        created.meta.as_ref().expect("meta").ehr_id,
        "",
        "a relationship has no EHR scope"
    );

    // get current (bare HIER_OBJECT_ID)
    let got = svc
        .party_relationship_get(vo.clone(), None)
        .await
        .expect("get relationship");
    assert_eq!(got.body["name"]["value"], "parent-of");

    // get by OBJECT_VERSION_ID
    let by_ovid = svc
        .party_relationship_get(ovid_v1.clone(), None)
        .await
        .expect("get by ovid");
    assert_eq!(uid(&by_ovid.body), ovid_v1);

    // time-travel: capture a time inside v1 FROM THE DB CLOCK, then update. The
    // reference instant MUST come from the server clock (the clock that stamps
    // `version.committed_at`), never the test-process wall clock — a client/DB
    // clock skew under parallel-load testcontainers races the at-time read
    // against the version validity intervals (a real flake this fixes).
    let t_v1 = db_now(&pool).await;
    // A short gap so v2's commit transaction timestamp is strictly greater than
    // t_v1 even at microsecond resolution (both are now on the DB clock).
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    // update (If-Match = current OVID) → v2
    let updated = svc
        .party_relationship_update(
            vo.clone(),
            ovid_v1.clone(),
            typed(&relationship("guardian-of", src, tgt)),
            None,
        )
        .await
        .expect("update relationship");
    let ovid_v2 = uid(&updated.body).to_owned();
    assert!(ovid_v2.ends_with("::2"), "second version, got {ovid_v2}");
    assert_eq!(updated.body["name"]["value"], "guardian-of");

    // at-time read returns v1
    let at_v1 = svc
        .party_relationship_get(vo.clone(), Some(t_v1.to_string()))
        .await
        .expect("at-time");
    assert_eq!(at_v1.body["name"]["value"], "parent-of");

    // stale If-Match → 412
    let stale = svc
        .party_relationship_update(
            vo.clone(),
            ovid_v1.clone(),
            typed(&relationship("stale", src, tgt)),
            None,
        )
        .await;
    assert!(
        matches!(stale, Err(ServiceError::VersionConflict(_))),
        "stale update, got {stale:?}"
    );

    // VERSIONED_OBJECT + revision history + version-by-id
    let vp = svc
        .versioned_party_relationship_get(vo.clone())
        .await
        .expect("versioned relationship");
    assert_eq!(vp.body["_type"], "VERSIONED_OBJECT");
    let rh = svc
        .party_relationship_revision_history(vo.clone())
        .await
        .expect("revision_history");
    assert_eq!(rh.body["items"].as_array().expect("items").len(), 2);
    let ov = svc
        .party_relationship_version_get_by_id(vo.clone(), ovid_v1.clone())
        .await
        .expect("version by id");
    assert_eq!(ov.body["_type"], "ORIGINAL_VERSION");

    // version at-time (VERSION resource + ETag/Location meta)
    let vat = svc
        .party_relationship_version_get_at_time(vo.clone(), None)
        .await
        .expect("version at time");
    assert_eq!(vat.body["_type"], "ORIGINAL_VERSION");
    assert!(vat.meta.is_some());

    // delete (mandatory OBJECT_VERSION_ID) → 204; subsequent get → 204 (Null)
    let deleted = svc
        .party_relationship_delete(ovid_v2.clone(), None, None)
        .await
        .expect("delete");
    assert!(deleted.is_empty());
    let after = svc
        .party_relationship_get(vo.clone(), None)
        .await
        .expect("get after delete");
    assert!(after.is_empty(), "deleted current read is 204 (Null body)");
}

#[tokio::test]
async fn relationship_error_cases() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // unknown id → 404
    let unknown = svc
        .party_relationship_get(Uuid::now_v7().to_string(), None)
        .await;
    assert!(
        matches!(
            unknown,
            Err(ServiceError::NotFound(Refusal {
                kind: Missing::VersionedObject,
                ..
            }))
        ),
        "unknown relationship is 404, got {unknown:?}"
    );

    // Invalid content (missing `target`): `PARTY_RELATIONSHIP.target` is a
    // mandatory `PARTY_REF` (RM demographic `party_relationship.adoc`
    // §Attributes), so a body without it is not a `PARTY_RELATIONSHIP` at all.
    // The typed seam's argument is the RM value, which makes the defect
    // unrepresentable past the door; the refusal is the strict canonical
    // reader's — the PARSE class, answered `400` on the wire (ITS-REST
    // overview `Requests_and_responses.md` §HTTP status codes: content that
    // "could not be parsed or is invalid"), never the semantic `422`.
    let mut bad = relationship("no-target", "src", "tgt");
    bad.as_object_mut()
        .expect("the fixture is an object")
        .remove("target");
    let refused =
        openehr_its::json::from_canonical_value::<openehr_rm::prelude::PartyRelationship>(&bad);
    assert!(
        refused.is_err(),
        "a PARTY_RELATIONSHIP without its mandatory target must be refused, got {refused:?}"
    );

    // a PERSON is not a relationship → wrong-kind get is 404
    let created = svc
        .party_relationship_create(typed(&relationship("r", "a", "b")), None)
        .await
        .expect("create");
    let vo = vo_uuid(&created.body);
    // reading the same id as a relationship works; reading a fresh (unknown) id
    // as versioned_party_relationship is 404
    let vp_unknown = svc
        .versioned_party_relationship_get(Uuid::now_v7().to_string())
        .await;
    assert!(matches!(
        vp_unknown,
        Err(ServiceError::NotFound(Refusal {
            kind: Missing::VersionedObject,
            ..
        }))
    ));
    assert!(svc.party_relationship_get(vo, None).await.is_ok());
}

#[tokio::test]
async fn relationship_via_demographic_contribution() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // A demographic CONTRIBUTION accepts a PARTY_RELATIONSHIP version (the
    // ehr-less scope now covers relationships as well as party roots).
    let body = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "lifecycle_state": crate::fixtures::lifecycle("532"),
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": {
                    "_type": "DV_CODED_TEXT",
                    "value": "creation",
                    "defining_code": {
                        "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249"
                    }
                },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" }
            },
            "data": relationship("colleague-of", "p1", "p2")
        }],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" } }
    });

    let created = svc
        .demographic_contribution_create(body)
        .await
        .expect("create demographic contribution with a relationship");
    assert_eq!(created.body["_type"], "CONTRIBUTION");
    let versions = created.body["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["type"], "PARTY_RELATIONSHIP");
}
