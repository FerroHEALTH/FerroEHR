// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller facts (#5511): sensitivity clearances and permitted purposes
//! read from a capability's credentials, and care relationships read from a
//! `PERSON`'s relationships, each by its code of `ferroehr-operations` in an
//! archetype of the deployment's own, held on every replica beside the
//! grants. An admission and a discharge decide the next request on another
//! replica, a replica reads only the facts of the parties that changed, with
//! one indexed read, and only a caller holding `write_care_relationship` (or
//! an administrator) changes a care relationship. A provider
//! `ORGANISATION`'s `member_of` relationship is its membership of a `GROUP`
//! (#5575), held and guarded the same way. No openEHR spec gives a
//! capability or a relationship an access meaning: our own design.

use std::future::Future;

use serde_json::{Value, json};

use crate::fixtures::{if_match, object};
use crate::policy_fixtures::{capability, create, granted, granting, of_client};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::capabilities::with_capabilities;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// The HL7 v3 `ActCode` sensitivity terminology id.
const ACT_CODE: &str = "2.16.840.1.113883.5.25";
/// The HL7 v3 `PurposeOfUse` terminology id.
const PURPOSE_OF_USE: &str = "2.16.840.1.113883.5.8";

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

fn foreign(terminology: &str, code: &str) -> ForeignCode {
    ForeignCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    }
}

/// A credentials element named `name` of the terminology holding `value`.
fn qualifier(name: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0040",
            "name": coded(TERMINOLOGY, name), "value": value })
}

/// A `DV_INTERVAL<DV_DATE>` from `lower` to `upper`, each included; an upper
/// of `None` is unbounded.
fn interval(lower: &str, upper: Option<&str>) -> Value {
    let mut interval = json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": lower },
        "lower_unbounded": false, "upper_unbounded": upper.is_none(),
        "lower_included": true, "upper_included": upper.is_some() });
    if let Some(upper) = upper {
        interval["upper"] = json!({ "_type": "DV_DATE", "value": upper });
    }
    interval
}

fn party_ref(kind: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

/// A care relationship to `team` carrying `code` in its details, in an
/// archetype of the deployment's own, over `validity`.
fn care(code: &str, source: VoId, team: VoId, validity: &Value) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v3",
        "name": text("Ward stay"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("kind"),
                        "value": coded(TERMINOLOGY, code) }] },
        "time_validity": validity,
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// A plain `PERSON` named `name` holding `relationships`.
fn person(name: &str, relationships: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-PERSON.person.v1";
    let mut body = json!({
        "_type": "PERSON", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text(name),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text(name) }] } }]
    });
    if !relationships.is_empty() {
        body["relationships"] = Value::Array(relationships.to_vec());
    }
    body
}

/// A care team `GROUP`.
fn team() -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-GROUP.ward.v1";
    json!({
        "_type": "GROUP", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text("Cardiology"),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text("Cardiology") }] } }]
    })
}

/// A provider `ORGANISATION` holding `relationships`.
fn organisation(relationships: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.hospital.v1";
    let mut body = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text("Hospital"),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text("Hospital") }] } }]
    });
    if !relationships.is_empty() {
        body["relationships"] = Value::Array(relationships.to_vec());
    }
    body
}

/// A replica over `pool`, as the server installs it.
async fn replica(pool: &sqlx::PgPool) -> FerroEhrService {
    granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    ))
    .await
}

/// Runs `fut` as a request of the system client `client_id`, a caller that is
/// no administrator, inside its grants on `svc`.
async fn as_client<F: Future>(svc: &FerroEhrService, client_id: &str, fut: F) -> F::Output {
    let caller = CommitterIdentity {
        subject: client_id.to_owned(),
        id_type: "oauth2",
        issuer: Some(crate::policy_fixtures::ISSUER.to_owned()),
        client: Some(client_id.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: true,
        agent: None,
        external: None,
    };
    let granted = of_client(svc, client_id).await;
    with_capabilities(granted, with_committer(Some(caller), Box::pin(fut))).await
}

/// Commits `body` as the next version of the party of `kind` `vo` after `uid`
/// as the client `client_id`; the new version's uid.
async fn update_as(
    svc: &FerroEhrService,
    client_id: &str,
    (kind, vo, uid): (PartyKind, VoId, &str),
    body: &Value,
) -> Result<String, ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    let updated = as_client(svc, client_id, async {
        Box::pin(svc.party_update(kind, object(vo.to_string()), if_match(uid), write)).await
    })
    .await?;
    Ok(updated.meta.expect("a version").uid)
}

/// Today in UTC, and the day `days` from it, as `YYYY-MM-DD`.
fn day(days: i64) -> String {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    today
        .checked_add(jiff::Span::new().days(days))
        .expect("a date")
        .to_string()
}

#[tokio::test]
async fn a_capability_clears_and_permits_by_its_coded_credentials() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    granted(
        &svc,
        "psychiatry",
        &[capability(
            "access_demographic",
            &[
                qualifier("sensitivity_clearance", &coded(ACT_CODE, "PSY")),
                qualifier("permitted_purpose", &coded(PURPOSE_OF_USE, "TREAT")),
            ],
        )],
    )
    .await;
    let facts = of_client(&svc, "psychiatry").await.facts().clone();
    assert!(facts.clears(&foreign(ACT_CODE, "PSY")));
    assert!(!facts.clears(&foreign(ACT_CODE, "HIV")));
    assert!(facts.permits(&foreign(PURPOSE_OF_USE, "TREAT")));
    assert!(!facts.permits(&foreign(PURPOSE_OF_USE, "HOPERAT")));
}

#[tokio::test]
async fn a_clearance_that_is_no_foreign_code_is_refused_at_commit() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    let (agent, _) = create(&svc, PartyKind::Person, &person("Dr C", &[]))
        .await
        .expect("the PERSON commits");
    let role = crate::policy_fixtures::role(
        agent,
        &[capability(
            "access_demographic",
            &[qualifier(
                "sensitivity_clearance",
                &coded(TERMINOLOGY, "open"),
            )],
        )],
    );
    let refused = create(&svc, PartyKind::Role, &role)
        .await
        .expect_err("a clearance of FerroEHR's own terminology is refused");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );
}

#[tokio::test]
async fn an_admission_and_a_discharge_decide_the_next_request_on_another_replica() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (replica(&db.pool()).await, replica(&db.pool()).await);
    granted(&one, "adt", &[capability("write_care_relationship", &[])]).await;
    let (ward, _) = create(&one, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (doctor, doctor_uid) = create(&one, PartyKind::Person, &person("Dr D", &[]))
        .await
        .expect("the professional commits");
    let (patient, uid) = create(&one, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let roster = person(
        "Dr D",
        &[care("member_of", doctor, ward, &interval(&day(-30), None))],
    );
    update_as(
        &one,
        "adt",
        (PartyKind::Person, doctor, &doctor_uid),
        &roster,
    )
    .await
    .expect("the ADT system writes the membership");
    two.grants().refresh().await.expect("the copy reads");
    let held = two.grants().revision();
    let now = jiff::Timestamp::now();
    assert_eq!(two.grants().facts_of(doctor.0, now).care_teams().len(), 1);
    assert_eq!(two.grants().under_care_of(patient.0), Vec::new());

    let admitted = update_as(
        &one,
        "adt",
        (PartyKind::Person, patient, &uid),
        &person(
            "Mr E",
            &[care(
                "under_care_of",
                patient,
                ward,
                &interval(&day(0), None),
            )],
        ),
    )
    .await
    .expect("the ADT system admits the patient");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        two.grants().revision(),
        held,
        "an admission moves the facts revision alone"
    );
    let today: jiff::civil::Date = day(0).parse().expect("a date");
    let cared = two.grants().under_care_of(patient.0);
    assert_eq!(cared.len(), 1, "the admission decides the next request");
    assert_eq!(cared[0].team(), ward.0);
    assert!(cared[0].holds_on(today));

    update_as(
        &one,
        "adt",
        (PartyKind::Person, patient, &admitted),
        &person(
            "Mr E",
            &[care(
                "under_care_of",
                patient,
                ward,
                &interval(&day(-3), Some(&day(-1))),
            )],
        ),
    )
    .await
    .expect("the ADT system discharges the patient");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    let cared = two.grants().under_care_of(patient.0);
    assert_eq!(cared.len(), 1, "a discharge keeps the relationship it ends");
    assert!(
        !cared[0].holds_on(today),
        "the discharge decides the next request"
    );
    assert_eq!(cared[0].until(), Some(today));
}

#[tokio::test]
async fn only_a_write_care_relationship_grant_changes_a_care_relationship() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    granted(&svc, "registry", &[capability("access_demographic", &[])]).await;
    granted(&svc, "adt", &[capability("write_care_relationship", &[])]).await;
    let (ward, _) = create(&svc, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (patient, uid) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let admitted = person(
        "Mr E",
        &[care(
            "under_care_of",
            patient,
            ward,
            &interval(&day(0), None),
        )],
    );
    let refused = update_as(
        &svc,
        "registry",
        (PartyKind::Person, patient, &uid),
        &admitted,
    )
    .await
    .expect_err("a client without the grant is refused");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let renamed = update_as(
        &svc,
        "registry",
        (PartyKind::Person, patient, &uid),
        &person("Mr E.", &[]),
    )
    .await
    .expect("a version that leaves the care relationships alone commits");
    update_as(
        &svc,
        "adt",
        (PartyKind::Person, patient, &renamed),
        &admitted,
    )
    .await
    .expect("the ADT system writes the care relationship");
}

#[tokio::test]
async fn a_providers_group_membership_is_guarded_and_held_on_another_replica() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (replica(&db.pool()).await, replica(&db.pool()).await);
    granted(&one, "registry", &[capability("access_demographic", &[])]).await;
    granted(&one, "adt", &[capability("write_care_relationship", &[])]).await;
    let (region, _) = create(&one, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (hospital, uid) = create(&one, PartyKind::Organisation, &organisation(&[]))
        .await
        .expect("the ORGANISATION commits");
    let membership = |validity: &Value| {
        let mut relationship = care("member_of", hospital, region, validity);
        relationship["source"] = party_ref("ORGANISATION", hospital);
        organisation(&[relationship])
    };
    let joined = membership(&interval(&day(-30), None));
    let refused = update_as(
        &one,
        "registry",
        (PartyKind::Organisation, hospital, &uid),
        &joined,
    )
    .await
    .expect_err("a client without the grant is refused");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    two.grants().refresh().await.expect("the copy reads");
    let held = two.grants().revision();
    let joined_uid = update_as(
        &one,
        "adt",
        (PartyKind::Organisation, hospital, &uid),
        &joined,
    )
    .await
    .expect("the grant writes the membership");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    let now = jiff::Timestamp::now();
    assert_eq!(two.grants().memberships(hospital.0, now), vec![region.0]);
    assert_eq!(
        two.grants().revision(),
        held,
        "a membership moves the facts revision alone"
    );
    assert_eq!(
        two.grants().facts_of(hospital.0, now).care_teams().len(),
        1,
        "the membership is the ORGANISATION's own fact"
    );
    update_as(
        &one,
        "adt",
        (PartyKind::Organisation, hospital, &joined_uid),
        &membership(&interval(&day(-30), Some(&day(-1)))),
    )
    .await
    .expect("the grant ends the membership");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        two.grants().memberships(hospital.0, jiff::Timestamp::now()),
        Vec::<uuid::Uuid>::new(),
        "an ended membership admits nothing on the next request"
    );
}

#[tokio::test]
async fn a_care_relationship_of_its_own_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    let (ward, _) = create(&svc, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (patient, _) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let mut write = WriteRequest::new(crate::typed_body::typed(&care(
        "under_care_of",
        patient,
        ward,
        &interval(&day(0), None),
    )));
    write.lifecycle_state = Some("532".to_owned());
    let refused = crate::policy_fixtures::as_admin(async {
        Box::pin(svc.party_relationship_create(write)).await
    })
    .await
    .expect_err("a standalone care relationship is refused");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );
}

#[tokio::test]
async fn one_indexed_read_returns_the_facts_of_the_parties_that_changed() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    sqlx::query(
        "WITH p AS (SELECT uuidv7() AS id, n FROM generate_series(1, 20000) AS n), \
         s AS (INSERT INTO party.binding_caller_fact_party (party_id, revision) \
               SELECT id, n FROM p) \
         INSERT INTO party.binding_caller_fact \
             (holder_id, ordinal, kind, party_id, target_id, lifecycle_state, role_validity, \
              validity) \
         SELECT id, 0, 'member_of', id, uuidv7(), '532', '(,)', '(,)' FROM p",
    )
    .execute(&pool)
    .await
    .expect("the facts seed");
    sqlx::query("VACUUM (ANALYZE) party.binding_caller_fact, party.binding_caller_fact_party")
        .execute(&pool)
        .await
        .expect("vacuum");
    let mut conn = pool.acquire().await.expect("a connection");
    sqlx::query("SET search_path = party, ext, public")
        .execute(&mut *conn)
        .await
        .expect("search path");
    let explain = format!(
        "EXPLAIN (FORMAT JSON) {}",
        ferroehr::party_binding::grants::caller::store::FACTS
    );
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
        .bind(19_990_i64)
        .fetch_one(&mut *conn)
        .await
        .expect("the plan");
    let text = plan.to_string();
    assert!(
        text.contains("ix_binding_caller_fact_party_revision")
            && text.contains("ix_binding_caller_fact_party\""),
        "the facts read is not answered from the stamp and party indexes: {text}"
    );
    assert!(
        !text.contains("\"Seq Scan\""),
        "the facts read scans a table: {text}"
    );
    let read = sqlx::query(ferroehr::party_binding::grants::caller::store::FACTS)
        .bind(19_990_i64)
        .fetch_all(&mut *conn)
        .await
        .expect("the facts read");
    assert_eq!(read.len(), 10, "only the parties stamped since are read");
}
