// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who writes a care relationship or a membership (#6073; draft access
//! control decision procedure, Writing Relationships): a
//! `write_care_relationship` grant writes for the providers and target kinds
//! its elements name, a membership for the provider the target `GROUP`
//! belongs to, and an administrator writes every one; a caller is never
//! admitted on a relationship it wrote naming itself. Our own design over
//! that procedure.

use std::future::Future;

use serde_json::{Value, json};

use crate::fixtures::{if_match, object};
use crate::policy_fixtures::{
    CARE_PROVIDER, ISSUER, as_admin, capability, care_provider, care_writer, create, granted,
    granting, of_client, provider_team, scope,
};
use ferroehr::access_eval::procedure::Policy;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::caller::care_of::ProviderKey;
use ferroehr::party_binding::grants::capabilities::with_capabilities;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::storage::access_projection::EhrSubject;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

fn party_ref(kind: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

/// A party of `class` and `archetype` named `name`.
fn party(class: &str, archetype: &str, name: &str) -> Value {
    json!({
        "_type": class, "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text(name),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text(name) }] } }]
    })
}

fn person(name: &str) -> Value {
    party("PERSON", "openEHR-DEMOGRAPHIC-PERSON.person.v1", name)
}

fn ward(name: &str) -> Value {
    party("GROUP", "openEHR-DEMOGRAPHIC-GROUP.ward.v1", name)
}

/// The `PERSON` `name`, `member` itself, a member of `team` from 2026-10-01.
fn rostered(name: &str, member: VoId, team: VoId) -> Value {
    let mut body = person(name);
    body["relationships"] = json!([{
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.rostering.v1",
        "name": text("Rostering"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("kind"),
                        "value": coded("HL7_RoleClass", "MBR") }] },
        "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": "2026-10-01" },
            "lower_unbounded": false, "upper_unbounded": true, "lower_included": true },
        "source": party_ref("PERSON", member),
        "target": party_ref("GROUP", team)
    }]);
    body
}

/// `source` under the care of the `GROUP` `team` in the role `PART`, in the
/// care of [`CARE_PROVIDER`], over a year from 2026-11-01.
fn care_of(source: VoId, team: VoId) -> Value {
    let (issuer, id) = CARE_PROVIDER;
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
                  "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id,
                             "type": "ORGANISATION" } }] },
        "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": "2026-11-01" },
            "upper": { "_type": "DV_DATE", "value": "2027-11-01" },
            "lower_unbounded": false, "upper_unbounded": false,
            "lower_included": true, "upper_included": false },
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// Runs `fut` as a request of the system client `client_id`, whose `AGENT`
/// is `agent`, inside its grants on `svc`: its commits name `agent` as the
/// committer's `external_ref`.
async fn as_client<F: Future>(
    svc: &FerroEhrService,
    (client_id, agent): (&str, VoId),
    fut: F,
) -> F::Output {
    let caller = CommitterIdentity {
        subject: client_id.to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        local_issuer: ferroehr::service::DEFAULT_SYSTEM_ID.to_owned(),
        client: Some(client_id.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: true,
        agent: Some(agent.0),
        external: None,
    };
    let granted = of_client(svc, client_id).await;
    with_capabilities(granted, with_committer(Some(caller), Box::pin(fut))).await
}

/// Creates the care relationship `body` as the client of `client`.
async fn relate(
    svc: &FerroEhrService,
    client: (&str, VoId),
    body: &Value,
) -> Result<VoId, ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let created = as_client(svc, client, async {
        Box::pin(svc.party_relationship_create(write)).await
    })
    .await?;
    Ok(crate::fixtures::written_vo(&created))
}

/// Commits `body` as the next version of the `PERSON` `vo` after `uid` as the
/// client of `client`, or as an administrator when `client` is `None`; the
/// new version's uid.
async fn update_person(
    svc: &FerroEhrService,
    client: Option<(&str, VoId)>,
    (vo, uid): (VoId, &str),
    body: &Value,
) -> Result<String, ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    let update = async {
        Box::pin(svc.party_update(
            PartyKind::Person,
            object(vo.to_string()),
            if_match(uid),
            write,
        ))
        .await
    };
    let updated = match client {
        Some(client) => as_client(svc, client, update).await?,
        None => as_admin(update).await?,
    };
    Ok(updated.meta.expect("a version").uid)
}

async fn replica(db: &testkit::TestDb) -> FerroEhrService {
    granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await
}

fn forbidden(result: Result<impl std::fmt::Debug, ServiceError>, why: &str) {
    match result {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("{why}: {other:?}"),
    }
}

#[tokio::test]
async fn a_care_relationship_outside_the_grants_providers_or_target_kinds_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db).await;
    let (adt, _) = granted(&svc, "adt", &[care_writer()]).await;
    let elsewhere = capability(
        "write_care_relationship",
        &[
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0031",
                "name": coded(TERMINOLOGY, "provider"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": CARE_PROVIDER.0,
                           "id": "clinic" } }),
            scope("target_kind", "GROUP"),
        ],
    );
    let (clinic, _) = granted(&svc, "clinic-adt", &[elsewhere]).await;
    let persons_only = capability(
        "write_care_relationship",
        &[
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0031",
                "name": coded(TERMINOLOGY, "provider"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": CARE_PROVIDER.0,
                           "id": CARE_PROVIDER.1 } }),
            scope("target_kind", "PERSON"),
        ],
    );
    let (rosters, _) = granted(&svc, "person-adt", &[persons_only]).await;
    let (bare, _) = granted(
        &svc,
        "bare-adt",
        &[capability("write_care_relationship", &[])],
    )
    .await;
    let (team, _) = create(&svc, PartyKind::Group, &ward("Cardiology"))
        .await
        .expect("the GROUP commits");
    let (patient, _) = create(&svc, PartyKind::Person, &person("Mr E"))
        .await
        .expect("the patient commits");
    let stay = care_of(patient, team);
    forbidden(
        relate(&svc, ("clinic-adt", clinic), &stay).await,
        "a grant of another provider",
    );
    forbidden(
        relate(&svc, ("person-adt", rosters), &stay).await,
        "a grant of another target kind",
    );
    forbidden(
        relate(&svc, ("bare-adt", bare), &stay).await,
        "a grant naming neither writes none",
    );
    relate(&svc, ("adt", adt), &stay)
        .await
        .expect("a grant naming the provider and the target kind writes it");
}

#[tokio::test]
async fn a_membership_is_written_for_the_provider_its_group_belongs_to() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db).await;
    let (adt, _) = granted(&svc, "adt", &[care_writer()]).await;
    let hospital = care_provider(&svc, CARE_PROVIDER).await;
    let clinic = care_provider(&svc, (CARE_PROVIDER.0, "clinic")).await;
    let (own, _) = provider_team(&svc, hospital, &ward("Cardiology")).await;
    let (other, _) = provider_team(&svc, clinic, &ward("Clinic ward")).await;
    let (region, _) = create(&svc, PartyKind::Group, &ward("Region"))
        .await
        .expect("the GROUP commits");
    let (doctor, uid) = create(&svc, PartyKind::Person, &person("Dr D"))
        .await
        .expect("the professional commits");
    let client = Some(("adt", adt));
    forbidden(
        update_person(
            &svc,
            client,
            (doctor, &uid),
            &rostered("Dr D", doctor, other),
        )
        .await,
        "a GROUP of another provider",
    );
    forbidden(
        update_person(
            &svc,
            client,
            (doctor, &uid),
            &rostered("Dr D", doctor, region),
        )
        .await,
        "a GROUP that belongs to no provider",
    );
    let uid = update_person(&svc, client, (doctor, &uid), &rostered("Dr D", doctor, own))
        .await
        .expect("the grant writes a membership of its provider's GROUP");
    update_person(
        &svc,
        None,
        (doctor, &uid),
        &rostered("Dr D", doctor, region),
    )
    .await
    .expect("an administrator writes a membership of a GROUP of no provider");
}

#[tokio::test]
async fn a_relationship_never_admits_the_principal_that_wrote_it() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db).await;
    let (adt, _) = granted(&svc, "adt", &[care_writer()]).await;
    let (team, _) = create(&svc, PartyKind::Group, &ward("Cardiology"))
        .await
        .expect("the GROUP commits");
    let (patient, _) = create(&svc, PartyKind::Person, &person("Mr E"))
        .await
        .expect("the patient commits");
    relate(&svc, ("adt", adt), &care_of(patient, team))
        .await
        .expect("the ADT system writes the care relationship");
    svc.grants().refresh().await.expect("the copy reads");
    let held = svc
        .grants()
        .under_care_of(patient.0)
        .await
        .expect("the care reads");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].writer(), Some(adt.0), "the fact names its writer");
    let provider = [ProviderKey::new(CARE_PROVIDER.0, CARE_PROVIDER.1)];
    let day = "2026-11-20".parse().expect("a date");
    let cared = |person: Option<uuid::Uuid>| {
        let svc = &svc;
        let provider = &provider;
        async move {
            let mut facts = ferroehr::party_binding::grants::caller::facts::CallerFacts::new(
                Vec::new(),
                vec![ferroehr::party_binding::grants::caller::CareTeam::spanning(
                    team.0, None, None,
                )],
            )
            .with_providers(provider.iter().cloned());
            if let Some(person) = person {
                facts = facts.with_party(person);
            }
            let principals = facts.principals();
            svc.grants()
                .cared_for_by(&facts, &principals, day, &Policy::default())
                .await
                .expect("the census reads")
                .into_iter()
                .map(|cared| cared.subject)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        cared(None).await,
        vec![EhrSubject::new(
            &*svc.grants().namespace(),
            patient.0.to_string()
        )],
        "a caller of the team that did not write it"
    );
    assert_eq!(
        cared(Some(adt.0)).await,
        Vec::<EhrSubject>::new(),
        "a caller among whose principals is its writer"
    );
}
