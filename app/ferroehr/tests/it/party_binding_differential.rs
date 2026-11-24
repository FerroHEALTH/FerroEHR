// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding copy against the store (#6403): a seeded sequence of party
//! writes, of keys, lifecycles, memberships, groups, representations,
//! professional roles and deletes, on two replicas of one store, after each
//! of which every login's lookup on either replica equals a fresh read of
//! the store, both before the replicas hear the write and once they apply
//! its change set. No openEHR spec governs resolving a login to a party: our
//! own design.

use std::sync::Arc;
use std::time::Duration;

use ferrohealth_smart::party::source::Lookup;
use rand::rngs::StdRng;
use rand::{Rng as _, SeedableRng as _};
use serde_json::{Value, json};

use crate::fixtures::{delete_precondition, if_match, object, written_vo};
use crate::party_binding_source::{
    GROUP_TEMPLATE, GROUPS, GUID, LOGIN, MEMBERSHIP, NAMESPACE, PERSON_TEMPLATE, REPRESENTATION,
    ROLE_TEMPLATE, TODAY, group, key, mapping, party_ref, person, representation, role, version,
};
use ferroehr::db::domain::DomainPools;
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The writes of the sequence.
const STEPS: usize = 2000;

/// The seed of the sequence.
const SEED: u64 = 6403;

/// The login keys the sequence binds, each looked up after every write.
const KEYS: [&str; 5] = [
    "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
    "1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e",
    "2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f",
    "3d4e5f6a-7b8c-4d9e-8f1a-2b3c4d5e6f7a",
    "4e5f6a7b-8c9d-4e0f-9a1b-3c4d5e6f7a8b",
];

/// The external ids the groups take.
const TEAMS: [&str; 3] = ["ward", "theatre", "clinic"];

/// How long a replica may take to apply a change set.
const HEARD: Duration = Duration::from_secs(10);

/// A `PERSON` the sequence wrote.
struct Person {
    vo: VoId,
    uid: String,
    key: Option<usize>,
    lifecycle: &'static str,
    groups: Vec<VoId>,
    roles: Vec<VoId>,
}

/// A `GROUP` the sequence wrote.
struct Group {
    vo: VoId,
    uid: String,
    team: usize,
}

/// What the sequence wrote that is still live.
#[derive(Default)]
struct Model {
    persons: Vec<Person>,
    groups: Vec<Group>,
}

/// The body of `person`.
fn person_body(person_state: &Person) -> Value {
    let identifiers: Vec<Value> = person_state.key.map(|k| key(KEYS[k])).into_iter().collect();
    let mut body = person(PERSON_TEMPLATE, &identifiers);
    if !person_state.groups.is_empty() {
        body["relationships"] = Value::Array(
            person_state
                .groups
                .iter()
                .map(|g| {
                    json!({
                        "_type": "PARTY_RELATIONSHIP",
                        "archetype_node_id": MEMBERSHIP,
                        "name": { "_type": "DV_CODED_TEXT", "value": "relationship",
                            "defining_code": { "_type": "CODE_PHRASE",
                                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                                "code_string": "at0001" } },
                        "source": party_ref("PERSON", person_state.vo),
                        "target": party_ref("GROUP", *g)
                    })
                })
                .collect(),
        );
    }
    if !person_state.roles.is_empty() {
        body["roles"] = Value::Array(
            person_state
                .roles
                .iter()
                .map(|r| party_ref("ROLE", *r))
                .collect(),
        );
    }
    body
}

/// The body of `team` named by its external id.
fn group_body(team: &Group) -> Value {
    group(GROUP_TEMPLATE, TEAMS[team.team])
}

/// The version uid a write answered.
fn uid_of(response: &ferroehr::service::response::ServiceResponse) -> String {
    response.meta.as_ref().expect("a version").uid.clone()
}

/// Commits the next version of a party of `kind`, `vo` after `uid`; its new
/// uid, or `None` when the write is refused.
async fn next(
    svc: &FerroEhrService,
    kind: PartyKind,
    (vo, uid): (VoId, &str),
    body: &Value,
    lifecycle: &str,
) -> Option<String> {
    Box::pin(svc.party_update(
        kind,
        object(vo.to_string()),
        if_match(uid),
        version(body, lifecycle, Some(uid)),
    ))
    .await
    .ok()
    .map(|response| uid_of(&response))
}

/// One random write of the sequence on `svc`.
async fn write(svc: &FerroEhrService, model: &mut Model, rng: &mut StdRng) {
    let pick = |rng: &mut StdRng, n: usize| (n > 0).then(|| rng.gen_range(0..n));
    let lifecycle = |rng: &mut StdRng| if rng.gen_bool(0.8) { "532" } else { "553" };
    match rng.gen_range(0..8) {
        0 | 1 => {
            let mut fresh = Person {
                vo: VoId(uuid::Uuid::nil()),
                uid: String::new(),
                key: rng.gen_bool(0.8).then(|| rng.gen_range(0..KEYS.len())),
                lifecycle: "532",
                groups: Vec::new(),
                roles: Vec::new(),
            };
            let body = person_body(&fresh);
            if let Ok(created) =
                Box::pin(svc.party_create(PartyKind::Person, version(&body, "532", None))).await
            {
                fresh.vo = written_vo(&created);
                fresh.uid = uid_of(&created);
                model.persons.push(fresh);
            }
        }
        2 => {
            let Some(at) = pick(rng, model.persons.len()) else {
                return;
            };
            let groups: Vec<VoId> = model
                .groups
                .iter()
                .filter(|_| rng.gen_bool(0.5))
                .map(|g| g.vo)
                .collect();
            let key = match rng.gen_range(0..3) {
                0 => None,
                1 => Some(rng.gen_range(0..KEYS.len())),
                _ => model.persons[at].key,
            };
            let state = lifecycle(rng);
            let candidate = Person {
                vo: model.persons[at].vo,
                uid: model.persons[at].uid.clone(),
                key,
                lifecycle: state,
                groups,
                roles: model.persons[at].roles.clone(),
            };
            let current = (candidate.vo, candidate.uid.as_str());
            if let Some(uid) = next(
                svc,
                PartyKind::Person,
                current,
                &person_body(&candidate),
                state,
            )
            .await
            {
                model.persons[at] = Person { uid, ..candidate };
            }
        }
        3 => {
            let Some(at) = pick(rng, model.persons.len()) else {
                return;
            };
            let uid = model.persons[at].uid.clone();
            if Box::pin(svc.party_delete(
                PartyKind::Person,
                object(&uid),
                delete_precondition(&uid, None),
                WriteRequest::new(()),
            ))
            .await
            .is_ok()
            {
                model.persons.remove(at);
            }
        }
        4 => {
            let team = rng.gen_range(0..TEAMS.len());
            let mut fresh = Group {
                vo: VoId(uuid::Uuid::nil()),
                uid: String::new(),
                team,
            };
            if let Ok(created) = Box::pin(
                svc.party_create(PartyKind::Group, version(&group_body(&fresh), "532", None)),
            )
            .await
            {
                fresh.vo = written_vo(&created);
                fresh.uid = uid_of(&created);
                model.groups.push(fresh);
            }
        }
        5 => {
            let Some(at) = pick(rng, model.groups.len()) else {
                return;
            };
            let state = ["532", "553", "800"][rng.gen_range(0..3)];
            let team = rng.gen_range(0..TEAMS.len());
            let current = &model.groups[at];
            let candidate = Group {
                vo: current.vo,
                uid: current.uid.clone(),
                team,
            };
            let at_uid = (candidate.vo, candidate.uid.as_str());
            if let Some(uid) = next(
                svc,
                PartyKind::Group,
                at_uid,
                &group_body(&candidate),
                state,
            )
            .await
            {
                model.groups[at] = Group { uid, ..candidate };
            }
        }
        6 => {
            let (Some(a), Some(b)) = (
                pick(rng, model.persons.len()),
                pick(rng, model.persons.len()),
            ) else {
                return;
            };
            let pair = (model.persons[a].vo, model.persons[b].vo);
            let body = representation(REPRESENTATION, pair, "GUARD", None);
            drop(
                Box::pin(crate::fixtures::create_relationship(
                    svc,
                    version(&body, "532", None),
                ))
                .await,
            );
        }
        _ => {
            let Some(at) = pick(rng, model.persons.len()) else {
                return;
            };
            let performer = model.persons[at].vo;
            let Ok(created) = Box::pin(svc.party_create(
                PartyKind::Role,
                version(&role(ROLE_TEMPLATE, performer, None), "532", None),
            ))
            .await
            else {
                return;
            };
            let mut candidate = Person {
                vo: performer,
                uid: model.persons[at].uid.clone(),
                key: model.persons[at].key,
                lifecycle: model.persons[at].lifecycle,
                groups: model.persons[at].groups.clone(),
                roles: model.persons[at].roles.clone(),
            };
            candidate.roles.push(written_vo(&created));
            let state = candidate.lifecycle;
            let current = (candidate.vo, candidate.uid.as_str());
            if let Some(uid) = next(
                svc,
                PartyKind::Person,
                current,
                &person_body(&candidate),
                state,
            )
            .await
            {
                model.persons[at] = Person { uid, ..candidate };
            }
        }
    }
}

/// What `svc` answers for the login key `k`, with the memberships under the
/// group issuer when `groups`.
async fn lookup(svc: &FerroEhrService, k: &str, groups: bool) -> Lookup {
    svc.party_source(NAMESPACE)
        .read(LOGIN, GUID, k, groups.then_some(GROUPS), TODAY)
        .await
        .expect("the source answers")
}

/// Every login's lookup on each replica equals the store's.
async fn agree(replicas: &[&FerroEhrService; 2], store: &FerroEhrService, step: usize, when: &str) {
    for k in KEYS {
        for groups in [false, true] {
            let fresh = lookup(store, k, groups).await;
            for (n, replica) in replicas.iter().enumerate() {
                assert_eq!(
                    lookup(replica, k, groups).await,
                    fresh,
                    "step {step} {when}: replica {n}, key {k}, memberships {groups}"
                );
            }
        }
    }
}

/// The store's binding revision.
async fn revision(db: &testkit::TestDb) -> i64 {
    sqlx::query_scalar("SELECT binding_revision FROM party.binding_grant_revision")
        .fetch_one(&db.pool())
        .await
        .expect("the revision reads")
}

/// Waits until each replica applied the change set of the store's binding
/// revision, once a write moved it past `start`.
async fn applied(db: &testkit::TestDb, replicas: &[&FerroEhrService; 2], start: i64) {
    let revision = revision(db).await;
    if revision == start {
        return;
    }
    let deadline = tokio::time::Instant::now() + HEARD;
    for replica in replicas {
        while replica
            .bindings()
            .confirmed()
            .is_none_or(|at| at < revision)
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the replica applies revision {revision}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

/// Every lookup either replica answers equals the store's, through a seeded
/// sequence of party writes on both.
#[tokio::test]
async fn every_held_lookup_equals_the_store_through_a_sequence_of_party_writes() {
    let (db, first) = crate::fixtures::service().await;
    crate::fixtures::open_key_spaces(&db, &[LOGIN]).await;
    let first = first.with_binding_mapping(mapping());
    let second =
        FerroEhrService::new(&DomainPools::from_shared(&db.pool())).with_binding_mapping(mapping());
    let store =
        FerroEhrService::new(&DomainPools::from_shared(&db.pool())).with_binding_mapping(mapping());
    for replica in [&first, &second] {
        let listened = replica.bindings().listened();
        crate::policy_fixtures::listen(&db, &[Arc::clone(listened)]);
        for _ in 0..250 {
            if listened.is_listening() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(listened.is_listening(), "the binding listener connects");
    }
    let start = revision(&db).await;
    let replicas = [&first, &second];
    let mut model = Model::default();
    let mut rng = StdRng::seed_from_u64(SEED);
    for step in 0..STEPS {
        let writer = replicas[rng.gen_range(0..2)];
        write(writer, &mut model, &mut rng).await;
        agree(&replicas, &store, step, "before the change set").await;
        applied(&db, &replicas, start).await;
        agree(&replicas, &store, step, "after the change set").await;
    }
    for replica in replicas {
        let (lookups, _) = replica.bindings().held();
        assert!(
            lookups.entries > 0,
            "the replicas hold lookups: {lookups:?}"
        );
    }
}
