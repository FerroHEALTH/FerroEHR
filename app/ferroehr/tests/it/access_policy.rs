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
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS;
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
                        "value": "HL7_ActCode" },
                    "code_string": "HIV" } }] }],
        "care_relationship_required": care,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
        "purpose_required": false
    })
}

/// A coded value of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
            "code_string": code } })
}

/// The nurse rule: the role `NURSE` of `urn:example:roles` may not commit
/// the diagnosis template.
fn nurse_rule() -> Value {
    json!({ "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Nurses do not record diagnoses" },
        "archetype_node_id": "id15",
        "kind": coded(TERMINOLOGY_ID_ACCESS, "controller_rule"),
        "effect": coded(openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "deny"),
        "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "role": coded("urn:example:roles", "NURSE") }],
        "operations": [coded(TERMINOLOGY_ID_ACCESS, "write")],
        "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["diagnosis.v1"] }],
        "emergency_override": false })
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
    assert!(first.ends_with("::1"), "the first trunk version");
    assert_eq!(svc.access_policy().in_force().version, 1);
    let second = svc
        .put_access_policy(&PolicyPrecondition::Replace(first.clone()), &policy(true))
        .await
        .expect("the second version commits");
    assert!(
        second.version_uid.ends_with("::2"),
        "the second trunk version"
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
        "the version names the versioned object"
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
    let cases: [Defect; 11] = [
        ("Rules_kind", |p| {
            let mut rule = nurse_rule();
            rule["kind"] = coded(TERMINOLOGY_ID_ACCESS, "consent");
            p["rules"] = json!([rule]);
        }),
        ("a rule's role in the draft access terminology", |p| {
            let mut rule = nurse_rule();
            rule["applies_to"][0]["role"] = coded(TERMINOLOGY_ID_ACCESS, "NURSE");
            p["rules"] = json!([rule]);
        }),
        ("an empty list of rules", |p| p["rules"] = json!([])),
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
        ("a sensitivity in the draft access terminology", |p| {
            p["classifications"][0]["sensitivities"][0]["defining_code"]["terminology_id"]["value"] =
                json!(TERMINOLOGY_ID_ACCESS);
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
async fn a_policy_carries_the_controllers_rules() {
    let (_db, svc) = crate::fixtures::service().await;
    let mut body = policy(false);
    body["rules"] = json!([nurse_rule()]);
    create(&svc, &body).await;
    svc.read_access_policy().await.expect("the policy reads");
    let in_force = svc.access_policy().in_force();
    let rules = in_force
        .policy
        .as_deref()
        .and_then(|policy| policy.rules.as_deref())
        .expect("the rules are in force");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].applies_to.as_deref().map(<[_]>::len),
        Some(1),
        "the rule reads back whole"
    );
}

#[tokio::test]
async fn a_change_on_one_replica_decides_the_next_read_on_another() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    let listened = reader.access_policy().listened();
    crate::policy_fixtures::listen(&db, &[std::sync::Arc::clone(listened)]);
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

/// A policy written on another replica decides the next request of a replica
/// whose notification never arrives: the request reads the newest version in
/// the store before it decides (#6048).
#[tokio::test]
async fn a_change_whose_notification_is_withheld_decides_the_next_request() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    let _elsewhere =
        crate::policy_fixtures::deaf(&[std::sync::Arc::clone(reader.access_policy().listened())])
            .await;
    reader.confirm_policy(false, true).await.expect("a read");
    assert_eq!(reader.access_policy().in_force().version, 0);
    create(&writer, &policy(true)).await;
    assert_eq!(
        reader.access_policy().listened().notifications(),
        0,
        "the reader heard nothing"
    );
    reader
        .confirm_policy(false, true)
        .await
        .expect("the request's read");
    let in_force = reader.access_policy().in_force();
    assert_eq!(in_force.version, 1);
    assert!(
        in_force
            .policy
            .as_ref()
            .is_some_and(|p| p.care_relationship_required)
    );
}

/// A register written on another replica decides the next request of a
/// replica whose notification never arrives (#6048).
#[tokio::test]
async fn a_register_whose_notification_is_withheld_decides_the_next_request() {
    use ferroehr::policy_register::Register;
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool());
    let reader = replica(&db.pool());
    let _elsewhere = crate::policy_fixtures::deaf(&[std::sync::Arc::clone(
        reader.policy_registers().listened(),
    )])
    .await;
    reader.confirm_policy(true, false).await.expect("a read");
    assert_eq!(reader.policy_registers().version(Register::AccessDelay), 0);
    writer
        .put_policy_register(
            Register::AccessDelay,
            0,
            r#"{"enabled": true, "categories": ["Patient-Summaries"], "max_delay": "72h"}"#,
        )
        .await
        .expect("the register is written");
    assert_eq!(
        reader.policy_registers().listened().notifications(),
        0,
        "the reader heard nothing"
    );
    reader
        .confirm_policy(true, false)
        .await
        .expect("the request's read");
    assert_eq!(reader.policy_registers().version(Register::AccessDelay), 1);
}

/// The policy revision moves once in each transaction that writes a register
/// or the access policy, the one row a request confirms its policy copies
/// with; a replica whose notification never arrives still installs both
/// changes at its next request (#6251).
#[tokio::test]
async fn the_policy_revision_moves_once_with_every_policy_write() {
    use ferroehr::policy_register::Register;
    use ferroehr::storage::policy_register::revisions;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let writer = replica(&pool);
    let reader = replica(&pool);
    let _elsewhere = crate::policy_fixtures::deaf(&[
        std::sync::Arc::clone(reader.policy_registers().listened()),
        std::sync::Arc::clone(reader.access_policy().listened()),
    ])
    .await;
    reader.confirm_policy(true, true).await.expect("a read");
    assert_eq!(revisions(&pool).await.expect("a read").policy, 0);
    writer
        .put_policy_register(
            Register::AccessDelay,
            0,
            r#"{"enabled": true, "categories": ["Patient-Summaries"], "max_delay": "72h"}"#,
        )
        .await
        .expect("the register is written");
    assert_eq!(revisions(&pool).await.expect("a read").policy, 1);
    create(&writer, &policy(true)).await;
    assert_eq!(revisions(&pool).await.expect("a read").policy, 2);
    reader
        .confirm_policy(true, true)
        .await
        .expect("the request's read");
    assert_eq!(reader.policy_registers().version(Register::AccessDelay), 1);
    assert_eq!(reader.access_policy().in_force().version, 1);
}

/// A policy write installs, and confirms, every change up to the revision its
/// own transaction produced: a replica that heard nothing of another
/// replica's policy version holds it once it writes a register itself, and a
/// write that fails leaves its copies as they were (#6265).
#[tokio::test]
async fn a_policy_write_installs_every_earlier_change_with_its_own() {
    use ferroehr::policy_register::Register;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let elsewhere = replica(&pool);
    let writer = replica(&pool);
    let _deaf = crate::policy_fixtures::deaf(&[
        std::sync::Arc::clone(writer.policy_registers().listened()),
        std::sync::Arc::clone(writer.access_policy().listened()),
    ])
    .await;
    writer.confirm_policy(true, true).await.expect("a read");
    create(&elsewhere, &policy(true)).await;
    assert_eq!(
        writer.access_policy().in_force().version,
        0,
        "heard nothing"
    );
    let delay = r#"{"enabled": true, "categories": ["Patient-Summaries"], "max_delay": "72h"}"#;
    writer
        .put_policy_register(Register::AccessDelay, 7, delay)
        .await
        .expect_err("a write over a version that is not the newest fails");
    assert_eq!(
        writer.access_policy().in_force().version,
        0,
        "a failed write installs nothing"
    );
    assert_eq!(writer.policy_registers().version(Register::AccessDelay), 0);
    writer
        .put_policy_register(Register::AccessDelay, 0, delay)
        .await
        .expect("the register is written");
    assert_eq!(writer.policy_registers().version(Register::AccessDelay), 1);
    assert_eq!(
        writer.access_policy().in_force().version,
        1,
        "the write installed the version another replica committed before it"
    );
}
