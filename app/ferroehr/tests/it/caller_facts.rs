// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller facts (#5511, #5621): the roles a caller performs, read from
//! the coded `name` of each `ROLE` in force it performs, and care
//! relationships read from a `PERSON`'s relationships by their code of
//! `ferroehr-operations`, in an archetype of the deployment's own, held on
//! every replica beside the grants. An admission and a discharge decide the next request on another
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
/// The jurisdiction's role code system.
const ROLES: &str = "urn:example:roles";

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

/// A `ROLE` of the deployment's archetype performed by `performer`, named
/// `name`, over `validity`, granting `access_demographic`.
fn named_role(performer: VoId, name: &Value, validity: Option<Value>) -> Value {
    let mut role =
        crate::policy_fixtures::role(performer, &[capability("access_demographic", &[])]);
    role["name"] = name.clone();
    if let Some(validity) = validity {
        role["time_validity"] = validity;
    }
    role
}

#[tokio::test]
async fn a_role_is_the_coded_name_of_a_role_in_force_the_caller_performs() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    let nurse = coded(ROLES, "NURSE");
    let rows = [
        ("a coded name", "nurse-client", nurse.clone(), None, true),
        ("a text name", "text-client", text("Nurse"), None, false),
        (
            "an operation code",
            "operation-client",
            coded(TERMINOLOGY, "NURSE"),
            None,
            false,
        ),
        (
            "a role that ended",
            "ended-client",
            nurse.clone(),
            Some(interval(&day(-30), Some(&day(-1)))),
            false,
        ),
        (
            "a role from today",
            "today-client",
            nurse,
            Some(interval(&day(0), None)),
            true,
        ),
    ];
    for (name, client_id, role_name, validity, holds) in rows {
        let (agent, _) = granted(&svc, client_id, &[capability("access_demographic", &[])]).await;
        create(
            &svc,
            PartyKind::Role,
            &named_role(agent, &role_name, validity),
        )
        .await
        .expect("the ROLE commits");
        let facts = of_client(&svc, client_id).await.facts().clone();
        assert_eq!(
            facts.roles().contains(&foreign(ROLES, "NURSE")),
            holds,
            "{name}: {:?}",
            facts.roles()
        );
    }
}

#[tokio::test]
async fn a_clearance_or_purpose_element_is_no_qualifier_any_more() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    let (agent, _) = create(&svc, PartyKind::Person, &person("Dr C", &[]))
        .await
        .expect("the PERSON commits");
    for name in ["sensitivity_clearance", "permitted_purpose"] {
        let role = crate::policy_fixtures::role(
            agent,
            &[capability(
                "access_demographic",
                &[qualifier(name, &coded(ACT_CODE, "PSY"))],
            )],
        );
        let refused = create(&svc, PartyKind::Role, &role)
            .await
            .expect_err("a removed code names no element");
        assert!(
            matches!(refused, ServiceError::Unprocessable { .. }),
            "{name}: {refused:?}"
        );
    }
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
    assert_eq!(
        two.grants()
            .under_care_of(patient.0)
            .await
            .expect("the care relationships read"),
        Vec::new()
    );
    let parties = two.grants().parties_held();

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
    let cared = two
        .grants()
        .under_care_of(patient.0)
        .await
        .expect("the care relationships read");
    assert_eq!(cared.len(), 1, "the admission decides the next request");
    assert_eq!(cared[0].team(), ward.0);
    assert!(cared[0].holds_on(today));
    assert_eq!(
        two.grants()
            .cared_for_by(&[ward.0], today, jiff::Span::new())
            .await
            .expect("the ward's patients read"),
        vec![ferroehr::storage::access_projection::EhrSubject::new(
            "demographic",
            patient.to_string()
        )],
        "the ward's patients are read by the ward, by their subject references"
    );
    // NOTE: #5615, a patient in care is no caller fact, so the replica's copy does not grow with it.
    assert_eq!(two.grants().parties_held(), parties);

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
    let cared = two
        .grants()
        .under_care_of(patient.0)
        .await
        .expect("the care relationships read");
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

/// The pseudonymisation domain of the record subject tests.
const PIXM: &str = "urn:oid:2.999.7.1";

/// A record subject element of a `PERSON`'s details holding `value` (#5634).
fn record_subject(value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0010",
            "name": coded(TERMINOLOGY, "record_subject"), "value": value })
}

/// A `DV_IDENTIFIER` naming `id` under `issuer`.
fn pseudonym(issuer: &str, id: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": "PI" })
}

/// `body` with `items` as its details.
fn detailed(mut body: Value, items: &[Value]) -> Value {
    body["details"] = json!({ "_type": "ITEM_TREE", "archetype_node_id": "at0009",
        "name": text("details"), "items": items });
    body
}

/// The subject reference of `id` under the pseudonymisation domain.
fn pixm_ref(id: &str) -> ferroehr::storage::access_projection::EhrSubject {
    ferroehr::storage::access_projection::EhrSubject::new(PIXM, id)
}

#[tokio::test]
async fn a_record_subject_decides_the_next_request_on_another_replica() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (replica(&db.pool()).await, replica(&db.pool()).await);
    granted(&one, "adt", &[capability("write_care_relationship", &[])]).await;
    let (patient, uid) = create(&one, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let id = uuid::Uuid::now_v7().to_string();
    two.grants().refresh().await.expect("the copy reads");
    let held = two.grants().revision();
    assert_eq!(
        two.grants()
            .person_of(&pixm_ref(&id))
            .await
            .expect("the subject resolves"),
        None
    );

    let declared = update_as(
        &one,
        "adt",
        (PartyKind::Person, patient, &uid),
        &detailed(
            person("Mr E", &[]),
            &[record_subject(&pseudonym(PIXM, &id))],
        ),
    )
    .await
    .expect("the ADT system declares the pseudonym");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        two.grants().revision(),
        held,
        "a record subject moves the facts revision alone"
    );
    assert_eq!(
        two.grants()
            .person_of(&pixm_ref(&id))
            .await
            .expect("the subject resolves"),
        Some(patient.0)
    );
    assert_eq!(
        two.grants()
            .person_of(&ferroehr::storage::access_projection::EhrSubject::new(
                "demographic",
                patient.to_string()
            ))
            .await
            .expect("the subject resolves"),
        Some(patient.0),
        "a reference in the CDR's own namespace names the PERSON directly"
    );
    assert_eq!(
        two.grants()
            .person_of(&ferroehr::storage::access_projection::EhrSubject::new(
                "local",
                patient.to_string()
            ))
            .await
            .expect("the subject resolves"),
        None,
        "a container id in another namespace names no PERSON"
    );

    update_as(
        &one,
        "adt",
        (PartyKind::Person, patient, &declared),
        &person("Mr E", &[]),
    )
    .await
    .expect("the ADT system withdraws the pseudonym");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        two.grants()
            .person_of(&pixm_ref(&id))
            .await
            .expect("the subject resolves"),
        None
    );
}

#[tokio::test]
async fn a_record_subject_is_refused_unless_it_is_a_persons_own_and_its_writer_may() {
    let db = testkit::db().await.expect("testkit database");
    let policy =
        ferroehr::privacy::PrivacyPolicy::compile(&ferroehr::privacy::config::PrivacyConfig {
            subject_namespaces: vec![PIXM.to_owned()],
            ..Default::default()
        })
        .expect("the privacy policy compiles");
    let svc = replica(&db.pool())
        .await
        .with_privacy(std::sync::Arc::new(policy));
    granted(&svc, "registry", &[capability("access_demographic", &[])]).await;
    granted(&svc, "adt", &[capability("write_care_relationship", &[])]).await;
    let (patient, uid) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let id = uuid::Uuid::now_v7().to_string();
    let declares = |value: &Value| detailed(person("Mr E", &[]), &[record_subject(value)]);
    let write = |client: &'static str, body: Value| {
        let svc = &svc;
        let uid = uid.clone();
        async move { update_as(svc, client, (PartyKind::Person, patient, &uid), &body).await }
    };

    let refused = write("registry", declares(&pseudonym(PIXM, &id)))
        .await
        .expect_err("a client without write_care_relationship is refused");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    for (why, value) in [
        ("no DV_IDENTIFIER", text(&id)),
        ("no issuer", json!({ "_type": "DV_IDENTIFIER", "id": id })),
        (
            "the own namespace",
            pseudonym("demographic", &patient.to_string()),
        ),
        (
            "an id the subject rule refuses",
            pseudonym(PIXM, "1793131381"),
        ),
    ] {
        let refused = write("adt", declares(&value))
            .await
            .expect_err("the record subject is refused");
        assert!(
            matches!(refused, ServiceError::Unprocessable { .. }),
            "{why}: {refused:?}"
        );
    }
    let (hospital, hospital_uid) = create(&svc, PartyKind::Organisation, &organisation(&[]))
        .await
        .expect("the ORGANISATION commits");
    let refused = update_as(
        &svc,
        "adt",
        (PartyKind::Organisation, hospital, &hospital_uid),
        &detailed(organisation(&[]), &[record_subject(&pseudonym(PIXM, &id))]),
    )
    .await
    .expect_err("an ORGANISATION declares no record subject");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );

    write("adt", declares(&pseudonym(PIXM, &id)))
        .await
        .expect("the ADT system declares the pseudonym");
    let (other, other_uid) = create(&svc, PartyKind::Person, &person("Mr F", &[]))
        .await
        .expect("the other patient commits");
    let refused = update_as(
        &svc,
        "adt",
        (PartyKind::Person, other, &other_uid),
        &detailed(
            person("Mr F", &[]),
            &[record_subject(&pseudonym(PIXM, &id))],
        ),
    )
    .await
    .expect_err("one pseudonym stands for one PERSON");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
}
