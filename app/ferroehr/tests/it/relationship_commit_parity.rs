// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows a standalone `PARTY_RELATIONSHIP` commit leaves (#6257). A
//! relationship write judges its pre-checks on one gate read and settles its
//! care fact, its kept end and its representation row in ordered statements,
//! so a create, an update, an end and a delete of each kind of relationship
//! (a care relationship to a team and to a person, a representation, and a
//! plain one) must leave the side table and the caller facts exactly as a
//! refill from the versions writes them, and the end each care relationship
//! took as the policy gives it. No openEHR spec governs the side table: our
//! own design.

use serde_json::{Value, json};

use crate::care_bounds::{care_of, interval, party, policy};
use crate::fixtures::{delete_precondition, if_match, object, written_vo};
use crate::party_binding_source::{
    LOGIN, PERSON_TEMPLATE, create, key, mapping, representation, side_rows, version,
};
use crate::policy_fixtures::{CARE_PROVIDER, care_provider, granting};
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The archetype of the representations of these tests.
const REPRESENTATION: &str = "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.custodian.v2";

/// A service over a fresh database holding the policy definitions, with the
/// binding mapping installed, so each relationship commit takes the
/// key-space gate and a representation's source rows are rebuilt.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let (db, svc) = crate::fixtures::service().await;
    crate::fixtures::open_key_spaces(&db, &[(LOGIN, false)]).await;
    let svc = granting(svc).await.with_binding_mapping(mapping());
    (db, svc)
}

/// Commits the relationship `body`; its container and version uid.
pub(crate) async fn relate(svc: &FerroEhrService, body: &Value) -> (VoId, String) {
    let created = Box::pin(svc.party_relationship_create(version(body, "532", None)))
        .await
        .expect("the relationship commits");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (written_vo(&created), uid)
}

/// Commits `body` as the next version of the relationship `vo` after `uid`;
/// the new version uid.
pub(crate) async fn amend(svc: &FerroEhrService, vo: VoId, uid: &str, body: &Value) -> String {
    Box::pin(svc.party_relationship_update(vo, if_match(uid), version(body, "532", Some(uid))))
        .await
        .expect("the relationship updates")
        .meta
        .expect("a version")
        .uid
}

/// Deletes the relationship at its version `uid`.
pub(crate) async fn remove(svc: &FerroEhrService, uid: &str) {
    Box::pin(svc.party_relationship_delete(
        object(uid),
        delete_precondition(uid, None),
        WriteRequest::new(()),
    ))
    .await
    .expect("the relationship deletes");
}

/// A plain relationship of its own from the `PERSON` `source` to the
/// `PERSON` `target`, of the local type `code`.
fn plain(source: VoId, target: VoId, code: &str) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.colleague.v1",
        "name": { "_type": "DV_CODED_TEXT", "value": "relationship",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code } },
        "source": crate::party_binding_source::party_ref("PERSON", source),
        "target": crate::party_binding_source::party_ref("PERSON", target)
    })
}

/// The end kept for every care relationship, by container, with the policy
/// version that set it.
async fn kept_ends(pool: &sqlx::PgPool) -> Vec<(uuid::Uuid, String, String, String)> {
    sqlx::query_as(
        "SELECT vo_id, version_uid, effective_until::text, policy_version \
         FROM party.binding_care_validity ORDER BY vo_id",
    )
    .fetch_all(pool)
    .await
    .expect("the kept ends read")
}

/// A create, an update, an end and a delete of each kind of relationship
/// leave the side table and the caller facts as a refill writes them, and
/// each care relationship the end its last version takes.
#[tokio::test]
async fn every_relationships_commits_leave_the_rows_a_refill_writes() {
    let (db, svc) = service().await;
    let policy_version = svc
        .put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the policy")
        .version_uid;
    let person = |id: &str| crate::party_binding_source::person(PERSON_TEMPLATE, &[key(id)]);
    let (patient, _) = create(
        &svc,
        PartyKind::Person,
        &person("5e6f7a8b-9c0d-4e1f-8a2b-4c5d6e7f8a9b"),
    )
    .await;
    let (guardian, _) = create(
        &svc,
        PartyKind::Person,
        &person("6f7a8b9c-0d1e-4f2a-9b3c-5d6e7f8a9b0c"),
    )
    .await;
    let (clinician, _) = create(
        &svc,
        PartyKind::Person,
        &person("7a8b9c0d-1e2f-4a3b-8c4d-6e7f8a9b0c1d"),
    )
    .await;
    care_provider(&svc, CARE_PROVIDER).await;
    let (team, _) = create(&svc, PartyKind::Group, &party("GROUP", "Cardiology")).await;
    let team = team.to_string();
    let clinician_id = clinician.to_string();

    // A care relationship to a team: created open, so it takes the policy's
    // end; updated to another role with an end of its own; ended; deleted.
    let open = care_of(
        patient,
        ("GROUP", &team),
        "ATND",
        &interval("2026-11-01", None),
    );
    let (stay, uid) = relate(&svc, &open).await;
    let ends = kept_ends(&db.pool()).await;
    assert_eq!(
        ends.iter()
            .map(|(vo, _, until, by)| (*vo, until.as_str(), by.as_str()))
            .collect::<Vec<_>>(),
        [(stay.0, "2026-12-01", policy_version.as_str())],
        "an open stay takes its start plus P30D"
    );
    let uid = amend(
        &svc,
        stay,
        &uid,
        &care_of(
            patient,
            ("GROUP", &team),
            "PPRF",
            &interval("2026-11-01", Some("2026-12-15")),
        ),
    )
    .await;
    assert!(
        kept_ends(&db.pool()).await.is_empty(),
        "an end of its own drops the kept one"
    );
    let uid = amend(
        &svc,
        stay,
        &uid,
        &care_of(
            patient,
            ("GROUP", &team),
            "ATND",
            &interval("2026-11-01", Some("2026-11-20")),
        ),
    )
    .await;
    remove(&svc, &uid).await;

    // A care relationship to a person, created open and left live.
    let (direct, direct_uid) = relate(
        &svc,
        &care_of(
            patient,
            ("PERSON", &clinician_id),
            "ATND",
            &interval("2026-11-05", None),
        ),
    )
    .await;
    // A care relationship to the team, created with an end, then opened, left live.
    let (second, second_uid) = relate(
        &svc,
        &care_of(
            patient,
            ("GROUP", &team),
            "ATND",
            &interval("2026-11-02", Some("2026-11-30")),
        ),
    )
    .await;
    let second_uid = amend(
        &svc,
        second,
        &second_uid,
        &care_of(
            patient,
            ("GROUP", &team),
            "ATND",
            &interval("2026-11-03", None),
        ),
    )
    .await;

    // A representation: created, given a validity, ended, deleted; another left live.
    let guardianship = |validity: Option<Value>| {
        representation(REPRESENTATION, (guardian, patient), "GUARD", validity)
    };
    let (custody, uid) = relate(&svc, &guardianship(None)).await;
    let uid = amend(
        &svc,
        custody,
        &uid,
        &guardianship(Some(crate::party_binding_source::interval(
            Some("2026-01-01"),
            None,
        ))),
    )
    .await;
    let uid = amend(
        &svc,
        custody,
        &uid,
        &guardianship(Some(crate::party_binding_source::interval(
            Some("2026-01-01"),
            Some("2026-06-30"),
        ))),
    )
    .await;
    remove(&svc, &uid).await;
    let (live_custody, _) = relate(&svc, &guardianship(None)).await;

    // A plain relationship: created, renamed, deleted; another left live.
    let (colleague, uid) = relate(&svc, &plain(patient, clinician, "x")).await;
    let uid = amend(&svc, colleague, &uid, &plain(patient, clinician, "y")).await;
    remove(&svc, &uid).await;
    relate(&svc, &plain(clinician, patient, "x")).await;

    let pool = db.pool();
    let ends = kept_ends(&pool).await;
    let mut expected = vec![
        (direct.0, "2026-12-05".to_owned()),
        (second.0, "2026-12-03".to_owned()),
    ];
    expected.sort_unstable();
    assert_eq!(
        ends.iter()
            .map(|(vo, _, until, _)| (*vo, until.clone()))
            .collect::<Vec<_>>(),
        expected,
        "each open care relationship keeps the end its last version takes"
    );
    let versions: Vec<&str> = ends.iter().map(|(_, uid, ..)| uid.as_str()).collect();
    assert!(
        versions.contains(&direct_uid.as_str()) && versions.contains(&second_uid.as_str()),
        "each end names the version it was kept for: {versions:?}"
    );
    let before = side_rows(&pool).await;
    let held = |table: &str| {
        before
            .iter()
            .find(|(name, _)| *name == table)
            .map_or(0, |(_, rows)| rows.len())
    };
    assert!(
        held("binding_caller_fact") >= 2 && held("binding_representation") == 1,
        "the commits wrote the care facts and the live representation: {before:#?}"
    );
    assert!(
        before.iter().any(|(_, rows)| {
            rows.iter()
                .any(|row| row.contains(&live_custody.to_string()))
        }),
        "the live representation holds its row"
    );
    svc.refill_binding_table().await.expect("the refill runs");
    assert_eq!(side_rows(&pool).await, before);
    assert_eq!(kept_ends(&pool).await, ends, "a refill moves no kept end");
}
