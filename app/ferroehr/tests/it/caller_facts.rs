// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller facts (#5511, #5621, #6055): the roles a caller performs, read
//! from the coded `name` of each `ROLE` in force it performs, memberships
//! read from a `PERSON`'s relationships by their code
//! (`openehr_access::member_of`), and care relationships, each a `PARTY_RELATIONSHIP` of
//! its own named `openehr_access::under_care_of` with its `care_role` and
//! `provider` (draft access control package, Care Relationships), in an
//! archetype of the deployment's own, held on every replica beside the
//! grants. An admission and a discharge decide the next request on another
//! replica, a replica reads only the facts of the parties that changed, with
//! one indexed read, and only a caller holding `write_care_relationship` (or
//! an administrator) changes a care relationship. A provider
//! `ORGANISATION`'s `member_of` relationship is its membership of a `GROUP`
//! (#5575), held and guarded the same way. No openEHR spec gives a
//! capability or a relationship an access meaning: our own design.

use ferroehr::access_eval::procedure::Policy;
use std::future::Future;

use serde_json::{Value, json};

use crate::fixtures::{if_match, object};
use crate::policy_fixtures::{
    CARE_PROVIDER, capability, care_provider, care_writer, create, granted, granting, of_client,
    provider_team,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::caller::care_of::ProviderKey;
use ferroehr::party_binding::grants::capabilities::with_capabilities;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// The HL7 v3 `ActCode` sensitivity terminology id.
const ACT_CODE: &str = "HL7_ActCode";
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

/// A professional's membership of a care team: `openehr_access::member_of`.
const MEMBER: (&str, &str) = ("openehr_access", "member_of");

/// The issuer of the provider the care relationships name.
const PROVIDERS: &str = "urn:example:providers";

/// A relationship to `team` carrying `code` in its details, in an archetype
/// of the deployment's own, over `validity`.
fn care((terminology, code): (&str, &str), source: VoId, team: VoId, validity: &Value) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v3",
        "name": text("Ward stay"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("kind"),
                        "value": coded(terminology, code) }] },
        "time_validity": validity,
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// The provider the care relationships of these tests are in the care of.
fn provider() -> ProviderKey {
    ProviderKey::new(PROVIDERS, "hospital")
}

/// A care relationship of its own: `source` under the care of the `GROUP`
/// `team` in the role `PART` of `HL7_ParticipationType`, in the care of
/// [`provider`], over `validity`.
fn care_of(source: VoId, team: VoId, validity: &Value) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v3",
        "name": coded(TERMINOLOGY, "under_care_of"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0003",
                  "name": coded(TERMINOLOGY, "care_role"),
                  "value": coded("HL7_ParticipationType", "PART") },
                { "_type": "ELEMENT", "archetype_node_id": "at0004",
                  "name": coded(TERMINOLOGY, "provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": PROVIDERS, "id": "hospital",
                             "type": "ORGANISATION" } }] },
        "time_validity": validity,
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// Creates the care relationship `body` as the client `client_id`; its
/// container and version uid.
async fn relate_as(
    svc: &FerroEhrService,
    client_id: &str,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let created = as_client(svc, client_id, async {
        Box::pin(svc.party_relationship_create(write)).await
    })
    .await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((crate::fixtures::written_vo(&created), uid))
}

/// Commits `body` as the next version of the relationship `vo` after `uid`
/// as the client `client_id`; the new version's uid.
async fn rerelate_as(
    svc: &FerroEhrService,
    client_id: &str,
    (vo, uid): (VoId, &str),
    body: &Value,
) -> Result<String, ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    let updated = as_client(svc, client_id, async {
        Box::pin(svc.party_relationship_update(vo, if_match(uid), write)).await
    })
    .await?;
    Ok(updated.meta.expect("a version").uid)
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
        local_issuer: ferroehr::service::DEFAULT_SYSTEM_ID.into(),
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
    granted(&one, "adt", &[care_writer()]).await;
    let hospital = care_provider(&one, CARE_PROVIDER).await;
    let (ward, _) = provider_team(&one, hospital, &team()).await;
    let (doctor, doctor_uid) = create(&one, PartyKind::Person, &person("Dr D", &[]))
        .await
        .expect("the professional commits");
    let (patient, _) = create(&one, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let roster = person(
        "Dr D",
        &[care(MEMBER, doctor, ward, &interval(&day(-30), None))],
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
    let parties = two.grants().facts_held().entries;

    let (stay, admitted) = relate_as(
        &one,
        "adt",
        &care_of(patient, ward, &interval(&day(0), Some(&day(365)))),
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
    assert_eq!(cared[0].carer(), ward.0);
    assert_eq!(cared[0].provider(), &provider());
    assert_eq!(cared[0].role(), &foreign("HL7_ParticipationType", "PART"));
    assert!(cared[0].live_on(today, jiff::Span::new()));
    let member = |provider: ProviderKey| {
        ferroehr::party_binding::grants::caller::facts::CallerFacts::new(
            Vec::new(),
            vec![ferroehr::party_binding::grants::caller::CareTeam::spanning(
                ward.0, None, None,
            )],
        )
        .with_providers([provider])
    };
    let subjects = |cared: Vec<ferroehr::access_eval::condition::care::CaredSubject>| {
        cared
            .into_iter()
            .map(|cared| cared.subject)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        subjects(
            two.grants()
                .cared_for_by(&member(provider()), &[ward.0], today, &Policy::default())
                .await
                .expect("the ward's patients read")
        ),
        vec![ferroehr::storage::access_projection::EhrSubject::new(
            "demographic",
            patient.to_string()
        )],
        "the ward's patients are read by the ward, by their subject references"
    );
    assert_eq!(
        subjects(
            two.grants()
                .cared_for_by(
                    &member(ProviderKey::new(PROVIDERS, "clinic")),
                    &[ward.0],
                    today,
                    &Policy::default()
                )
                .await
                .expect("the ward's patients read")
        ),
        Vec::new(),
        "a caller acting for another provider holds none of them"
    );
    // NOTE: #5615, a patient in care is no caller fact, so the replica's copy does not grow with it.
    assert_eq!(two.grants().facts_held().entries, parties);

    rerelate_as(
        &one,
        "adt",
        (stay, &admitted),
        &care_of(patient, ward, &interval(&day(-3), Some(&day(-1)))),
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
        !cared[0].live_on(today, jiff::Span::new()),
        "the discharge decides the next request"
    );
    assert_eq!(cared[0].until(), Some(today));
}

#[tokio::test]
async fn only_a_write_care_relationship_grant_changes_a_care_relationship() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    granted(&svc, "registry", &[capability("access_demographic", &[])]).await;
    granted(&svc, "adt", &[care_writer()]).await;
    care_provider(&svc, CARE_PROVIDER).await;
    let (ward, _) = create(&svc, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (patient, _) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let admitted = care_of(patient, ward, &interval(&day(0), Some(&day(365))));
    let refused = relate_as(&svc, "registry", &admitted)
        .await
        .expect_err("a client without the grant is refused");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let (stay, uid) = relate_as(&svc, "adt", &admitted)
        .await
        .expect("the ADT system writes the care relationship");
    let discharged = care_of(patient, ward, &interval(&day(-3), Some(&day(-1))));
    let refused = rerelate_as(&svc, "registry", (stay, &uid), &discharged)
        .await
        .expect_err("a client without the grant changes none");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    rerelate_as(&svc, "adt", (stay, &uid), &discharged)
        .await
        .expect("the ADT system ends the care relationship");
}

#[tokio::test]
async fn a_providers_group_membership_is_guarded_and_held_on_another_replica() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (replica(&db.pool()).await, replica(&db.pool()).await);
    granted(&one, "registry", &[capability("access_demographic", &[])]).await;
    granted(&one, "adt", &[care_writer()]).await;
    let owner = care_provider(&one, CARE_PROVIDER).await;
    let (region, _) = provider_team(&one, owner, &team()).await;
    let (hospital, uid) = create(&one, PartyKind::Organisation, &organisation(&[]))
        .await
        .expect("the ORGANISATION commits");
    let membership = |validity: &Value| {
        let mut relationship = care(MEMBER, hospital, region, validity);
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

/// A party of the RM class `class`, in an archetype of the deployment's own,
/// holding `relationships`.
fn own_party(class: &str, relationships: &[Value]) -> Value {
    let archetype = format!("org.example::openEHR-DEMOGRAPHIC-{class}.own.v1");
    let mut body = person("Own", relationships);
    body["_type"] = json!(class);
    body["archetype_node_id"] = json!(archetype);
    body["archetype_details"]["archetype_id"]["value"] = json!(archetype);
    body
}

/// Runs `fut` as the system client `client`, or as an administrator when
/// `None`.
async fn run_as<F: Future>(svc: &FerroEhrService, client: Option<&str>, fut: F) -> F::Output {
    match client {
        Some(client_id) => as_client(svc, client_id, fut).await,
        None => crate::policy_fixtures::as_admin(fut).await,
    }
}

/// Creates the party of `kind` and RM class `class` as `client` (an
/// administrator when `None`), then commits its next version holding the
/// relationship `code` from it to the party `target` names, over `validity`.
async fn relate_party_as(
    svc: &FerroEhrService,
    client: Option<&str>,
    (kind, class): (PartyKind, &str),
    (code, target): ((&str, &str), &str),
    validity: &Value,
) -> Result<VoId, ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(&own_party(class, &[])));
    write.lifecycle_state = Some("532".to_owned());
    let created = run_as(svc, client, async {
        Box::pin(svc.party_create(kind, write)).await
    })
    .await?;
    let vo = crate::fixtures::written_vo(&created);
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    let mut relationship = care(code, vo, vo, validity);
    relationship["source"] = party_ref(class, vo);
    relationship["target"]["id"]["value"] = json!(target);
    let mut write = WriteRequest::new(crate::typed_body::typed(&own_party(class, &[relationship])));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    run_as(svc, client, async {
        Box::pin(svc.party_update(kind, object(vo.to_string()), if_match(&uid), write)).await
    })
    .await?;
    Ok(vo)
}

/// A relationship coded with the generic HL7 v3 `RoleClass#MBR`, in a
/// client's own archetypes, is the client's own content: standalone or held
/// by any party kind, to any target, it commits with no grant beyond
/// `access_demographic` and makes no caller fact. The same relationship coded
/// `openehr_access::member_of` keeps every membership rule (#6504).
#[tokio::test]
async fn a_relationship_coded_with_the_hl7_member_code_takes_no_access_meaning() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (replica(&db.pool()).await, replica(&db.pool()).await);
    granted(&one, "registry", &[capability("access_demographic", &[])]).await;
    let (ward, _) = create(&one, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (stranger, _) = create(&one, PartyKind::Person, &person("Ms S", &[]))
        .await
        .expect("the PERSON commits");
    let (ward_id, elsewhere) = (ward.to_string(), uuid::Uuid::now_v7().to_string());
    let (ward_id, elsewhere, oid) = (ward_id.as_str(), elsewhere.as_str(), "1.2.840.113619");
    let hl7 = ("HL7_RoleClass", "MBR");
    let validity = interval(&day(-30), None);
    let mut held = Vec::new();
    for (party, target) in [
        ((PartyKind::Person, "PERSON"), ward_id),
        ((PartyKind::Person, "PERSON"), elsewhere),
        ((PartyKind::Person, "PERSON"), oid),
        ((PartyKind::Group, "GROUP"), ward_id),
        ((PartyKind::Agent, "AGENT"), ward_id),
        ((PartyKind::Organisation, "ORGANISATION"), oid),
    ] {
        let vo = relate_party_as(&one, Some("registry"), party, (hl7, target), &validity)
            .await
            .unwrap_or_else(|refused| panic!("{} to {target}: {refused:?}", party.1));
        held.push(vo);
    }
    relate_as(&one, "registry", &care(hl7, stranger, ward, &validity))
        .await
        .expect("a standalone relationship coded HL7_RoleClass::MBR commits");
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    let now = jiff::Timestamp::now();
    for vo in held {
        assert!(
            two.grants().facts_of(vo.0, now).care_teams().is_empty(),
            "no membership is read from {vo}"
        );
        assert_eq!(
            two.grants().memberships(vo.0, now),
            Vec::<uuid::Uuid>::new()
        );
    }
    for (party, target) in [
        ((PartyKind::Person, "PERSON"), oid),
        ((PartyKind::Group, "GROUP"), ward_id),
        ((PartyKind::Agent, "AGENT"), ward_id),
    ] {
        let refused = relate_party_as(&one, None, party, (MEMBER, target), &validity)
            .await
            .expect_err("a membership breaking a rule is refused");
        assert!(
            matches!(refused, ServiceError::Unprocessable { .. }),
            "{} to {target}: {refused:?}",
            party.1
        );
    }
    let mut write = WriteRequest::new(crate::typed_body::typed(&care(
        MEMBER, stranger, ward, &validity,
    )));
    write.lifecycle_state = Some("532".to_owned());
    let refused = crate::policy_fixtures::as_admin(async {
        Box::pin(one.party_relationship_create(write)).await
    })
    .await
    .expect_err("a standalone membership is refused");
    assert!(
        matches!(refused, ServiceError::Unprocessable { .. }),
        "{refused:?}"
    );
}

/// The single relationship is the stored unit (draft access control
/// package, The Relationship Store): a care relationship held in a
/// `PERSON`'s relationships is refused `422`, naming the resource that holds
/// it, and one of its own without a role or a provider, with a role of
/// another code system than `HL7_ParticipationType` while the policy binds
/// none, or with a target of no carer kind is refused `422` (#6055).
#[tokio::test]
async fn a_care_relationship_is_its_own_with_a_role_and_a_provider() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool()).await;
    granted(&svc, "adt", &[care_writer()]).await;
    care_provider(&svc, CARE_PROVIDER).await;
    let (ward, _) = create(&svc, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (patient, uid) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let inline = update_as(
        &svc,
        "adt",
        (PartyKind::Person, patient, &uid),
        &person(
            "Mr E",
            &[care_of(patient, ward, &interval(&day(0), Some(&day(365))))],
        ),
    )
    .await
    .expect_err("a care relationship in a PERSON's relationships is refused");
    assert!(
        matches!(inline, ServiceError::Unprocessable { .. })
            && format!("{inline:?}").contains("/demographic/party_relationship"),
        "{inline:?}"
    );
    let valid = care_of(patient, ward, &interval(&day(0), Some(&day(365))));
    let mut no_role = valid.clone();
    no_role["details"]["items"]
        .as_array_mut()
        .expect("items")
        .remove(0);
    let mut no_provider = valid.clone();
    no_provider["details"]["items"]
        .as_array_mut()
        .expect("items")
        .remove(1);
    let mut local_role = valid.clone();
    local_role["details"]["items"][0]["value"] = coded("local", "lead");
    let mut no_carer = valid.clone();
    no_carer["target"]["type"] = json!("AGENT");
    for (name, body) in [
        ("no care_role", no_role),
        ("no provider", no_provider),
        ("a role of another code system", local_role),
        ("a target of no carer kind", no_carer),
    ] {
        let refused = relate_as(&svc, "adt", &body)
            .await
            .expect_err("the relationship is refused");
        assert!(
            matches!(
                refused,
                ServiceError::Unprocessable { .. } | ServiceError::ValidationFailed(_)
            ),
            "{name}: {refused:?}"
        );
    }
    relate_as(&svc, "adt", &valid)
        .await
        .expect("a care relationship with its role and provider commits");
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn one_indexed_read_returns_the_facts_of_the_parties_that_changed() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    sqlx::query(
        "WITH p AS (SELECT uuidv7() AS id, n FROM generate_series(1, 20000) AS n), \
         s AS (INSERT INTO party.binding_caller_fact_party (party_id, stamped_xid) \
               SELECT id, n::text::xid8 FROM p) \
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
        .bind("19991:19991:")
        .fetch_one(&mut *conn)
        .await
        .expect("the plan");
    let text = plan.to_string();
    assert!(
        text.contains("ix_binding_caller_fact_party_stamped")
            && text.contains("ix_binding_caller_fact_party\""),
        "the facts read is not answered from the stamp and party indexes: {text}"
    );
    assert!(
        !text.contains("\"Seq Scan\""),
        "the facts read scans a table: {text}"
    );
    let read = sqlx::query(ferroehr::party_binding::grants::caller::store::FACTS)
        .bind("19991:19991:")
        .fetch_all(&mut *conn)
        .await
        .expect("the facts read");
    assert_eq!(read.len(), 10, "only the parties stamped since are read");
}

/// A store that holds its `ROLE`s and no role rows, with the grant revision
/// reset, is refilled by the next boot before it serves, with no command run
/// (#5675). The v5 party migration that made a `ROLE`'s coded name a caller
/// fact (#5621) left a store in that state; the store is built here and its
/// role rows deleted, then the revision is reset as that migration reset it.
#[tokio::test]
async fn the_boot_after_a_revision_reset_reads_the_roles_already_stored() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = replica(&pool).await;
    let (agent, _) = granted(
        &svc,
        "stored-client",
        &[capability("access_demographic", &[])],
    )
    .await;
    create(
        &svc,
        PartyKind::Role,
        &named_role(agent, &coded(ROLES, "NURSE"), None),
    )
    .await
    .expect("the ROLE commits");
    sqlx::query("DELETE FROM party.binding_caller_fact WHERE kind = 'role'")
        .execute(&pool)
        .await
        .expect("the store with no role rows");
    sqlx::query("UPDATE party.binding_grant_revision SET revision = 0, revised_at = now()")
        .execute(&pool)
        .await
        .expect("the grant revision reset");

    let upgraded = replica(&pool).await;
    assert!(
        !of_client(&upgraded, "stored-client")
            .await
            .facts()
            .roles()
            .contains(&foreign(ROLES, "NURSE")),
        "the store holds no role row until a boot"
    );
    upgraded
        .read_grants()
        .await
        .expect("the boot reads the grants");
    let facts = of_client(&upgraded, "stored-client").await.facts().clone();
    assert!(
        facts.roles().contains(&foreign(ROLES, "NURSE")),
        "the boot refilled the role: {:?}",
        facts.roles()
    );
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
    granted(&one, "adt", &[care_writer()]).await;
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
        ferroehr::privacy::PrivacyPolicy::from(&ferroehr::privacy::config::PrivacyConfig {
            subject_namespaces: vec![PIXM.to_owned()],
            ..Default::default()
        });
    let svc = replica(&db.pool())
        .await
        .with_privacy(std::sync::Arc::new(policy));
    granted(&svc, "registry", &[capability("access_demographic", &[])]).await;
    granted(&svc, "adt", &[care_writer()]).await;
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

/// Two creates of a `PERSON` declaring one record subject at once, each
/// with its records held first: one commits and the other answers `409` from
/// the deferred key at its `COMMIT`, its records discarded at once, so only
/// the winner's are held (#6257).
#[tokio::test]
async fn of_two_creates_declaring_one_record_subject_one_commits() {
    let db = testkit::db().await.expect("testkit database");
    let policy =
        ferroehr::privacy::PrivacyPolicy::from(&ferroehr::privacy::config::PrivacyConfig {
            subject_namespaces: vec![PIXM.to_owned()],
            ..Default::default()
        });
    let config = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        ..Default::default()
    };
    let (sender, handle) = ferroehr::system_log::sender::start(config, None, db.pool())
        .await
        .expect("the audit sender");
    drop(handle);
    let svc = replica(&db.pool())
        .await
        .with_privacy(std::sync::Arc::new(policy))
        .with_audit(sender);
    let before = crate::party_commit_contention::held(&db.pool()).await;
    let id = uuid::Uuid::now_v7().to_string();
    let declaring =
        |name: &str| detailed(person(name, &[]), &[record_subject(&pseudonym(PIXM, &id))]);
    let attempt = |body: Value| {
        let recorder = crate::party_commit_contention::primed();
        let kept = recorder.clone();
        let svc = &svc;
        async move {
            let outcome = Box::pin(ferroehr::db::acquire::request_scope(
                ferroehr::system_log::recorder::with_recorder(recorder, async move {
                    Box::pin(create(svc, PartyKind::Person, &body)).await
                }),
            ))
            .await;
            (outcome, kept)
        }
    };
    let (one, two) = tokio::join!(attempt(declaring("Mr E")), attempt(declaring("Mr F")));
    let mut kept = Vec::new();
    let mut refused = 0;
    for (outcome, recorder) in [one, two] {
        match outcome {
            Ok(_) => kept.extend(recorder.reserved().into_iter().map(|r| r.id)),
            Err(error) => {
                assert!(matches!(error, ServiceError::Conflict(_)), "{error:?}");
                assert!(
                    recorder.reserved().is_empty(),
                    "the loser keeps no reservation"
                );
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 1, "one pseudonym stands for one PERSON");
    assert_eq!(kept.len(), 1, "the winner holds its records");
    let now: Vec<uuid::Uuid> = crate::party_commit_contention::held(&db.pool())
        .await
        .into_iter()
        .filter(|r| !before.contains(r))
        .collect();
    assert_eq!(now, kept, "only the winner's records are held");
}

/// Whether the projection of `pool` says the subject of `ehr` is under the
/// care of `team` in the care of [`provider`] on `day`, read as a query's
/// probe reads it.
async fn projected_care(
    pool: &sqlx::PgPool,
    ehr: ferroehr::ids::EhrId,
    team: VoId,
    day: &str,
) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ehr e JOIN access_care c \
         ON c.subject_digest = e.subject_digest WHERE e.id = $1 AND c.carer_id = $2 \
         AND c.provider_issuer = $4 AND c.provider_id = $5 \
         AND (c.care_from IS NULL OR c.care_from <= $3::date) \
         AND (c.care_until IS NULL OR $3::date < c.care_until))",
    )
    .bind(ehr.0)
    .bind(team.0)
    .bind(day)
    .bind(PROVIDERS)
    .bind("hospital")
    .fetch_one(pool)
    .await
    .expect("the projection reads")
}

/// A discharge on one replica keeps the next request on another off the
/// care projection until it applies the discharge: the projection is read
/// only while it is as recent as the facts the replica holds, so an ended
/// relationship is never granted from a stale row; and an erased `PERSON`'s
/// care leaves the projection in the flow that erases it (#5699).
#[tokio::test]
async fn a_discharge_on_one_replica_is_never_read_stale_from_the_projection_on_another() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (one, two) = (replica(&pool).await, replica(&pool).await);
    granted(&one, "adt", &[care_writer()]).await;
    care_provider(&one, CARE_PROVIDER).await;
    let (ward, _) = create(&one, PartyKind::Group, &team())
        .await
        .expect("the GROUP commits");
    let (patient, _) = create(&one, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let ehr = crate::fixtures::create_ehr_for_party(&one, patient).await;
    let (stay, admitted) = relate_as(
        &one,
        "adt",
        &care_of(patient, ward, &interval(&day(-3), Some(&day(365)))),
    )
    .await
    .expect("the ADT system admits the patient");
    two.grants().refresh().await.expect("the copy reads");
    two.project_care().await.expect("the projection is written");
    assert!(
        two.care_projection_current(),
        "the projection holds the admission"
    );
    assert!(
        projected_care(&pool, ehr, ward, &day(0)).await,
        "the admitted patient is in the ward's care"
    );

    rerelate_as(
        &one,
        "adt",
        (stay, &admitted),
        &care_of(patient, ward, &interval(&day(-3), Some(&day(-1)))),
    )
    .await
    .expect("the ADT system discharges the patient");
    // The next request on the other replica reads the facts again, and its
    // projection no longer holds them.
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert!(
        !two.care_projection_current(),
        "a projection older than the facts held is read by no query"
    );
    assert!(
        projected_care(&pool, ehr, ward, &day(0)).await,
        "the stale row still says the patient is in care, which no query reads"
    );
    two.project_care()
        .await
        .expect("the discharge is projected");
    assert!(two.care_projection_current());
    assert!(
        !projected_care(&pool, ehr, ward, &day(0)).await,
        "once projected, the discharge decides"
    );

    one.physical_party_delete(patient)
        .await
        .expect("the patient is erased");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM access_care WHERE person_id = $1")
        .bind(patient.0)
        .fetch_one(&pool)
        .await
        .expect("the projection reads");
    assert_eq!(left, 0, "the erasure removes the patient's care with it");
}

/// The facts check reads the stamps from the snapshot's `xmin` upward through
/// the stamp index on the generic plan its function settles on, so twenty
/// thousand stamped parties cost a few index pages a call, never a scan of
/// every stamp (#6635).
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn the_facts_check_reads_the_stamp_index_from_xmin_on_its_generic_plan() {
    // NOTE: our own design (#6635), the measured buffers of a planned call, plus a margin.
    const BUFFERS: i64 = 8;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut conn = pool.acquire().await.expect("a connection");
    sqlx::query(
        "INSERT INTO party.binding_caller_fact_party (party_id, stamped_xid) \
         SELECT uuidv7(), (pg_current_xact_id()::text::bigint - 30000 + g)::text::xid8 \
         FROM generate_series(1, 20000) g",
    )
    .execute(&mut *conn)
    .await
    .expect("the stamps are written");
    sqlx::query("ANALYZE party.binding_caller_fact_party")
        .execute(&mut *conn)
        .await
        .expect("the stamps are analyzed");
    sqlx::query("SET plan_cache_mode = force_generic_plan")
        .execute(&mut *conn)
        .await
        .expect("the generic plan is forced");
    sqlx::query("SELECT party.facts_behind(pg_current_snapshot())")
        .execute(&mut *conn)
        .await
        .expect("the facts check plans once");
    let plan: Value = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) SELECT party.facts_behind(pg_current_snapshot())",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("the facts check runs");
    let top = &plan[0]["Plan"];
    let buffers = top["Shared Hit Blocks"].as_i64().expect("the hit buffers")
        + top["Shared Read Blocks"]
            .as_i64()
            .expect("the read buffers");
    eprintln!(
        "the facts check: {buffers} buffers, {} ms with 20,000 stamped parties",
        top["Actual Total Time"]
    );
    assert!(
        buffers <= BUFFERS,
        "the facts check reads {buffers} buffers with 20,000 stamped parties"
    );
}
