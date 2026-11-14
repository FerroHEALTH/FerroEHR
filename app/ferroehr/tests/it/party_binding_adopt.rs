// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr party-binding adopt` (#5009) over a real PostgreSQL 18: each
//! prepared `PERSON` gets a professional `ROLE` of the mapping's role
//! template naming it as `performer` and a new version listing it in `roles`
//! (RM demographic `org.openehr.rm.demographic.actor.adoc`, `…role.adoc`), in
//! one demographic CONTRIBUTION committed by the named administrator, with
//! one access record each; a `PERSON` whose key the provenance rule refuses
//! stops the whole adoption. No openEHR spec governs a deployment's
//! migration: our own design.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::fixtures::change_type;
use crate::scim_users::{
    KEY_PATH, LOGIN, NAME_PATH, NAMESPACE, PERSON, identifier, provision, service, user,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::check::{Administrator, CheckInput};
use ferroehr::party_binding::provisioned::UserFilter;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::adopt::AdoptInput;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The token issuer of the SCIM client `scim_users::provision` commits as.
const MACHINES: &str = "https://as.example";
/// That client.
const CLIENT: &str = "provisioning";
/// The committer name of the declared migration.
const MIGRATION: &str = "Cadasto V1 migration";

/// A prepared `PERSON` carrying the key `key`, committed by `committer`
/// with no professional `ROLE`.
async fn prepared(svc: &FerroEhrService, key: &str, committer: &Value) -> VoId {
    let body = svc
        .mapped_party_body(
            None,
            PERSON,
            &[
                (KEY_PATH.to_owned(), identifier(LOGIN, "guid", key)),
                (
                    NAME_PATH.to_owned(),
                    json!({ "_type": "DV_TEXT", "value": "prepared professional" }),
                ),
            ],
        )
        .await
        .expect("the body maps");
    let mut write = WriteRequest::new(crate::typed_body::typed(&body));
    write.audit = Some(
        openehr_its::json::from_canonical_value(&json!({
            "change_type": change_type("249", "creation"), "committer": committer
        }))
        .expect("an UPDATE_AUDIT"),
    );
    let created = Box::pin(svc.party_create(PartyKind::Person, write))
        .await
        .expect("the PERSON commits");
    crate::fixtures::written_vo(&created)
}

fn named(name: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name })
}

fn by_administrator(admin: VoId) -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED",
        "external_ref": {
            "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": admin.to_string() }
        }
    })
}

/// An adoption of `parties` by the administrator `admin`.
fn adoption(admin: Option<VoId>, parties: &[VoId]) -> AdoptInput {
    AdoptInput {
        check: CheckInput {
            key_types: BTreeMap::from([(LOGIN.to_owned(), "guid".to_owned())]),
            scim_clients: vec![(MACHINES.to_owned(), CLIENT.to_owned())],
            administrators: admin
                .map(|a| Administrator::Party(a.0))
                .into_iter()
                .collect(),
            migrations: vec![MIGRATION.to_owned()],
            today: ferroehr::support::today_utc().to_string(),
            ..CheckInput::default()
        },
        parties: parties.to_vec(),
        namespace: NAMESPACE.to_owned(),
    }
}

/// Whether `party` is a User of the client of [`LOGIN`]: a `PERSON` with a
/// live professional `ROLE`.
async fn is_user(svc: &FerroEhrService, party: VoId) -> bool {
    let filter = UserFilter {
        ids: vec![party.0],
        ..UserFilter::default()
    };
    svc.provisioned_users(LOGIN, &filter, 0, 1)
        .await
        .expect("the store answers")
        .total
        == 1
}

const ADMIN_KEY: &str = "3a4b5c6d-7e8f-4a9b-8c0d-1e2f3a4b5c6d";
const MIGRATED_KEY: &str = "4b5c6d7e-8f9a-4b0c-9d1e-2f3a4b5c6d7e";
const BY_ADMIN_KEY: &str = "5c6d7e8f-9a0b-4c1d-ae2f-3a4b5c6d7e8f";
const PLANTED_KEY: &str = "6d7e8f9a-0b1c-4d2e-bf3a-4b5c6d7e8f9a";

#[tokio::test]
async fn adoption_gives_each_prepared_person_its_role_in_one_contribution() {
    let (db, svc) = service().await;
    let audit = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        store: ferroehr::system_log::config::StoreConfig {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let (sender, handle) = ferroehr::system_log::sender::start(audit, None, Some(db.pool()))
        .await
        .expect("the audit sender");
    let svc = svc.with_audit(sender);
    let admin = provision(
        &svc,
        LOGIN,
        user(LOGIN, ADMIN_KEY, "admin", "x-admin", "Admin"),
    )
    .await
    .expect("the administrator is provisioned")
    .row
    .party;
    let migrated = prepared(&svc, MIGRATED_KEY, &named(MIGRATION)).await;
    let by_admin = prepared(&svc, BY_ADMIN_KEY, &by_administrator(admin)).await;
    assert!(
        !is_user(&svc, migrated).await,
        "a prepared PERSON binds no login"
    );

    let report = svc
        .adopt_persons(&adoption(Some(admin), &[migrated, by_admin, admin]))
        .await
        .expect("the adoption commits");

    let mut adopted: Vec<Uuid> = report.adopted.iter().map(|a| a.party).collect();
    adopted.sort();
    let mut expected = vec![migrated.0, by_admin.0];
    expected.sort();
    assert_eq!(adopted, expected);
    assert_eq!(
        report.already,
        [admin.0],
        "a professional is adopted already"
    );
    assert!(is_user(&svc, migrated).await);
    assert!(is_user(&svc, by_admin).await);
    let contribution = report.contribution.expect("one CONTRIBUTION");
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM party.version WHERE contribution_id = $1 ORDER BY kind",
    )
    .bind(contribution)
    .fetch_all(&db.pool())
    .await
    .expect("the versions read");
    assert_eq!(kinds, ["PERSON", "PERSON", "ROLE", "ROLE"]);
    let committers: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT a.committer -> 'external_ref' -> 'id' ->> 'value' FROM party.version v \
         JOIN party.commit_audit a ON a.id = v.commit_audit_id WHERE v.contribution_id = $1",
    )
    .bind(contribution)
    .fetch_all(&db.pool())
    .await
    .expect("the audits read");
    assert_eq!(committers, [admin.to_string()], "the administrator commits");

    let again = svc
        .adopt_persons(&adoption(Some(admin), &[migrated]))
        .await
        .expect("a second adoption commits nothing");
    assert_eq!(again.contribution, None);
    assert_eq!(again.already, [migrated.0]);

    drop(svc);
    handle.shutdown(std::time::Duration::from_secs(5)).await;
    let recorded: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT resource_id, principal FROM audit.access_trail \
         WHERE resource_id LIKE '%#professional-role:%' ORDER BY resource_id",
    )
    .fetch_all(&db.pool())
    .await
    .expect("the audit store reads");
    assert_eq!(
        recorded.len(),
        2,
        "one access record per adopted PERSON: {recorded:?}"
    );
    for adopted in &report.adopted {
        let resource = format!("party:{}#professional-role:{}", adopted.party, adopted.role);
        assert!(
            recorded
                .iter()
                .any(|(r, p)| *r == resource && p.as_deref() == Some(admin.to_string().as_str())),
            "{resource} recorded under the administrator: {recorded:?}"
        );
    }
}

#[tokio::test]
async fn a_planted_key_stops_the_adoption_by_name() {
    let (_db, svc) = service().await;
    let admin = provision(
        &svc,
        LOGIN,
        user(LOGIN, ADMIN_KEY, "admin", "x-admin", "Admin"),
    )
    .await
    .expect("the administrator is provisioned")
    .row
    .party;
    let migrated = prepared(&svc, MIGRATED_KEY, &named(MIGRATION)).await;
    let planted = prepared(&svc, PLANTED_KEY, &named("a demographic writer")).await;

    let refused = svc
        .adopt_persons(&adoption(Some(admin), &[migrated, planted]))
        .await;

    match refused {
        Err(ServiceError::Forbidden(refusal)) => {
            let message = refusal.to_string();
            assert!(message.contains(&planted.to_string()), "{message}");
            assert!(!message.contains(&migrated.to_string()), "{message}");
        }
        other => panic!("expected a 403 naming the planted PERSON, got {other:?}"),
    }
    assert!(!is_user(&svc, migrated).await, "nothing commits");
}

#[tokio::test]
async fn an_adoption_names_its_administrator() {
    let (_db, svc) = service().await;
    let migrated = prepared(&svc, MIGRATED_KEY, &named(MIGRATION)).await;

    let refused = svc.adopt_persons(&adoption(None, &[migrated])).await;

    assert!(
        matches!(refused, Err(ServiceError::Conflict(_))),
        "{refused:?}"
    );
    assert!(!is_user(&svc, migrated).await);
}

#[tokio::test]
async fn a_party_with_no_key_is_refused() {
    let (_db, svc) = service().await;
    let admin = provision(
        &svc,
        LOGIN,
        user(LOGIN, ADMIN_KEY, "admin", "x-admin", "Admin"),
    )
    .await
    .expect("the administrator is provisioned")
    .row
    .party;
    let unknown = VoId(Uuid::now_v7());

    let refused = svc.adopt_persons(&adoption(Some(admin), &[unknown])).await;

    match refused {
        Err(ServiceError::Conflict(refusal)) => {
            assert!(
                refusal.to_string().contains(&unknown.to_string()),
                "{refusal}"
            );
        }
        other => panic!("expected a 409 naming the party, got {other:?}"),
    }
}
