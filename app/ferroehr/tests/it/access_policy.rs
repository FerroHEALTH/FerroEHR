// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's `ACCESS_POLICY` (#5512): a versioned object committed in
//! contributions with their audit, checked strictly against the draft BMM,
//! its default archetype stored at boot, and held in memory on every replica
//! with `LISTEN`/`NOTIFY` invalidation. The draft package is no openEHR
//! release: our own design/extension.

use std::time::Duration;

use ferroehr::access_policy::ARCHETYPE;
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::demographic::default_definitions::Seeded;
use serde_json::{Value, json};

const HOSPITAL: &str = "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7";

/// A policy over the default archetype, with `care` as its care relationship
/// requirement.
fn policy(care: bool) -> Value {
    json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": ARCHETYPE,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": ARCHETYPE },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": HOSPITAL } },
        "classifications": [{ "_type": "ACCESS_CLASSIFICATION",
            "name": { "_type": "DV_TEXT", "value": "HIV results" }, "archetype_node_id": "id2",
            "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
            "sensitivities": [{ "_type": "DV_CODED_TEXT", "value": "HIV",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID",
                        "value": "http://terminology.hl7.org/CodeSystem/v3-ActCode" },
                    "code_string": "HIV" } }] }],
        "care_relationship_required": care,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
        "purpose_required": false
    })
}

/// One replica over `pool`.
fn replica(pool: &sqlx::PgPool) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
}

/// The version uid of the first write.
async fn create(svc: &FerroEhrService, body: &Value) -> String {
    svc.put_access_policy(&PolicyPrecondition::Create, body)
        .await
        .expect("the first version commits")
        .version_uid
}

#[tokio::test]
async fn the_default_archetype_is_stored_at_boot_against_the_rm_with_the_drafts() {
    let (_db, svc) = crate::fixtures::service().await;
    let seeded = svc
        .seed_default_definitions()
        .await
        .expect("a boot seeds the default definitions");
    let id = format!("{ARCHETYPE}.0.0");
    assert!(
        seeded.contains(&Seeded::Stored(id.clone())),
        "{id} is stored: {seeded:?}"
    );
    let again = svc.seed_default_definitions().await.expect("a second boot");
    assert!(
        again.iter().all(|s| matches!(s, Seeded::Kept(_))),
        "{again:?}"
    );
}

#[tokio::test]
async fn the_policy_is_a_versioned_object_with_contributions_and_audit() {
    let (_db, svc) = crate::fixtures::service().await;
    assert!(svc.access_policy_current().await.expect("a read").is_none());
    assert!(
        svc.access_policy_history()
            .await
            .expect("a read")
            .is_empty()
    );
    assert!(
        svc.access_policy_versioned()
            .await
            .expect("a read")
            .is_none()
    );
    assert_eq!(svc.access_policy().in_force().version, 0);

    let first = create(&svc, &policy(false)).await;
    assert!(first.ends_with("::1"), "{first}");
    assert_eq!(svc.access_policy().in_force().version, 1);
    let second = svc
        .put_access_policy(&PolicyPrecondition::Replace(first.clone()), &policy(true))
        .await
        .expect("the second version commits");
    assert!(
        second.version_uid.ends_with("::2"),
        "{}",
        second.version_uid
    );
    assert_eq!(second.data["care_relationship_required"], true);
    let in_force = svc.access_policy().in_force();
    assert_eq!(in_force.version, 2);
    assert!(
        in_force
            .policy
            .as_ref()
            .is_some_and(|p| p.care_relationship_required)
    );

    let history = svc.access_policy_history().await.expect("the history");
    assert_eq!(history.len(), 2);
    let newest = &history[0].original_version;
    assert_eq!(newest["_type"], "ORIGINAL_VERSION");
    assert_eq!(newest["uid"]["value"], second.version_uid.as_str());
    assert_eq!(newest["preceding_version_uid"]["value"], first.as_str());
    assert_eq!(newest["contribution"]["type"], "CONTRIBUTION");
    assert_eq!(newest["commit_audit"]["_type"], "AUDIT_DETAILS");
    assert_eq!(
        newest["commit_audit"]["change_type"]["defining_code"]["code_string"],
        "251"
    );
    assert_eq!(
        newest["lifecycle_state"]["defining_code"]["code_string"],
        "532"
    );
    assert_eq!(newest["data"]["_type"], "ACCESS_POLICY");
    let oldest = &history[1].original_version;
    assert!(oldest.get("preceding_version_uid").is_none(), "{oldest}");
    assert_eq!(
        oldest["commit_audit"]["change_type"]["defining_code"]["code_string"],
        "249"
    );
    assert_ne!(
        newest["contribution"]["id"]["value"], oldest["contribution"]["id"]["value"],
        "one contribution per version"
    );

    let versioned = svc
        .access_policy_versioned()
        .await
        .expect("a read")
        .expect("the versioned object");
    assert_eq!(versioned.body["_type"], "VERSIONED_ACCESS_POLICY");
    assert_eq!(versioned.body["owner_id"]["type"], "ORGANISATION");
    assert!(
        first.starts_with(&versioned.uid),
        "{first} {}",
        versioned.uid
    );
}

#[tokio::test]
async fn a_write_over_a_version_that_is_not_the_newest_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let refused = svc
        .put_access_policy(
            &PolicyPrecondition::Replace(format!("{HOSPITAL}::x::1")),
            &policy(false),
        )
        .await
        .expect_err("nothing to replace");
    assert!(
        matches!(refused, ServiceError::VersionConflict(_)),
        "{refused:?}"
    );
    let first = create(&svc, &policy(false)).await;
    let refused = svc
        .put_access_policy(&PolicyPrecondition::Create, &policy(true))
        .await
        .expect_err("a second first version");
    assert!(
        matches!(refused, ServiceError::VersionConflict(_)),
        "{refused:?}"
    );
    svc.put_access_policy(&PolicyPrecondition::Replace(first.clone()), &policy(true))
        .await
        .expect("the second version");
    let refused = svc
        .put_access_policy(&PolicyPrecondition::Replace(first), &policy(false))
        .await
        .expect_err("a stale If-Match");
    assert!(
        matches!(refused, ServiceError::VersionConflict(_)),
        "{refused:?}"
    );
    let refused = svc
        .put_access_policy(
            &PolicyPrecondition::Replace("not a version".to_owned()),
            &policy(false),
        )
        .await
        .expect_err("no OBJECT_VERSION_ID");
    assert!(
        matches!(refused, ServiceError::BadRequest(_)),
        "{refused:?}"
    );
    assert_eq!(svc.access_policy().in_force().version, 2);
}

/// A defect by name, and how a valid policy is broken to carry it.
type Defect = (&'static str, fn(&mut Value));

#[tokio::test]
async fn a_policy_that_breaks_the_bmm_is_refused_whole() {
    let (_db, svc) = crate::fixtures::service().await;
    let cases: [Defect; 7] = [
        ("an undeclared member", |p| p["colour"] = json!("red")),
        ("a missing mandatory member", |p| {
            p.as_object_mut()
                .expect("an object")
                .remove("purpose_required");
        }),
        ("an empty optional list", |p| {
            p["classifications"] = json!([]);
        }),
        ("Grace_valid", |p| {
            p["relationship_grace"]["value"] = json!("-P1D");
        }),
        ("ACCESS_CLASSIFICATION.No_objects", |p| {
            p["classifications"][0]["data"]["objects"] = json!([{ "_type": "OBJECT_REF",
                "namespace": "local", "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": HOSPITAL } }]);
        }),
        ("a controller that is no ORGANISATION", |p| {
            p["controller"]["type"] = json!("PERSON");
        }),
        ("a root that is not its archetype's", |p| {
            p["archetype_node_id"] = json!("openEHR-EHR-ACCESS_POLICY.other.v1");
        }),
    ];
    for (defect, break_it) in cases {
        let mut body = policy(false);
        break_it(&mut body);
        let refused = svc
            .put_access_policy(&PolicyPrecondition::Create, &body)
            .await
            .expect_err(defect);
        assert!(
            matches!(refused, ServiceError::BadRequest(_)),
            "{defect}: {refused:?}"
        );
    }
    assert!(
        svc.access_policy_history()
            .await
            .expect("a read")
            .is_empty()
    );
    let first = create(&svc, &policy(false)).await;
    let mut other = policy(false);
    other["controller"]["id"]["value"] = json!("3d4e5f6a-7b8c-4d9e-8f0a-b1c2d3e4f5a6");
    let refused = svc
        .put_access_policy(&PolicyPrecondition::Replace(first), &other)
        .await
        .expect_err("another controller");
    assert!(
        matches!(refused, ServiceError::BadRequest(_)),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_change_on_one_replica_decides_the_next_read_on_another() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    let listened = reader.access_policy().listened();
    listened.spawn_listener();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !listened.is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the listener connects"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    reader.refresh_access_policy().await.expect("a read");
    assert_eq!(reader.access_policy().in_force().version, 0);
    let seen = listened.notifications();
    create(&writer, &policy(true)).await;
    assert!(
        listened.notified_after(seen, Duration::from_secs(10)).await,
        "the commit notifies"
    );
    reader
        .refresh_access_policy()
        .await
        .expect("the notified read");
    let in_force = reader.access_policy().in_force();
    assert_eq!(in_force.version, 1);
    assert!(
        in_force
            .policy
            .as_ref()
            .is_some_and(|p| p.care_relationship_required)
    );
}

#[tokio::test]
async fn a_replica_without_its_listener_reads_on_every_request() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    assert!(!reader.access_policy().listened().is_listening());
    reader.refresh_access_policy().await.expect("a read");
    create(&writer, &policy(false)).await;
    reader.refresh_access_policy().await.expect("a read");
    assert_eq!(reader.access_policy().in_force().version, 1);
}
